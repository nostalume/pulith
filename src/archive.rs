//! Archive preparation into an exclusive guarded scratch tree.
//!
//! This module connects a decoder's entry stream to pulith's custody contract. It owns entry-path
//! and entry-type policy, decoded and materialized limits, path-escape and collision guards, and
//! failure atomicity. It owns no format knowledge: the caller decides whether a stream is ZIP or
//! TAR and how it was compressed, then hands the reader in. Preparation writes into
//! caller-provided private custody and returns evidence; the final destination is never touched.
//! Feature-gated on `zip`/`tar`.
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
    /// The archive invalid path outcome.
    ArchiveInvalidPath(String),
    /// The archive limit exceeded outcome.
    ArchiveLimitExceeded {
        /// The limit value.
        limit: &'static str,
        /// The actual value.
        actual: u64,
        /// The max value.
        max: u64,
    },
    /// The archive size mismatch outcome.
    ArchiveSizeMismatch {
        /// The path value.
        path: PathBuf,
        /// The declared value.
        declared: u64,
        /// The observed value.
        observed: u64,
    },
    /// The archive path conflict outcome.
    ArchivePathConflict(PathBuf),
    /// Extraction failed and resetting the exclusive workspace also failed.
    ///
    /// `extraction` is the primary source. `cleanup` remains available for callers that need to
    /// detect a contaminated workspace.
    ArchiveCleanupFailed {
        /// The workspace value.
        workspace: PathBuf,
        /// The extraction value.
        extraction: Box<ArchiveError>,
        /// The cleanup value.
        cleanup: Box<ArchiveError>,
    },
    /// The unsupported archive entry outcome.
    UnsupportedArchiveEntry(PathBuf),
    /// The invalid preparation outcome.
    InvalidPreparation(String),
    /// The io outcome.
    Io {
        /// The action value.
        action: &'static str,
        /// The path value.
        path: PathBuf,
        /// The source value.
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
    /// The strip components value.
    pub strip_components: usize,
    /// The max entries value.
    pub max_entries: Option<u64>,
    /// The max entry bytes value.
    pub max_entry_bytes: Option<u64>,
    /// The max total bytes value.
    pub max_total_bytes: Option<u64>,
    /// The max decoded bytes value.
    pub max_decoded_bytes: Option<u64>,
}

impl ArchivePolicy {
    /// Constructs the default restrictive extraction policy without optional limits.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the number of leading path components removed from each archive entry.
    pub fn strip_components(mut self, strip_components: usize) -> Self {
        self.strip_components = strip_components;
        self
    }

    /// Sets the maximum number of materialized entries.
    pub fn max_entries(mut self, max_entries: u64) -> Self {
        self.max_entries = Some(max_entries);
        self
    }

    /// Sets the maximum materialized size of one regular-file entry.
    pub fn max_entry_bytes(mut self, max_entry_bytes: u64) -> Self {
        self.max_entry_bytes = Some(max_entry_bytes);
        self
    }

    /// Sets the maximum total materialized regular-file bytes.
    pub fn max_total_bytes(mut self, max_total_bytes: u64) -> Self {
        self.max_total_bytes = Some(max_total_bytes);
        self
    }

    /// Sets the maximum decoded container bytes consumed while preparing the archive.
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

/// Evidence observed while preparing an archive tree.
///
/// `total_bytes` counts materialized regular-file bytes. `decoded_bytes` counts the bytes pulled
/// from the decoder stream the caller supplied: for TAR that includes headers, padding,
/// extensions, and stripped entries; for ZIP, whose entries are decoded individually, it equals
/// `total_bytes`.
#[non_exhaustive]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ArchiveEvidence {
    /// The root value.
    pub root: PathBuf,
    /// The entries value.
    pub entries: u64,
    /// The total bytes value.
    pub total_bytes: u64,
    /// The decoded bytes value.
    pub decoded_bytes: u64,
    /// The files value.
    pub files: u64,
    /// The directories value.
    pub directories: u64,
    /// The symlinks value.
    pub symlinks: u64,
}

impl ArchiveEvidence {
    fn empty(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            entries: 0,
            total_bytes: 0,
            decoded_bytes: 0,
            files: 0,
            directories: 0,
            symlinks: 0,
        }
    }
}

/// Shared driver: reset the exclusive root, extract, clean up on failure, and pair the prepared
/// tree with its attestation.
fn prepare_into(
    root: &Path,
    policy: ArchivePolicy,
    extract: impl FnOnce(&Path, &ArchivePolicy) -> Result<ArchiveEvidence, ArchiveError>,
) -> Result<ArchiveEvidence, ArchiveError> {
    let root = root.to_path_buf();
    reset_extract_root(&root)?;
    match extract(&root, &policy) {
        Ok(evidence) => Ok(evidence),
        Err(error) => {
            let cleanup = reset_extract_root(&root);
            Err(combine_archive_failure(&root, error, cleanup))
        }
    }
}

/// Prepares a TAR stream into caller-owned exclusive custody.
///
/// The caller owns the encoding. Pass a `File` for plain TAR, or any reader that yields TAR bytes
/// (`flate2::read::GzDecoder`, `xz2::read::XzDecoder`, `zstd::stream::Decoder`) for a compressed
/// stream; this function takes no position on how those bytes were produced. `max_decoded_bytes`
/// bounds the bytes pulled through `reader`, so a decompression bomb is stopped before it reaches
/// the entry parser.
#[cfg(feature = "tar")]
pub fn prepare_tar<R: Read>(
    reader: R,
    root: &Path,
    policy: ArchivePolicy,
) -> Result<ArchiveEvidence, ArchiveError> {
    prepare_into(root, policy, |root, policy| {
        extract_tar_reader(reader, root, policy)
    })
}

/// Prepares a ZIP stream into caller-owned exclusive custody.
///
/// The stream is parsed by `zip`: entry names, entry types, and header consistency are its
/// knowledge. Path policy, collision policy, limits, evidence, and failure atomicity are this
/// module's.
#[cfg(feature = "zip")]
pub fn prepare_zip<R: Read + io::Seek>(
    reader: R,
    root: &Path,
    policy: ArchivePolicy,
) -> Result<ArchiveEvidence, ArchiveError> {
    prepare_into(root, policy, |root, policy| {
        extract_zip(reader, root, policy)
    })
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
fn extract_zip<R: Read + io::Seek>(
    reader: R,
    root: &Path,
    policy: &ArchivePolicy,
) -> Result<ArchiveEvidence, ArchiveError> {
    let mut archive = zip::ZipArchive::new(reader)
        .map_err(|err| ArchiveError::InvalidPreparation(format!("invalid zip archive: {err}")))?;
    // Delegate header validation to `zip`: it rejects duplicate names, local/central mismatches,
    // overlapping entries, and local headers missing from the central directory. Entries that
    // defer sizes to a trailing data descriptor are still accepted.
    archive
        .validate_header_consistency()
        .map_err(|err| ArchiveError::InvalidPreparation(format!("invalid zip archive: {err}")))?;
    let mut evidence = ArchiveEvidence::empty(root);
    let mut paths = ArchivePathIndex::default();

    for index in 0..archive.len() {
        evidence.entries += 1;
        check_limit("entry-count", evidence.entries, policy.max_entries)?;

        let mut file = archive
            .by_index(index)
            .map_err(|err| ArchiveError::InvalidPreparation(format!("invalid zip entry: {err}")))?;
        let name = file
            .name()
            .map_err(|err| {
                ArchiveError::InvalidPreparation(format!("invalid zip entry name: {err}"))
            })?
            .into_owned();
        let enclosed = file
            .enclosed_name()
            .ok_or(ArchiveError::ArchiveInvalidPath(name))?;
        let Some(relative) = sanitize_relative(&enclosed, policy.strip_components)? else {
            continue;
        };
        let target = root.join(&relative);
        ensure_under_root(root, &target)?;
        reject_existing_symlink_path(root, &target)?;

        if file.is_symlink() {
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
fn extract_tar_reader<R: Read>(
    reader: R,
    root: &Path,
    policy: &ArchivePolicy,
) -> Result<ArchiveEvidence, ArchiveError> {
    let reader = DecodedLimitReader::new(reader, policy.max_decoded_bytes);
    let mut archive = tar::Archive::new(reader);
    let mut evidence = ArchiveEvidence::empty(root);
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
    // A decode failure reports corrupt archive bytes, not host I/O on `path`. Classify it as an
    // invalid archive so callers do not retry or blame the filesystem for a malformed input.
    if error.kind() == io::ErrorKind::InvalidData {
        return ArchiveError::InvalidPreparation(format!("{action} {}: {error}", path.display()));
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

#[cfg(unix)]
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

#[cfg(all(test, feature = "zip"))]
mod tests {
    use std::error::Error as _;
    use std::fs::{self, File};
    use std::io::{self, Write};
    use std::path::{Path, PathBuf};

    use super::{
        ArchiveEntryKind, ArchiveError, ArchivePathIndex, ArchivePolicy, combine_archive_failure,
    };

    fn prepare(
        source: &Path,
        workspace: &Path,
        policy: ArchivePolicy,
    ) -> Result<super::ArchiveEvidence, ArchiveError> {
        super::prepare_zip(File::open(source).unwrap(), workspace, policy)
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

        let evidence = prepare(&zip_path, &extract_root, ArchivePolicy::default()).unwrap();

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

        prepare(&zip_path, &extract_root, ArchivePolicy::default()).unwrap();

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

        let err = prepare(&zip_path, &root.join("extract"), ArchivePolicy::default()).unwrap_err();

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

        let err = prepare(&zip_path, &root.join("extract"), ArchivePolicy::default()).unwrap_err();

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
    fn zip_prepare_rejects_local_header_mismatch() {
        let root = temp_root("local-header-mismatch");
        let zip_path = root.join("payload.zip");
        let extract_root = root.join("extract");
        fs::create_dir_all(&root).unwrap();
        write_zip(&zip_path, &[("payload.txt", b"12345678")]);

        // Rewrite only the local header's uncompressed size; the central directory still says 8.
        // Iterating entries alone would read the real 8 bytes and accept the archive.
        let mut bytes = fs::read(&zip_path).unwrap();
        let local = bytes
            .windows(4)
            .position(|window| window == [0x50, 0x4b, 0x03, 0x04])
            .unwrap();
        bytes[local + 22..local + 26].copy_from_slice(&3u32.to_le_bytes());
        fs::write(&zip_path, bytes).unwrap();

        let err = prepare(&zip_path, &extract_root, ArchivePolicy::default()).unwrap_err();

        assert!(
            matches!(
                &err,
                ArchiveError::InvalidPreparation(message) if message.contains("header inconsistency")
            ),
            "unexpected error: {err:?}"
        );
        assert_eq!(fs::read_dir(&extract_root).unwrap().count(), 0);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn zip_prepare_rejects_entry_larger_than_declared_size() {
        let root = temp_root("observed-byte-limit");
        let zip_path = root.join("payload.zip");
        let extract_root = root.join("extract");
        fs::create_dir_all(&root).unwrap();
        write_zip_with_understated_size(&zip_path);

        let err = prepare(
            &zip_path,
            &extract_root,
            ArchivePolicy::new().max_total_bytes(4),
        )
        .unwrap_err();

        // `zip` 9 rejects the entry while reading, before the declared-total guard is reached, so
        // the outcome is an invalid archive rather than a limit breach. The law that matters is
        // unchanged: the workspace is left clean.
        assert!(
            matches!(
                &err,
                ArchiveError::InvalidPreparation(message)
                    if message.contains("payload.txt") && message.contains("declared")
            ),
            "unexpected error: {err:?}"
        );
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

        let err = prepare(&zip_path, &extract_root, ArchivePolicy::default()).unwrap_err();

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

        let err = prepare(&zip_path, &extract_root, ArchivePolicy::default()).unwrap_err();

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

            let err = prepare(&zip_path, &extract_root, ArchivePolicy::default()).unwrap_err();

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
}

#[cfg(all(test, feature = "tar"))]
mod tar_tests {
    use std::fs::{self, File};
    use std::io::{Cursor, Write};
    use std::path::{Path, PathBuf};

    use super::{ArchiveError, ArchivePolicy};

    /// The test plays the caller: it owns the decoder choice, exactly as a real caller does.
    #[derive(Clone, Copy)]
    enum Codec {
        Plain,
        Gzip,
    }

    fn prepare(
        source: &Path,
        workspace: &Path,
        policy: ArchivePolicy,
        codec: Codec,
    ) -> Result<super::ArchiveEvidence, ArchiveError> {
        let file = File::open(source).unwrap();
        match codec {
            Codec::Plain => super::prepare_tar(file, workspace, policy),
            Codec::Gzip => {
                super::prepare_tar(flate2::read::GzDecoder::new(file), workspace, policy)
            }
        }
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

    fn tar_bytes(build: impl FnOnce(&mut tar::Builder<Vec<u8>>)) -> Vec<u8> {
        let mut builder = tar::Builder::new(Vec::new());
        build(&mut builder);
        builder.into_inner().unwrap()
    }

    fn write_tar_gzip(path: &Path, build: impl FnOnce(&mut tar::Builder<Vec<u8>>)) {
        write_gzip_bytes(path, &tar_bytes(build));
    }

    fn write_gzip_bytes(path: &Path, bytes: &[u8]) {
        let file = File::create(path).unwrap();
        let mut encoder = flate2::write::GzEncoder::new(file, flate2::Compression::default());
        encoder.write_all(bytes).unwrap();
        encoder.finish().unwrap();
    }

    /// A compressed stream is not an entry stream: the caller must decode before handing it in.
    fn assert_raw_compressed_stream_rejected(
        name: &str,
        extension: &str,
        write: impl FnOnce(&Path),
    ) {
        let root = temp_root(name);
        let source = root.join(format!("payload.{extension}"));
        let extract_root = root.join("extract");
        fs::create_dir_all(&root).unwrap();
        write(&source);

        assert!(
            prepare(
                &source,
                &extract_root,
                ArchivePolicy::default(),
                Codec::Gzip
            )
            .is_err()
        );
        assert_eq!(fs::read_dir(&extract_root).unwrap().count(), 0);
        fs::remove_dir_all(root).unwrap();
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
            Codec::Plain,
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
            Codec::Plain,
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
            Codec::Plain,
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
            Codec::Plain,
        )
        .unwrap()
        .decoded_bytes;

        prepare(
            &tar_path,
            &root.join("exact"),
            ArchivePolicy::new().max_decoded_bytes(measured),
            Codec::Plain,
        )
        .unwrap();

        let below_root = root.join("below");
        let error = prepare(
            &tar_path,
            &below_root,
            ArchivePolicy::new().max_decoded_bytes(measured - 1),
            Codec::Plain,
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
            Codec::Plain,
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
            Codec::Plain,
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
            Codec::Plain,
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
                Codec::Plain,
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
                Codec::Plain,
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
            Codec::Plain,
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
            Codec::Plain,
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
            Codec::Plain,
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
            Codec::Plain,
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
            Codec::Plain,
        )
        .unwrap_err();

        assert!(matches!(error, ArchiveError::Io { .. }), "{error:?}");
        assert_eq!(fs::read_dir(&extract_root).unwrap().count(), 0);
        fs::remove_dir_all(root).unwrap();
    }

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
            Codec::Gzip,
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
    fn tar_gzip_prepare_rejects_a_raw_gzip_stream() {
        assert_raw_compressed_stream_rejected("gzip-raw-stream", "gz", |path| {
            write_gzip_bytes(path, b"not a tar archive");
        });
    }

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
            Codec::Gzip,
        )
        .unwrap_err();

        assert!(matches!(err, ArchiveError::ArchiveInvalidPath(_)));
        assert!(!root.parent().unwrap().join("escape.txt").exists());
        fs::remove_dir_all(root).unwrap();
    }

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
            Codec::Gzip,
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
            Codec::Gzip,
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
            Codec::Gzip,
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
}
