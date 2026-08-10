//! ZIP/TAR preparation into an exclusive guarded scratch tree.
//!
//! Owns archive-extraction semantics: entry paths, entry types, decoded and materialized limits,
//! and path-escape guards. Preparation writes into caller-provided private custody and returns
//! evidence; the final destination is never touched. Feature-gated on `zip`/`tar`
//! (with gzip/xz/zstd codecs composing tar).
use std::collections::BTreeMap;
#[cfg(windows)]
use std::ffi::OsStr;
use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Component, Path, PathBuf};

/// Errors produced by archive preparation and extraction.
#[non_exhaustive]
#[derive(Debug)]
pub enum ArchiveError {
    ArchiveRequiresFile(PathBuf),
    ArchiveInvalidPath(String),
    ArchiveLimitExceeded {
        limit: &'static str,
        actual: u64,
        max: u64,
    },
    ArchiveSizeMismatch {
        path: PathBuf,
        declared: u64,
        observed: u64,
    },
    ArchivePathConflict(PathBuf),
    /// Extraction failed and resetting the exclusive workspace also failed.
    ///
    /// `extraction` is the primary source. `cleanup` remains available for callers that need to
    /// detect a contaminated workspace.
    ArchiveCleanupFailed {
        workspace: PathBuf,
        extraction: Box<ArchiveError>,
        cleanup: Box<ArchiveError>,
    },
    UnsupportedArchiveEntry(PathBuf),
    InvalidPreparation(String),
    Io {
        action: &'static str,
        path: PathBuf,
        source: io::Error,
    },
}

impl ArchiveError {
    fn io(action: &'static str, path: impl AsRef<Path>, source: io::Error) -> Self {
        Self::Io {
            action,
            path: path.as_ref().to_path_buf(),
            source,
        }
    }
}

impl std::fmt::Display for ArchiveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ArchiveRequiresFile(path) => {
                write!(f, "archive preparation requires a file: {}", path.display())
            }
            Self::ArchiveInvalidPath(path) => write!(f, "archive entry path is invalid: {path}"),
            Self::ArchiveLimitExceeded { limit, actual, max } => {
                write!(f, "archive {limit} limit exceeded: {actual} > {max}")
            }
            Self::ArchiveSizeMismatch {
                path,
                declared,
                observed,
            } => write!(
                f,
                "archive entry size mismatch for {}: declared {declared}, observed {observed}",
                path.display()
            ),
            Self::ArchivePathConflict(path) => {
                write!(f, "archive entries conflict at path: {}", path.display())
            }
            Self::ArchiveCleanupFailed { workspace, .. } => write!(
                f,
                "archive extraction and cleanup both failed for {}",
                workspace.display()
            ),
            Self::UnsupportedArchiveEntry(path) => {
                write!(f, "archive entry is unsupported: {}", path.display())
            }
            Self::InvalidPreparation(message) => write!(f, "invalid preparation: {message}"),
            Self::Io {
                action,
                path,
                source,
            } => write!(f, "failed to {action} {}: {source}", path.display()),
        }
    }
}

impl std::error::Error for ArchiveError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::ArchiveCleanupFailed { extraction, .. } => Some(extraction.as_ref()),
            Self::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ArchiveEntryKind {
    File,
    Directory,
}

#[derive(Default)]
struct ArchivePathIndex {
    nodes: BTreeMap<PathBuf, ArchivePathState>,
}

#[derive(Default)]
struct ArchivePathState {
    entry: Option<ArchiveEntryKind>,
    has_descendant: bool,
}

impl ArchivePathIndex {
    fn admit(&mut self, path: &Path, kind: ArchiveEntryKind) -> Result<(), ArchiveError> {
        let key = archive_collision_key(path);
        if self.nodes.get(&key).and_then(|state| state.entry).is_some() {
            return Err(ArchiveError::ArchivePathConflict(path.to_path_buf()));
        }
        for ancestor in key
            .ancestors()
            .skip(1)
            .filter(|ancestor| !ancestor.as_os_str().is_empty())
        {
            if self.nodes.get(ancestor).and_then(|state| state.entry)
                == Some(ArchiveEntryKind::File)
            {
                return Err(ArchiveError::ArchivePathConflict(path.to_path_buf()));
            }
        }
        if kind == ArchiveEntryKind::File
            && self
                .nodes
                .get(&key)
                .is_some_and(|state| state.has_descendant)
        {
            return Err(ArchiveError::ArchivePathConflict(path.to_path_buf()));
        }
        for ancestor in key
            .ancestors()
            .skip(1)
            .filter(|ancestor| !ancestor.as_os_str().is_empty())
        {
            self.nodes
                .entry(ancestor.to_path_buf())
                .or_default()
                .has_descendant = true;
        }
        self.nodes.entry(key).or_default().entry = Some(kind);
        Ok(())
    }
}

struct ArchiveCopyLimits {
    observed_total: u64,
    max_entry: Option<u64>,
    max_total: Option<u64>,
    max_decoded: Option<u64>,
}

#[derive(Debug)]
struct DecodedLimitExceeded {
    actual: u64,
    max: u64,
}

impl std::fmt::Display for DecodedLimitExceeded {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "decoded archive byte limit exceeded")
    }
}

impl std::error::Error for DecodedLimitExceeded {}

#[cfg(feature = "tar")]
struct DecodedLimitReader<R> {
    inner: R,
    observed: u64,
    max: Option<u64>,
}

#[cfg(feature = "tar")]
impl<R> DecodedLimitReader<R> {
    fn new(inner: R, max: Option<u64>) -> Self {
        Self {
            inner,
            observed: 0,
            max,
        }
    }
}

#[cfg(feature = "tar")]
impl<R: Read> Read for DecodedLimitReader<R> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let capped_len = match self.max {
            Some(max) => {
                let remaining = max.saturating_sub(self.observed).saturating_add(1);
                buffer
                    .len()
                    .min(usize::try_from(remaining).unwrap_or(usize::MAX))
            }
            None => buffer.len(),
        };
        let read = self.inner.read(&mut buffer[..capped_len])?;
        self.observed = self.observed.saturating_add(read as u64);
        if let Some(max) = self.max
            && self.observed > max
        {
            return Err(io::Error::other(DecodedLimitExceeded {
                actual: self.observed,
                max,
            }));
        }
        Ok(read)
    }
}

/// Resource and path policy for archive preparation.
///
/// Construct this evolving policy with [`ArchivePolicy::new`] and its builder methods rather than
/// a struct literal.
///
/// ```compile_fail
/// use pulith::archive::ArchivePolicy;
///
/// let _ = ArchivePolicy {
///     strip_components: 0,
///     max_entries: None,
///     max_entry_bytes: None,
///     max_total_bytes: None,
///     max_decoded_bytes: None,
/// };
/// ```
#[non_exhaustive]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ArchivePolicy {
    pub strip_components: usize,
    pub max_entries: Option<u64>,
    pub max_entry_bytes: Option<u64>,
    pub max_total_bytes: Option<u64>,
    pub max_decoded_bytes: Option<u64>,
}

impl ArchivePolicy {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn strip_components(mut self, strip_components: usize) -> Self {
        self.strip_components = strip_components;
        self
    }

    pub fn max_entries(mut self, max_entries: u64) -> Self {
        self.max_entries = Some(max_entries);
        self
    }

    pub fn max_entry_bytes(mut self, max_entry_bytes: u64) -> Self {
        self.max_entry_bytes = Some(max_entry_bytes);
        self
    }

    pub fn max_total_bytes(mut self, max_total_bytes: u64) -> Self {
        self.max_total_bytes = Some(max_total_bytes);
        self
    }

    pub fn max_decoded_bytes(mut self, max_decoded_bytes: u64) -> Self {
        self.max_decoded_bytes = Some(max_decoded_bytes);
        self
    }
}

impl Default for ArchivePolicy {
    fn default() -> Self {
        Self {
            strip_components: 0,
            max_entries: Some(16_384),
            max_entry_bytes: Some(4 * 1024 * 1024 * 1024),
            max_total_bytes: Some(4 * 1024 * 1024 * 1024),
            max_decoded_bytes: Some(8 * 1024 * 1024 * 1024),
        }
    }
}

/// The concrete archive format of a prepared tree, as data.
///
/// `ArchiveKind` is the canonical data representation of an archive format: [`ArchiveKind::sniff`]
/// detects it from magic bytes, `prepare` dispatches on it, and [`ArchiveEvidence`] attests it.
/// Cargo features select compiled decoders; this value selects one of them.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ArchiveKind {
    /// No extraction: the tree was copied as-is (plain file or directory material).
    Plain,
    #[cfg(feature = "zip")]
    Zip,
    #[cfg(feature = "tar")]
    Tar,
    #[cfg(feature = "gzip")]
    TarGz,
    #[cfg(feature = "xz")]
    TarXz,
    #[cfg(feature = "zstd")]
    TarZstd,
}

/// Evidence observed while preparing an archive tree.
///
/// `total_bytes` counts materialized regular-file bytes. For TAR families, `decoded_bytes` counts
/// the decoded container stream, including headers, padding, extensions, and stripped entries.
/// For ZIP, `decoded_bytes` counts decoded entry material; ZIP container metadata is seek-parsed
/// rather than emitted through an equivalent decoded stream. `format` attests the archive kind
/// the tree was prepared as (data, not a type).
#[non_exhaustive]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ArchiveEvidence {
    pub root: PathBuf,
    pub entries: u64,
    pub total_bytes: u64,
    pub decoded_bytes: u64,
    pub files: u64,
    pub directories: u64,
    pub symlinks: u64,
    pub format: ArchiveKind,
}

impl ArchiveEvidence {
    fn empty(root: impl Into<PathBuf>, format: ArchiveKind) -> Self {
        Self {
            root: root.into(),
            entries: 0,
            total_bytes: 0,
            decoded_bytes: 0,
            files: 0,
            directories: 0,
            symlinks: 0,
            format,
        }
    }
}

impl ArchiveKind {
    /// Prepare a source archive into caller-owned exclusive custody and return extraction evidence.
    /// `Plain` is a copy operation and is therefore rejected here.
    pub fn prepare(
        self,
        source: &Path,
        workspace: &Path,
        policy: ArchivePolicy,
    ) -> Result<ArchiveEvidence, ArchiveError> {
        match self {
            Self::Plain => Err(ArchiveError::InvalidPreparation(
                "a plain material is copied, not extracted".into(),
            )),
            #[cfg(feature = "zip")]
            Self::Zip => extract_source(source, workspace, policy, extract_zip),
            #[cfg(feature = "tar")]
            Self::Tar => extract_source(source, workspace, policy, extract_tar_plain),
            #[cfg(feature = "gzip")]
            Self::TarGz => extract_source(source, workspace, policy, extract_tar_gzip),
            #[cfg(feature = "xz")]
            Self::TarXz => extract_source(source, workspace, policy, extract_tar_xz),
            #[cfg(feature = "zstd")]
            Self::TarZstd => extract_source(source, workspace, policy, extract_tar_zstd),
        }
    }
}

/// Shared extraction driver: reset the exclusive root, extract, clean up on failure, and pair
/// the prepared tree with its attestation.
fn extract_source(
    source: &Path,
    root: &Path,
    policy: ArchivePolicy,
    extract: fn(&Path, &Path, &ArchivePolicy) -> Result<ArchiveEvidence, ArchiveError>,
) -> Result<ArchiveEvidence, ArchiveError> {
    let root = root.to_path_buf();
    reset_extract_root(&root)?;
    let evidence = match extract(source, &root, &policy) {
        Ok(evidence) => evidence,
        Err(error) => {
            let cleanup = reset_extract_root(&root);
            return Err(combine_archive_failure(&root, error, cleanup));
        }
    };
    Ok(evidence)
}

impl ArchiveKind {
    /// Detect an archive format from its magic bytes, reading the head once.
    ///
    /// Returns `Ok(None)` when the material is not a recognized archive (the caller copies it
    /// as-is), and an unsupported error for a recognized format whose decoder feature is disabled.
    /// Detection is read-only; it never extracts or opens a decoder.
    pub fn sniff(path: &Path) -> io::Result<Option<Self>> {
        let mut head = [0u8; 512];
        let mut file = File::open(path)?;
        let read = read_head(&mut file, &mut head)?;
        let head = &head[..read];
        if head.len() >= 4
            && (head[..4] == [0x50, 0x4b, 0x03, 0x04]
                || head[..4] == [0x50, 0x4b, 0x05, 0x06]
                || head[..4] == [0x50, 0x4b, 0x07, 0x08])
        {
            #[cfg(feature = "zip")]
            return Ok(Some(ArchiveKind::Zip));
            #[cfg(not(feature = "zip"))]
            return Err(unsupported_format("ZIP"));
        }
        if head.len() >= 2 && head[..2] == [0x1f, 0x8b] {
            #[cfg(feature = "gzip")]
            return Ok(Some(ArchiveKind::TarGz));
            #[cfg(not(feature = "gzip"))]
            return Err(unsupported_format("gzip"));
        }
        if head.len() >= 6 && head[..6] == [0xfd, 0x37, 0x7a, 0x58, 0x5a, 0x00] {
            #[cfg(feature = "xz")]
            return Ok(Some(ArchiveKind::TarXz));
            #[cfg(not(feature = "xz"))]
            return Err(unsupported_format("XZ"));
        }
        if head.len() >= 4 && head[..4] == [0x28, 0xb5, 0x2f, 0xfd] {
            #[cfg(feature = "zstd")]
            return Ok(Some(ArchiveKind::TarZstd));
            #[cfg(not(feature = "zstd"))]
            return Err(unsupported_format("Zstandard"));
        }
        if head.len() >= 262 && &head[257..262] == b"ustar" {
            #[cfg(feature = "tar")]
            return Ok(Some(ArchiveKind::Tar));
            #[cfg(not(feature = "tar"))]
            return Err(unsupported_format("tar"));
        }
        Ok(None)
    }
}

#[cfg(not(all(
    feature = "zip",
    feature = "tar",
    feature = "gzip",
    feature = "xz",
    feature = "zstd"
)))]
fn unsupported_format(format: &'static str) -> io::Error {
    io::Error::new(
        io::ErrorKind::Unsupported,
        format!("{format} archive support is not enabled"),
    )
}

fn read_head(file: &mut File, head: &mut [u8]) -> io::Result<usize> {
    let mut filled = 0;
    while filled < head.len() {
        match file.read(&mut head[filled..]) {
            Ok(0) => break,
            Ok(bytes) => filled += bytes,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }
    Ok(filled)
}

fn combine_archive_failure(
    workspace: &Path,
    extraction: ArchiveError,
    cleanup: Result<(), ArchiveError>,
) -> ArchiveError {
    match cleanup {
        Ok(()) => extraction,
        Err(cleanup) => ArchiveError::ArchiveCleanupFailed {
            workspace: workspace.to_path_buf(),
            extraction: Box::new(extraction),
            cleanup: Box::new(cleanup),
        },
    }
}

#[cfg(feature = "zip")]
fn extract_zip(
    archive_path: &Path,
    root: &Path,
    policy: &ArchivePolicy,
) -> Result<ArchiveEvidence, ArchiveError> {
    let file = File::open(archive_path)
        .map_err(|err| ArchiveError::io("open zip archive", archive_path, err))?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|err| ArchiveError::InvalidPreparation(format!("invalid zip archive: {err}")))?;
    let mut evidence = ArchiveEvidence::empty(root, ArchiveKind::Zip);
    let mut paths = ArchivePathIndex::default();

    for index in 0..archive.len() {
        evidence.entries += 1;
        check_limit("entry-count", evidence.entries, policy.max_entries)?;

        let mut file = archive
            .by_index(index)
            .map_err(|err| ArchiveError::InvalidPreparation(format!("invalid zip entry: {err}")))?;
        let enclosed = file
            .enclosed_name()
            .ok_or_else(|| ArchiveError::ArchiveInvalidPath(file.name().to_string()))?;
        let Some(relative) = sanitize_relative(&enclosed, policy.strip_components)? else {
            continue;
        };
        let target = root.join(&relative);
        ensure_under_root(root, &target)?;
        reject_existing_symlink_path(root, &target)?;

        if is_zip_symlink(file.unix_mode()) {
            evidence.symlinks += 1;
            return Err(ArchiveError::UnsupportedArchiveEntry(relative));
        }

        let kind = if file.is_dir() {
            ArchiveEntryKind::Directory
        } else {
            ArchiveEntryKind::File
        };
        paths.admit(&relative, kind)?;

        if kind == ArchiveEntryKind::Directory {
            evidence.directories += 1;
            fs::create_dir_all(&target)
                .map_err(|err| ArchiveError::io("create archive directory", &target, err))?;
            continue;
        }

        let declared = file.size();
        check_limit("entry-bytes", declared, policy.max_entry_bytes)?;
        let declared_total = evidence.total_bytes.checked_add(declared).ok_or(
            ArchiveError::ArchiveLimitExceeded {
                limit: "total-bytes",
                actual: u64::MAX,
                max: policy.max_total_bytes.unwrap_or(u64::MAX),
            },
        )?;
        check_limit("total-bytes", declared_total, policy.max_total_bytes)?;

        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)
                .map_err(|err| ArchiveError::io("create archive file parent", parent, err))?;
        }
        let observed = copy_archive_file(
            &mut file,
            &target,
            &relative,
            declared,
            ArchiveCopyLimits {
                observed_total: evidence.total_bytes,
                max_entry: policy.max_entry_bytes,
                max_total: policy.max_total_bytes,
                max_decoded: policy.max_decoded_bytes,
            },
        )?;
        evidence.files += 1;
        evidence.total_bytes = evidence.total_bytes.checked_add(observed).ok_or(
            ArchiveError::ArchiveLimitExceeded {
                limit: "total-bytes",
                actual: u64::MAX,
                max: policy.max_total_bytes.unwrap_or(u64::MAX),
            },
        )?;
        evidence.decoded_bytes = evidence.total_bytes;
    }

    Ok(evidence)
}

#[cfg(feature = "tar")]
fn extract_tar_plain(
    archive_path: &Path,
    root: &Path,
    policy: &ArchivePolicy,
) -> Result<ArchiveEvidence, ArchiveError> {
    let file = File::open(archive_path)
        .map_err(|err| ArchiveError::io("open tar archive", archive_path, err))?;
    extract_tar_reader(file, root, policy, ArchiveKind::Tar)
}

#[cfg(feature = "gzip")]
fn extract_tar_gzip(
    archive_path: &Path,
    root: &Path,
    policy: &ArchivePolicy,
) -> Result<ArchiveEvidence, ArchiveError> {
    let file = File::open(archive_path)
        .map_err(|err| ArchiveError::io("open gzip tar archive", archive_path, err))?;
    extract_tar_reader(
        flate2::read::GzDecoder::new(file),
        root,
        policy,
        ArchiveKind::TarGz,
    )
}

#[cfg(feature = "xz")]
fn extract_tar_xz(
    archive_path: &Path,
    root: &Path,
    policy: &ArchivePolicy,
) -> Result<ArchiveEvidence, ArchiveError> {
    let file = File::open(archive_path)
        .map_err(|err| ArchiveError::io("open xz tar archive", archive_path, err))?;
    extract_tar_reader(
        xz2::read::XzDecoder::new(file),
        root,
        policy,
        ArchiveKind::TarXz,
    )
}

#[cfg(feature = "zstd")]
fn extract_tar_zstd(
    archive_path: &Path,
    root: &Path,
    policy: &ArchivePolicy,
) -> Result<ArchiveEvidence, ArchiveError> {
    let file = File::open(archive_path)
        .map_err(|err| ArchiveError::io("open zstd tar archive", archive_path, err))?;
    let decoder = zstd::stream::Decoder::new(file)
        .map_err(|err| ArchiveError::io("open zstd tar decoder", archive_path, err))?;
    extract_tar_reader(decoder, root, policy, ArchiveKind::TarZstd)
}

#[cfg(feature = "tar")]
fn extract_tar_reader<R: Read>(
    reader: R,
    root: &Path,
    policy: &ArchivePolicy,
    format: ArchiveKind,
) -> Result<ArchiveEvidence, ArchiveError> {
    let reader = DecodedLimitReader::new(reader, policy.max_decoded_bytes);
    let mut archive = tar::Archive::new(reader);
    let mut evidence = ArchiveEvidence::empty(root, format);
    let mut paths = ArchivePathIndex::default();
    let entries = archive
        .entries()
        .map_err(|err| archive_io_error("read tar archive", root, err))?;

    for entry in entries {
        evidence.entries += 1;
        check_limit("entry-count", evidence.entries, policy.max_entries)?;

        let mut entry = entry.map_err(|err| archive_io_error("read tar entry", root, err))?;
        let raw_path = entry
            .path()
            .map_err(|err| archive_io_error("read tar entry path", root, err))?
            .into_owned();
        let Some(relative) = sanitize_relative(&raw_path, policy.strip_components)? else {
            continue;
        };
        let target = root.join(&relative);
        ensure_under_root(root, &target)?;
        reject_existing_symlink_path(root, &target)?;

        let entry_type = entry.header().entry_type();
        if entry_type.is_symlink() || entry_type.is_hard_link() {
            evidence.symlinks += 1;
            return Err(ArchiveError::UnsupportedArchiveEntry(relative));
        }

        let kind = if entry_type.is_dir() {
            ArchiveEntryKind::Directory
        } else if entry_type.is_file() {
            ArchiveEntryKind::File
        } else {
            return Err(ArchiveError::UnsupportedArchiveEntry(relative));
        };
        paths.admit(&relative, kind)?;

        if kind == ArchiveEntryKind::Directory {
            evidence.directories += 1;
            fs::create_dir_all(&target)
                .map_err(|err| ArchiveError::io("create archive directory", &target, err))?;
            continue;
        }

        let declared = entry.size();
        check_limit("entry-bytes", declared, policy.max_entry_bytes)?;
        let declared_total = evidence.total_bytes.checked_add(declared).ok_or(
            ArchiveError::ArchiveLimitExceeded {
                limit: "total-bytes",
                actual: u64::MAX,
                max: policy.max_total_bytes.unwrap_or(u64::MAX),
            },
        )?;
        check_limit("total-bytes", declared_total, policy.max_total_bytes)?;

        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)
                .map_err(|err| ArchiveError::io("create archive file parent", parent, err))?;
        }
        let observed = copy_archive_file(
            &mut entry,
            &target,
            &relative,
            declared,
            ArchiveCopyLimits {
                observed_total: evidence.total_bytes,
                max_entry: policy.max_entry_bytes,
                max_total: policy.max_total_bytes,
                max_decoded: None,
            },
        )?;
        evidence.files += 1;
        evidence.total_bytes = evidence.total_bytes.checked_add(observed).ok_or(
            ArchiveError::ArchiveLimitExceeded {
                limit: "total-bytes",
                actual: u64::MAX,
                max: policy.max_total_bytes.unwrap_or(u64::MAX),
            },
        )?;
    }

    evidence.decoded_bytes = archive.into_inner().observed;
    check_limit(
        "decoded-bytes",
        evidence.decoded_bytes,
        policy.max_decoded_bytes,
    )?;
    Ok(evidence)
}

fn archive_collision_key(path: &Path) -> PathBuf {
    path.components()
        .filter_map(|component| match component {
            Component::Normal(part) => Some(part.to_string_lossy().to_lowercase()),
            _ => None,
        })
        .collect()
}

fn copy_archive_file<R: Read>(
    reader: &mut R,
    target: &Path,
    relative: &Path,
    declared: u64,
    limits: ArchiveCopyLimits,
) -> Result<u64, ArchiveError> {
    let ArchiveCopyLimits {
        observed_total,
        max_entry,
        max_total,
        max_decoded,
    } = limits;
    let mut output =
        File::create(target).map_err(|err| ArchiveError::io("create archive file", target, err))?;
    let total_remaining = max_total.map(|max| max.saturating_sub(observed_total));
    let decoded_remaining = max_decoded.map(|max| max.saturating_sub(observed_total));
    let remaining = match (max_entry, total_remaining, decoded_remaining) {
        (Some(entry), Some(total), Some(decoded)) => Some(entry.min(total).min(decoded)),
        (Some(entry), Some(total), None) => Some(entry.min(total)),
        (Some(entry), None, Some(decoded)) => Some(entry.min(decoded)),
        (None, Some(total), Some(decoded)) => Some(total.min(decoded)),
        (Some(entry), None, None) => Some(entry),
        (None, Some(total), None) => Some(total),
        (None, None, Some(decoded)) => Some(decoded),
        (None, None, None) => None,
    };
    let copied = match remaining {
        Some(remaining) => io::copy(&mut reader.take(remaining.saturating_add(1)), &mut output),
        None => io::copy(reader, &mut output),
    };
    let observed = match copied {
        Ok(observed) => observed,
        Err(error) => {
            drop(output);
            let _ = fs::remove_file(target);
            return Err(archive_io_error("extract archive file", target, error));
        }
    };

    if let Some(max) = max_entry
        && observed > max
    {
        drop(output);
        let _ = fs::remove_file(target);
        return Err(ArchiveError::ArchiveLimitExceeded {
            limit: "entry-bytes",
            actual: observed,
            max,
        });
    }

    if let Some(max) = max_total {
        let actual = observed_total.saturating_add(observed);
        if actual > max {
            drop(output);
            let _ = fs::remove_file(target);
            return Err(ArchiveError::ArchiveLimitExceeded {
                limit: "total-bytes",
                actual,
                max,
            });
        }
    }

    if let Some(max) = max_decoded {
        let actual = observed_total.saturating_add(observed);
        if actual > max {
            drop(output);
            let _ = fs::remove_file(target);
            return Err(ArchiveError::ArchiveLimitExceeded {
                limit: "decoded-bytes",
                actual,
                max,
            });
        }
    }

    if observed != declared {
        drop(output);
        let _ = fs::remove_file(target);
        return Err(ArchiveError::ArchiveSizeMismatch {
            path: relative.to_path_buf(),
            declared,
            observed,
        });
    }

    Ok(observed)
}

fn archive_io_error(action: &'static str, path: &Path, error: io::Error) -> ArchiveError {
    if let Some(limit) = error
        .get_ref()
        .and_then(|source| source.downcast_ref::<DecodedLimitExceeded>())
    {
        return ArchiveError::ArchiveLimitExceeded {
            limit: "decoded-bytes",
            actual: limit.actual,
            max: limit.max,
        };
    }

    let mut source: &(dyn std::error::Error + 'static) = &error;
    loop {
        if let Some(limit) = source.downcast_ref::<DecodedLimitExceeded>() {
            return ArchiveError::ArchiveLimitExceeded {
                limit: "decoded-bytes",
                actual: limit.actual,
                max: limit.max,
            };
        }
        let Some(next) = source.source() else {
            break;
        };
        source = next;
    }
    ArchiveError::io(action, path, error)
}

fn check_limit(limit: &'static str, actual: u64, max: Option<u64>) -> Result<(), ArchiveError> {
    if let Some(max) = max
        && actual > max
    {
        return Err(ArchiveError::ArchiveLimitExceeded { limit, actual, max });
    }
    Ok(())
}

fn reset_extract_root(root: &Path) -> Result<(), ArchiveError> {
    if let Ok(metadata) = fs::symlink_metadata(root) {
        let file_type = metadata.file_type();
        if file_type.is_symlink() {
            return Err(ArchiveError::UnsupportedArchiveEntry(root.to_path_buf()));
        }
        if file_type.is_dir() {
            fs::remove_dir_all(root)
                .map_err(|err| ArchiveError::io("clear archive root", root, err))?;
        } else {
            fs::remove_file(root)
                .map_err(|err| ArchiveError::io("clear archive root file", root, err))?;
        }
    }
    fs::create_dir_all(root).map_err(|err| ArchiveError::io("create archive root", root, err))
}

fn sanitize_relative(
    path: &Path,
    strip_components: usize,
) -> Result<Option<PathBuf>, ArchiveError> {
    let mut relative = PathBuf::new();
    let mut seen = 0usize;

    for component in path.components() {
        match component {
            Component::Normal(part) => {
                if seen >= strip_components {
                    validate_archive_component(part)?;
                    relative.push(part);
                }
                seen += 1;
            }
            Component::CurDir => {}
            other => return Err(ArchiveError::ArchiveInvalidPath(format!("{other:?}"))),
        }
    }

    if relative.as_os_str().is_empty() {
        Ok(None)
    } else {
        Ok(Some(relative))
    }
}

#[cfg(windows)]
fn validate_archive_component(component: &OsStr) -> Result<(), ArchiveError> {
    let value = component.to_string_lossy();
    let has_forbidden_character = value
        .chars()
        .any(|character| character <= '\u{1f}' || "<>:\"/\\|?*".contains(character));
    let has_unsafe_suffix = value.ends_with('.') || value.ends_with(' ');
    let stem = value
        .split('.')
        .next()
        .unwrap_or_default()
        .trim_end_matches([' ', '.'])
        .to_ascii_uppercase();
    let is_device = matches!(
        stem.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
    ) || stem.strip_prefix("COM").is_some_and(|suffix| {
        matches!(
            suffix,
            "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
        )
    }) || stem.strip_prefix("LPT").is_some_and(|suffix| {
        matches!(
            suffix,
            "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
        )
    });

    if has_forbidden_character || has_unsafe_suffix || is_device {
        return Err(ArchiveError::ArchiveInvalidPath(value.into_owned()));
    }
    Ok(())
}

#[cfg(not(windows))]
fn validate_archive_component(_component: &std::ffi::OsStr) -> Result<(), ArchiveError> {
    Ok(())
}

fn ensure_under_root(root: &Path, target: &Path) -> Result<(), ArchiveError> {
    if !target.starts_with(root) {
        return Err(ArchiveError::ArchiveInvalidPath(
            target.display().to_string(),
        ));
    }
    Ok(())
}

fn reject_existing_symlink_path(root: &Path, target: &Path) -> Result<(), ArchiveError> {
    let relative = target
        .strip_prefix(root)
        .map_err(|_| ArchiveError::ArchiveInvalidPath(target.display().to_string()))?;
    let mut cursor = root.to_path_buf();
    for component in relative.components() {
        if let Component::Normal(part) = component {
            cursor.push(part);
            if let Ok(metadata) = fs::symlink_metadata(&cursor)
                && metadata.file_type().is_symlink()
            {
                return Err(ArchiveError::UnsupportedArchiveEntry(cursor));
            }
        }
    }
    Ok(())
}

#[cfg(feature = "zip")]
fn is_zip_symlink(mode: Option<u32>) -> bool {
    mode.is_some_and(|mode| mode & 0o170000 == 0o120000)
}

#[cfg(all(test, feature = "zip", feature = "local"))]
mod tests {
    use std::error::Error as _;
    use std::fs::{self, File};
    use std::io::{self, Write};
    use std::path::{Path, PathBuf};

    use super::{
        ArchiveEntryKind, ArchiveError, ArchiveKind, ArchivePathIndex, ArchivePolicy,
        combine_archive_failure,
    };

    fn prepare(
        source: &Path,
        workspace: &Path,
        policy: ArchivePolicy,
        kind: ArchiveKind,
    ) -> Result<super::ArchiveEvidence, ArchiveError> {
        kind.prepare(source, workspace, policy)
    }

    fn temp_root(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "pulith-archive-{name}-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn archive_failure_preserves_cleanup_error() {
        let workspace = PathBuf::from("extract");
        let extraction = ArchiveError::InvalidPreparation("broken archive".into());
        let cleanup = ArchiveError::io(
            "clear archive root",
            &workspace,
            io::Error::new(io::ErrorKind::PermissionDenied, "locked"),
        );

        let error = combine_archive_failure(&workspace, extraction, Err(cleanup));

        assert_eq!(
            error.to_string(),
            "archive extraction and cleanup both failed for extract"
        );
        assert_eq!(
            error.source().map(ToString::to_string).as_deref(),
            Some("invalid preparation: broken archive")
        );

        assert!(matches!(
            error,
            ArchiveError::ArchiveCleanupFailed {
                workspace: path,
                extraction,
                cleanup,
            } if path == workspace
                && matches!(*extraction, ArchiveError::InvalidPreparation(_))
                && matches!(*cleanup, ArchiveError::Io { .. })
        ));
    }

    #[test]
    fn plain_kind_is_rejected_without_extracting() {
        let error = prepare(
            Path::new("unused-source"),
            Path::new("unused-workspace"),
            ArchivePolicy::default(),
            ArchiveKind::Plain,
        )
        .unwrap_err();

        assert!(
            matches!(error, ArchiveError::InvalidPreparation(message) if message.contains("plain"))
        );
    }

    #[test]
    fn archive_path_index_preserves_file_and_directory_conflict_laws() {
        let mut file_parent = ArchivePathIndex::default();
        file_parent
            .admit(Path::new("bin"), ArchiveEntryKind::File)
            .unwrap();
        assert!(matches!(
            file_parent.admit(Path::new("bin/tool"), ArchiveEntryKind::File),
            Err(ArchiveError::ArchivePathConflict(path)) if path == Path::new("bin/tool")
        ));

        let mut file_above_descendant = ArchivePathIndex::default();
        file_above_descendant
            .admit(Path::new("bin/tool"), ArchiveEntryKind::File)
            .unwrap();
        assert!(matches!(
            file_above_descendant.admit(Path::new("bin"), ArchiveEntryKind::File),
            Err(ArchiveError::ArchivePathConflict(path)) if path == Path::new("bin")
        ));

        let mut directories = ArchivePathIndex::default();
        directories
            .admit(Path::new("bin/tool"), ArchiveEntryKind::File)
            .unwrap();
        directories
            .admit(Path::new("bin"), ArchiveEntryKind::Directory)
            .unwrap();
        directories
            .admit(Path::new("bin/readme"), ArchiveEntryKind::File)
            .unwrap();
        assert!(matches!(
            directories.admit(Path::new("BIN/README"), ArchiveEntryKind::File),
            Err(ArchiveError::ArchivePathConflict(path)) if path == Path::new("BIN/README")
        ));
    }

    fn write_zip(path: &Path, entries: &[(&str, &[u8])]) {
        let file = File::create(path).unwrap();
        let mut writer = zip::ZipWriter::new(file);
        for (name, content) in entries {
            writer
                .start_file(name, zip::write::SimpleFileOptions::default())
                .unwrap();
            writer.write_all(content).unwrap();
        }
        writer.finish().unwrap();
    }

    fn write_zip_with_directory(path: &Path) {
        let file = File::create(path).unwrap();
        let mut writer = zip::ZipWriter::new(file);
        writer
            .add_directory("root/bin/", zip::write::SimpleFileOptions::default())
            .unwrap();
        writer
            .start_file(
                "root/bin/tool.txt",
                zip::write::SimpleFileOptions::default(),
            )
            .unwrap();
        writer.write_all(b"pulith").unwrap();
        writer.finish().unwrap();
    }

    fn write_zip_with_symlink(path: &Path) {
        let file = File::create(path).unwrap();
        let mut writer = zip::ZipWriter::new(file);
        writer
            .start_file("link", zip::write::SimpleFileOptions::default())
            .unwrap();
        writer.write_all(b"target").unwrap();
        writer.finish().unwrap();

        let mut bytes = fs::read(path).unwrap();
        let central = bytes
            .windows(4)
            .position(|window| window == [0x50, 0x4b, 0x01, 0x02])
            .unwrap();
        bytes[central + 4] = 20;
        bytes[central + 5] = 3;
        let mode = ((0o120777u32) << 16).to_le_bytes();
        bytes[central + 38..central + 42].copy_from_slice(&mode);
        fs::write(path, bytes).unwrap();
    }

    fn write_zip_with_understated_size(path: &Path) {
        let file = File::create(path).unwrap();
        let mut writer = zip::ZipWriter::new(file);
        writer
            .start_file(
                "payload.txt",
                zip::write::SimpleFileOptions::default()
                    .compression_method(zip::CompressionMethod::Stored),
            )
            .unwrap();
        writer.write_all(b"12345678").unwrap();
        writer.finish().unwrap();

        let mut bytes = fs::read(path).unwrap();
        let local = bytes
            .windows(4)
            .position(|window| window == [0x50, 0x4b, 0x03, 0x04])
            .unwrap();
        let central = bytes
            .windows(4)
            .position(|window| window == [0x50, 0x4b, 0x01, 0x02])
            .unwrap();
        bytes[local + 22..local + 26].copy_from_slice(&1u32.to_le_bytes());
        bytes[central + 24..central + 28].copy_from_slice(&1u32.to_le_bytes());
        fs::write(path, bytes).unwrap();
    }

    #[test]
    fn zip_prepare_extracts_archive_tree() {
        let root = temp_root("extract");
        let zip_path = root.join("payload.zip");
        let extract_root = root.join("extract");
        fs::create_dir_all(&root).unwrap();
        write_zip(&zip_path, &[("bin/tool.txt", b"pulith")]);

        let evidence = prepare(
            &zip_path,
            &extract_root,
            ArchivePolicy::default(),
            ArchiveKind::Zip,
        )
        .unwrap();

        assert_eq!(evidence.root, extract_root);
        assert_eq!(evidence.entries, 1);
        assert_eq!(evidence.files, 1);
        assert_eq!(
            fs::read_to_string(root.join("extract/bin/tool.txt")).unwrap(),
            "pulith"
        );

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn zip_prepare_clears_stale_extract_root() {
        let root = temp_root("clear-stale");
        let zip_path = root.join("payload.zip");
        let extract_root = root.join("extract");
        fs::create_dir_all(extract_root.join("old")).unwrap();
        fs::write(extract_root.join("old/stale.txt"), "stale").unwrap();
        write_zip(&zip_path, &[("fresh.txt", b"fresh")]);

        prepare(
            &zip_path,
            &extract_root,
            ArchivePolicy::default(),
            ArchiveKind::Zip,
        )
        .unwrap();

        assert_eq!(
            fs::read_to_string(extract_root.join("fresh.txt")).unwrap(),
            "fresh"
        );
        assert!(!extract_root.join("old/stale.txt").exists());

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn zip_prepare_honors_strip_components_and_directories() {
        let root = temp_root("strip-dir");
        let zip_path = root.join("payload.zip");
        let extract_root = root.join("extract");
        fs::create_dir_all(&root).unwrap();
        write_zip_with_directory(&zip_path);

        let evidence = prepare(
            &zip_path,
            &extract_root,
            ArchivePolicy::new().strip_components(1),
            ArchiveKind::Zip,
        )
        .unwrap();

        assert_eq!(evidence.entries, 2);
        assert_eq!(evidence.directories, 1);
        assert_eq!(evidence.files, 1);
        assert_eq!(
            fs::read_to_string(extract_root.join("bin/tool.txt")).unwrap(),
            "pulith"
        );
        assert!(!extract_root.join("root/bin/tool.txt").exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn zip_prepare_rejects_entry_limit() {
        let root = temp_root("entry-limit");
        let zip_path = root.join("payload.zip");
        fs::create_dir_all(&root).unwrap();
        write_zip(&zip_path, &[("a.txt", b"a"), ("b.txt", b"b")]);

        let err = prepare(
            &zip_path,
            &root.join("extract"),
            ArchivePolicy::new().max_entries(1),
            ArchiveKind::Zip,
        )
        .unwrap_err();

        assert!(matches!(
            err,
            ArchiveError::ArchiveLimitExceeded {
                limit: "entry-count",
                ..
            }
        ));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn zip_prepare_rejects_zip_slip_path() {
        let root = temp_root("zip-slip");
        let zip_path = root.join("payload.zip");
        fs::create_dir_all(&root).unwrap();
        write_zip(&zip_path, &[("../escape.txt", b"nope")]);

        let err = prepare(
            &zip_path,
            &root.join("extract"),
            ArchivePolicy::default(),
            ArchiveKind::Zip,
        )
        .unwrap_err();

        assert!(matches!(err, ArchiveError::ArchiveInvalidPath(_)));
        assert!(!root.parent().unwrap().join("escape.txt").exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn zip_prepare_rejects_symlink_entry() {
        let root = temp_root("symlink");
        let zip_path = root.join("payload.zip");
        fs::create_dir_all(&root).unwrap();
        write_zip_with_symlink(&zip_path);

        let err = prepare(
            &zip_path,
            &root.join("extract"),
            ArchivePolicy::default(),
            ArchiveKind::Zip,
        )
        .unwrap_err();

        assert!(matches!(err, ArchiveError::UnsupportedArchiveEntry(_)));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn zip_prepare_rejects_byte_limit() {
        let root = temp_root("byte-limit");
        let zip_path = root.join("payload.zip");
        fs::create_dir_all(&root).unwrap();
        write_zip(&zip_path, &[("a.txt", b"abcd")]);

        let err = prepare(
            &zip_path,
            &root.join("extract"),
            ArchivePolicy::new().max_total_bytes(3),
            ArchiveKind::Zip,
        )
        .unwrap_err();

        assert!(matches!(
            err,
            ArchiveError::ArchiveLimitExceeded {
                limit: "total-bytes",
                ..
            }
        ));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn zip_prepare_enforces_observed_byte_limit() {
        let root = temp_root("observed-byte-limit");
        let zip_path = root.join("payload.zip");
        let extract_root = root.join("extract");
        fs::create_dir_all(&root).unwrap();
        write_zip_with_understated_size(&zip_path);

        let err = prepare(
            &zip_path,
            &extract_root,
            ArchivePolicy::new().max_total_bytes(4),
            ArchiveKind::Zip,
        )
        .unwrap_err();

        assert!(matches!(
            err,
            ArchiveError::ArchiveLimitExceeded {
                limit: "total-bytes",
                actual: 5,
                max: 4,
            }
        ));
        assert_eq!(fs::read_dir(&extract_root).unwrap().count(), 0);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn zip_prepare_rejects_file_directory_collision() {
        let root = temp_root("path-collision");
        let zip_path = root.join("payload.zip");
        let extract_root = root.join("extract");
        fs::create_dir_all(&root).unwrap();
        write_zip(&zip_path, &[("foo", b"file"), ("foo/child.txt", b"child")]);

        let err = prepare(
            &zip_path,
            &extract_root,
            ArchivePolicy::default(),
            ArchiveKind::Zip,
        )
        .unwrap_err();

        assert!(matches!(
            err,
            ArchiveError::ArchivePathConflict(path) if path == Path::new("foo/child.txt")
        ));
        assert_eq!(fs::read_dir(&extract_root).unwrap().count(), 0);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn zip_prepare_rejects_case_folded_path_collision() {
        let root = temp_root("case-folded-collision");
        let zip_path = root.join("payload.zip");
        let extract_root = root.join("extract");
        fs::create_dir_all(&root).unwrap();
        write_zip(&zip_path, &[("Foo", b"file"), ("foo/child.txt", b"child")]);

        let err = prepare(
            &zip_path,
            &extract_root,
            ArchivePolicy::default(),
            ArchiveKind::Zip,
        )
        .unwrap_err();

        assert!(matches!(
            err,
            ArchiveError::ArchivePathConflict(path) if path == Path::new("foo/child.txt")
        ));
        assert_eq!(fs::read_dir(&extract_root).unwrap().count(), 0);
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn zip_prepare_rejects_windows_device_and_stream_paths() {
        for name in [
            "NUL.txt",
            "CONIN$",
            "CONOUT$",
            "COM¹.txt",
            "LPT³.txt",
            "payload.txt:stream",
            "trailing.",
        ] {
            let root = temp_root("windows-unsafe-path");
            let zip_path = root.join("payload.zip");
            let extract_root = root.join("extract");
            fs::create_dir_all(&root).unwrap();
            write_zip(&zip_path, &[(name, b"unsafe")]);

            let err = prepare(
                &zip_path,
                &extract_root,
                ArchivePolicy::default(),
                ArchiveKind::Zip,
            )
            .unwrap_err();

            assert!(matches!(err, ArchiveError::ArchiveInvalidPath(_)), "{name}");
            assert_eq!(fs::read_dir(&extract_root).unwrap().count(), 0);
            fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn zip_prepare_rejects_entry_byte_limit() {
        let root = temp_root("entry-byte-limit");
        let zip_path = root.join("payload.zip");
        let extract_root = root.join("extract");
        fs::create_dir_all(&root).unwrap();
        write_zip(&zip_path, &[("payload.txt", b"1234")]);

        let err = prepare(
            &zip_path,
            &extract_root,
            ArchivePolicy::new().max_entry_bytes(3),
            ArchiveKind::Zip,
        )
        .unwrap_err();

        assert!(matches!(
            err,
            ArchiveError::ArchiveLimitExceeded {
                limit: "entry-bytes",
                actual: 4,
                max: 3,
            }
        ));
        assert_eq!(fs::read_dir(&extract_root).unwrap().count(), 0);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn sniff_format_detects_zip_from_magic() {
        let root = temp_root("sniff-zip");
        fs::create_dir_all(&root).unwrap();
        let zip_path = root.join("tool.zip");
        write_zip(&zip_path, &[("bin/tool", b"payload")]);
        assert_eq!(
            ArchiveKind::sniff(&zip_path).unwrap(),
            Some(ArchiveKind::Zip)
        );

        let empty_zip_path = root.join("empty.zip");
        fs::write(
            &empty_zip_path,
            b"PK\x05\x06\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0",
        )
        .unwrap();
        assert_eq!(
            ArchiveKind::sniff(&empty_zip_path).unwrap(),
            Some(ArchiveKind::Zip)
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(not(feature = "gzip"))]
    #[test]
    fn sniff_format_rejects_a_known_disabled_format() {
        let root = temp_root("sniff-disabled-gzip");
        fs::create_dir_all(&root).unwrap();
        let gzip_path = root.join("tool.tar.gz");
        fs::write(&gzip_path, [0x1f, 0x8b]).unwrap();

        assert_eq!(
            ArchiveKind::sniff(&gzip_path).unwrap_err().kind(),
            io::ErrorKind::Unsupported
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn preparation_evidence_attests_its_format_as_data() {
        let root = temp_root("prepared-format");
        fs::create_dir_all(&root).unwrap();
        let zip_path = root.join("tool.zip");
        write_zip(&zip_path, &[("bin/tool", b"payload")]);
        let extract_root = root.join("extract");

        let evidence = prepare(
            &zip_path,
            &extract_root,
            ArchivePolicy::default(),
            ArchiveKind::Zip,
        )
        .unwrap();

        assert_eq!(evidence.format, ArchiveKind::Zip);
        fs::remove_dir_all(root).unwrap();
    }
}

#[cfg(all(test, feature = "tar", feature = "local"))]
mod tar_tests {
    use std::fs::{self, File};
    use std::io::{self, Cursor, Write};
    use std::path::{Path, PathBuf};

    use super::{ArchiveError, ArchiveKind, ArchivePolicy};

    fn prepare(
        source: &Path,
        workspace: &Path,
        policy: ArchivePolicy,
        kind: ArchiveKind,
    ) -> Result<super::ArchiveEvidence, ArchiveError> {
        kind.prepare(source, workspace, policy)
    }

    fn temp_root(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "pulith-tar-{name}-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    fn append_file<W: Write>(builder: &mut tar::Builder<W>, path: &str, bytes: &[u8]) {
        let mut header = tar::Header::new_gnu();
        header.set_entry_type(tar::EntryType::Regular);
        header.set_size(bytes.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        builder
            .append_data(&mut header, path, Cursor::new(bytes))
            .unwrap();
    }

    fn append_dir<W: Write>(builder: &mut tar::Builder<W>, path: &str) {
        let mut header = tar::Header::new_gnu();
        header.set_entry_type(tar::EntryType::Directory);
        header.set_size(0);
        header.set_mode(0o755);
        header.set_path(path).unwrap();
        header.set_cksum();
        builder.append(&header, &[][..]).unwrap();
    }

    fn append_symlink<W: Write>(builder: &mut tar::Builder<W>, path: &str, target: &str) {
        let mut header = tar::Header::new_gnu();
        header.set_entry_type(tar::EntryType::Symlink);
        header.set_size(0);
        header.set_mode(0o777);
        header.set_path(path).unwrap();
        header.set_link_name(target).unwrap();
        header.set_cksum();
        builder.append(&header, &[][..]).unwrap();
    }

    fn write_tar(path: &Path, build: impl FnOnce(&mut tar::Builder<File>)) {
        let file = File::create(path).unwrap();
        let mut builder = tar::Builder::new(file);
        build(&mut builder);
        builder.finish().unwrap();
    }

    fn write_truncated_tar(path: &Path) {
        let mut header = tar::Header::new_gnu();
        header.set_entry_type(tar::EntryType::Regular);
        header.set_path("payload.txt").unwrap();
        header.set_size(8);
        header.set_mode(0o644);
        header.set_cksum();
        let mut bytes = header.as_bytes().to_vec();
        bytes.extend_from_slice(b"1234");
        fs::write(path, bytes).unwrap();
    }

    #[cfg(any(feature = "gzip", feature = "xz", feature = "zstd"))]
    fn tar_bytes(build: impl FnOnce(&mut tar::Builder<Vec<u8>>)) -> Vec<u8> {
        let mut builder = tar::Builder::new(Vec::new());
        build(&mut builder);
        builder.into_inner().unwrap()
    }

    #[cfg(feature = "gzip")]
    fn write_tar_gzip(path: &Path, build: impl FnOnce(&mut tar::Builder<Vec<u8>>)) {
        write_gzip_bytes(path, &tar_bytes(build));
    }

    #[cfg(feature = "gzip")]
    fn write_gzip_bytes(path: &Path, bytes: &[u8]) {
        let file = File::create(path).unwrap();
        let mut encoder = flate2::write::GzEncoder::new(file, flate2::Compression::default());
        encoder.write_all(bytes).unwrap();
        encoder.finish().unwrap();
    }

    fn assert_raw_compressed_stream_rejected(
        name: &str,
        extension: &str,
        kind: ArchiveKind,
        write: impl FnOnce(&Path),
    ) {
        let root = temp_root(name);
        let source = root.join(format!("payload.{extension}"));
        let extract_root = root.join("extract");
        fs::create_dir_all(&root).unwrap();
        write(&source);

        assert!(prepare(&source, &extract_root, ArchivePolicy::default(), kind).is_err());
        assert_eq!(fs::read_dir(&extract_root).unwrap().count(), 0);
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(feature = "xz")]
    fn write_tar_xz(path: &Path, build: impl FnOnce(&mut tar::Builder<Vec<u8>>)) {
        let file = File::create(path).unwrap();
        let mut encoder = xz2::write::XzEncoder::new(file, 6);
        encoder.write_all(&tar_bytes(build)).unwrap();
        encoder.finish().unwrap();
    }

    #[cfg(feature = "zstd")]
    fn write_tar_zstd(path: &Path, build: impl FnOnce(&mut tar::Builder<Vec<u8>>)) {
        let file = File::create(path).unwrap();
        let mut encoder = zstd::stream::Encoder::new(file, 3).unwrap();
        encoder.write_all(&tar_bytes(build)).unwrap();
        encoder.finish().unwrap();
    }

    fn patch_first_tar_path(path: &Path, name: &[u8]) {
        let mut bytes = fs::read(path).unwrap();
        patch_first_tar_path_bytes(&mut bytes, name);
        fs::write(path, bytes).unwrap();
    }

    fn patch_first_tar_path_bytes(bytes: &mut [u8], name: &[u8]) {
        assert!(name.len() <= 100);
        bytes[0..100].fill(0);
        bytes[0..name.len()].copy_from_slice(name);
        bytes[148..156].fill(b' ');
        let checksum: u32 = bytes[0..512].iter().map(|byte| *byte as u32).sum();
        let encoded = format!("{checksum:06o}\0 ");
        bytes[148..156].copy_from_slice(encoded.as_bytes());
    }

    #[test]
    fn tar_prepare_extracts_archive_tree() {
        let root = temp_root("extract");
        let tar_path = root.join("payload.tar");
        let extract_root = root.join("extract");
        fs::create_dir_all(&root).unwrap();
        write_tar(&tar_path, |builder| {
            append_file(builder, "bin/tool.txt", b"pulith")
        });

        let evidence = prepare(
            &tar_path,
            &extract_root,
            ArchivePolicy::default(),
            ArchiveKind::Tar,
        )
        .unwrap();

        assert_eq!(evidence.entries, 1);
        assert_eq!(evidence.files, 1);
        assert_eq!(
            fs::read_to_string(extract_root.join("bin/tool.txt")).unwrap(),
            "pulith"
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn tar_prepare_honors_strip_components_and_directories() {
        let root = temp_root("strip-dir");
        let tar_path = root.join("payload.tar");
        let extract_root = root.join("extract");
        fs::create_dir_all(&root).unwrap();
        write_tar(&tar_path, |builder| {
            append_dir(builder, "root/bin");
            append_file(builder, "root/bin/tool.txt", b"pulith");
        });

        let evidence = prepare(
            &tar_path,
            &extract_root,
            ArchivePolicy::new().strip_components(1),
            ArchiveKind::Tar,
        )
        .unwrap();

        assert_eq!(evidence.entries, 2);
        assert_eq!(evidence.directories, 1);
        assert_eq!(evidence.files, 1);
        assert_eq!(
            fs::read_to_string(extract_root.join("bin/tool.txt")).unwrap(),
            "pulith"
        );
        assert!(!extract_root.join("root/bin/tool.txt").exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn tar_prepare_rejects_zero_decoded_budget() {
        let root = temp_root("zero-decoded-budget");
        let tar_path = root.join("payload.tar");
        let extract_root = root.join("extract");
        fs::create_dir_all(&root).unwrap();
        write_tar(&tar_path, |builder| {
            append_file(builder, "payload.txt", b"x")
        });

        let error = prepare(
            &tar_path,
            &extract_root,
            ArchivePolicy::new().max_decoded_bytes(0),
            ArchiveKind::Tar,
        )
        .unwrap_err();

        assert!(matches!(
            error,
            ArchiveError::ArchiveLimitExceeded {
                limit: "decoded-bytes",
                actual: 1,
                max: 0,
            }
        ));
        assert_eq!(fs::read_dir(&extract_root).unwrap().count(), 0);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn tar_prepare_accepts_exact_decoded_budget_and_rejects_one_less() {
        let root = temp_root("exact-decoded-budget");
        let tar_path = root.join("payload.tar");
        fs::create_dir_all(&root).unwrap();
        write_tar(&tar_path, |builder| {
            append_file(builder, "payload.txt", b"pulith")
        });

        let measured = prepare(
            &tar_path,
            &root.join("measure"),
            ArchivePolicy::default(),
            ArchiveKind::Tar,
        )
        .unwrap()
        .decoded_bytes;

        prepare(
            &tar_path,
            &root.join("exact"),
            ArchivePolicy::new().max_decoded_bytes(measured),
            ArchiveKind::Tar,
        )
        .unwrap();

        let below_root = root.join("below");
        let error = prepare(
            &tar_path,
            &below_root,
            ArchivePolicy::new().max_decoded_bytes(measured - 1),
            ArchiveKind::Tar,
        )
        .unwrap_err();
        assert!(matches!(
            error,
            ArchiveError::ArchiveLimitExceeded {
                limit: "decoded-bytes",
                actual,
                max,
            } if actual == measured && max == measured - 1
        ));
        assert_eq!(fs::read_dir(&below_root).unwrap().count(), 0);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn tar_prepare_enforces_decoded_budget_during_regular_file_copy() {
        let root = temp_root("regular-copy-decoded-budget");
        let tar_path = root.join("payload.tar");
        let extract_root = root.join("extract");
        fs::create_dir_all(&root).unwrap();
        write_tar(&tar_path, |builder| {
            append_file(builder, "payload.txt", b"pulith")
        });

        let error = prepare(
            &tar_path,
            &extract_root,
            ArchivePolicy::new().max_decoded_bytes(512),
            ArchiveKind::Tar,
        )
        .unwrap_err();

        assert!(matches!(
            error,
            ArchiveError::ArchiveLimitExceeded {
                limit: "decoded-bytes",
                actual: 513,
                max: 512,
            }
        ));
        assert_eq!(fs::read_dir(&extract_root).unwrap().count(), 0);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn tar_prepare_rejects_entry_limit() {
        let root = temp_root("entry-limit");
        let tar_path = root.join("payload.tar");
        fs::create_dir_all(&root).unwrap();
        write_tar(&tar_path, |builder| {
            append_file(builder, "a.txt", b"a");
            append_file(builder, "b.txt", b"b");
        });

        let err = prepare(
            &tar_path,
            &root.join("extract"),
            ArchivePolicy::new().max_entries(1),
            ArchiveKind::Tar,
        )
        .unwrap_err();

        assert!(matches!(
            err,
            ArchiveError::ArchiveLimitExceeded {
                limit: "entry-count",
                ..
            }
        ));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn tar_prepare_rejects_parent_path() {
        let root = temp_root("path");
        let tar_path = root.join("payload.tar");
        fs::create_dir_all(&root).unwrap();
        write_tar(&tar_path, |builder| {
            append_file(builder, "safe.txt", b"nope")
        });
        patch_first_tar_path(&tar_path, b"../escape.txt");

        let err = prepare(
            &tar_path,
            &root.join("extract"),
            ArchivePolicy::default(),
            ArchiveKind::Tar,
        )
        .unwrap_err();

        assert!(matches!(err, ArchiveError::ArchiveInvalidPath(_)));
        assert!(!root.parent().unwrap().join("escape.txt").exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn tar_prepare_sanitizes_pax_path_overrides() {
        for path in ["../escape.txt", "/rooted.txt"] {
            let root = temp_root("pax-path");
            let tar_path = root.join("payload.tar");
            let extract_root = root.join("extract");
            fs::create_dir_all(&root).unwrap();
            write_tar(&tar_path, |builder| {
                builder
                    .append_pax_extensions([("path", path.as_bytes())])
                    .unwrap();
                append_file(builder, "safe.txt", b"nope");
            });

            let error = prepare(
                &tar_path,
                &extract_root,
                ArchivePolicy::default(),
                ArchiveKind::Tar,
            )
            .unwrap_err();

            assert!(matches!(error, ArchiveError::ArchiveInvalidPath(_)));
            assert_eq!(fs::read_dir(&extract_root).unwrap().count(), 0);
            fs::remove_dir_all(root).unwrap();
        }
    }

    #[cfg(windows)]
    #[test]
    fn tar_prepare_applies_windows_rules_to_pax_path_overrides() {
        for path in ["C:\\escape.txt", "NUL.txt", "payload.txt:stream"] {
            let root = temp_root("pax-windows-path");
            let tar_path = root.join("payload.tar");
            let extract_root = root.join("extract");
            fs::create_dir_all(&root).unwrap();
            write_tar(&tar_path, |builder| {
                builder
                    .append_pax_extensions([("path", path.as_bytes())])
                    .unwrap();
                append_file(builder, "safe.txt", b"nope");
            });

            let error = prepare(
                &tar_path,
                &extract_root,
                ArchivePolicy::default(),
                ArchiveKind::Tar,
            )
            .unwrap_err();

            assert!(matches!(error, ArchiveError::ArchiveInvalidPath(_)));
            assert_eq!(fs::read_dir(&extract_root).unwrap().count(), 0);
            fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn tar_prepare_rejects_symlink_entry() {
        let root = temp_root("symlink");
        let tar_path = root.join("payload.tar");
        fs::create_dir_all(&root).unwrap();
        write_tar(&tar_path, |builder| {
            append_symlink(builder, "link", "target")
        });

        let err = prepare(
            &tar_path,
            &root.join("extract"),
            ArchivePolicy::default(),
            ArchiveKind::Tar,
        )
        .unwrap_err();

        assert!(matches!(err, ArchiveError::UnsupportedArchiveEntry(_)));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn tar_prepare_rejects_byte_limit() {
        let root = temp_root("byte-limit");
        let tar_path = root.join("payload.tar");
        fs::create_dir_all(&root).unwrap();
        write_tar(&tar_path, |builder| append_file(builder, "a.txt", b"abcd"));

        let err = prepare(
            &tar_path,
            &root.join("extract"),
            ArchivePolicy::new().max_total_bytes(3),
            ArchiveKind::Tar,
        )
        .unwrap_err();

        assert!(matches!(
            err,
            ArchiveError::ArchiveLimitExceeded {
                limit: "total-bytes",
                ..
            }
        ));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn tar_prepare_rejects_duplicate_path() {
        let root = temp_root("duplicate-path");
        let tar_path = root.join("payload.tar");
        let extract_root = root.join("extract");
        fs::create_dir_all(&root).unwrap();
        write_tar(&tar_path, |builder| {
            append_file(builder, "payload.txt", b"first");
            append_file(builder, "payload.txt", b"second");
        });

        let err = prepare(
            &tar_path,
            &extract_root,
            ArchivePolicy::default(),
            ArchiveKind::Tar,
        )
        .unwrap_err();

        assert!(matches!(
            err,
            ArchiveError::ArchivePathConflict(path) if path == Path::new("payload.txt")
        ));
        assert_eq!(fs::read_dir(&extract_root).unwrap().count(), 0);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn tar_prepare_rejects_truncated_entry() {
        let root = temp_root("truncated-entry");
        let tar_path = root.join("payload.tar");
        let extract_root = root.join("extract");
        fs::create_dir_all(&root).unwrap();
        write_truncated_tar(&tar_path);

        let err = prepare(
            &tar_path,
            &extract_root,
            ArchivePolicy::default(),
            ArchiveKind::Tar,
        )
        .unwrap_err();

        assert!(matches!(
            err,
            ArchiveError::ArchiveSizeMismatch {
                path,
                declared: 8,
                observed: 4,
            } if path == Path::new("payload.txt")
        ));
        assert_eq!(fs::read_dir(&extract_root).unwrap().count(), 0);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn tar_prepare_rejects_truncated_terminal_entry_when_stripped() {
        let root = temp_root("truncated-stripped-entry");
        let tar_path = root.join("payload.tar");
        let extract_root = root.join("extract");
        fs::create_dir_all(&root).unwrap();
        write_truncated_tar(&tar_path);

        let error = prepare(
            &tar_path,
            &extract_root,
            ArchivePolicy::new().strip_components(1),
            ArchiveKind::Tar,
        )
        .unwrap_err();

        assert!(matches!(error, ArchiveError::Io { .. }), "{error:?}");
        assert_eq!(fs::read_dir(&extract_root).unwrap().count(), 0);
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(feature = "gzip")]
    #[test]
    fn tar_gzip_prepare_extracts_archive_tree() {
        let root = temp_root("gzip-extract");
        let tar_path = root.join("payload.tar.gz");
        let extract_root = root.join("extract");
        fs::create_dir_all(&root).unwrap();
        write_tar_gzip(&tar_path, |builder| {
            append_file(builder, "bin/tool.txt", b"pulith")
        });

        let evidence = prepare(
            &tar_path,
            &extract_root,
            ArchivePolicy::default(),
            ArchiveKind::TarGz,
        )
        .unwrap();

        assert_eq!(evidence.entries, 1);
        assert_eq!(evidence.files, 1);
        assert_eq!(
            fs::read_to_string(extract_root.join("bin/tool.txt")).unwrap(),
            "pulith"
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(feature = "gzip")]
    #[test]
    fn tar_gzip_prepare_rejects_a_raw_gzip_stream() {
        assert_raw_compressed_stream_rejected(
            "gzip-raw-stream",
            "gz",
            ArchiveKind::TarGz,
            |path| {
                write_gzip_bytes(path, b"not a tar archive");
            },
        );
    }

    #[cfg(feature = "gzip")]
    #[test]
    fn tar_gzip_prepare_rejects_parent_path() {
        let root = temp_root("gzip-path");
        let tar_path = root.join("payload.tar.gz");
        fs::create_dir_all(&root).unwrap();
        let mut bytes = tar_bytes(|builder| append_file(builder, "safe.txt", b"nope"));
        patch_first_tar_path_bytes(&mut bytes, b"../escape.txt");
        write_gzip_bytes(&tar_path, &bytes);

        let err = prepare(
            &tar_path,
            &root.join("extract"),
            ArchivePolicy::default(),
            ArchiveKind::TarGz,
        )
        .unwrap_err();

        assert!(matches!(err, ArchiveError::ArchiveInvalidPath(_)));
        assert!(!root.parent().unwrap().join("escape.txt").exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(feature = "gzip")]
    #[test]
    fn tar_gzip_prepare_rejects_byte_limit() {
        let root = temp_root("gzip-byte-limit");
        let tar_path = root.join("payload.tar.gz");
        fs::create_dir_all(&root).unwrap();
        write_tar_gzip(&tar_path, |builder| append_file(builder, "a.txt", b"abcd"));

        let err = prepare(
            &tar_path,
            &root.join("extract"),
            ArchivePolicy::new().max_total_bytes(3),
            ArchiveKind::TarGz,
        )
        .unwrap_err();

        assert!(matches!(
            err,
            ArchiveError::ArchiveLimitExceeded {
                limit: "total-bytes",
                ..
            }
        ));
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(feature = "gzip")]
    #[test]
    fn tar_gzip_prepare_limits_all_decoded_container_bytes() {
        let root = temp_root("gzip-decoded-limit");
        let tar_path = root.join("payload.tar.gz");
        let extract_root = root.join("extract");
        fs::create_dir_all(&root).unwrap();
        let payload = vec![0; 32 * 1024];
        write_tar_gzip(&tar_path, |builder| {
            append_file(builder, "stripped/payload.bin", &payload)
        });

        let err = prepare(
            &tar_path,
            &extract_root,
            ArchivePolicy::new()
                .strip_components(2)
                .max_decoded_bytes(1024),
            ArchiveKind::TarGz,
        )
        .unwrap_err();

        assert!(
            matches!(
                err,
                ArchiveError::ArchiveLimitExceeded {
                    limit: "decoded-bytes",
                    actual: 1025,
                    max: 1024,
                }
            ),
            "{err:?}"
        );
        assert_eq!(fs::read_dir(&extract_root).unwrap().count(), 0);
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(feature = "gzip")]
    #[test]
    fn tar_gzip_prepare_limits_hidden_pax_bytes() {
        let root = temp_root("gzip-pax-limit");
        let tar_path = root.join("payload.tar.gz");
        let extract_root = root.join("extract");
        fs::create_dir_all(&root).unwrap();
        let extension = vec![b'x'; 32 * 1024];
        write_tar_gzip(&tar_path, |builder| {
            builder
                .append_pax_extensions([("comment", extension.as_slice())])
                .unwrap();
            append_file(builder, "payload.txt", b"ok");
        });

        let err = prepare(
            &tar_path,
            &extract_root,
            ArchivePolicy::new().max_decoded_bytes(1024),
            ArchiveKind::TarGz,
        )
        .unwrap_err();

        assert!(matches!(
            err,
            ArchiveError::ArchiveLimitExceeded {
                limit: "decoded-bytes",
                actual: 1025,
                max: 1024,
            }
        ));
        assert_eq!(fs::read_dir(&extract_root).unwrap().count(), 0);
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(feature = "xz")]
    #[test]
    fn tar_xz_prepare_extracts_archive_tree() {
        let root = temp_root("xz-extract");
        let tar_path = root.join("payload.tar.xz");
        let extract_root = root.join("extract");
        fs::create_dir_all(&root).unwrap();
        write_tar_xz(&tar_path, |builder| {
            append_file(builder, "bin/tool.txt", b"pulith")
        });

        let evidence = prepare(
            &tar_path,
            &extract_root,
            ArchivePolicy::default(),
            ArchiveKind::TarXz,
        )
        .unwrap();

        assert_eq!(evidence.entries, 1);
        assert_eq!(
            fs::read_to_string(extract_root.join("bin/tool.txt")).unwrap(),
            "pulith"
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(feature = "xz")]
    #[test]
    fn tar_xz_prepare_rejects_a_raw_xz_stream() {
        assert_raw_compressed_stream_rejected("xz-raw-stream", "xz", ArchiveKind::TarXz, |path| {
            let file = File::create(path).unwrap();
            let mut encoder = xz2::write::XzEncoder::new(file, 6);
            encoder.write_all(b"not a tar archive").unwrap();
            encoder.finish().unwrap();
        });
    }

    #[cfg(feature = "zstd")]
    #[test]
    fn tar_zstd_prepare_extracts_archive_tree() {
        let root = temp_root("zstd-extract");
        let tar_path = root.join("payload.tar.zst");
        let extract_root = root.join("extract");
        fs::create_dir_all(&root).unwrap();
        write_tar_zstd(&tar_path, |builder| {
            append_file(builder, "bin/tool.txt", b"pulith")
        });

        let evidence = prepare(
            &tar_path,
            &extract_root,
            ArchivePolicy::default(),
            ArchiveKind::TarZstd,
        )
        .unwrap();

        assert_eq!(evidence.entries, 1);
        assert_eq!(
            fs::read_to_string(extract_root.join("bin/tool.txt")).unwrap(),
            "pulith"
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(feature = "zstd")]
    #[test]
    fn tar_zstd_prepare_rejects_a_raw_zstd_stream() {
        assert_raw_compressed_stream_rejected(
            "zstd-raw-stream",
            "zst",
            ArchiveKind::TarZstd,
            |path| {
                let file = File::create(path).unwrap();
                let mut encoder = zstd::stream::Encoder::new(file, 3).unwrap();
                encoder.write_all(b"not a tar archive").unwrap();
                encoder.finish().unwrap();
            },
        );
    }
    #[test]
    fn sniff_format_detects_archives_from_magic_and_plain_bytes() {
        let root = temp_root("sniff-format");
        fs::create_dir_all(&root).unwrap();

        let tar_path = root.join("tool.tar");
        write_tar(&tar_path, |builder| {
            let mut header = tar::Header::new_gnu();
            header.set_size(3);
            header.set_mode(0o644);
            header.set_cksum();
            builder
                .append_data(&mut header, "tool", b"bin".as_slice())
                .unwrap();
        });
        assert_eq!(
            ArchiveKind::sniff(&tar_path).unwrap(),
            Some(ArchiveKind::Tar)
        );

        let gz_path = root.join("tool.tar.gz");
        write_tar_gzip(&gz_path, |builder| {
            let mut header = tar::Header::new_gnu();
            header.set_size(3);
            header.set_mode(0o644);
            header.set_cksum();
            builder
                .append_data(&mut header, "tool", b"bin".as_slice())
                .unwrap();
        });
        assert_eq!(
            ArchiveKind::sniff(&gz_path).unwrap(),
            Some(ArchiveKind::TarGz)
        );

        #[cfg(feature = "xz")]
        {
            let xz_path = root.join("tool.tar.xz");
            write_tar_xz(&xz_path, |builder| {
                let mut header = tar::Header::new_gnu();
                header.set_size(3);
                header.set_mode(0o644);
                header.set_cksum();
                builder
                    .append_data(&mut header, "tool", b"bin".as_slice())
                    .unwrap();
            });
            assert_eq!(
                ArchiveKind::sniff(&xz_path).unwrap(),
                Some(ArchiveKind::TarXz)
            );
        }

        #[cfg(feature = "zstd")]
        {
            let zstd_path = root.join("tool.tar.zst");
            write_tar_zstd(&zstd_path, |builder| {
                let mut header = tar::Header::new_gnu();
                header.set_size(3);
                header.set_mode(0o644);
                header.set_cksum();
                builder
                    .append_data(&mut header, "tool", b"bin".as_slice())
                    .unwrap();
            });
            assert_eq!(
                ArchiveKind::sniff(&zstd_path).unwrap(),
                Some(ArchiveKind::TarZstd)
            );
        }

        let plain_path = root.join("plain.bin");
        fs::write(&plain_path, b"not an archive at all").unwrap();
        assert_eq!(ArchiveKind::sniff(&plain_path).unwrap(), None);

        assert_eq!(
            ArchiveKind::sniff(&root.join("missing.bin"))
                .unwrap_err()
                .kind(),
            io::ErrorKind::NotFound
        );
        fs::remove_dir_all(root).unwrap();
    }
}
