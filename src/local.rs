use std::fs::{self, File};
use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::evidence::LocalApplyStats;
pub use crate::evidence::{ApplyEvidence, LocalAcquireEvidence, LocalPlacement};
use crate::{
    Acquire, Acquired, Applied, Apply, EvidenceChain, Forget, Inspect, Inspected, Materialize,
    MaterializeMode, PulithError, Reconcile, Reconciled, Verified,
};

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

#[derive(Clone, Debug, Default)]
pub struct LocalAcquire;

impl<I, T> Acquire<Materialize<I, LocalPath, T>> for LocalAcquire {
    type Error = PulithError;
    type Output = Acquired<Materialize<I, LocalPath, T>, LocalMaterial, LocalAcquireEvidence>;

    fn acquire(&self, node: Materialize<I, LocalPath, T>) -> Result<Self::Output, Self::Error> {
        let path = node.source.path.clone();
        if !path.exists() {
            return Err(PulithError::MissingSource(path));
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

/// No-follow local filesystem entry classification.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LocalEntryKind {
    Missing,
    File,
    Directory,
    Symlink,
    Other,
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
    type Error = PulithError;
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
            Err(error) => return Err(PulithError::io("inspect local target", &node.path, error)),
        };

        Ok(Inspected {
            input: node,
            observation,
            evidence: LocalInspectEvidence,
        })
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

#[derive(Clone, Copy, Debug, Default)]
pub struct LocalApply;

type LocalApplied<I, S, E> =
    Applied<Materialize<I, S, LocalTarget>, EvidenceChain<E, ApplyEvidence>>;

impl<I, S, E> Apply<Acquired<Materialize<I, S, LocalTarget>, LocalMaterial, E>> for LocalApply {
    type Error = PulithError;
    type Output = Applied<Materialize<I, S, LocalTarget>, EvidenceChain<E, ApplyEvidence>>;

    fn apply(
        &self,
        node: Acquired<Materialize<I, S, LocalTarget>, LocalMaterial, E>,
    ) -> Result<Self::Output, Self::Error> {
        apply_material(node.input, node.material, node.evidence)
    }
}

impl<I, S, E> Apply<Verified<Materialize<I, S, LocalTarget>, LocalMaterial, E>> for LocalApply {
    type Error = PulithError;
    type Output = Applied<Materialize<I, S, LocalTarget>, EvidenceChain<E, ApplyEvidence>>;

    fn apply(
        &self,
        node: Verified<Materialize<I, S, LocalTarget>, LocalMaterial, E>,
    ) -> Result<Self::Output, Self::Error> {
        apply_material(node.input, node.material, node.evidence)
    }
}

/// Removes the exact caller-authorized local target without acquiring a source.
impl<I> Apply<Forget<I, LocalTarget>> for LocalApply {
    type Error = PulithError;
    type Output = Applied<Forget<I, LocalTarget>, ApplyEvidence>;

    fn apply(&self, node: Forget<I, LocalTarget>) -> Result<Self::Output, Self::Error> {
        match remove_existing(&node.target.path) {
            Ok(()) => {}
            Err(PulithError::Io { source, .. }) if source.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        Ok(Applied {
            input: node,
            evidence: ApplyEvidence::removed(),
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PublishMode {
    Create,
    Replace,
    CreateOrReplace,
}

pub(crate) fn apply_material<I, S, E>(
    input: Materialize<I, S, LocalTarget>,
    material: LocalMaterial,
    evidence: E,
) -> Result<LocalApplied<I, S, E>, PulithError> {
    let target = input.target.path.clone();
    let mode = match input.mode {
        MaterializeMode::Create => {
            if target.exists() {
                return Err(PulithError::ApplyWouldOverwrite(target));
            }
            PublishMode::Create
        }
        MaterializeMode::Replace => {
            if !target.exists() {
                return Err(PulithError::ApplyMissingTarget(target));
            }
            PublishMode::Replace
        }
        MaterializeMode::CreateOrReplace => PublishMode::CreateOrReplace,
    };
    reject_unsupported_entry(material.path())?;
    reject_same_source_target(material.path(), &target)?;
    let stats = match &material {
        LocalMaterial::File { path } => publish_file(path, &target, mode)?,
        LocalMaterial::Directory { path } => publish_directory(path, &target, mode)?,
        LocalMaterial::StagedFile { path } => publish_file(path.as_ref(), &target, mode)?,
    };
    Ok(Applied {
        input,
        evidence: EvidenceChain {
            previous: evidence,
            current: ApplyEvidence::new(stats),
        },
    })
}

fn publish_file(
    source: &Path,
    target: &Path,
    mode: PublishMode,
) -> Result<LocalApplyStats, PulithError> {
    let parent = target_parent(target)?;
    fs::create_dir_all(parent)
        .map_err(|err| PulithError::io("create parent directory", parent, err))?;

    let mut source_file =
        File::open(source).map_err(|err| PulithError::io("open source file", source, err))?;
    let mut staged = tempfile::NamedTempFile::new_in(parent)
        .map_err(|err| PulithError::io("create staged file", parent, err))?;
    let bytes = io::copy(&mut source_file, staged.as_file_mut())
        .map_err(|err| PulithError::io("copy file to staged file", source, err))?;

    match mode {
        PublishMode::Create => {
            staged
                .persist_noclobber(target)
                .map_err(|err| PulithError::io("persist staged file", target, err.error))?;
        }
        PublishMode::Replace | PublishMode::CreateOrReplace => {
            staged
                .persist(target)
                .map_err(|err| PulithError::io("persist staged file", target, err.error))?;
        }
    }

    Ok(LocalApplyStats::copied_file(bytes))
}

fn publish_directory(
    source: &Path,
    target: &Path,
    mode: PublishMode,
) -> Result<LocalApplyStats, PulithError> {
    reject_directory_conflict(source, target)?;

    let parent = target_parent(target)?;
    fs::create_dir_all(parent)
        .map_err(|err| PulithError::io("create parent directory", parent, err))?;

    let staging = tempfile::Builder::new()
        .prefix(".pulith-stage-")
        .tempdir_in(parent)
        .map_err(|err| PulithError::io("create staged directory", parent, err))?;
    let stats = copy_directory_to_stage(source, staging.path())?;
    let staged_path = staging.keep();

    let result = match mode {
        PublishMode::Create => rename_dir(&staged_path, target),
        PublishMode::Replace => replace_directory_with_backup(&staged_path, target),
        PublishMode::CreateOrReplace if target.exists() => {
            replace_directory_with_backup(&staged_path, target)
        }
        PublishMode::CreateOrReplace => rename_dir(&staged_path, target),
    };

    if let Err(error) = result {
        let _ = fs::remove_dir_all(&staged_path);
        return Err(error);
    }

    Ok(stats)
}

fn copy_directory_to_stage(source: &Path, stage: &Path) -> Result<LocalApplyStats, PulithError> {
    let mut files = 0usize;
    let mut directories = 0usize;
    let mut bytes = 0u64;

    for entry in walkdir::WalkDir::new(source).follow_links(false) {
        let entry = entry.map_err(|err| {
            PulithError::io(
                "walk source directory",
                err.path().unwrap_or(source),
                io::Error::other(err.to_string()),
            )
        })?;
        let file_type = entry.file_type();
        if file_type.is_symlink() {
            return Err(PulithError::UnsupportedLocalEntry(
                entry.path().to_path_buf(),
            ));
        }

        let relative = entry.path().strip_prefix(source).map_err(|err| {
            PulithError::io(
                "strip source prefix",
                entry.path(),
                io::Error::other(err.to_string()),
            )
        })?;
        if relative.as_os_str().is_empty() {
            directories += 1;
            continue;
        }

        let destination = stage.join(relative);
        if file_type.is_dir() {
            fs::create_dir_all(&destination)
                .map_err(|err| PulithError::io("create staged directory", &destination, err))?;
            directories += 1;
        } else if file_type.is_file() {
            if let Some(parent) = destination.parent() {
                fs::create_dir_all(parent)
                    .map_err(|err| PulithError::io("create staged file parent", parent, err))?;
            }
            let copied = fs::copy(entry.path(), &destination).map_err(|err| {
                PulithError::io("copy file to staged directory", &destination, err)
            })?;
            files += 1;
            bytes += copied;
        } else {
            return Err(PulithError::UnsupportedLocalEntry(
                entry.path().to_path_buf(),
            ));
        }
    }

    Ok(LocalApplyStats::copied_tree(files, directories, bytes))
}

fn replace_directory_with_backup(staged_path: &Path, target: &Path) -> Result<(), PulithError> {
    let backup = backup_path(target);
    rename_dir(target, &backup)?;

    match rename_dir(staged_path, target) {
        Ok(()) => {
            // Publication is already committed. Cleanup is best-effort so callers are not told
            // that apply failed after the new target became live.
            let _ = remove_existing(&backup);
            Ok(())
        }
        Err(error) => {
            let _ = rename_dir(&backup, target);
            Err(error)
        }
    }
}

fn rename_dir(source: &Path, target: &Path) -> Result<(), PulithError> {
    fs::rename(source, target).map_err(|err| PulithError::io("rename directory", target, err))
}

fn remove_existing(path: &Path) -> Result<(), PulithError> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|err| PulithError::io("read target metadata", path, err))?;
    if metadata.file_type().is_dir() && !metadata.file_type().is_symlink() {
        fs::remove_dir_all(path).map_err(|err| PulithError::io("remove directory", path, err))
    } else {
        fs::remove_file(path).map_err(|err| PulithError::io("remove file", path, err))
    }
}

fn reject_unsupported_entry(path: &Path) -> Result<(), PulithError> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|err| PulithError::io("read source metadata", path, err))?;
    let file_type = metadata.file_type();
    if file_type.is_symlink() || !(file_type.is_file() || file_type.is_dir()) {
        return Err(PulithError::UnsupportedLocalEntry(path.to_path_buf()));
    }
    Ok(())
}

fn reject_same_source_target(source: &Path, target: &Path) -> Result<(), PulithError> {
    if target.exists() {
        let is_same = same_file::is_same_file(source, target)
            .map_err(|err| PulithError::io("compare source and target", target, err))?;
        if is_same {
            return Err(PulithError::ApplySameFile(target.to_path_buf()));
        }
    }
    Ok(())
}

fn reject_directory_conflict(source: &Path, target: &Path) -> Result<(), PulithError> {
    let source = source
        .canonicalize()
        .map_err(|err| PulithError::io("canonicalize source directory", source, err))?;
    let target_candidate = canonical_target_candidate(target)?;

    if target_candidate.starts_with(&source) || source.starts_with(&target_candidate) {
        return Err(PulithError::ApplyPathConflict {
            source,
            target: target_candidate,
        });
    }
    Ok(())
}

fn canonical_target_candidate(target: &Path) -> Result<PathBuf, PulithError> {
    if target.exists() {
        return target
            .canonicalize()
            .map_err(|err| PulithError::io("canonicalize target", target, err));
    }

    let parent = target_parent(target)?;
    let parent = parent
        .canonicalize()
        .map_err(|err| PulithError::io("canonicalize target parent", parent, err))?;
    Ok(match target.file_name() {
        Some(name) => parent.join(name),
        None => parent,
    })
}

fn target_parent(target: &Path) -> Result<&Path, PulithError> {
    target
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .ok_or_else(|| {
            PulithError::InvalidPreparation(format!(
                "target path must have a parent directory: {}",
                target.display()
            ))
        })
}

fn backup_path(target: &Path) -> PathBuf {
    let mut backup = target.to_path_buf();
    let name = target
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("target");
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    backup.set_file_name(format!(".{name}.pulith-backup-{nonce}"));
    backup
}

#[cfg(test)]
mod tests {
    use std::fs;

    #[cfg(unix)]
    use std::os::unix::fs::symlink as symlink_file;
    #[cfg(windows)]
    use std::os::windows::fs::symlink_file;

    use super::*;
    use crate::{Acquire, Apply, Forget, Inspect, Materialize, MaterializeMode, Reconcile};

    fn temp_root(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "pulith-{name}-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn local_tree_runs_create_or_replace_file() {
        let root = temp_root("tree-file");
        let source = root.join("source.txt");
        let target = root.join("target").join("resource.txt");
        fs::create_dir_all(&root).unwrap();
        fs::write(&source, "pulith").unwrap();

        let applied = LocalApply
            .apply(acquire(MaterializeMode::CreateOrReplace, &source, &target))
            .unwrap();

        assert_eq!(fs::read_to_string(&target).unwrap(), "pulith");
        assert_eq!(applied.input.item, "demo");
        assert_eq!(applied.evidence.current.strategy, LocalPlacement::Copied);

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn staged_file_apply_publishes_and_releases_custody() {
        let root = temp_root("staged-file-apply");
        let target = root.join("target.bin");
        fs::create_dir_all(&root).unwrap();
        let staged = tempfile::NamedTempFile::new_in(&root).unwrap();
        fs::write(staged.path(), b"staged").unwrap();
        let staged_path = staged.path().to_path_buf();
        let node = crate::Acquired {
            input: Materialize::new(
                "demo",
                LocalPath::new(&staged_path),
                LocalTarget::new(&target),
                MaterializeMode::Create,
            ),
            material: LocalMaterial::StagedFile {
                path: staged.into_temp_path(),
            },
            evidence: (),
        };

        LocalApply.apply(node).unwrap();

        assert_eq!(fs::read(&target).unwrap(), b"staged");
        assert!(!staged_path.exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn staged_file_apply_failure_releases_custody_without_touching_target() {
        let root = temp_root("staged-file-apply-fail");
        let target = root.join("target.bin");
        fs::create_dir_all(&root).unwrap();
        fs::write(&target, b"original").unwrap();
        let staged = tempfile::NamedTempFile::new_in(&root).unwrap();
        fs::write(staged.path(), b"replacement").unwrap();
        let staged_path = staged.path().to_path_buf();
        let node = crate::Acquired {
            input: Materialize::new(
                "demo",
                LocalPath::new(&staged_path),
                LocalTarget::new(&target),
                MaterializeMode::Create,
            ),
            material: LocalMaterial::StagedFile {
                path: staged.into_temp_path(),
            },
            evidence: (),
        };

        assert!(LocalApply.apply(node).is_err());

        assert_eq!(fs::read(&target).unwrap(), b"original");
        assert!(!staged_path.exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn staged_file_publication_io_failure_releases_custody() {
        let root = temp_root("staged-file-publication-fail");
        let blocked_parent = root.join("blocked");
        let target = blocked_parent.join("target.bin");
        fs::create_dir_all(&root).unwrap();
        fs::write(&blocked_parent, b"existing parent file").unwrap();
        let staged = tempfile::NamedTempFile::new_in(&root).unwrap();
        fs::write(staged.path(), b"staged").unwrap();
        let staged_path = staged.path().to_path_buf();
        let node = crate::Acquired {
            input: Materialize::new(
                "demo",
                LocalPath::new(&staged_path),
                LocalTarget::new(&target),
                MaterializeMode::Create,
            ),
            material: LocalMaterial::StagedFile {
                path: staged.into_temp_path(),
            },
            evidence: (),
        };

        assert!(LocalApply.apply(node).is_err());

        assert_eq!(fs::read(&blocked_parent).unwrap(), b"existing parent file");
        assert!(!target.exists());
        assert!(!staged_path.exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn create_and_replace_are_explicit_apply_laws() {
        let root = temp_root("tree-apply");
        let source = root.join("source.txt");
        let target = root.join("target.txt");
        fs::create_dir_all(&root).unwrap();
        fs::write(&source, "first").unwrap();
        fs::write(&target, "existing").unwrap();

        assert!(
            LocalApply
                .apply(acquire(MaterializeMode::Create, &source, &target))
                .is_err()
        );
        assert_eq!(fs::read_to_string(&target).unwrap(), "existing");

        let replaced = LocalApply
            .apply(acquire(MaterializeMode::Replace, &source, &target))
            .unwrap();
        assert_eq!(fs::read_to_string(&target).unwrap(), "first");
        assert_eq!(replaced.evidence.current.files, 1);
        assert_eq!(replaced.evidence.current.directories, 0);
        assert_eq!(replaced.evidence.current.bytes, 5);
        assert_eq!(replaced.evidence.current.strategy, LocalPlacement::Copied);

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn forget_applies_directly_without_acquiring_a_source() {
        let root = temp_root("forget-direct");
        let target = root.join("target.txt");
        fs::create_dir_all(&root).unwrap();
        fs::write(&target, "obsolete").unwrap();

        let applied = LocalApply
            .apply(Forget::new("demo", LocalTarget::new(&target)))
            .unwrap();

        assert!(!target.exists());
        assert_eq!(applied.input.item, "demo");
        assert_eq!(applied.input.target.path, target);
        assert_eq!(applied.evidence.strategy, LocalPlacement::Removed);

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn forget_is_idempotent_when_target_is_absent() {
        let root = temp_root("forget-absent");
        let target = root.join("missing.txt");
        fs::create_dir_all(&root).unwrap();

        let applied = LocalApply
            .apply(Forget::new("demo", LocalTarget::new(&target)))
            .unwrap();

        assert!(!target.exists());
        assert_eq!(applied.input.target.path, target);
        assert_eq!(applied.evidence.strategy, LocalPlacement::Removed);

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn forget_removes_dangling_target_symlink() {
        let root = temp_root("forget-dangling-symlink");
        let target = root.join("target-link");
        fs::create_dir_all(&root).unwrap();
        symlink_file(root.join("missing-source"), &target).unwrap();
        assert!(!target.exists());
        assert!(
            fs::symlink_metadata(&target)
                .unwrap()
                .file_type()
                .is_symlink()
        );

        LocalApply
            .apply(Forget::new("demo", LocalTarget::new(&target)))
            .unwrap();

        assert_eq!(
            fs::symlink_metadata(&target).unwrap_err().kind(),
            std::io::ErrorKind::NotFound
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn forget_removes_existing_directory_target() {
        let root = temp_root("forget-directory");
        let target = root.join("target");
        fs::create_dir_all(target.join("nested")).unwrap();
        fs::write(target.join("nested/file.txt"), "obsolete").unwrap();

        LocalApply
            .apply(Forget::new("demo", LocalTarget::new(&target)))
            .unwrap();

        assert!(!target.exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn create_or_replace_rejects_same_file_source_target() {
        let root = temp_root("same-file");
        let source = root.join("source.txt");
        fs::create_dir_all(&root).unwrap();
        fs::write(&source, "same").unwrap();

        let acquired = acquire(MaterializeMode::CreateOrReplace, &source, &source);
        assert!(LocalApply.apply(acquired).is_err());
        assert_eq!(fs::read_to_string(&source).unwrap(), "same");

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn file_replace_stages_before_touching_target() {
        let root = temp_root("file-stage-fail");
        let source = root.join("missing-source.txt");
        let target = root.join("target.txt");
        fs::create_dir_all(&root).unwrap();
        fs::write(&target, "old").unwrap();

        let node = crate::Acquired {
            input: Materialize::new(
                "demo",
                LocalPath::new(&source),
                LocalTarget::new(&target),
                MaterializeMode::Replace,
            ),
            material: LocalMaterial::File { path: source },
            evidence: LocalAcquireEvidence {
                path: target.clone(),
            },
        };

        assert!(LocalApply.apply(node).is_err());
        assert_eq!(fs::read_to_string(&target).unwrap(), "old");

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn directory_create_stages_tree_and_records_counts() {
        let root = temp_root("dir-create");
        let source = root.join("source");
        let target = root.join("target");
        fs::create_dir_all(source.join("nested")).unwrap();
        fs::write(source.join("a.txt"), "alpha").unwrap();
        fs::write(source.join("nested").join("b.txt"), "beta").unwrap();

        let applied = LocalApply
            .apply(acquire(MaterializeMode::Create, &source, &target))
            .unwrap();

        assert_eq!(fs::read_to_string(target.join("a.txt")).unwrap(), "alpha");
        assert_eq!(
            fs::read_to_string(target.join("nested").join("b.txt")).unwrap(),
            "beta"
        );
        assert_eq!(applied.evidence.current.files, 2);
        assert_eq!(applied.evidence.current.directories, 2);
        assert_eq!(applied.evidence.current.bytes, 9);
        assert_eq!(applied.evidence.current.strategy, LocalPlacement::Copied);

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn directory_replace_preserves_old_target_when_preflight_fails() {
        let root = temp_root("dir-replace-fail");
        let source = root.join("source");
        let target = source.join("target");
        fs::create_dir_all(&source).unwrap();
        fs::create_dir_all(&target).unwrap();
        fs::write(target.join("old.txt"), "old").unwrap();

        let result = LocalApply.apply(acquire(MaterializeMode::Replace, &source, &target));
        assert!(result.is_err());
        assert_eq!(fs::read_to_string(target.join("old.txt")).unwrap(), "old");

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn directory_replace_over_file_commits_without_false_failure() {
        let root = temp_root("dir-over-file");
        let source = root.join("source");
        let target = root.join("target");
        fs::create_dir_all(&source).unwrap();
        fs::write(source.join("new.txt"), "new").unwrap();
        fs::write(&target, "old").unwrap();

        LocalApply
            .apply(acquire(MaterializeMode::Replace, &source, &target))
            .unwrap();

        assert_eq!(fs::read_to_string(target.join("new.txt")).unwrap(), "new");
        assert!(fs::read_dir(&root).unwrap().all(|entry| {
            !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .contains(".pulith-backup-")
        }));

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn directory_create_rejects_target_inside_source() {
        let root = temp_root("dir-cycle");
        let source = root.join("source");
        let target = source.join("nested").join("target");
        fs::create_dir_all(&source).unwrap();

        let result = LocalApply.apply(acquire(MaterializeMode::Create, &source, &target));
        assert!(result.is_err());
        assert!(!target.exists());

        fs::remove_dir_all(root).unwrap();
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

    #[cfg(unix)]
    #[test]
    fn directory_apply_rejects_symlink_by_default() {
        use std::os::unix::fs::symlink;

        let root = temp_root("dir-symlink");
        let source = root.join("source");
        let target = root.join("target");
        fs::create_dir_all(&source).unwrap();
        fs::write(source.join("real.txt"), "real").unwrap();
        symlink("real.txt", source.join("link.txt")).unwrap();

        let result = LocalApply.apply(acquire(MaterializeMode::Create, &source, &target));
        assert!(result.is_err());
        assert!(!target.exists());

        fs::remove_dir_all(root).unwrap();
    }

    fn acquire(
        mode: MaterializeMode,
        source: &std::path::Path,
        target: &std::path::Path,
    ) -> crate::Acquired<
        Materialize<&'static str, LocalPath, LocalTarget>,
        LocalMaterial,
        LocalAcquireEvidence,
    > {
        LocalAcquire
            .acquire(Materialize::new(
                "demo",
                LocalPath::new(source),
                LocalTarget::new(target),
                mode,
            ))
            .unwrap()
    }
}
