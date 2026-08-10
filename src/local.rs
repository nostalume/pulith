//! Local adapter facade: acquire, staged apply, activation, inspection, and pure reconciliation.
//!
//! This module owns the local filesystem semantics: entry-kind classification without following
//! links, private staged-tree custody, single-target publication, create-only activation, and the
//! explicit active-view switch. It never claims package ownership, durable manager state, or
//! automatic repair. Concrete publication lives in `local/apply.rs` and activation in
//! `local/view.rs`; this facade re-exports their public vocabulary. The whole module is
//! feature-gated on `local`.
use std::fmt;
use std::fs;
#[cfg(any(feature = "blake3", feature = "sha2"))]
use std::fs::File;
use std::io;
use std::path::{Path, PathBuf};

use crate::{Acquire, Inspect, Reconcile};

mod apply;
#[cfg(any(feature = "zip", feature = "tar"))]
mod materialize;
mod record;
mod view;

pub use apply::{ApplyEvidence, RemoveEvidence};
#[cfg(any(feature = "zip", feature = "tar"))]
pub use materialize::{MaterializeError, PreparationEvidence};
pub use record::{
    RecordChange, RecordError, RecordEvidence, RecordLimit, RecordObservation, RecordStore,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LinkChange {
    Created,
    Replaced,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LinkEvidence {
    pub source: PathBuf,
    pub view: PathBuf,
    pub change: LinkChange,
}

#[derive(Debug)]
pub enum LinkError {
    BeforeLink {
        view: PathBuf,
        cause: LocalError,
    },
    InvalidExpose {
        expose: PathBuf,
    },
    ExposeNotDirectory {
        path: PathBuf,
        observed: LocalObservation,
    },
    ViewConflict {
        view: PathBuf,
        observed: LocalObservation,
    },
    CapabilityUnavailable {
        view: PathBuf,
        cause: LocalError,
    },
}

impl fmt::Display for LinkError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BeforeLink { cause, .. } => write!(formatter, "link failed: {cause}"),
            Self::InvalidExpose { expose } => write!(
                formatter,
                "expose path {} must be a relative, non-escaping subpath",
                expose.display()
            ),
            Self::ExposeNotDirectory { path, .. } => write!(
                formatter,
                "expose path {} is not a directory",
                path.display()
            ),
            Self::ViewConflict { view, observed } => write!(
                formatter,
                "view {} holds {observed:?}, which is not a directory-symlink view; nothing replaced",
                view.display()
            ),
            Self::CapabilityUnavailable { cause, .. } => write!(
                formatter,
                "directory symlink activation is unavailable: {cause}"
            ),
        }
    }
}

impl std::error::Error for LinkError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::BeforeLink { cause, .. } | Self::CapabilityUnavailable { cause, .. } => {
                Some(cause)
            }
            _ => None,
        }
    }
}

#[derive(Debug)]
pub enum UnlinkError {
    Observe {
        view: PathBuf,
        cause: LocalError,
    },
    NotActiveView {
        view: PathBuf,
        observed: LocalObservation,
    },
    Remove {
        view: PathBuf,
        cause: LocalError,
    },
}

impl fmt::Display for UnlinkError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Observe { view, .. } => {
                write!(formatter, "failed to observe view {}", view.display())
            }
            Self::NotActiveView { view, observed } => write!(
                formatter,
                "view {} holds {observed:?}, which is not an active view",
                view.display()
            ),
            Self::Remove { view, .. } => {
                write!(formatter, "failed to remove active view {}", view.display())
            }
        }
    }
}

impl std::error::Error for UnlinkError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Observe { cause, .. } | Self::Remove { cause, .. } => Some(cause),
            Self::NotActiveView { .. } => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UnlinkChange {
    Removed,
    Unchanged,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UnlinkEvidence {
    pub view: PathBuf,
    pub change: UnlinkChange,
}

/// Errors produced by local acquisition, observation, activation, and publication behaviors.
#[non_exhaustive]
#[derive(Debug)]
pub enum LocalError {
    MissingSource(PathBuf),
    /// Immutable publication found an existing predecessor and did not commit the target.
    AlreadyPublished(PathBuf),
    UnsupportedLocalEntry(PathBuf),
    InvalidStagePath(PathBuf),
    PublishUnavailable {
        path: PathBuf,
        source: io::Error,
    },
    InvalidPreparation(String),
    /// The source law: the empty path is not a valid source reference.
    InvalidSource(PathBuf),
    /// The target law: the empty path is not a valid target reference.
    InvalidTarget(PathBuf),
    Io {
        action: &'static str,
        path: PathBuf,
        source: io::Error,
    },
}

impl LocalError {
    pub(crate) fn io(action: &'static str, path: impl AsRef<Path>, source: io::Error) -> Self {
        Self::Io {
            action,
            path: path.as_ref().to_path_buf(),
            source,
        }
    }
}

impl fmt::Display for LocalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingSource(path) => write!(f, "source does not exist: {}", path.display()),
            Self::AlreadyPublished(path) => {
                write!(f, "target is already published: {}", path.display())
            }
            Self::UnsupportedLocalEntry(path) => {
                write!(
                    f,
                    "local filesystem entry is unsupported: {}",
                    path.display()
                )
            }
            Self::InvalidStagePath(path) => {
                write!(
                    f,
                    "stage destination must be a contained relative path: {}",
                    path.display()
                )
            }
            Self::PublishUnavailable { path, source } => {
                write!(
                    f,
                    "atomic no-replace publication is unavailable for {}: {source}",
                    path.display()
                )
            }
            Self::InvalidPreparation(message) => write!(f, "invalid preparation: {message}"),
            Self::InvalidSource(path) => {
                write!(f, "invalid source (empty path): {}", path.display())
            }
            Self::InvalidTarget(path) => {
                write!(f, "invalid target (empty path): {}", path.display())
            }
            Self::Io {
                action,
                path,
                source,
            } => {
                write!(f, "failed to {action} {}: {source}", path.display())
            }
        }
    }
}

impl std::error::Error for LocalError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } | Self::PublishUnavailable { source, .. } => Some(source),
            _ => None,
        }
    }
}

#[cfg(any(feature = "blake3", feature = "sha2"))]
pub(crate) enum OpenedLocalArtifact {
    Missing,
    File(File),
    Directory,
    Symlink,
    #[cfg(windows)]
    Reparse,
    Other,
}

#[cfg(all(unix, any(feature = "blake3", feature = "sha2")))]
pub(crate) fn open_local_artifact(path: &Path) -> Result<OpenedLocalArtifact, LocalError> {
    use rustix::fs::{Mode, OFlags, open};
    use rustix::io::Errno;

    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            return Ok(OpenedLocalArtifact::Symlink);
        }
        Ok(metadata) if metadata.is_dir() => return Ok(OpenedLocalArtifact::Directory),
        Ok(metadata) if !metadata.is_file() => return Ok(OpenedLocalArtifact::Other),
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok(OpenedLocalArtifact::Missing);
        }
        Err(error) => return Err(LocalError::io("inspect exact local artifact", path, error)),
    }
    let descriptor = match open(
        path,
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK,
        Mode::empty(),
    ) {
        Ok(descriptor) => descriptor,
        Err(Errno::NOENT) => return Ok(OpenedLocalArtifact::Missing),
        Err(Errno::LOOP) => return Ok(OpenedLocalArtifact::Symlink),
        Err(error) => {
            return Err(LocalError::io(
                "open exact local artifact",
                path,
                io::Error::from(error),
            ));
        }
    };
    let file = File::from(descriptor);
    let metadata = file
        .metadata()
        .map_err(|error| LocalError::io("inspect exact local artifact", path, error))?;
    if metadata.is_file() {
        Ok(OpenedLocalArtifact::File(file))
    } else if metadata.is_dir() {
        Ok(OpenedLocalArtifact::Directory)
    } else {
        Ok(OpenedLocalArtifact::Other)
    }
}

#[cfg(all(windows, any(feature = "blake3", feature = "sha2")))]
pub(crate) fn open_local_artifact(path: &Path) -> Result<OpenedLocalArtifact, LocalError> {
    use std::mem::MaybeUninit;
    use std::os::windows::fs::OpenOptionsExt;
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Foundation::{ERROR_SUCCESS, GetLastError, SetLastError};
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_REPARSE_POINT, FILE_ATTRIBUTE_TAG_INFO,
        FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_SHARE_DELETE,
        FILE_SHARE_READ, FILE_SHARE_WRITE, FileAttributeTagInfo, GetFileInformationByHandleEx,
        GetFileType,
    };
    use windows_sys::Win32::System::SystemServices::IO_REPARSE_TAG_SYMLINK;

    let file = match File::options()
        .read(true)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS)
        .open(path)
    {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok(OpenedLocalArtifact::Missing);
        }
        Err(error) => return Err(LocalError::io("open exact local artifact", path, error)),
    };
    let mut information = MaybeUninit::<FILE_ATTRIBUTE_TAG_INFO>::zeroed();
    // SAFETY: `file` owns a valid handle and `information` is correctly sized writable storage.
    let succeeded = unsafe {
        GetFileInformationByHandleEx(
            file.as_raw_handle(),
            FileAttributeTagInfo,
            information.as_mut_ptr().cast(),
            std::mem::size_of::<FILE_ATTRIBUTE_TAG_INFO>() as u32,
        )
    };
    if succeeded == 0 {
        return Err(LocalError::io(
            "inspect exact local artifact",
            path,
            io::Error::last_os_error(),
        ));
    }
    // SAFETY: GetFileInformationByHandleEx succeeded and initialized the structure.
    let information = unsafe { information.assume_init() };
    if information.FileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return if information.ReparseTag == IO_REPARSE_TAG_SYMLINK {
            Ok(OpenedLocalArtifact::Symlink)
        } else {
            Ok(OpenedLocalArtifact::Reparse)
        };
    }
    if information.FileAttributes & FILE_ATTRIBUTE_DIRECTORY != 0 {
        return Ok(OpenedLocalArtifact::Directory);
    }
    // SAFETY: setting and reading the calling thread's last-error slot surrounds one file query.
    unsafe { SetLastError(ERROR_SUCCESS) };
    // SAFETY: `file` owns a valid handle for the duration of the query.
    let file_type = unsafe { GetFileType(file.as_raw_handle()) };
    // SAFETY: GetLastError reads the calling thread's last-error slot.
    let last_error = unsafe { GetLastError() };
    if !classify_windows_file_type(file_type, last_error)
        .map_err(|error| LocalError::io("inspect exact local artifact", path, error))?
    {
        return Ok(OpenedLocalArtifact::Other);
    }
    Ok(OpenedLocalArtifact::File(file))
}

#[cfg(all(windows, any(feature = "blake3", feature = "sha2")))]
fn classify_windows_file_type(file_type: u32, last_error: u32) -> io::Result<bool> {
    use windows_sys::Win32::Foundation::ERROR_SUCCESS;
    use windows_sys::Win32::Storage::FileSystem::{FILE_TYPE_DISK, FILE_TYPE_UNKNOWN};

    if file_type == FILE_TYPE_UNKNOWN && last_error != ERROR_SUCCESS {
        Err(io::Error::from_raw_os_error(last_error as i32))
    } else {
        Ok(file_type == FILE_TYPE_DISK)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LocalAcquireEvidence {
    pub path: PathBuf,
}

/// A local acquisition source: a non-empty filesystem reference (restricted law newtype —
/// carries the acquire behavior; the pure-data law is enforced here, I/O laws stay in acquire).
pub struct LocalSource(PathBuf);

impl LocalSource {
    /// Admit one non-empty local path. Relative/absolute and `..` are allowed (an input
    /// reference — the caller's own filesystem); only the empty path is refused.
    pub fn new(path: impl Into<PathBuf>) -> Result<Self, LocalError> {
        let path = path.into();
        if path.as_os_str().is_empty() {
            return Err(LocalError::InvalidSource(path));
        }
        Ok(Self(path))
    }

    pub fn as_path(&self) -> &Path {
        &self.0
    }
}

impl Acquire for LocalSource {
    type Error = LocalError;
    type Output = LocalMaterial;

    fn acquire(self) -> Result<Self::Output, Self::Error> {
        let path = self.0;
        if !path.exists() {
            return Err(LocalError::MissingSource(path));
        }
        Ok(if path.is_dir() {
            LocalMaterial::Directory { path }
        } else {
            LocalMaterial::File { path }
        })
    }
}

/// A local target path for observation, activation, or removal (restricted law newtype —
/// non-empty; I/O laws stay in each behavior).
pub struct LocalTarget(PathBuf);

impl LocalTarget {
    pub fn new(path: impl Into<PathBuf>) -> Result<Self, LocalError> {
        let path = path.into();
        if path.as_os_str().is_empty() {
            return Err(LocalError::InvalidTarget(path));
        }
        Ok(Self(path))
    }

    pub fn as_path(&self) -> &Path {
        &self.0
    }

    /// Creates eager private custody beside this target without publishing it.
    pub fn stage(&self) -> Result<StagedTree, LocalError> {
        apply::stage(self.as_path())
    }
}

/// Local material with explicit caller-owned or adapter-owned custody.
///
/// `File` and `Directory` paths survive drop. `StagedFile` owns its temporary path and removes it
/// when the material or any canonical state carrying it is dropped.
#[derive(Debug)]
pub enum LocalMaterial {
    /// A caller-owned regular-file path. Dropping the value does not remove the file.
    File { path: PathBuf },
    /// A caller-owned directory path. Dropping the value does not remove the tree.
    Directory { path: PathBuf },
    /// An adapter-owned temporary file. Dropping the value removes the stage.
    StagedFile { path: tempfile::TempPath },
}

impl PartialEq for LocalMaterial {
    fn eq(&self, other: &Self) -> bool {
        matches!(
            (self, other),
            (Self::File { path: left }, Self::File { path: right })
                | (Self::Directory { path: left }, Self::Directory { path: right })
                if left == right
        ) || matches!(
            (self, other),
            (Self::StagedFile { path: left }, Self::StagedFile { path: right })
                if <tempfile::TempPath as AsRef<Path>>::as_ref(left)
                    == <tempfile::TempPath as AsRef<Path>>::as_ref(right)
        )
    }
}

impl Eq for LocalMaterial {}

impl LocalMaterial {
    /// Returns the material path without changing its caller-owned or staged custody.
    pub fn path(&self) -> &Path {
        match self {
            Self::File { path } | Self::Directory { path } => path,
            Self::StagedFile { path } => path.as_ref(),
        }
    }
}

/// Adapter-owned temporary directory custody for one selected local output tree.
///
/// The workspace is removed when this value is dropped. Callers can inspect the selected root but
/// cannot construct or clone the custody value.
#[derive(Debug)]
pub struct StagedTree {
    workspace: tempfile::TempDir,
    root: PathBuf,
    placement: StagePlacement,
}

#[derive(Debug)]
enum StagePlacement {
    #[cfg(feature = "process")]
    Scratch,
    Destination(PathBuf),
}

impl StagedTree {
    #[cfg(feature = "process")]
    pub(crate) fn new(workspace: tempfile::TempDir, root: PathBuf) -> Self {
        Self {
            workspace,
            root,
            placement: StagePlacement::Scratch,
        }
    }

    pub(crate) fn destination(
        workspace: tempfile::TempDir,
        root: PathBuf,
        parent: PathBuf,
    ) -> Self {
        Self {
            workspace,
            root,
            placement: StagePlacement::Destination(parent),
        }
    }

    /// Returns the selected tree root by shared reference.
    pub fn root(&self) -> &Path {
        debug_assert!(self.root.starts_with(self.workspace.path()));
        &self.root
    }
}
/// No-follow local filesystem entry classification.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LocalEntryKind {
    Missing,
    File,
    Directory,
    Symlink,
    Reparse,
    Other,
}

/// No-follow local artifact facts with an optional regular-file attestation.
///
/// The exact-inspection behavior supplies `T` only after it has admitted and read a regular file.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LocalArtifactObservation<T> {
    Missing,
    File { attestation: T },
    Directory,
    Symlink,
    Reparse,
    Other,
}

impl<T> LocalArtifactObservation<T> {
    pub fn kind(&self) -> LocalEntryKind {
        match self {
            Self::Missing => LocalEntryKind::Missing,
            Self::File { .. } => LocalEntryKind::File,
            Self::Directory => LocalEntryKind::Directory,
            Self::Symlink => LocalEntryKind::Symlink,
            Self::Reparse => LocalEntryKind::Reparse,
            Self::Other => LocalEntryKind::Other,
        }
    }
}

/// Read-only facts observed for one [`PathBuf`].
///
/// A symlink is classified by its resolved target: `SymlinkToDirectory` when the link's
/// (possibly relative) target resolves to a directory, `SymlinkToFile` when it resolves to a
/// file or is missing/unreadable (not a directory). This is the single home of the
/// link-target classification — callers never read links themselves.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LocalObservation {
    Missing,
    File { bytes: u64 },
    Directory,
    SymlinkToDirectory,
    SymlinkToFile,
    Other,
}

/// The observation law's receipt: the observed path.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LocalInspectEvidence {
    pub path: PathBuf,
}

impl LocalObservation {
    pub fn kind(&self) -> LocalEntryKind {
        match self {
            Self::Missing => LocalEntryKind::Missing,
            Self::File { .. } => LocalEntryKind::File,
            Self::Directory => LocalEntryKind::Directory,
            Self::SymlinkToDirectory | Self::SymlinkToFile => LocalEntryKind::Symlink,
            Self::Other => LocalEntryKind::Other,
        }
    }
}

/// Caller-owned expected state compared by the [`Reconcile`] trait.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LocalExpectation {
    Missing,
    File,
    FileSize(u64),
    Directory,
    Symlink,
    Other,
}

impl LocalExpectation {
    pub fn kind(&self) -> LocalEntryKind {
        match self {
            Self::Missing => LocalEntryKind::Missing,
            Self::File | Self::FileSize(_) => LocalEntryKind::File,
            Self::Directory => LocalEntryKind::Directory,
            Self::Symlink => LocalEntryKind::Symlink,
            Self::Other => LocalEntryKind::Other,
        }
    }
}

/// Difference between caller-owned local expectation and observed state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LocalReconciliation {
    Matches,
    Missing,
    Unexpected,
    WrongKind {
        expected: LocalEntryKind,
        observed: LocalEntryKind,
    },
    Modified {
        expected_bytes: u64,
        observed_bytes: u64,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LocalReconcileEvidence {
    pub expected: LocalExpectation,
    pub observed: LocalObservation,
}

/// The observation law: classify one path (the single home of the classification).
///
/// A symlink is classified by its resolved target (`SymlinkToDirectory`/`SymlinkToFile`); a
/// missing path is `Missing`. `Inspect` builds its receipt node from this, so the observation
/// is never produced by discarding a node's field.
pub(crate) fn observe_path(path: &Path) -> Result<LocalObservation, LocalError> {
    Ok(match fs::symlink_metadata(path) {
        Ok(metadata) => {
            let file_type = metadata.file_type();
            if file_type.is_symlink() {
                classify_symlink(path)
            } else if file_type.is_file() {
                LocalObservation::File {
                    bytes: metadata.len(),
                }
            } else if file_type.is_dir() {
                LocalObservation::Directory
            } else {
                LocalObservation::Other
            }
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => LocalObservation::Missing,
        Err(error) => return Err(LocalError::io("inspect local target", path, error)),
    })
}

impl Inspect<()> for LocalTarget {
    type Error = LocalError;
    type Output = (LocalObservation, LocalInspectEvidence);

    fn inspect(self, (): ()) -> Result<Self::Output, Self::Error> {
        let path = self.0;
        let observation = observe_path(&path)?;
        Ok((observation, LocalInspectEvidence { path }))
    }
}

/// Classify a symlink by its resolved target: `SymlinkToDirectory` when the (possibly relative)
/// target resolves to a directory, `SymlinkToFile` otherwise (file target, dangling, or an
/// unreadable link). This is the single home of the link-target classification; the no-follow
/// metadata already confirmed the entry is a symlink.
fn classify_symlink(path: &Path) -> LocalObservation {
    match fs::read_link(path) {
        Ok(target) => {
            let resolved = if target.is_absolute() {
                target
            } else {
                path.parent().unwrap_or_else(|| Path::new("")).join(&target)
            };
            match fs::metadata(&resolved) {
                Ok(metadata) if metadata.is_dir() => LocalObservation::SymlinkToDirectory,
                _ => LocalObservation::SymlinkToFile,
            }
        }
        Err(_) => LocalObservation::SymlinkToFile,
    }
}

/// Pure local expected/observed comparison; it never mutates the target. The observation is
/// the impl caller: `LocalObservation::reconcile(expected)`.
impl Reconcile<LocalExpectation> for LocalObservation {
    type Error = std::convert::Infallible;
    type Output = (LocalReconciliation, LocalReconcileEvidence);

    fn reconcile(self, expected: LocalExpectation) -> Result<Self::Output, Self::Error> {
        let reconciliation = match (&expected, &self) {
            (LocalExpectation::Missing, LocalObservation::Missing) => LocalReconciliation::Matches,
            (LocalExpectation::Missing, _) => LocalReconciliation::Unexpected,
            (_, LocalObservation::Missing) => LocalReconciliation::Missing,
            (LocalExpectation::FileSize(expected_bytes), LocalObservation::File { bytes })
                if expected_bytes != bytes =>
            {
                LocalReconciliation::Modified {
                    expected_bytes: *expected_bytes,
                    observed_bytes: *bytes,
                }
            }
            _ if expected.kind() == self.kind() => LocalReconciliation::Matches,
            _ => LocalReconciliation::WrongKind {
                expected: expected.kind(),
                observed: self.kind(),
            },
        };
        Ok((
            reconciliation,
            LocalReconcileEvidence {
                expected,
                observed: self,
            },
        ))
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    #[cfg(unix)]
    use std::os::unix::fs::symlink as symlink_file;
    #[cfg(windows)]
    use std::os::windows::fs::symlink_file;

    use super::*;
    use crate::{Inspect, Reconcile};

    fn temp_root(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "pulith-{name}-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[cfg(any(feature = "blake3", feature = "sha2"))]
    #[test]
    fn exact_local_handle_is_not_redirected_by_path_replacement() {
        use std::io::Read;

        let root = temp_root("exact-open-replacement");
        let path = root.join("artifact");
        let archived = root.join("opened-artifact");
        fs::create_dir_all(&root).unwrap();
        fs::write(&path, b"opened").unwrap();
        let OpenedLocalArtifact::File(mut file) = open_local_artifact(&path).unwrap() else {
            panic!("regular file was not opened as a file");
        };

        fs::rename(&path, &archived).unwrap();
        fs::write(&path, b"replacement").unwrap();
        let mut observed = Vec::new();
        file.read_to_end(&mut observed).unwrap();

        assert_eq!(observed, b"opened");
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(all(windows, any(feature = "blake3", feature = "sha2")))]
    #[test]
    fn windows_unknown_file_type_with_error_is_failure() {
        use windows_sys::Win32::Storage::FileSystem::FILE_TYPE_UNKNOWN;

        let error = classify_windows_file_type(FILE_TYPE_UNKNOWN, 5).unwrap_err();
        assert_eq!(error.raw_os_error(), Some(5));
    }

    #[test]
    fn local_inspect_reports_missing_file_and_directory_without_mutation() {
        let root = temp_root("inspect-entry-kinds");
        let missing = root.join("missing");
        let file = root.join("file.txt");
        let directory = root.join("directory");
        fs::create_dir_all(&directory).unwrap();
        fs::write(&file, "pulith").unwrap();

        let (missing_observation, _) = LocalTarget::new(missing.as_path())
            .unwrap()
            .inspect(())
            .unwrap();
        let (file_observation, file_evidence) = LocalTarget::new(file.as_path())
            .unwrap()
            .inspect(())
            .unwrap();
        let (directory_observation, _) = LocalTarget::new(directory.as_path())
            .unwrap()
            .inspect(())
            .unwrap();

        assert_eq!(missing_observation, LocalObservation::Missing);
        assert_eq!(file_observation, LocalObservation::File { bytes: 6 });
        assert_eq!(directory_observation, LocalObservation::Directory);
        assert_eq!(file_evidence, LocalInspectEvidence { path: file.clone() });
        assert_eq!(fs::read_to_string(&file).unwrap(), "pulith");

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn local_inspect_observes_dangling_symlink_without_following_or_removing_it() {
        let root = temp_root("inspect-dangling-symlink");
        let target = root.join("target-link");
        fs::create_dir_all(&root).unwrap();
        symlink_file(root.join("missing-source"), &target).unwrap();

        let (observation, _) = LocalTarget::new(target.clone())
            .unwrap()
            .inspect(())
            .unwrap();

        assert_eq!(observation, LocalObservation::SymlinkToFile);
        assert!(
            fs::symlink_metadata(&target)
                .unwrap()
                .file_type()
                .is_symlink()
        );

        fs::remove_file(target).unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn local_reconcile_classifies_resource_differences_without_mutation() {
        let root = temp_root("reconcile");
        let missing = root.join("missing");
        let file = root.join("file.txt");
        let directory = root.join("directory");
        fs::create_dir_all(&directory).unwrap();
        fs::write(&file, "pulith").unwrap();

        let (matched, matched_reconcile_evidence, matched_inspect_evidence) =
            reconcile(&file, LocalExpectation::FileSize(6));
        let (missing, _, _) = reconcile(&missing, LocalExpectation::File);
        let (unexpected, _, _) = reconcile(&file, LocalExpectation::Missing);
        let (wrong_kind, _, _) = reconcile(&directory, LocalExpectation::File);
        let (modified, _, _) = reconcile(&file, LocalExpectation::FileSize(7));

        assert_eq!(matched, LocalReconciliation::Matches);
        assert_eq!(missing, LocalReconciliation::Missing);
        assert_eq!(unexpected, LocalReconciliation::Unexpected);
        assert_eq!(
            wrong_kind,
            LocalReconciliation::WrongKind {
                expected: LocalEntryKind::File,
                observed: LocalEntryKind::Directory,
            }
        );
        assert_eq!(
            modified,
            LocalReconciliation::Modified {
                expected_bytes: 7,
                observed_bytes: 6,
            }
        );
        assert_eq!(
            matched_inspect_evidence,
            LocalInspectEvidence { path: file.clone() }
        );
        assert_eq!(
            matched_reconcile_evidence.expected,
            LocalExpectation::FileSize(6)
        );
        assert_eq!(
            matched_reconcile_evidence.observed,
            LocalObservation::File { bytes: 6 }
        );
        assert_eq!(fs::read_to_string(&file).unwrap(), "pulith");

        fs::remove_dir_all(root).unwrap();
    }

    fn reconcile(
        path: &std::path::Path,
        expected: LocalExpectation,
    ) -> (
        LocalReconciliation,
        LocalReconcileEvidence,
        LocalInspectEvidence,
    ) {
        let (observation, inspect_evidence) = LocalTarget::new(path).unwrap().inspect(()).unwrap();
        let (reconciliation, reconcile_evidence) = observation.reconcile(expected).unwrap();
        (reconciliation, reconcile_evidence, inspect_evidence)
    }
}
