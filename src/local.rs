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

use crate::{
    Acquire, Acquired, Applied, EvidenceChain, Forget, Inspect, Inspected, Materialize, Reconcile,
    Reconciled,
};

mod apply;
mod view;

#[cfg(any(feature = "zip", feature = "tar"))]
pub(crate) use apply::apply_material;
pub use apply::{ApplyEvidence, LocalApply, LocalPlacement};
pub use view::{
    LocalActivate, LocalActivateError, LocalActivationEvidence, LocalActivationStrategy,
    LocalDeactivate, LocalDeactivateError, LocalDeactivateEvidence, LocalDeactivatePrior,
    LocalSwitch, LocalSwitchBackend, LocalSwitchError, LocalSwitchEvidence,
};

/// Errors produced by local acquisition, observation, activation, and publication behaviors.
#[non_exhaustive]
#[derive(Debug)]
pub enum LocalError {
    MissingSource(PathBuf),
    /// A `CreateNew` publication found an existing predecessor and did not commit the target.
    ApplyWouldOverwrite(PathBuf),
    ApplyMissingTarget(PathBuf),
    ApplySameFile(PathBuf),
    ApplyPathConflict {
        source: PathBuf,
        target: PathBuf,
    },
    UnsupportedLocalEntry(PathBuf),
    InvalidPreparation(String),
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
            Self::ApplyWouldOverwrite(path) => {
                write!(
                    f,
                    "apply would overwrite existing target: {}",
                    path.display()
                )
            }
            Self::ApplyMissingTarget(path) => {
                write!(f, "target does not exist: {}", path.display())
            }
            Self::ApplySameFile(path) => {
                write!(
                    f,
                    "source and target refer to the same file: {}",
                    path.display()
                )
            }
            Self::ApplyPathConflict { source, target } => {
                write!(
                    f,
                    "source and target paths conflict: {} -> {}",
                    source.display(),
                    target.display()
                )
            }
            Self::UnsupportedLocalEntry(path) => {
                write!(
                    f,
                    "local filesystem entry is unsupported: {}",
                    path.display()
                )
            }
            Self::InvalidPreparation(message) => write!(f, "invalid preparation: {message}"),
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
            Self::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LocalPath {
    pub path: PathBuf,
}

impl LocalPath {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LocalTarget {
    pub path: PathBuf,
}

impl LocalTarget {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
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

#[derive(Clone, Copy, Debug, Default)]
pub struct LocalAcquire;

impl<I, T> Acquire<Materialize<I, LocalPath, T>> for LocalAcquire {
    type Error = LocalError;
    type Output = Acquired<Materialize<I, LocalPath, T>, LocalMaterial, LocalAcquireEvidence>;

    fn acquire(&self, node: Materialize<I, LocalPath, T>) -> Result<Self::Output, Self::Error> {
        let path = node.source.path.clone();
        if !path.exists() {
            return Err(LocalError::MissingSource(path));
        }
        let material = if path.is_dir() {
            LocalMaterial::Directory { path: path.clone() }
        } else {
            LocalMaterial::File { path: path.clone() }
        };
        Ok(Acquired {
            input: node,
            material,
            evidence: LocalAcquireEvidence { path },
        })
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
    pub(crate) fn path(&self) -> &Path {
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
}

impl StagedTree {
    #[cfg(feature = "process")]
    pub(crate) fn new(workspace: tempfile::TempDir, root: PathBuf) -> Self {
        Self { workspace, root }
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

/// No-follow local artifact facts with an optional regular-file attestation payload.
///
/// Local owns every entry-kind variant. A separate adapter supplies `T` only after it has read a
/// regular file admitted by the local no-follow opening boundary.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LocalArtifactObservation<T> {
    Missing,
    File { attestation: T },
    Directory,
    Symlink,
    Reparse,
    Other,
}

#[cfg(feature = "hash")]
impl<T> LocalArtifactObservation<T> {
    pub(crate) fn kind(&self) -> LocalEntryKind {
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

/// Read-only facts observed for one [`LocalTarget`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LocalObservation {
    Missing,
    File { bytes: u64 },
    Directory,
    Symlink,
    Other,
}

/// Evidence that no-follow metadata produced the local observation.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct LocalInspectEvidence;

impl LocalObservation {
    pub fn kind(&self) -> LocalEntryKind {
        match self {
            Self::Missing => LocalEntryKind::Missing,
            Self::File { .. } => LocalEntryKind::File,
            Self::Directory => LocalEntryKind::Directory,
            Self::Symlink => LocalEntryKind::Symlink,
            Self::Other => LocalEntryKind::Other,
        }
    }
}

/// Caller-owned expected state used by [`LocalReconcile`].
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

/// Read-only, no-follow inspection of one local target.
#[derive(Clone, Copy, Debug, Default)]
pub struct LocalInspect;

impl Inspect<LocalTarget> for LocalInspect {
    type Error = LocalError;
    type Output = Inspected<LocalTarget, LocalObservation, LocalInspectEvidence>;

    fn inspect(&self, node: LocalTarget) -> Result<Self::Output, Self::Error> {
        let observation = match fs::symlink_metadata(&node.path) {
            Ok(metadata) => {
                let file_type = metadata.file_type();
                if file_type.is_symlink() {
                    LocalObservation::Symlink
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
            Err(error) => return Err(LocalError::io("inspect local target", &node.path, error)),
        };

        Ok(Inspected {
            input: node,
            observation,
            evidence: LocalInspectEvidence,
        })
    }
}

/// Read-only local observation performed after a completed local target effect.
///
/// This adapter accepts only local [`Applied`] receipts that retain an exact [`LocalTarget`]. It
/// preserves prior apply evidence on success, while its error preserves the completed receipt when
/// no valid observation can be produced. It does not publish, remove, repair, or decide desired
/// state; callers may pass the result to [`LocalReconcile`] with their own [`LocalExpectation`].
#[derive(Clone, Copy, Debug, Default)]
pub struct LocalPostInspect;

/// An unavailable post-apply observation together with the completed local effect receipt.
///
/// The effect represented by [`Self::applied`] has already completed. Retaining it prevents a
/// later read failure from being mistaken for a failed or safely repeatable apply.
#[derive(Debug)]
pub struct LocalPostInspectError<N, E> {
    pub applied: Applied<N, E>,
    pub cause: LocalError,
}

impl<N, E> fmt::Display for LocalPostInspectError<N, E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "post-apply local inspection failed: {}", self.cause)
    }
}

impl<N: fmt::Debug, E: fmt::Debug> std::error::Error for LocalPostInspectError<N, E> {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.cause)
    }
}

type LocalPostInspected<E> =
    Inspected<LocalTarget, LocalObservation, EvidenceChain<E, LocalInspectEvidence>>;

fn post_inspect<N, E>(
    applied: Applied<N, E>,
    target: LocalTarget,
) -> Result<LocalPostInspected<E>, LocalPostInspectError<N, E>> {
    match LocalInspect.inspect(target) {
        Ok(Inspected {
            input,
            observation,
            evidence,
        }) => Ok(Inspected {
            input,
            observation,
            evidence: EvidenceChain {
                previous: applied.evidence,
                current: evidence,
            },
        }),
        Err(cause) => Err(LocalPostInspectError { applied, cause }),
    }
}

impl<I, S, E> Inspect<Applied<Materialize<I, S, LocalTarget>, E>> for LocalPostInspect {
    type Error = LocalPostInspectError<Materialize<I, S, LocalTarget>, E>;
    type Output = Inspected<LocalTarget, LocalObservation, EvidenceChain<E, LocalInspectEvidence>>;

    fn inspect(
        &self,
        applied: Applied<Materialize<I, S, LocalTarget>, E>,
    ) -> Result<Self::Output, Self::Error> {
        let target = applied.input.target.clone();
        post_inspect(applied, target)
    }
}

impl<I, E> Inspect<Applied<Forget<I, LocalTarget>, E>> for LocalPostInspect {
    type Error = LocalPostInspectError<Forget<I, LocalTarget>, E>;
    type Output = Inspected<LocalTarget, LocalObservation, EvidenceChain<E, LocalInspectEvidence>>;

    fn inspect(
        &self,
        applied: Applied<Forget<I, LocalTarget>, E>,
    ) -> Result<Self::Output, Self::Error> {
        let target = applied.input.target.clone();
        post_inspect(applied, target)
    }
}

/// Pure local expected/observed comparison; it never mutates the target.
#[derive(Clone, Copy, Debug, Default)]
pub struct LocalReconcile;

impl<E> Reconcile<Inspected<LocalTarget, LocalObservation, E>, LocalExpectation>
    for LocalReconcile
{
    type Error = std::convert::Infallible;
    type Output =
        Reconciled<LocalTarget, LocalReconciliation, EvidenceChain<E, LocalReconcileEvidence>>;

    fn reconcile(
        &self,
        node: Inspected<LocalTarget, LocalObservation, E>,
        expected: LocalExpectation,
    ) -> Result<Self::Output, Self::Error> {
        let Inspected {
            input,
            observation,
            evidence: inspect_evidence,
        } = node;
        let reconciliation = match (&expected, &observation) {
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
            _ if expected.kind() == observation.kind() => LocalReconciliation::Matches,
            _ => LocalReconciliation::WrongKind {
                expected: expected.kind(),
                observed: observation.kind(),
            },
        };
        let evidence = EvidenceChain {
            previous: inspect_evidence,
            current: LocalReconcileEvidence {
                expected,
                observed: observation,
            },
        };

        Ok(Reconciled {
            input,
            reconciliation,
            evidence,
        })
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

        let missing = LocalInspect.inspect(LocalTarget::new(&missing)).unwrap();
        let file = LocalInspect.inspect(LocalTarget::new(&file)).unwrap();
        let directory = LocalInspect.inspect(LocalTarget::new(&directory)).unwrap();

        assert_eq!(missing.observation, LocalObservation::Missing);
        assert_eq!(file.observation, LocalObservation::File { bytes: 6 });
        assert_eq!(directory.observation, LocalObservation::Directory);
        assert_eq!(file.evidence, LocalInspectEvidence);
        assert_eq!(fs::read_to_string(&file.input.path).unwrap(), "pulith");

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn local_inspect_observes_dangling_symlink_without_following_or_removing_it() {
        let root = temp_root("inspect-dangling-symlink");
        let target = root.join("target-link");
        fs::create_dir_all(&root).unwrap();
        symlink_file(root.join("missing-source"), &target).unwrap();

        let inspected = LocalInspect.inspect(LocalTarget::new(&target)).unwrap();

        assert_eq!(inspected.observation, LocalObservation::Symlink);
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

        let matched = reconcile(&file, LocalExpectation::FileSize(6));
        let missing = reconcile(&missing, LocalExpectation::File);
        let unexpected = reconcile(&file, LocalExpectation::Missing);
        let wrong_kind = reconcile(&directory, LocalExpectation::File);
        let modified = reconcile(&file, LocalExpectation::FileSize(7));

        assert_eq!(matched.reconciliation, LocalReconciliation::Matches);
        assert_eq!(missing.reconciliation, LocalReconciliation::Missing);
        assert_eq!(unexpected.reconciliation, LocalReconciliation::Unexpected);
        assert_eq!(
            wrong_kind.reconciliation,
            LocalReconciliation::WrongKind {
                expected: LocalEntryKind::File,
                observed: LocalEntryKind::Directory,
            }
        );
        assert_eq!(
            modified.reconciliation,
            LocalReconciliation::Modified {
                expected_bytes: 7,
                observed_bytes: 6,
            }
        );
        assert_eq!(matched.evidence.previous, LocalInspectEvidence);
        assert_eq!(
            matched.evidence.current.expected,
            LocalExpectation::FileSize(6)
        );
        assert_eq!(
            matched.evidence.current.observed,
            LocalObservation::File { bytes: 6 }
        );
        assert_eq!(fs::read_to_string(&file).unwrap(), "pulith");

        fs::remove_dir_all(root).unwrap();
    }

    fn reconcile(
        path: &std::path::Path,
        expected: LocalExpectation,
    ) -> crate::Reconciled<
        LocalTarget,
        LocalReconciliation,
        crate::EvidenceChain<LocalInspectEvidence, LocalReconcileEvidence>,
    > {
        LocalReconcile
            .reconcile(
                LocalInspect.inspect(LocalTarget::new(path)).unwrap(),
                expected,
            )
            .unwrap()
    }
}
