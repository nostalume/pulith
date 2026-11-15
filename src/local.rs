//! Local entry inspection, staging, publication, views, and records.
//!
//! This module owns the local filesystem semantics: entry-kind classification without following
//! links, private staged-tree custody, single-target publication, and explicit consumer views. It
//! never claims package ownership, application state, or automatic repair. Concrete publication
//! lives in `local/apply.rs`, views in `local/view.rs`, and bounded records in `local/record.rs`;
//! this facade re-exports their public vocabulary. The whole module is
//! feature-gated on `local`.
use std::fmt;
use std::fs::{self, File};
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Component, Path, PathBuf};

mod apply;
mod materialize;
#[cfg(all(windows, feature = "windows-private-preview"))]
mod private_windows;
mod record;
mod view;

#[cfg(feature = "process")]
pub(crate) use apply::copy_open_file;
pub use apply::{
    CommitOutcome, FileAdmission, FilePublication, RemoveEvidence, TreeAdmission, TreePublication,
};
pub use record::{RecordChange, RecordEdit, RecordError, RecordEvidence, RecordLimit, RecordStore};

/// Read-only result of comparing a view with its declared directory.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ViewStatus {
    /// The view resolves to the declared directory.
    Matches,
    /// The view entry is absent.
    Missing,
    /// The view entry is not a symlink.
    Occupied(LocalObservation),
    /// The symlink's referent is absent.
    Dangling,
    /// The symlink resolves to an existing entry other than the declared directory.
    WrongTarget,
}

/// The entry at a view path and, for a symlink, its referent kind.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ViewEntry {
    /// The view path is absent.
    Missing,
    /// The view path contains a non-symlink entry.
    Occupied(LocalObservation),
    /// A symlink resolving to a directory.
    DirectoryLink,
    /// A symlink resolving to a non-directory.
    OtherLink,
    /// A symlink with an absent referent.
    DanglingLink,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
/// Whether linking created a new consumer view or replaced an existing valid view.
pub enum LinkChange {
    /// The created outcome.
    Created,
    /// The replaced outcome.
    Replaced,
}

#[derive(Clone, Debug, Eq, PartialEq)]
/// Paths and outcome of the native operation that changed the consumer view.
pub struct LinkEvidence {
    /// The source value.
    pub source: PathBuf,
    /// The view value.
    pub view: PathBuf,
    /// The change value.
    pub change: LinkChange,
}

#[derive(Debug)]
/// Failure to validate or commit one consumer-view link operation.
pub enum LinkError {
    /// The before link outcome.
    BeforeLink {
        /// The view value.
        view: PathBuf,
        /// The cause value.
        cause: LocalError,
    },
    /// The link was committed, but its resulting relationship could not be confirmed.
    AfterLink {
        /// Evidence of the native link operation.
        evidence: LinkEvidence,
        /// The cause value.
        cause: LocalError,
    },
    /// The invalid expose outcome.
    InvalidExpose {
        /// The expose value.
        expose: PathBuf,
    },
    /// The expose not directory outcome.
    ExposeNotDirectory {
        /// The path value.
        path: PathBuf,
        /// The observed value.
        observed: LocalObservation,
    },
    /// The view conflict outcome.
    ViewConflict {
        /// The view value.
        view: PathBuf,
        /// The observed value.
        observed: ViewEntry,
    },
    /// The capability unavailable outcome.
    CapabilityUnavailable {
        /// The view value.
        view: PathBuf,
        /// The cause value.
        cause: LocalError,
    },
}

impl fmt::Display for LinkError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BeforeLink { cause, .. } => write!(formatter, "link failed: {cause}"),
            Self::AfterLink { evidence, cause } => write!(
                formatter,
                "linked {} but could not confirm the result: {cause}",
                evidence.view.display()
            ),
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
            Self::BeforeLink { cause, .. }
            | Self::AfterLink { cause, .. }
            | Self::CapabilityUnavailable { cause, .. } => Some(cause),
            _ => None,
        }
    }
}

#[derive(Debug)]
/// Failure to observe or remove one consumer view.
pub enum UnlinkError {
    /// The observe outcome.
    Observe {
        /// The view value.
        view: PathBuf,
        /// The cause value.
        cause: LocalError,
    },
    /// The not active view outcome.
    NotActiveView {
        /// The view value.
        view: PathBuf,
        /// The observed value.
        observed: ViewEntry,
    },
    /// The remove outcome.
    Remove {
        /// The view value.
        view: PathBuf,
        /// The cause value.
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
/// Whether unlinking removed an active view or found it already absent.
pub enum UnlinkChange {
    /// The removed outcome.
    Removed,
    /// The unchanged outcome.
    Unchanged,
}

#[derive(Clone, Debug, Eq, PartialEq)]
/// Consumer view and outcome observed after unlinking.
pub struct UnlinkEvidence {
    /// The view value.
    pub view: PathBuf,
    /// The change value.
    pub change: UnlinkChange,
}

/// Errors produced by local acquisition, observation, activation, and publication behaviors.
#[non_exhaustive]
#[derive(Debug)]
pub enum LocalError {
    /// The missing source outcome.
    MissingSource(PathBuf),
    /// Immutable publication found an existing predecessor and did not commit the target.
    TargetExists(PathBuf),
    /// The required regular predecessor was absent or its bytes differed.
    PredecessorChanged(PathBuf),
    /// Publication found an occupant it never replaces: a directory, symlink, reparse point, or
    /// other entry. The target was classified without following links and was left untouched.
    OccupiedTarget {
        /// The admitted target path.
        path: PathBuf,
        /// The no-follow entry kind found at the path.
        kind: LocalEntryKind,
    },
    /// A read requires a regular file and found another kind of entry.
    NotRegularFile {
        /// The path value.
        path: PathBuf,
        /// The observed entry kind.
        kind: LocalEntryKind,
    },
    /// A descendant path crossed an entry that is not an ordinary directory.
    NotRegularDirectory {
        /// The first refused ancestor below the supplied directory.
        path: PathBuf,
        /// The observed entry kind.
        kind: LocalEntryKind,
    },
    /// The unsupported local entry outcome.
    UnsupportedLocalEntry(PathBuf),
    /// The selected expose path escapes its tree.
    InvalidExpose(PathBuf),
    /// The invalid stage path outcome.
    InvalidStagePath(PathBuf),
    /// The publish unavailable outcome.
    PublishUnavailable {
        /// The path value.
        path: PathBuf,
        /// The source value.
        source: io::Error,
    },
    /// The platform does not support owner-only publication.
    PrivatePublicationUnavailable(PathBuf),
    /// The invalid preparation outcome.
    InvalidPreparation(String),
    /// An empty path or a path without a usable final component.
    InvalidPath(PathBuf),
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
            Self::TargetExists(path) => write!(
                f,
                "existing predecessor entry found; target was not committed: {}",
                path.display()
            ),
            Self::PredecessorChanged(path) => write!(
                f,
                "{} changed after reading; review the current file and retry",
                path.display()
            ),
            Self::OccupiedTarget { path, kind } => write!(
                f,
                "target is occupied by a {kind:?} entry, which publication never replaces: {}",
                path.display()
            ),
            Self::NotRegularFile { path, kind } => {
                write!(
                    f,
                    "cannot read {kind:?} as a regular file: {}",
                    path.display()
                )
            }
            Self::NotRegularDirectory { path, kind } => write!(
                f,
                "cannot traverse {kind:?} as an ordinary directory: {}",
                path.display()
            ),
            Self::UnsupportedLocalEntry(path) => {
                write!(
                    f,
                    "local filesystem entry is unsupported: {}",
                    path.display()
                )
            }
            Self::InvalidExpose(path) => write!(f, "invalid expose path: {}", path.display()),
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
            Self::PrivatePublicationUnavailable(path) => {
                write!(
                    f,
                    "owner-only publication is unavailable for {}",
                    path.display()
                )
            }
            Self::InvalidPreparation(message) => write!(f, "invalid preparation: {message}"),
            Self::InvalidPath(path) => {
                write!(f, "invalid local path: {}", path.display())
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

fn refuse_read(path: &Path, kind: LocalEntryKind) -> LocalError {
    LocalError::NotRegularFile {
        path: path.to_path_buf(),
        kind,
    }
}

#[cfg(unix)]
fn open_regular_file(path: &Path) -> Result<Option<File>, LocalError> {
    use rustix::fs::{Mode, OFlags, open};
    use rustix::io::Errno;

    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            return Err(refuse_read(path, LocalEntryKind::Symlink));
        }
        Ok(metadata) if metadata.is_dir() => {
            return Err(refuse_read(path, LocalEntryKind::Directory));
        }
        Ok(metadata) if !metadata.is_file() => {
            return Err(refuse_read(path, LocalEntryKind::Other));
        }
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok(None);
        }
        Err(error) => return Err(LocalError::io("inspect local file", path, error)),
    }
    let descriptor = match open(
        path,
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK,
        Mode::empty(),
    ) {
        Ok(descriptor) => descriptor,
        Err(Errno::NOENT) => return Ok(None),
        Err(Errno::LOOP) => return Err(refuse_read(path, LocalEntryKind::Symlink)),
        Err(error) => {
            return Err(LocalError::io(
                "open local file",
                path,
                io::Error::from(error),
            ));
        }
    };
    let file = File::from(descriptor);
    let metadata = file
        .metadata()
        .map_err(|error| LocalError::io("inspect local file", path, error))?;
    if metadata.is_file() {
        Ok(Some(file))
    } else if metadata.is_dir() {
        Err(refuse_read(path, LocalEntryKind::Directory))
    } else {
        Err(refuse_read(path, LocalEntryKind::Other))
    }
}

#[cfg(windows)]
fn open_regular_file(path: &Path) -> Result<Option<File>, LocalError> {
    use std::os::windows::fs::OpenOptionsExt;
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Foundation::{ERROR_SUCCESS, GetLastError, SetLastError};
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_BACKUP_SEMANTICS,
        FILE_FLAG_OPEN_REPARSE_POINT, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
        GetFileType,
    };

    let file = match File::options()
        .read(true)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS)
        .open(path)
    {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok(None);
        }
        Err(error) => return Err(LocalError::io("open local file", path, error)),
    };
    let information = windows_attribute_tag(&file)
        .map_err(|error| LocalError::io("inspect opened file", path, error))?;
    if information.FileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Err(refuse_read(
            path,
            windows_reparse_kind(information.ReparseTag),
        ));
    }
    if information.FileAttributes & FILE_ATTRIBUTE_DIRECTORY != 0 {
        return Err(refuse_read(path, LocalEntryKind::Directory));
    }
    // SAFETY: setting and reading the calling thread's last-error slot surrounds one file query.
    unsafe { SetLastError(ERROR_SUCCESS) };
    // SAFETY: `file` owns a valid handle for the duration of the query.
    let file_type = unsafe { GetFileType(file.as_raw_handle()) };
    // SAFETY: GetLastError reads the calling thread's last-error slot.
    let last_error = unsafe { GetLastError() };
    if !classify_windows_file_type(file_type, last_error)
        .map_err(|error| LocalError::io("inspect opened file", path, error))?
    {
        return Err(refuse_read(path, LocalEntryKind::Other));
    }
    Ok(Some(file))
}

#[cfg(windows)]
fn windows_attribute_tag(
    file: &File,
) -> io::Result<windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_TAG_INFO> {
    use std::mem::MaybeUninit;
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_ATTRIBUTE_TAG_INFO, FileAttributeTagInfo, GetFileInformationByHandleEx,
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
        return Err(io::Error::last_os_error());
    }
    // SAFETY: GetFileInformationByHandleEx succeeded and initialized the structure.
    Ok(unsafe { information.assume_init() })
}

#[cfg(windows)]
fn windows_reparse_kind(tag: u32) -> LocalEntryKind {
    use windows_sys::Win32::System::SystemServices::IO_REPARSE_TAG_SYMLINK;

    if tag == IO_REPARSE_TAG_SYMLINK {
        LocalEntryKind::Symlink
    } else {
        LocalEntryKind::Reparse
    }
}

pub(super) fn entry_kind(
    path: &Path,
    metadata: &fs::Metadata,
) -> Result<LocalEntryKind, LocalError> {
    #[cfg(windows)]
    {
        use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
        use windows_sys::Win32::Storage::FileSystem::{
            FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT,
            FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
        };

        if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            let file = File::options()
                .access_mode(0)
                .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
                .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS)
                .open(path)
                .map_err(|error| LocalError::io("open entry metadata", path, error))?;
            let information = windows_attribute_tag(&file)
                .map_err(|error| LocalError::io("inspect entry metadata", path, error))?;
            // An ordinary entry may have replaced the reparse point during lookup. Refuse
            // the original observation conservatively instead of using its stale metadata.
            return Ok(windows_reparse_kind(information.ReparseTag));
        }
    }
    #[cfg(unix)]
    let _ = path;
    let kind = metadata.file_type();
    Ok(if kind.is_symlink() {
        LocalEntryKind::Symlink
    } else if kind.is_dir() {
        LocalEntryKind::Directory
    } else if kind.is_file() {
        LocalEntryKind::File
    } else {
        LocalEntryKind::Other
    })
}

#[cfg(windows)]
fn classify_windows_file_type(file_type: u32, last_error: u32) -> io::Result<bool> {
    use windows_sys::Win32::Foundation::ERROR_SUCCESS;
    use windows_sys::Win32::Storage::FileSystem::{FILE_TYPE_DISK, FILE_TYPE_UNKNOWN};

    if file_type == FILE_TYPE_UNKNOWN && last_error != ERROR_SUCCESS {
        Err(io::Error::from_raw_os_error(last_error as i32))
    } else {
        Ok(file_type == FILE_TYPE_DISK)
    }
}

/// A non-empty local path. Filesystem operations validate whichever role it plays.
#[derive(Clone)]
pub struct LocalPath(PathBuf);

impl LocalPath {
    /// Admits a non-empty local path without embedded NUL bytes.
    pub fn new(path: impl Into<PathBuf>) -> Result<Self, LocalError> {
        let path = path.into();
        if path.as_os_str().is_empty() || path.as_os_str().as_encoded_bytes().contains(&0) {
            return Err(LocalError::InvalidPath(path));
        }
        Ok(Self(path))
    }

    /// Returns the admitted path.
    pub fn as_path(&self) -> &Path {
        &self.0
    }

    /// Opens a regular file for reading without following the final path component.
    ///
    /// A missing entry returns `None`. Non-regular entries are refused by kind. Read bytes and
    /// metadata through the handle so a later path replacement cannot redirect either.
    /// An open handle still observes changes made to its file in place.
    pub fn open_file(self) -> Result<Option<OpenedFile>, LocalError> {
        Ok(open_regular_file(&self.0)?.map(|file| OpenedFile { file, path: self.0 }))
    }

    /// Inspects the final entry without following a symlink or reparse point.
    pub fn inspect_entry(&self) -> Result<LocalObservation, LocalError> {
        inspect_entry(self.as_path())
    }

    /// Inspects a relative descendant without following links below this directory.
    ///
    /// A missing component yields `Missing`. Other ancestors must be ordinary directories.
    /// The final entry is classified even when it is a link. The caller supplies the base
    /// directory. This path-based observation assumes its namespace stays stable through use.
    pub fn inspect_descendant(&self, relative: &Path) -> Result<LocalObservation, LocalError> {
        let mut components = relative.components().peekable();
        if components.peek().is_none() || !relative.components().all(descendant_component) {
            return Err(LocalError::InvalidPath(relative.to_path_buf()));
        }

        let mut path = self.0.clone();
        while let Some(part) = components.next() {
            path.push(part.as_os_str());
            let observed = inspect_entry(&path)?;
            if components.peek().is_none() || observed == LocalObservation::Missing {
                return Ok(observed);
            }
            if observed != LocalObservation::Directory {
                return Err(LocalError::NotRegularDirectory {
                    path,
                    kind: observed.kind(),
                });
            }
        }
        unreachable!("a descendant has at least one component")
    }

    /// Resolves a directory path for placement, following directory links.
    ///
    /// Missing suffixes are kept beneath the nearest existing directory. An existing
    /// dangling link is an error. The result observes the current namespace for planning.
    pub fn resolve_directory(&self) -> Result<PathBuf, LocalError> {
        let mut path = self.as_path();
        let mut missing = Vec::new();
        let mut resolved = loop {
            match fs::canonicalize(path) {
                Ok(resolved) => match inspect_entry(&resolved)? {
                    LocalObservation::Directory => break resolved,
                    observed => {
                        return Err(LocalError::NotRegularDirectory {
                            path: path.to_path_buf(),
                            kind: observed.kind(),
                        });
                    }
                },
                Err(error) if error.kind() == io::ErrorKind::NotFound => {
                    if inspect_entry(path)? != LocalObservation::Missing {
                        return Err(LocalError::io("resolve directory link", path, error));
                    }
                    let (Some(parent), Some(name)) = (path.parent(), path.file_name()) else {
                        return Err(LocalError::io("resolve directory", path, error));
                    };
                    missing.push(name);
                    path = if parent.as_os_str().is_empty() {
                        Path::new(".")
                    } else {
                        parent
                    };
                }
                Err(error) => return Err(LocalError::io("resolve directory", path, error)),
            }
        };
        resolved.extend(missing.into_iter().rev());
        Ok(resolved)
    }

    /// Observes a file position by resolving its parent and retaining the final name.
    ///
    /// Missing parents are retained by the same rule as [Self::resolve_directory].
    /// The final entry is not opened or followed.
    pub fn resolve_file_position(&self) -> Result<PathBuf, LocalError> {
        let path = self.as_path();
        let (Some(parent), Some(name)) = (path.parent(), path.file_name()) else {
            return Err(LocalError::InvalidPath(path.to_path_buf()));
        };
        let parent = if parent.as_os_str().is_empty() {
            Path::new(".")
        } else {
            parent
        };
        Ok(Self::new(parent)?.resolve_directory()?.join(name))
    }

    /// Creates eager private custody beside this target without publishing it.
    ///
    /// Effect law: this is not a zero-write observation. Staging creates the target's ancestor
    /// directories before creating the private staged-tree workspace.
    pub fn stage(&self) -> Result<StagedTree, LocalError> {
        apply::stage(self.as_path())
    }

    /// Atomically publishes one file and reports which commit branch succeeded.
    ///
    /// Stages the bytes beside the target and rechecks admission before the native operation.
    /// Digest admission observes the predecessor before rename. Both replacement branches
    /// assume the namespace stays stable between those operations.
    pub fn publish_file(
        &self,
        bytes: impl AsRef<[u8]>,
        admission: impl Into<FileAdmission>,
    ) -> Result<FilePublication, LocalError> {
        apply::publish_file(self.as_path(), bytes.as_ref(), admission.into())
    }

    /// Checks known private publication constraints without creating a file or directory.
    ///
    /// The publisher verifies protection on the actual staging handle before writing.
    pub fn check_private_publication(&self) -> Result<(), LocalError> {
        #[cfg(unix)]
        {
            Ok(())
        }
        #[cfg(all(windows, feature = "windows-private-preview"))]
        {
            private_windows::check_destination(self.as_path())
        }
        #[cfg(all(windows, not(feature = "windows-private-preview")))]
        {
            Err(LocalError::PrivatePublicationUnavailable(self.0.clone()))
        }
    }

    /// Publishes an owner-only file, verifying protection before writing and syncing the
    /// staged file before publication. The supplied admission still governs the destination.
    /// Windows requires the `windows-private-preview` feature and a local NTFS volume.
    pub fn publish_private_file(
        &self,
        bytes: &[u8],
        admission: impl Into<FileAdmission>,
    ) -> Result<FilePublication, LocalError> {
        #[cfg(any(unix, all(windows, feature = "windows-private-preview")))]
        {
            self.check_private_publication()?;
            apply::publish_private_file(self.as_path(), bytes, admission.into())
        }
        #[cfg(all(windows, not(feature = "windows-private-preview")))]
        {
            let _ = (bytes, admission);
            Err(LocalError::PrivatePublicationUnavailable(self.0.clone()))
        }
    }
}

fn descendant_component(part: Component<'_>) -> bool {
    let Component::Normal(name) = part else {
        return false;
    };
    let bytes = name.as_encoded_bytes();
    if bytes.contains(&0) {
        return false;
    }
    #[cfg(windows)]
    if bytes.contains(&b':') || matches!(bytes.last(), Some(b'.' | b' ')) {
        return false;
    }
    true
}

#[cfg(windows)]
fn wide_path(path: &Path) -> io::Result<Vec<u16>> {
    use std::os::windows::ffi::OsStrExt;

    if path.as_os_str().as_encoded_bytes().contains(&0) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "embedded NUL in path",
        ));
    }
    let mut wide: Vec<u16> = path.as_os_str().encode_wide().collect();
    // Match Win32 normal path handling for short absolute paths. Longer or relative paths need
    // absolute normalization before adding a verbatim prefix.
    if !path.is_absolute() || wide.len() >= 248 {
        let absolute = std::path::absolute(path)?;
        let units = absolute.as_os_str().encode_wide();
        let prefix = match absolute.components().next() {
            Some(Component::Prefix(prefix)) => Some(prefix.kind()),
            _ => None,
        };
        wide = match prefix {
            Some(std::path::Prefix::Disk(_)) => r"\\?\".encode_utf16().chain(units).collect(),
            Some(std::path::Prefix::UNC(..)) => {
                r"\\?\UNC\".encode_utf16().chain(units.skip(2)).collect()
            }
            _ => units.collect(),
        };
    }
    wide.push(0);
    Ok(wide)
}

/// A read-only regular-file handle bound to the object opened at this path.
#[derive(Debug)]
pub struct OpenedFile {
    pub(crate) file: File,
    pub(crate) path: PathBuf,
}

impl OpenedFile {
    /// Returns the path used to open this handle.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Checks owner-only protection on this opened file.
    pub fn is_owner_only(&self) -> Result<bool, LocalError> {
        #[cfg(unix)]
        {
            private_file_mode(&self.file)
                .map_err(|error| LocalError::io("inspect file protection", &self.path, error))
        }
        #[cfg(all(windows, feature = "windows-private-preview"))]
        {
            private_windows::is_owner_only(&self.file).map_err(|error| {
                private_windows::protection_error("inspect file protection", &self.path, error)
            })
        }
        #[cfg(all(windows, not(feature = "windows-private-preview")))]
        {
            Err(LocalError::PrivatePublicationUnavailable(self.path.clone()))
        }
    }

    /// Reads metadata through the same handle as the file content.
    pub fn metadata(&self) -> io::Result<fs::Metadata> {
        self.file.metadata()
    }
}

#[cfg(unix)]
fn private_file_mode(file: &File) -> io::Result<bool> {
    use std::os::unix::fs::PermissionsExt;

    Ok(file.metadata()?.permissions().mode() & 0o7777 == 0o600)
}

impl Read for OpenedFile {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        self.file.read(buffer)
    }
}

impl Seek for OpenedFile {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        self.file.seek(position)
    }
}

/// An owned, unnamed byte file with read and seek access.
///
/// Dropping it releases the bytes. Its writable handle stays inside the adapter.
#[derive(Debug)]
pub struct FileSnapshot {
    pub(crate) file: File,
    pub(crate) source: PathBuf,
}

impl Read for FileSnapshot {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        self.file.read(buffer)
    }
}

impl Seek for FileSnapshot {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        self.file.seek(position)
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
    /// The missing outcome.
    Missing,
    /// The file outcome.
    File,
    /// The directory outcome.
    Directory,
    /// The symlink outcome.
    Symlink,
    /// The reparse outcome.
    Reparse,
    /// The other outcome.
    Other,
}

impl LocalEntryKind {
    /// Classifies a directory entry without opening its referent.
    pub fn from_dir_entry(entry: &fs::DirEntry) -> io::Result<Self> {
        let kind = entry.file_type()?;
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            use windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT;

            if entry.metadata()?.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
                && !kind.is_symlink()
            {
                return Ok(Self::Reparse);
            }
        }
        Ok(if kind.is_symlink() {
            Self::Symlink
        } else if kind.is_dir() {
            Self::Directory
        } else if kind.is_file() {
            Self::File
        } else {
            Self::Other
        })
    }
}

/// Read-only facts about a final filesystem entry, without following it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LocalObservation {
    /// The missing outcome.
    Missing,
    /// The file outcome.
    File {
        /// Observed file length in bytes.
        bytes: u64,
    },
    /// The directory outcome.
    Directory,
    /// The symlink outcome.
    Symlink,
    /// A non-symlink reparse point.
    Reparse,
    /// The other outcome.
    Other,
}

impl LocalObservation {
    /// Returns the entry kind represented by this observation.
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

/// Caller-owned expectation for an entry comparison.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LocalExpectation {
    /// The missing outcome.
    Missing,
    /// The file outcome.
    File,
    /// The file size outcome.
    FileSize(u64),
    /// The directory outcome.
    Directory,
    /// The symlink outcome.
    Symlink,
    /// The other outcome.
    Other,
}

impl LocalExpectation {
    /// Returns the entry kind required by this expectation.
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
    /// The matches outcome.
    Matches,
    /// The missing outcome.
    Missing,
    /// The unexpected outcome.
    Unexpected,
    /// The wrong kind outcome.
    WrongKind {
        /// The expected value.
        expected: LocalEntryKind,
        /// The observed value.
        observed: LocalEntryKind,
    },
    /// The modified outcome.
    Modified {
        /// The expected bytes value.
        expected_bytes: u64,
        /// The observed bytes value.
        observed_bytes: u64,
    },
}

fn inspect_entry(path: &Path) -> Result<LocalObservation, LocalError> {
    Ok(match fs::symlink_metadata(path) {
        Ok(metadata) => match entry_kind(path, &metadata)? {
            LocalEntryKind::Missing => unreachable!("metadata returned for an existing entry"),
            LocalEntryKind::File => LocalObservation::File {
                bytes: metadata.len(),
            },
            LocalEntryKind::Directory => LocalObservation::Directory,
            LocalEntryKind::Symlink => LocalObservation::Symlink,
            LocalEntryKind::Reparse => LocalObservation::Reparse,
            LocalEntryKind::Other => LocalObservation::Other,
        },
        Err(error) if error.kind() == io::ErrorKind::NotFound => LocalObservation::Missing,
        Err(error) => return Err(LocalError::io("inspect local target", path, error)),
    })
}

impl LocalObservation {
    /// Compares this observation with the caller's expectation.
    pub fn compare(&self, expected: &LocalExpectation) -> LocalReconciliation {
        match (expected, self) {
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
        }
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

    fn temp_root(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "pulith-{name}-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[cfg(windows)]
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

        let missing_observation = LocalPath::new(missing.as_path())
            .unwrap()
            .inspect_entry()
            .unwrap();
        let file_observation = LocalPath::new(file.as_path())
            .unwrap()
            .inspect_entry()
            .unwrap();
        let directory_observation = LocalPath::new(directory.as_path())
            .unwrap()
            .inspect_entry()
            .unwrap();

        assert_eq!(missing_observation, LocalObservation::Missing);
        assert_eq!(file_observation, LocalObservation::File { bytes: 6 });
        assert_eq!(directory_observation, LocalObservation::Directory);
        assert_eq!(fs::read_to_string(&file).unwrap(), "pulith");

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn local_inspect_observes_dangling_symlink_without_following_or_removing_it() {
        let root = temp_root("inspect-dangling-symlink");
        let target = root.join("target-link");
        fs::create_dir_all(&root).unwrap();
        symlink_file(root.join("missing-source"), &target).unwrap();

        let observation = LocalPath::new(target.clone())
            .unwrap()
            .inspect_entry()
            .unwrap();

        assert_eq!(observation, LocalObservation::Symlink);
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
        assert_eq!(fs::read_to_string(&file).unwrap(), "pulith");

        fs::remove_dir_all(root).unwrap();
    }

    fn reconcile(path: &std::path::Path, expected: LocalExpectation) -> LocalReconciliation {
        LocalPath::new(path)
            .unwrap()
            .inspect_entry()
            .unwrap()
            .compare(&expected)
    }
}
