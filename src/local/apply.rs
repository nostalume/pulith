//! Local publication: atomically commit one eagerly populated stage, or remove one target.
//!
//! Owns the exact single-target effect law: one native rename commit boundary chosen by an
//! explicit [`TreeAdmission`], and direct idempotent target removal. It never republishes, follows
//! links, claims ownership, or retries. Evidence is adapter-attested effect data, not an
//! authorization. Feature-gated on `local`.
//!
//! Staging is not a zero-write operation. `stage` creates the target's ancestor directories
//! before it creates the private staged-tree workspace beside the target; callers that require a
//! plan with no filesystem writes must not call it. Inspection and reconciliation stay read-only.
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use super::{
    LocalEntryKind, LocalError, LocalPath, OpenedFile, StagePlacement, StagedTree, entry_kind,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
/// The occupancy precondition a commit may proceed under.
///
/// The variants select different native rename forms, not a preference: `Absent` maps to a
/// no-replace rename and `ReplaceRegularFile` to a replacing rename.
pub enum TreeAdmission {
    /// Commit only when the target is absent.
    Absent,
    /// Commit when the target is absent or a regular file.
    ///
    /// An existing regular file is replaced in place. On POSIX it keeps its permission bits; on
    /// Windows it is replaced as-is, and a read-only or otherwise denied target surfaces as an
    /// [`LocalError::Io`] with the target path, leaving the predecessor intact.
    ReplaceRegularFile,
}

/// The predecessor required by a single-file publication.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FileAdmission {
    /// Publish only into a free name.
    Absent,
    /// Publish into a free name or replace a regular file.
    ReplaceRegularFile,
    /// Replace a regular file whose observed bytes match the digest.
    #[cfg(any(feature = "blake3", feature = "sha2"))]
    ReplaceMatching(crate::hash::DigestValue),
}

impl From<TreeAdmission> for FileAdmission {
    fn from(admission: TreeAdmission) -> Self {
        match admission {
            TreeAdmission::Absent => Self::Absent,
            TreeAdmission::ReplaceRegularFile => Self::ReplaceRegularFile,
        }
    }
}

fn regular_file_metadata(target: &Path) -> Result<Option<fs::Metadata>, LocalError> {
    let metadata = match fs::symlink_metadata(target) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(LocalError::io("inspect target occupancy", target, error)),
    };
    match super::entry_kind(target, &metadata)? {
        LocalEntryKind::File => Ok(Some(metadata)),
        kind => Err(LocalError::OccupiedTarget {
            path: target.to_path_buf(),
            kind,
        }),
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
/// Materialized file, directory, and byte totals published with one staged tree.
pub struct TreePublication {
    /// The files value.
    pub files: usize,
    /// The directories value.
    pub directories: usize,
    /// The bytes value.
    pub bytes: u64,
}

/// The native publication branch that completed for one file.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CommitOutcome {
    /// The file was published with a no-replace operation into a free name.
    Created,
    /// A replacing rename succeeded after a regular predecessor was observed.
    ///
    /// The observation and rename are separate. The predecessor can change between them.
    Replaced,
}

/// Evidence returned after a single file has been published.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FilePublication {
    /// The successful commit branch.
    pub outcome: CommitOutcome,
    /// The number of bytes published.
    pub bytes: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
/// Whether direct target removal changed the filesystem.
pub enum RemoveEvidence {
    /// The removed outcome.
    Removed,
    /// The unchanged outcome.
    Unchanged,
}

pub(crate) fn stage(target: &Path) -> Result<StagedTree, LocalError> {
    let parent = target_parent(target)?;
    fs::create_dir_all(parent)
        .map_err(|error| LocalError::io("create stage parent", parent, error))?;
    let parent = parent
        .canonicalize()
        .map_err(|error| LocalError::io("canonicalize stage parent", parent, error))?;
    let mut builder = tempfile::Builder::new();
    builder.prefix(".pulith-stage-");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        builder.permissions(fs::Permissions::from_mode(0o700));
    }
    let workspace = builder
        .tempdir_in(&parent)
        .map_err(|error| LocalError::io("create staged tree", &parent, error))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(workspace.path(), fs::Permissions::from_mode(0o700)).map_err(
            |error| LocalError::io("protect staged tree workspace", workspace.path(), error),
        )?;
    }
    let root = workspace.path().join("tree");
    Ok(StagedTree::destination(workspace, root, parent))
}

/// Publishes `bytes` to one target with a single rename, without building a staged tree.
///
/// The atomic boundary is the same one [`StagedTree::publish`] uses: a reader sees the
/// predecessor or the new bytes, never half of either. What is absent is the tree — no workspace
/// directory, no `tree/` child to walk, no directory to remove afterwards. A caller that decides
/// per file (one target, one decision) paid three directory operations per file for a tree it
/// never published as a tree.
///
/// Effects match [`stage`] on the one point that matters: ancestor directories are created on
/// demand. A refusal leaves the target untouched and removes only a temporary file created by
/// this call.
pub(crate) fn publish_file(
    target: &Path,
    bytes: &[u8],
    admission: FileAdmission,
) -> Result<FilePublication, LocalError> {
    publish_bytes(target, bytes, admission, FileProtection::Ordinary)
}

#[cfg(any(unix, all(windows, feature = "windows-private-preview")))]
pub(super) fn publish_private_file(
    target: &Path,
    bytes: &[u8],
    admission: FileAdmission,
) -> Result<FilePublication, LocalError> {
    publish_bytes(target, bytes, admission, FileProtection::OwnerOnly)
}

#[derive(Clone, Copy)]
pub(super) enum FileProtection {
    Ordinary,
    #[cfg(any(unix, all(windows, feature = "windows-private-preview")))]
    OwnerOnly,
}

fn publish_bytes(
    target: &Path,
    bytes: &[u8],
    admission: FileAdmission,
    protection: FileProtection,
) -> Result<FilePublication, LocalError> {
    publish_single_file(
        target,
        admission,
        protection,
        std::convert::identity,
        |file, staged| write_bytes(file, staged, bytes),
    )
    .map(|(publication, ())| publication)
}

fn write_bytes(file: &mut fs::File, staged: &Path, bytes: &[u8]) -> Result<(u64, ()), LocalError> {
    use std::io::Write;

    file.write_all(bytes)
        .map_err(|error| LocalError::io("write staged file", staged, error))?;
    Ok((bytes.len() as u64, ()))
}

pub(super) fn publish_single_file<T, E>(
    target: &Path,
    admission: FileAdmission,
    protection: FileProtection,
    local_error: impl Fn(LocalError) -> E,
    write: impl FnOnce(&mut fs::File, &Path) -> Result<(u64, T), E>,
) -> Result<(FilePublication, T), E> {
    let parent = target_parent(target).map_err(&local_error)?;
    let staged = sibling_temp(target, parent).map_err(&local_error)?;
    publish_sibling(&staged, target, admission, protection, local_error, write)
}

fn publish_sibling<T, E>(
    staged: &Path,
    target: &Path,
    admission: FileAdmission,
    protection: FileProtection,
    local_error: impl Fn(LocalError) -> E,
    write: impl FnOnce(&mut fs::File, &Path) -> Result<(u64, T), E>,
) -> Result<(FilePublication, T), E> {
    let mut file = match create_sibling_file(staged, protection) {
        Ok(file) => file,
        // The parent directory is the only thing whose absence shows up here, and it is absent
        // only for the first file of a new subtree. Creating it on that path costs one syscall
        // per new subtree instead of one probe per file.
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            let parent = target_parent(target).map_err(&local_error)?;
            fs::create_dir_all(parent).map_err(|error| {
                local_error(LocalError::io("create target parent", parent, error))
            })?;
            create_sibling_file(staged, protection)
                .map_err(|error| local_error(staging_error(staged, target, protection, error)))?
        }
        Err(error) => {
            return Err(local_error(staging_error(
                staged, target, protection, error,
            )));
        }
    };
    let published = (|| {
        protect_sibling(&file, staged, target, protection).map_err(&local_error)?;
        let (bytes, product) = write(&mut file, staged)?;
        let publication = rename_sibling(file, staged, target, bytes, admission, protection)
            .map_err(local_error)?;
        Ok((publication, product))
    })();
    if published.is_err() {
        // The exclusive create owns this name. Preserve the primary error if cleanup fails.
        let _ = fs::remove_file(staged);
    }
    published
}

fn staging_error(
    staged: &Path,
    _target: &Path,
    _protection: FileProtection,
    error: io::Error,
) -> LocalError {
    #[cfg(all(windows, feature = "windows-private-preview"))]
    if matches!(_protection, FileProtection::OwnerOnly)
        && super::private_windows::unsupported(&error)
    {
        return LocalError::PrivatePublicationUnavailable(_target.to_path_buf());
    }
    LocalError::io("create staged file", staged, error)
}

fn protect_sibling(
    file: &fs::File,
    staged: &Path,
    target: &Path,
    protection: FileProtection,
) -> Result<(), LocalError> {
    #[cfg(windows)]
    let _ = (file, staged, target, protection);
    #[cfg(unix)]
    if matches!(protection, FileProtection::OwnerOnly) {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(fs::Permissions::from_mode(0o600))
            .map_err(|error| LocalError::io("protect staged file", staged, error))?;
        if !super::private_file_mode(file)
            .map_err(|error| LocalError::io("inspect staged file protection", staged, error))?
        {
            return Err(LocalError::PrivatePublicationUnavailable(
                target.to_path_buf(),
            ));
        }
    }
    #[cfg(all(windows, feature = "windows-private-preview"))]
    if matches!(protection, FileProtection::OwnerOnly) {
        use super::private_windows::{is_owner_only, protection_error, supports_private_files};

        let protected = supports_private_files(file)
            .and_then(|supported| {
                if supported {
                    is_owner_only(file)
                } else {
                    Ok(false)
                }
            })
            .map_err(|error| protection_error("inspect staged file protection", target, error))?;
        if !protected {
            return Err(LocalError::PrivatePublicationUnavailable(
                target.to_path_buf(),
            ));
        }
    }
    Ok(())
}

fn rename_sibling(
    file: fs::File,
    staged: &Path,
    target: &Path,
    bytes: u64,
    admission: FileAdmission,
    protection: FileProtection,
) -> Result<FilePublication, LocalError> {
    #[cfg(any(unix, all(windows, feature = "windows-private-preview")))]
    if matches!(protection, FileProtection::OwnerOnly) {
        file.sync_all()
            .map_err(|error| LocalError::io("sync private file", staged, error))?;
    }
    drop(file);
    let predecessor = match admission {
        FileAdmission::Absent => None,
        FileAdmission::ReplaceRegularFile => regular_file_metadata(target)?,
        #[cfg(any(feature = "blake3", feature = "sha2"))]
        FileAdmission::ReplaceMatching(expected) => Some(matching_predecessor(target, &expected)?),
    };
    let outcome = match predecessor {
        None => {
            publish_absent_file(staged, target, protection)?;
            CommitOutcome::Created
        }
        Some(metadata) => {
            if matches!(protection, FileProtection::Ordinary) {
                inherit_mode(&metadata, staged)?;
            }
            rename_replace(staged, target)?;
            CommitOutcome::Replaced
        }
    };
    Ok(FilePublication { outcome, bytes })
}

#[cfg(any(feature = "blake3", feature = "sha2"))]
fn matching_predecessor(
    target: &Path,
    expected: &crate::hash::DigestValue,
) -> Result<fs::Metadata, LocalError> {
    let file = LocalPath::new(target)?
        .open_file()
        .map_err(|error| match error {
            LocalError::NotRegularFile { path, kind } => LocalError::OccupiedTarget { path, kind },
            other => other,
        })?
        .ok_or_else(|| LocalError::PredecessorChanged(target.to_path_buf()))?;
    let metadata = file
        .metadata()
        .map_err(|error| LocalError::io("read predecessor metadata", target, error))?;
    let actual = file.digest(expected.algorithm()).map_err(|error| {
        LocalError::io("read predecessor digest", target, io::Error::other(error))
    })?;
    if &actual.digest != expected {
        return Err(LocalError::PredecessorChanged(target.to_path_buf()));
    }
    Ok(metadata)
}

fn publish_absent_file(
    staged: &Path,
    target: &Path,
    _protection: FileProtection,
) -> Result<(), LocalError> {
    #[cfg(unix)]
    if matches!(_protection, FileProtection::OwnerOnly) {
        match fs::hard_link(staged, target) {
            Ok(()) => {
                let _ = fs::remove_file(staged);
                return Ok(());
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                return Err(LocalError::TargetExists(target.to_path_buf()));
            }
            Err(error) => return Err(LocalError::io("publish private file", target, error)),
        }
    }
    rename_no_replace(staged, target)
}

/// A candidate hidden name beside the target.
///
/// The process id separates concurrent writers and the counter separates sequential ones; the
/// name is only a proposal until `create_new` accepts it.
fn sibling_temp(target: &Path, parent: &Path) -> Result<PathBuf, LocalError> {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let name = target
        .file_name()
        .ok_or_else(|| LocalError::InvalidPath(target.to_path_buf()))?;
    let pid = std::process::id();
    loop {
        let serial = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let candidate = format!(".pulith-{pid}-{serial}");
        if !name.to_str().is_some_and(|name| {
            name.trim_end_matches(['.', ' '])
                .eq_ignore_ascii_case(&candidate)
        }) {
            return Ok(parent.join(candidate));
        }
    }
}

fn create_sibling_file(path: &Path, _protection: FileProtection) -> io::Result<fs::File> {
    #[cfg(unix)]
    if matches!(_protection, FileProtection::OwnerOnly) {
        use std::os::unix::fs::OpenOptionsExt;
        return fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path);
    }
    #[cfg(all(windows, feature = "windows-private-preview"))]
    if matches!(_protection, FileProtection::OwnerOnly) {
        return super::private_windows::create_file(path);
    }
    create_staged_file(path)
}

fn create_staged_file(path: &Path) -> io::Result<fs::File> {
    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn private_publication_replaces_regular_files_without_inheriting_public_mode() {
        use std::os::unix::fs::{PermissionsExt, symlink};

        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("target");
        let receipt = publish_private_file(&target, b"first", FileAdmission::Absent).unwrap();
        assert_eq!(receipt.outcome, CommitOutcome::Created);
        assert!(matches!(
            publish_private_file(&target, b"other", FileAdmission::Absent),
            Err(LocalError::TargetExists(_))
        ));
        fs::set_permissions(&target, fs::Permissions::from_mode(0o644)).unwrap();
        let receipt =
            publish_private_file(&target, b"second", FileAdmission::ReplaceRegularFile).unwrap();
        assert_eq!(receipt.outcome, CommitOutcome::Replaced);
        assert_eq!(
            fs::metadata(&target).unwrap().permissions().mode() & 0o777,
            0o600
        );
        let link = dir.path().join("link");
        symlink(&target, &link).unwrap();
        assert!(matches!(
            publish_private_file(&link, b"blocked", FileAdmission::ReplaceRegularFile),
            Err(LocalError::OccupiedTarget { .. })
        ));
        assert_eq!(fs::read(&target).unwrap(), b"second");
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 2);
    }

    #[test]
    fn publication_admits_long_file_names_and_rejects_embedded_nul() {
        let root = tempfile::tempdir().unwrap();
        let invalid = root.path().join("unexpected\0suffix");
        assert!(matches!(
            LocalPath::new(invalid),
            Err(LocalError::InvalidPath(_))
        ));
        assert!(!root.path().join("unexpected").exists());

        let target = root.path().join("a".repeat(250));
        let receipt = LocalPath::new(&target)
            .unwrap()
            .publish_file(b"new", FileAdmission::Absent)
            .unwrap();
        assert_eq!(receipt.outcome, CommitOutcome::Created);
        assert_eq!(fs::read(target).unwrap(), b"new");
    }

    #[test]
    fn publish_file_keeps_a_sibling_it_could_not_create() {
        let root = tempfile::tempdir().unwrap();
        let target = root.path().join("target");
        let staged = root.path().join("sibling");
        fs::write(&target, b"predecessor").unwrap();
        fs::write(&staged, b"someone else's bytes").unwrap();

        let error = publish_sibling(
            &staged,
            &target,
            FileAdmission::ReplaceRegularFile,
            FileProtection::Ordinary,
            std::convert::identity,
            |file, staged| write_bytes(file, staged, b"new bytes"),
        )
        .unwrap_err();

        assert!(matches!(
            error,
            LocalError::Io { path, source, .. }
                if path == staged && source.kind() == io::ErrorKind::AlreadyExists
        ));
        assert_eq!(fs::read(&staged).unwrap(), b"someone else's bytes");
        assert_eq!(fs::read(&target).unwrap(), b"predecessor");
    }

    #[cfg(feature = "sha2")]
    #[test]
    fn matching_publication_returns_effects_only_for_the_expected_predecessor() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("file");
        let target = LocalPath::new(&path).unwrap();
        let based_on = FileAdmission::ReplaceMatching(crate::hash::DigestValue::sha256(b"old"));
        assert!(matches!(
            target.publish_file(b"new", based_on.clone()),
            Err(LocalError::PredecessorChanged(_))
        ));
        assert!(!path.exists());
        fs::write(&path, b"changed").unwrap();
        assert!(matches!(
            target.publish_file(b"new", based_on.clone()),
            Err(LocalError::PredecessorChanged(_))
        ));
        assert_eq!(fs::read(&path).unwrap(), b"changed");
        fs::write(&path, b"old").unwrap();
        let published = target.publish_file(b"new", based_on).unwrap();
        assert_eq!(
            published,
            FilePublication {
                outcome: CommitOutcome::Replaced,
                bytes: 3
            }
        );
        assert_eq!(fs::read(&path).unwrap(), b"new");
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    }
}

impl StagedTree {
    /// Writes generated bytes into this private stage without exposing a partial target.
    pub fn write_file(
        self,
        bytes: impl AsRef<[u8]>,
        destination: impl Into<PathBuf>,
    ) -> Result<Self, LocalError> {
        use std::io::Write;
        let (mut file, destination) = self.create_file(destination.into())?;
        file.write_all(bytes.as_ref())
            .map_err(|error| LocalError::io("write generated stage file", &destination, error))?;
        Ok(self)
    }

    pub(super) fn create_file(&self, relative: PathBuf) -> Result<(fs::File, PathBuf), LocalError> {
        let destination = stage_destination(&self.root, relative)?;
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| LocalError::io("create stage file parent", parent, error))?;
        }
        let file = create_staged_file(&destination)
            .map_err(|error| LocalError::io("create stage file", &destination, error))?;
        Ok((file, destination))
    }

    /// Copies one admitted regular file into a contained path in this private stage.
    pub fn copy_file(
        self,
        source: LocalPath,
        destination: impl Into<PathBuf>,
    ) -> Result<Self, LocalError> {
        let destination = stage_destination(&self.root, destination.into())?;
        let source = source.0;
        let input = open_copy_source(&source)?;
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| LocalError::io("create stage file parent", parent, error))?;
        }
        copy_open_file(input, &destination)
            .map_err(|error| LocalError::io("copy file into stage", &source, error))?;
        Ok(self)
    }

    /// Recursively copies one admitted directory tree into a contained stage path.
    pub fn copy_tree(
        self,
        source: LocalPath,
        destination: impl Into<PathBuf>,
    ) -> Result<Self, LocalError> {
        let destination = stage_destination(&self.root, destination.into())?;
        let source = source.0;
        let metadata = fs::symlink_metadata(&source)
            .map_err(|error| LocalError::io("inspect stage tree source", &source, error))?;
        if entry_kind(&source, &metadata)? != LocalEntryKind::Directory {
            return Err(LocalError::UnsupportedLocalEntry(source));
        }
        copy_directory_to_stage(&source, &destination)?;
        Ok(self)
    }

    /// Atomically commits this complete stage under an explicit [`TreeAdmission`].
    ///
    /// The staged tree is re-staged beside the target when it was built elsewhere, so the final
    /// rename is always a same-directory operation: that is what makes the commit atomic and, on
    /// Windows, what lets the new file inherit the target parent's ACL.
    pub fn publish(
        self,
        target: LocalPath,
        admission: TreeAdmission,
    ) -> Result<TreePublication, LocalError> {
        let target_path = target.0;
        let target_parent = target_parent(&target_path)?
            .canonicalize()
            .map_err(|error| LocalError::io("canonicalize publish parent", &target_path, error))?;

        let StagedTree {
            workspace,
            root,
            placement,
        } = self;
        let (workspace, root) = if matches!(&placement, StagePlacement::Destination(parent) if parent == &target_parent)
        {
            (workspace, root)
        } else {
            let destination = stage(&target_path)?;
            let destination = if fs::symlink_metadata(&root)
                .map_err(|error| LocalError::io("inspect staged root", &root, error))?
                .file_type()
                .is_dir()
            {
                destination.copy_tree(LocalPath(root), PathBuf::new())?
            } else {
                destination.copy_file(LocalPath(root), PathBuf::new())?
            };
            drop(workspace);
            let StagedTree {
                workspace, root, ..
            } = destination;
            (workspace, root)
        };

        let stats = inspect_regular_tree(&root)?;
        match admission {
            TreeAdmission::Absent => rename_no_replace(&root, &target_path)?,
            TreeAdmission::ReplaceRegularFile => match regular_file_metadata(&target_path)? {
                None => rename_no_replace(&root, &target_path)?,
                Some(metadata) => {
                    if !fs::symlink_metadata(&root)
                        .map_err(|error| LocalError::io("inspect staged root", &root, error))?
                        .file_type()
                        .is_file()
                    {
                        return Err(LocalError::InvalidPreparation(format!(
                            "a regular-file target cannot be replaced by a staged directory: {}",
                            target_path.display()
                        )));
                    }
                    inherit_mode(&metadata, &root)?;
                    rename_replace(&root, &target_path)?;
                }
            },
        }
        drop(workspace);
        Ok(TreePublication {
            files: stats.files,
            directories: stats.directories,
            bytes: stats.bytes,
        })
    }
}

fn stage_destination(root: &Path, relative: PathBuf) -> Result<PathBuf, LocalError> {
    use std::path::Component;
    if !relative.as_os_str().is_empty()
        && (relative.is_absolute()
            || relative
                .components()
                .any(|component| !matches!(component, Component::Normal(_))))
    {
        return Err(LocalError::InvalidStagePath(relative));
    }
    Ok(if relative.as_os_str().is_empty() {
        root.to_path_buf()
    } else {
        root.join(relative)
    })
}

#[derive(Clone, Copy, Default)]
struct TreeStats {
    files: usize,
    directories: usize,
    bytes: u64,
}

enum RegularEntry {
    File,
    Directory,
}

impl RegularEntry {
    fn admit(entry: &walkdir::DirEntry) -> Result<Self, LocalError> {
        let kind = entry.file_type();
        if !kind.is_symlink() && !is_reparse(entry)? {
            if kind.is_file() {
                return Ok(Self::File);
            }
            if kind.is_dir() {
                return Ok(Self::Directory);
            }
        }
        Err(LocalError::UnsupportedLocalEntry(entry.path().into()))
    }
}

#[cfg(unix)]
fn is_reparse(_: &walkdir::DirEntry) -> Result<bool, LocalError> {
    Ok(false)
}

#[cfg(windows)]
fn is_reparse(entry: &walkdir::DirEntry) -> Result<bool, LocalError> {
    let metadata = entry.metadata().map_err(|error| {
        LocalError::io(
            "inspect staged entry",
            entry.path(),
            io::Error::other(error),
        )
    })?;
    use std::os::windows::fs::MetadataExt;
    use windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT;

    Ok(metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0)
}

fn walk_error(action: &'static str, root: &Path, error: walkdir::Error) -> LocalError {
    let path = error.path().unwrap_or(root).to_path_buf();
    LocalError::io(action, path, io::Error::other(error))
}

fn inspect_regular_tree(root: &Path) -> Result<TreeStats, LocalError> {
    let metadata = fs::symlink_metadata(root)
        .map_err(|error| LocalError::io("inspect staged root", root, error))?;
    let kind = entry_kind(root, &metadata)?;
    if kind == LocalEntryKind::File {
        return Ok(TreeStats {
            files: 1,
            directories: 0,
            bytes: metadata.len(),
        });
    }
    if kind != LocalEntryKind::Directory {
        return Err(LocalError::UnsupportedLocalEntry(root.to_path_buf()));
    }
    let mut stats = TreeStats::default();
    for entry in walkdir::WalkDir::new(root)
        .follow_links(false)
        .follow_root_links(false)
    {
        let entry = entry.map_err(|error| walk_error("walk staged tree", root, error))?;
        match RegularEntry::admit(&entry)? {
            RegularEntry::Directory => stats.directories += 1,
            RegularEntry::File => {
                stats.files += 1;
                stats.bytes += entry
                    .metadata()
                    .map_err(|error| {
                        LocalError::io(
                            "inspect staged file",
                            entry.path(),
                            io::Error::other(error.to_string()),
                        )
                    })?
                    .len();
            }
        }
    }
    Ok(stats)
}

/// Carries the replaced target's permission bits onto the staged file before the rename.
///
/// POSIX only: a rename keeps the *source* file's mode, so the target's mode has to travel with
/// the staged file. Windows has no POSIX mode, and its only permission bit (read-only) makes a
/// target unreplaceable, so nothing is inherited there.
#[cfg(unix)]
fn inherit_mode(predecessor: &fs::Metadata, staged: &Path) -> Result<(), LocalError> {
    use std::os::unix::fs::PermissionsExt;
    let mode = predecessor.permissions().mode() & 0o7777;
    fs::set_permissions(staged, fs::Permissions::from_mode(mode))
        .map_err(|error| LocalError::io("apply replaced target mode", staged, error))
}

#[cfg(windows)]
fn inherit_mode(_predecessor: &fs::Metadata, _staged: &Path) -> Result<(), LocalError> {
    Ok(())
}

/// Replaces an existing regular file with the staged file in one rename.
///
/// [`std::fs::rename`] is already a replacing rename on every supported platform.
fn rename_replace(source: &Path, target: &Path) -> Result<(), LocalError> {
    fs::rename(source, target).map_err(|error| LocalError::io("replace target", target, error))
}

#[cfg(unix)]
fn rename_no_replace(source: &Path, target: &Path) -> Result<(), LocalError> {
    use rustix::fs::{CWD, RenameFlags, renameat_with};
    use rustix::io::Errno;
    renameat_with(CWD, source, CWD, target, RenameFlags::NOREPLACE).map_err(|error| {
        if matches!(error, Errno::EXIST | Errno::NOTEMPTY) {
            LocalError::TargetExists(target.to_path_buf())
        } else if matches!(error, Errno::NOSYS | Errno::OPNOTSUPP) {
            LocalError::PublishUnavailable {
                path: target.to_path_buf(),
                source: io::Error::from(error),
            }
        } else {
            LocalError::io("publish target", target, io::Error::from(error))
        }
    })
}

#[cfg(windows)]
fn rename_no_replace(source: &Path, target: &Path) -> Result<(), LocalError> {
    use windows_sys::Win32::Foundation::{ERROR_ALREADY_EXISTS, ERROR_FILE_EXISTS};
    let wide = |path| {
        super::wide_path(path)
            .map_err(|error| LocalError::io("encode publication path", path, error))
    };
    let source = wide(source)?;
    let target_wide = wide(target)?;
    // SAFETY: both buffers are NUL-terminated and live for the duration of the call.
    let moved = unsafe {
        windows_sys::Win32::Storage::FileSystem::MoveFileExW(
            source.as_ptr(),
            target_wide.as_ptr(),
            0,
        )
    };
    if moved != 0 {
        Ok(())
    } else {
        let error = io::Error::last_os_error();
        if matches!(
            error.raw_os_error().map(|code| code as u32),
            Some(ERROR_ALREADY_EXISTS) | Some(ERROR_FILE_EXISTS)
        ) {
            Err(LocalError::TargetExists(target.to_path_buf()))
        } else {
            Err(LocalError::io("publish target", target, error))
        }
    }
}
/// Remove one exact local target directly (no source, no verification, no ownership claim).
impl LocalPath {
    /// Removes this entry or tree and reports whether it was present.
    pub fn remove(&self) -> Result<RemoveEvidence, LocalError> {
        let target = self.as_path();
        match remove_existing(target) {
            Ok(()) => Ok(RemoveEvidence::Removed),
            Err(LocalError::Io { source, .. }) if source.kind() == io::ErrorKind::NotFound => {
                Ok(RemoveEvidence::Unchanged)
            }
            Err(error) => Err(error),
        }
    }
}

fn copy_directory_to_stage(source: &Path, stage: &Path) -> Result<(), LocalError> {
    for entry in walkdir::WalkDir::new(source)
        .follow_links(false)
        .follow_root_links(false)
    {
        let entry = entry.map_err(|error| walk_error("walk source directory", source, error))?;
        let relative = entry.path().strip_prefix(source).map_err(|err| {
            LocalError::io(
                "strip source prefix",
                entry.path(),
                io::Error::other(err.to_string()),
            )
        })?;
        let destination = if relative.as_os_str().is_empty() {
            stage.to_path_buf()
        } else {
            stage.join(relative)
        };
        match RegularEntry::admit(&entry)? {
            RegularEntry::Directory => {
                fs::create_dir_all(&destination)
                    .map_err(|err| LocalError::io("create staged directory", &destination, err))?;
            }
            RegularEntry::File => {
                if let Some(parent) = destination.parent() {
                    fs::create_dir_all(parent)
                        .map_err(|err| LocalError::io("create staged file parent", parent, err))?;
                }
                let input = open_copy_source(entry.path())?;
                copy_open_file(input, &destination).map_err(|err| {
                    LocalError::io("copy file to staged directory", &destination, err)
                })?;
            }
        }
    }

    Ok(())
}

fn open_copy_source(path: &Path) -> Result<OpenedFile, LocalError> {
    match LocalPath::new(path)?.open_file() {
        Ok(Some(file)) => Ok(file),
        Ok(None) => Err(LocalError::io(
            "open stage source",
            path,
            io::ErrorKind::NotFound.into(),
        )),
        Err(LocalError::NotRegularFile { path, .. }) => {
            Err(LocalError::UnsupportedLocalEntry(path))
        }
        Err(error) => Err(error),
    }
}

pub(crate) fn copy_open_file(mut input: OpenedFile, destination: &Path) -> io::Result<()> {
    let permissions = input.metadata()?.permissions();
    let mut output = create_staged_file(destination)?;
    io::copy(&mut input, &mut output)?;
    output.set_permissions(permissions)
}

fn remove_existing(path: &Path) -> Result<(), LocalError> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|err| LocalError::io("read target metadata", path, err))?;
    if metadata.file_type().is_dir() && !metadata.file_type().is_symlink() {
        fs::remove_dir_all(path).map_err(|err| LocalError::io("remove directory", path, err))
    } else {
        fs::remove_file(path).map_err(|err| LocalError::io("remove file", path, err))
    }
}

fn target_parent(target: &Path) -> Result<&Path, LocalError> {
    target
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .ok_or_else(|| {
            LocalError::InvalidPreparation(format!(
                "target path must have a parent directory: {}",
                target.display()
            ))
        })
}
