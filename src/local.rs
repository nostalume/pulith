use std::fs::{self, File};
use std::io;
use std::marker::PhantomData;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::{
    AcquireEvidence, AcquireNode, Acquired, Applied, ApplyEvidence, ApplyNode, Create,
    CreateOrReplace, EvidenceChain, Forget, Intent, Item, LocalApplyStats, LocalPath, LocalTarget,
    NoEvidence, PrepareEvidence, PrepareNode, Prepared, PulithError, Receipt, RememberEvidence,
    RememberNode, Remembered, Replace, SelectNode, Verified, WithSource,
};

pub(crate) type LocalApplied<O, E> =
    Applied<Intent<Item, LocalTarget, O>, Receipt<O>, EvidenceChain<E, ApplyEvidence>>;

#[derive(Clone, Copy, Debug, Default)]
pub struct SelectFirst;

impl<I, S> WithSource<I, S> {
    pub fn select_first(self) -> Result<crate::Chosen<I, S>, PulithError> {
        SelectFirst.select_node(self)
    }
}

impl<I, S> SelectNode<WithSource<I, S>> for SelectFirst {
    type Source = S;
    type Error = PulithError;
    type Output = crate::Chosen<I, S>;

    fn select_node(&self, node: WithSource<I, S>) -> Result<Self::Output, Self::Error> {
        Ok(crate::Chosen::from_selected(node.input, node.source))
    }
}

#[derive(Clone, Debug, Default)]
pub struct LocalAcquire;

impl<I> AcquireNode<crate::Chosen<I, LocalPath>> for LocalAcquire {
    type Material = LocalMaterial;
    type Evidence = AcquireEvidence;
    type Error = PulithError;
    type Output = Acquired<I, LocalMaterial, AcquireEvidence>;

    fn acquire_node(&self, node: crate::Chosen<I, LocalPath>) -> Result<Self::Output, Self::Error> {
        let path = node.source.path;
        if !path.exists() {
            return Err(PulithError::MissingSource(path));
        }
        let kind = if path.is_dir() {
            MaterialKind::Directory
        } else {
            MaterialKind::File
        };
        Ok(Acquired::from_acquire(
            node.input,
            LocalMaterial {
                path: path.clone(),
                kind,
            },
            AcquireEvidence { path },
        ))
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LocalMaterial {
    pub path: PathBuf,
    pub kind: MaterialKind,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LocalPrepared {
    pub path: PathBuf,
    pub kind: MaterialKind,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MaterialKind {
    File,
    Directory,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Identity;

#[derive(Clone, Copy, Debug, Default)]
pub struct IdentityVerify;

impl<I, E> crate::VerifyNode<Acquired<I, LocalMaterial, E>> for IdentityVerify {
    type Need = Identity;
    type Evidence = NoEvidence;
    type Error = PulithError;
    type Output = Verified<I, LocalMaterial, E>;

    fn verify_node(
        &self,
        node: Acquired<I, LocalMaterial, E>,
        _need: Self::Need,
    ) -> Result<Self::Output, Self::Error> {
        Ok(Verified::from_verify(
            node.input,
            node.material,
            node.evidence,
        ))
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct IdentityPrepare;

impl<I, E> PrepareNode<Verified<I, LocalMaterial, E>> for IdentityPrepare {
    type Need = Identity;
    type Prepared = LocalPrepared;
    type Evidence = PrepareEvidence;
    type Error = PulithError;
    type Output = Prepared<I, LocalPrepared, EvidenceChain<E, PrepareEvidence>>;

    fn prepare_node(
        &self,
        node: Verified<I, LocalMaterial, E>,
        _need: Self::Need,
    ) -> Result<Self::Output, Self::Error> {
        let prepared = LocalPrepared {
            path: node.material.path.clone(),
            kind: node.material.kind,
        };
        Ok(Prepared::from_prepare(
            node.input,
            prepared,
            EvidenceChain::new(
                node.evidence,
                PrepareEvidence {
                    path: node.material.path,
                },
            ),
        ))
    }
}

#[derive(Clone, Debug, Default)]
pub struct LocalApply<O> {
    _op: PhantomData<O>,
}

impl<O> LocalApply<O> {
    pub fn new() -> Self {
        Self { _op: PhantomData }
    }
}

impl<E> ApplyNode<Prepared<Intent<Item, LocalTarget, Create>, LocalPrepared, E>>
    for LocalApply<Create>
{
    type Receipt = Receipt<Create>;
    type Evidence = ApplyEvidence;
    type Error = PulithError;
    type Output = Applied<
        Intent<Item, LocalTarget, Create>,
        Receipt<Create>,
        EvidenceChain<E, ApplyEvidence>,
    >;

    fn apply_node(
        &self,
        node: Prepared<Intent<Item, LocalTarget, Create>, LocalPrepared, E>,
    ) -> Result<Self::Output, Self::Error> {
        if node.input.target.path.exists() {
            return Err(PulithError::ApplyWouldOverwrite(node.input.target.path));
        }
        apply_staged(node, Create, PublishMode::Create)
    }
}

impl<E> ApplyNode<Prepared<Intent<Item, LocalTarget, Replace>, LocalPrepared, E>>
    for LocalApply<Replace>
{
    type Receipt = Receipt<Replace>;
    type Evidence = ApplyEvidence;
    type Error = PulithError;
    type Output = Applied<
        Intent<Item, LocalTarget, Replace>,
        Receipt<Replace>,
        EvidenceChain<E, ApplyEvidence>,
    >;

    fn apply_node(
        &self,
        node: Prepared<Intent<Item, LocalTarget, Replace>, LocalPrepared, E>,
    ) -> Result<Self::Output, Self::Error> {
        if !node.input.target.path.exists() {
            return Err(PulithError::ApplyMissingTarget(node.input.target.path));
        }
        apply_staged(node, Replace, PublishMode::Replace)
    }
}

impl<E> ApplyNode<Prepared<Intent<Item, LocalTarget, CreateOrReplace>, LocalPrepared, E>>
    for LocalApply<CreateOrReplace>
{
    type Receipt = Receipt<CreateOrReplace>;
    type Evidence = ApplyEvidence;
    type Error = PulithError;
    type Output = Applied<
        Intent<Item, LocalTarget, CreateOrReplace>,
        Receipt<CreateOrReplace>,
        EvidenceChain<E, ApplyEvidence>,
    >;

    fn apply_node(
        &self,
        node: Prepared<Intent<Item, LocalTarget, CreateOrReplace>, LocalPrepared, E>,
    ) -> Result<Self::Output, Self::Error> {
        apply_staged(node, CreateOrReplace, PublishMode::CreateOrReplace)
    }
}

/// Removes the exact caller-authorized local target without acquiring a source.
///
/// Directory targets are removed recursively and symlink targets are removed as links. As with
/// other local behaviors, parent directories are trusted and hostile concurrent mutation is not
/// sandboxed.
impl ApplyNode<Intent<Item, LocalTarget, Forget>> for LocalApply<Forget> {
    type Receipt = Receipt<Forget>;
    type Evidence = ApplyEvidence;
    type Error = PulithError;
    type Output = Applied<Intent<Item, LocalTarget, Forget>, Receipt<Forget>, ApplyEvidence>;

    fn apply_node(
        &self,
        node: Intent<Item, LocalTarget, Forget>,
    ) -> Result<Self::Output, Self::Error> {
        match remove_existing(&node.target.path) {
            Ok(()) => {}
            Err(PulithError::Io { source, .. }) if source.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        let target = node.target.path.clone();
        let receipt = Receipt {
            item: node.item.name.clone(),
            target: target.clone(),
            op: Forget,
        };
        Ok(Applied::from_apply(
            node,
            receipt,
            ApplyEvidence::removed(target),
        ))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PublishMode {
    Create,
    Replace,
    CreateOrReplace,
}

fn apply_staged<O, E>(
    node: Prepared<Intent<Item, LocalTarget, O>, LocalPrepared, E>,
    op: O,
    mode: PublishMode,
) -> Result<LocalApplied<O, E>, PulithError> {
    let target = node.input.target.path.clone();
    reject_unsupported_entry(&node.prepared.path)?;
    reject_same_source_target(&node.prepared.path, &target)?;

    let stats = match node.prepared.kind {
        MaterialKind::File => publish_file(&node.prepared.path, &target, mode)?,
        MaterialKind::Directory => publish_directory(&node.prepared.path, &target, mode)?,
    };

    let receipt = Receipt {
        item: node.input.item.name.clone(),
        target: target.clone(),
        op,
    };
    Ok(Applied::from_apply(
        node.input,
        receipt,
        EvidenceChain::new(node.evidence, ApplyEvidence::new(target, stats)),
    ))
}

#[derive(Clone, Debug, Default)]
pub struct MemoryRemember;

impl<I, R, E> RememberNode<Applied<I, R, E>> for MemoryRemember
where
    R: RememberedItem,
{
    type Evidence = RememberEvidence;
    type Error = PulithError;
    type Output = Remembered<I, R, EvidenceChain<E, RememberEvidence>>;

    fn remember_node(&self, node: Applied<I, R, E>) -> Result<Self::Output, Self::Error> {
        let evidence = RememberEvidence {
            item: node.receipt.item_name().to_string(),
        };
        Ok(Remembered::from_remember(
            node.input,
            node.receipt,
            EvidenceChain::new(node.evidence, evidence),
        ))
    }
}

pub trait RememberedItem {
    fn item_name(&self) -> &str;
}

impl<O> RememberedItem for Receipt<O> {
    fn item_name(&self) -> &str {
        &self.item
    }
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

    use crate::{
        AcquireNode, ApplyNode, Create, CreateOrReplace, Forget, Identity, IdentityPrepare,
        IdentityVerify, Intent, Item, LocalAcquire, LocalApply, LocalPath, LocalPlacement,
        LocalTarget, MemoryRemember, PrepareNode, RememberNode, Replace, VerifyNode,
    };

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

        let chosen = Intent::new(Item::new("demo"), LocalTarget::new(&target))
            .with_source(LocalPath::new(&source))
            .select_first()
            .unwrap();
        let acquired = LocalAcquire.acquire_node(chosen).unwrap();
        let verified = IdentityVerify.verify_node(acquired, Identity).unwrap();
        let prepared = IdentityPrepare.prepare_node(verified, Identity).unwrap();
        let applied = LocalApply::<CreateOrReplace>::new()
            .apply_node(prepared)
            .unwrap();
        let remembered = MemoryRemember.remember_node(applied).unwrap();

        assert_eq!(fs::read_to_string(&target).unwrap(), "pulith");
        assert_eq!(remembered.receipt.item, "demo");
        assert_eq!(remembered.evidence.current.item, "demo");

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn create_and_replace_are_typed_apply_laws() {
        let root = temp_root("tree-apply");
        let source = root.join("source.txt");
        let target = root.join("target.txt");
        fs::create_dir_all(&root).unwrap();
        fs::write(&source, "first").unwrap();
        fs::write(&target, "existing").unwrap();

        let create_prepared = prepare::<Create>(&source, &target);
        assert!(
            LocalApply::<Create>::new()
                .apply_node(create_prepared)
                .is_err()
        );
        assert_eq!(fs::read_to_string(&target).unwrap(), "existing");

        let replace_prepared = prepare::<Replace>(&source, &target);
        let replaced = LocalApply::<Replace>::new()
            .apply_node(replace_prepared)
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

        let intent = Intent::new(Item::new("demo"), LocalTarget::new(&target)).op::<Forget>();
        let applied = LocalApply::<Forget>::new().apply_node(intent).unwrap();

        assert!(!target.exists());
        assert_eq!(applied.receipt.item, "demo");
        assert_eq!(applied.receipt.target, target);
        assert_eq!(applied.evidence.strategy, LocalPlacement::Removed);

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn forget_is_idempotent_when_target_is_absent() {
        let root = temp_root("forget-absent");
        let target = root.join("missing.txt");
        fs::create_dir_all(&root).unwrap();

        let applied = LocalApply::<Forget>::new()
            .apply_node(Intent::new(Item::new("demo"), LocalTarget::new(&target)).op::<Forget>())
            .unwrap();

        assert!(!target.exists());
        assert_eq!(applied.receipt.target, target);
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

        LocalApply::<Forget>::new()
            .apply_node(Intent::new(Item::new("demo"), LocalTarget::new(&target)).op::<Forget>())
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

        LocalApply::<Forget>::new()
            .apply_node(Intent::new(Item::new("demo"), LocalTarget::new(&target)).op::<Forget>())
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

        let prepared = prepare::<CreateOrReplace>(&source, &source);
        assert!(
            LocalApply::<CreateOrReplace>::new()
                .apply_node(prepared)
                .is_err()
        );
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

        let node = crate::Prepared {
            input: Intent::new(Item::new("demo"), LocalTarget::new(&target)).op::<Replace>(),
            prepared: crate::LocalPrepared {
                path: source,
                kind: crate::MaterialKind::File,
            },
            evidence: crate::NoEvidence,
        };

        assert!(LocalApply::<Replace>::new().apply_node(node).is_err());
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

        let applied = LocalApply::<Create>::new()
            .apply_node(prepare::<Create>(&source, &target))
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

        let result = LocalApply::<Replace>::new().apply_node(prepare::<Replace>(&source, &target));
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

        LocalApply::<Replace>::new()
            .apply_node(prepare::<Replace>(&source, &target))
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

        let result = LocalApply::<Create>::new().apply_node(prepare::<Create>(&source, &target));
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

        let result = LocalApply::<Create>::new().apply_node(prepare::<Create>(&source, &target));
        assert!(result.is_err());
        assert!(!target.exists());

        fs::remove_dir_all(root).unwrap();
    }

    fn prepare<O>(
        source: &std::path::Path,
        target: &std::path::Path,
    ) -> crate::Prepared<
        crate::Intent<crate::Item, crate::LocalTarget, O>,
        crate::LocalPrepared,
        crate::EvidenceChain<crate::AcquireEvidence, crate::PrepareEvidence>,
    > {
        let chosen = Intent::new(Item::new("demo"), LocalTarget::new(target))
            .op::<O>()
            .with_source(LocalPath::new(source))
            .select_first()
            .unwrap();
        let acquired = LocalAcquire.acquire_node(chosen).unwrap();
        let verified = IdentityVerify.verify_node(acquired, Identity).unwrap();
        IdentityPrepare.prepare_node(verified, Identity).unwrap()
    }
}
