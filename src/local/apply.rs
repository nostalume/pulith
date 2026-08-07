//! Local publication: `LocalApply`, `Forget`, and the apply evidence records.
//!
//! Owns the exact single-target effect law: staged publication with an explicit `MaterializeMode`
//! commit boundary, and direct idempotent removal for `Forget`. It never republishes, follows
//! links, claims ownership, or retries. Evidence (`ApplyEvidence`, `LocalPlacement`,
//! `LocalApplyStats`) is adapter-attested effect data, not an authorization. Feature-gated on
//! `local`.
use std::fs::{self, File};
use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::{
    Acquired, Applied, Apply, EvidenceChain, Forget, Materialize, MaterializeMode, Verified,
};

use super::{LocalError, LocalMaterial, StagedTree};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ApplyEvidence {
    pub files: usize,
    pub directories: usize,
    pub bytes: u64,
    pub strategy: LocalPlacement,
}

impl ApplyEvidence {
    pub(crate) fn new(stats: LocalApplyStats) -> Self {
        Self {
            files: stats.files,
            directories: stats.directories,
            bytes: stats.bytes,
            strategy: stats.strategy,
        }
    }

    pub(crate) fn removed() -> Self {
        Self {
            files: 0,
            directories: 0,
            bytes: 0,
            strategy: LocalPlacement::Removed,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LocalPlacement {
    Copied,
    Removed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct LocalApplyStats {
    pub files: usize,
    pub directories: usize,
    pub bytes: u64,
    pub strategy: LocalPlacement,
}

impl LocalApplyStats {
    pub(crate) fn copied_file(bytes: u64) -> Self {
        Self {
            files: 1,
            directories: 0,
            bytes,
            strategy: LocalPlacement::Copied,
        }
    }

    pub(crate) fn copied_tree(files: usize, directories: usize, bytes: u64) -> Self {
        Self {
            files,
            directories,
            bytes,
            strategy: LocalPlacement::Copied,
        }
    }
}
/// Local publication adapter.
///
/// For regular-file [`MaterializeMode::CreateNew`], the final no-clobber persist is the authoritative
/// execution-time `Missing` predecessor check. A late winner produces
/// [`LocalError::ApplyWouldOverwrite`] without changing that target. Directory publication,
/// replacement modes, and [`Forget`] do not inherit this conditional-file claim.
#[derive(Clone, Copy, Debug, Default)]
pub struct LocalApply;

type LocalApplied<I, S, E> = Applied<Materialize<I, S, PathBuf>, EvidenceChain<E, ApplyEvidence>>;

impl LocalApply {
    /// Inherent mirror of [`Apply::apply`] — callable without importing the trait.
    ///
    /// The generic bound resolves the concrete input (acquired, verified, prepared, forget);
    /// inherent methods cannot be overloaded by signature, so the single generic mirror covers
    /// every input the family implements.
    pub fn apply<N>(&self, node: N) -> Result<<Self as Apply<N>>::Output, <Self as Apply<N>>::Error>
    where
        Self: Apply<N>,
    {
        Apply::apply(self, node)
    }
}

impl<I, S, E> Apply<crate::local::LocalAcquired<I, S, E>> for LocalApply {
    type Error = LocalError;
    type Output = Applied<Materialize<I, S, PathBuf>, EvidenceChain<E, ApplyEvidence>>;

    fn apply(
        &self,
        node: crate::local::LocalAcquired<I, S, E>,
    ) -> Result<Self::Output, Self::Error> {
        apply_material(node.input, node.material, node.evidence)
    }
}

impl<I, S, E> Apply<Verified<Materialize<I, S, PathBuf>, LocalMaterial, E>> for LocalApply {
    type Error = LocalError;
    type Output = Applied<Materialize<I, S, PathBuf>, EvidenceChain<E, ApplyEvidence>>;

    fn apply(
        &self,
        node: Verified<Materialize<I, S, PathBuf>, LocalMaterial, E>,
    ) -> Result<Self::Output, Self::Error> {
        apply_material(node.input, node.material, node.evidence)
    }
}

impl<I, S, E> Apply<Acquired<Materialize<I, S, PathBuf>, StagedTree, E>> for LocalApply {
    type Error = LocalError;
    type Output = Applied<Materialize<I, S, PathBuf>, EvidenceChain<E, ApplyEvidence>>;

    fn apply(
        &self,
        node: Acquired<Materialize<I, S, PathBuf>, StagedTree, E>,
    ) -> Result<Self::Output, Self::Error> {
        let input = node.input;
        let target = input.target.clone();
        let mode = match input.mode {
            MaterializeMode::CreateNew => {
                if target_entry_exists(&target)? {
                    return Err(LocalError::ApplyWouldOverwrite(target));
                }
                PublishMode::Create
            }
            MaterializeMode::ReplaceOrCreate => PublishMode::CreateOrReplace,
        };
        let source = node.material.root();
        reject_unsupported_entry(source)?;
        reject_same_source_target(source, &target)?;
        let stats = publish_directory(source, &target, mode)?;
        Ok(Applied {
            input,
            evidence: EvidenceChain {
                previous: node.evidence,
                current: ApplyEvidence::new(stats),
            },
        })
    }
}
/// Removes the exact caller-authorized local target without acquiring a source.
impl<I> Apply<Forget<I, PathBuf>> for LocalApply {
    type Error = LocalError;
    type Output = Applied<Forget<I, PathBuf>, ApplyEvidence>;

    fn apply(&self, node: Forget<I, PathBuf>) -> Result<Self::Output, Self::Error> {
        match remove_existing(&node.target) {
            Ok(()) => {}
            Err(LocalError::Io { source, .. }) if source.kind() == io::ErrorKind::NotFound => {}
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
    CreateOrReplace,
}

pub(crate) fn apply_material<I, S, E>(
    input: Materialize<I, S, PathBuf>,
    material: LocalMaterial,
    evidence: E,
) -> Result<LocalApplied<I, S, E>, LocalError> {
    let target = input.target.clone();
    let mode = match input.mode {
        MaterializeMode::CreateNew => {
            if target_entry_exists(&target)? {
                return Err(LocalError::ApplyWouldOverwrite(target));
            }
            PublishMode::Create
        }
        MaterializeMode::ReplaceOrCreate => PublishMode::CreateOrReplace,
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

fn target_entry_exists(path: &Path) -> Result<bool, LocalError> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(LocalError::io("read target metadata", path, error)),
    }
}

fn publish_file(
    source: &Path,
    target: &Path,
    mode: PublishMode,
) -> Result<LocalApplyStats, LocalError> {
    let parent = target_parent(target)?;
    fs::create_dir_all(parent)
        .map_err(|err| LocalError::io("create parent directory", parent, err))?;

    let mut source_file =
        File::open(source).map_err(|err| LocalError::io("open source file", source, err))?;
    let mut staged = tempfile::NamedTempFile::new_in(parent)
        .map_err(|err| LocalError::io("create staged file", parent, err))?;
    let bytes = io::copy(&mut source_file, staged.as_file_mut())
        .map_err(|err| LocalError::io("copy file to staged file", source, err))?;

    persist_staged_file(staged, target, mode)?;

    Ok(LocalApplyStats::copied_file(bytes))
}

fn persist_staged_file(
    staged: tempfile::NamedTempFile,
    target: &Path,
    mode: PublishMode,
) -> Result<(), LocalError> {
    match mode {
        PublishMode::Create => match staged.persist_noclobber(target) {
            Ok(_) => Ok(()),
            Err(error) if error.error.kind() == io::ErrorKind::AlreadyExists => {
                Err(LocalError::ApplyWouldOverwrite(target.to_path_buf()))
            }
            Err(error) => Err(LocalError::io("persist staged file", target, error.error)),
        },
        PublishMode::CreateOrReplace => staged
            .persist(target)
            .map(|_| ())
            .map_err(|error| LocalError::io("persist staged file", target, error.error)),
    }
}

fn publish_directory(
    source: &Path,
    target: &Path,
    mode: PublishMode,
) -> Result<LocalApplyStats, LocalError> {
    reject_directory_conflict(source, target)?;

    let parent = target_parent(target)?;
    fs::create_dir_all(parent)
        .map_err(|err| LocalError::io("create parent directory", parent, err))?;

    let staging = tempfile::Builder::new()
        .prefix(".pulith-stage-")
        .tempdir_in(parent)
        .map_err(|err| LocalError::io("create staged directory", parent, err))?;
    let stats = copy_directory_to_stage(source, staging.path())?;
    let staged_path = staging.keep();

    let result = match mode {
        PublishMode::Create => rename_dir(&staged_path, target),
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

fn copy_directory_to_stage(source: &Path, stage: &Path) -> Result<LocalApplyStats, LocalError> {
    let mut files = 0usize;
    let mut directories = 0usize;
    let mut bytes = 0u64;

    for entry in walkdir::WalkDir::new(source).follow_links(false) {
        let entry = entry.map_err(|err| {
            LocalError::io(
                "walk source directory",
                err.path().unwrap_or(source),
                io::Error::other(err.to_string()),
            )
        })?;
        let file_type = entry.file_type();
        if file_type.is_symlink() {
            return Err(LocalError::UnsupportedLocalEntry(
                entry.path().to_path_buf(),
            ));
        }

        let relative = entry.path().strip_prefix(source).map_err(|err| {
            LocalError::io(
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
                .map_err(|err| LocalError::io("create staged directory", &destination, err))?;
            directories += 1;
        } else if file_type.is_file() {
            if let Some(parent) = destination.parent() {
                fs::create_dir_all(parent)
                    .map_err(|err| LocalError::io("create staged file parent", parent, err))?;
            }
            let copied = fs::copy(entry.path(), &destination).map_err(|err| {
                LocalError::io("copy file to staged directory", &destination, err)
            })?;
            files += 1;
            bytes += copied;
        } else {
            return Err(LocalError::UnsupportedLocalEntry(
                entry.path().to_path_buf(),
            ));
        }
    }

    Ok(LocalApplyStats::copied_tree(files, directories, bytes))
}

fn replace_directory_with_backup(staged_path: &Path, target: &Path) -> Result<(), LocalError> {
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

fn rename_dir(source: &Path, target: &Path) -> Result<(), LocalError> {
    fs::rename(source, target).map_err(|err| LocalError::io("rename directory", target, err))
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

fn reject_unsupported_entry(path: &Path) -> Result<(), LocalError> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|err| LocalError::io("read source metadata", path, err))?;
    let file_type = metadata.file_type();
    if file_type.is_symlink() || !(file_type.is_file() || file_type.is_dir()) {
        return Err(LocalError::UnsupportedLocalEntry(path.to_path_buf()));
    }
    Ok(())
}

fn reject_same_source_target(source: &Path, target: &Path) -> Result<(), LocalError> {
    if target.exists() {
        let is_same = same_file::is_same_file(source, target)
            .map_err(|err| LocalError::io("compare source and target", target, err))?;
        if is_same {
            return Err(LocalError::ApplySameFile(target.to_path_buf()));
        }
    }
    Ok(())
}

fn reject_directory_conflict(source: &Path, target: &Path) -> Result<(), LocalError> {
    let source = source
        .canonicalize()
        .map_err(|err| LocalError::io("canonicalize source directory", source, err))?;
    let target_candidate = canonical_target_candidate(target)?;

    if target_candidate.starts_with(&source) || source.starts_with(&target_candidate) {
        return Err(LocalError::ApplyPathConflict {
            source,
            target: target_candidate,
        });
    }
    Ok(())
}

fn canonical_target_candidate(target: &Path) -> Result<PathBuf, LocalError> {
    if target.exists() {
        return target
            .canonicalize()
            .map_err(|err| LocalError::io("canonicalize target", target, err));
    }

    let parent = target_parent(target)?;
    let parent = parent
        .canonicalize()
        .map_err(|err| LocalError::io("canonicalize target parent", parent, err))?;
    Ok(match target.file_name() {
        Some(name) => parent.join(name),
        None => parent,
    })
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
    use crate::local::{LocalAcquire, LocalAcquireEvidence, LocalError, LocalMaterial, PathBuf};
    use crate::{Forget, Materialize, MaterializeMode};

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
            .apply(acquire(MaterializeMode::ReplaceOrCreate, &source, &target))
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
                staged_path.clone(),
                target.clone(),
                MaterializeMode::CreateNew,
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
                staged_path.clone(),
                target.clone(),
                MaterializeMode::CreateNew,
            ),
            material: LocalMaterial::StagedFile {
                path: staged.into_temp_path(),
            },
            evidence: (),
        };

        let error = LocalApply.apply(node).unwrap_err();

        assert!(matches!(
            error,
            LocalError::ApplyWouldOverwrite(path) if path == target
        ));
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
                staged_path.clone(),
                target.clone(),
                MaterializeMode::CreateNew,
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
                .apply(acquire(MaterializeMode::CreateNew, &source, &target))
                .is_err()
        );
        assert_eq!(fs::read_to_string(&target).unwrap(), "existing");

        let replaced = LocalApply
            .apply(acquire(MaterializeMode::ReplaceOrCreate, &source, &target))
            .unwrap();
        assert_eq!(fs::read_to_string(&target).unwrap(), "first");
        assert_eq!(replaced.evidence.current.files, 1);
        assert_eq!(replaced.evidence.current.directories, 0);
        assert_eq!(replaced.evidence.current.bytes, 5);
        assert_eq!(replaced.evidence.current.strategy, LocalPlacement::Copied);

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn create_file_commit_reports_late_target_as_conflict_without_overwrite() {
        use std::io::Write;

        let root = temp_root("create-late-conflict");
        let target = root.join("target.txt");
        fs::create_dir_all(&root).unwrap();
        let mut staged = tempfile::NamedTempFile::new_in(&root).unwrap();
        let staged_path = staged.path().to_path_buf();
        staged.write_all(b"replacement").unwrap();

        // This target appeared after the replacement was completely staged.
        fs::write(&target, b"winner").unwrap();

        let error = persist_staged_file(staged, &target, PublishMode::Create).unwrap_err();
        assert!(matches!(
            error,
            LocalError::ApplyWouldOverwrite(path) if path == target
        ));
        assert_eq!(fs::read(&target).unwrap(), b"winner");
        assert!(!staged_path.exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn create_treats_dangling_final_symlink_as_conflict_without_following() {
        let root = temp_root("create-dangling-conflict");
        let source = root.join("source.txt");
        let target = root.join("target-link");
        fs::create_dir_all(&root).unwrap();
        fs::write(&source, b"replacement").unwrap();
        symlink_file(root.join("missing-target"), &target).unwrap();

        let error = LocalApply
            .apply(acquire(MaterializeMode::CreateNew, &source, &target))
            .unwrap_err();

        assert!(matches!(
            error,
            LocalError::ApplyWouldOverwrite(path) if path == target
        ));
        assert!(
            fs::symlink_metadata(&target)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn create_treats_existing_directory_as_typed_conflict() {
        let root = temp_root("create-directory-conflict");
        let source = root.join("source.txt");
        let target = root.join("target");
        fs::create_dir_all(&target).unwrap();
        fs::write(&source, b"replacement").unwrap();

        let error = LocalApply
            .apply(acquire(MaterializeMode::CreateNew, &source, &target))
            .unwrap_err();

        assert!(matches!(
            error,
            LocalError::ApplyWouldOverwrite(path) if path == target
        ));
        assert!(target.is_dir());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn forget_applies_directly_without_acquiring_a_source() {
        let root = temp_root("forget-direct");
        let target = root.join("target.txt");
        fs::create_dir_all(&root).unwrap();
        fs::write(&target, "obsolete").unwrap();

        let applied = LocalApply
            .apply(Forget::new("demo", target.clone()))
            .unwrap();

        assert!(!target.exists());
        assert_eq!(applied.input.item, "demo");
        assert_eq!(applied.input.target, target);
        assert_eq!(applied.evidence.strategy, LocalPlacement::Removed);

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn forget_is_idempotent_when_target_is_absent() {
        let root = temp_root("forget-absent");
        let target = root.join("missing.txt");
        fs::create_dir_all(&root).unwrap();

        let applied = LocalApply
            .apply(Forget::new("demo", target.clone()))
            .unwrap();

        assert!(!target.exists());
        assert_eq!(applied.input.target, target);
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
            .apply(Forget::new("demo", target.clone()))
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
            .apply(Forget::new("demo", target.clone()))
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

        let acquired = acquire(MaterializeMode::ReplaceOrCreate, &source, &source);
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
                source.clone(),
                target.clone(),
                MaterializeMode::ReplaceOrCreate,
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
            .apply(acquire(MaterializeMode::CreateNew, &source, &target))
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

        let result = LocalApply.apply(acquire(MaterializeMode::ReplaceOrCreate, &source, &target));
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
            .apply(acquire(MaterializeMode::ReplaceOrCreate, &source, &target))
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

        let result = LocalApply.apply(acquire(MaterializeMode::CreateNew, &source, &target));
        assert!(result.is_err());
        assert!(!target.exists());

        fs::remove_dir_all(root).unwrap();
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

        let result = LocalApply.apply(acquire(MaterializeMode::CreateNew, &source, &target));
        assert!(result.is_err());
        assert!(!target.exists());

        fs::remove_dir_all(root).unwrap();
    }

    fn acquire(
        mode: MaterializeMode,
        source: &std::path::Path,
        target: &std::path::Path,
    ) -> crate::Acquired<
        Materialize<&'static str, PathBuf, PathBuf>,
        LocalMaterial,
        LocalAcquireEvidence,
    > {
        LocalAcquire
            .acquire(Materialize::new(
                "demo",
                source.to_path_buf(),
                target.to_path_buf(),
                mode,
            ))
            .unwrap()
    }
}
