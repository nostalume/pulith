use std::fs::{self, File};
use std::io::{self, Read};
use std::marker::PhantomData;
use std::path::{Component, Path, PathBuf};

use crate::{
    EvidenceChain, LocalMaterial, MaterialKind, PrepareNode, Prepared, PulithError, Verified,
};

type ArchivePrepared<I, E, A> = Prepared<I, ArchiveTree<A>, EvidenceChain<E, ArchiveEvidence<A>>>;

#[cfg(feature = "zip")]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Zip;

#[cfg(feature = "tar")]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Plain;

#[cfg(feature = "gzip")]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Gzip;

#[cfg(feature = "xz")]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Xz;

#[cfg(feature = "zstd")]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Zstd;

#[cfg(feature = "tar")]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Tar<C = Plain> {
    _codec: PhantomData<C>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ArchivePolicy {
    pub strip_components: usize,
    pub max_entries: Option<u64>,
    pub max_total_bytes: Option<u64>,
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

    pub fn max_total_bytes(mut self, max_total_bytes: u64) -> Self {
        self.max_total_bytes = Some(max_total_bytes);
        self
    }
}

impl Default for ArchivePolicy {
    fn default() -> Self {
        Self {
            strip_components: 0,
            max_entries: Some(16_384),
            max_total_bytes: Some(4 * 1024 * 1024 * 1024),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ArchiveNeed<A> {
    pub policy: ArchivePolicy,
    _archive: PhantomData<A>,
}

impl<A> ArchiveNeed<A> {
    pub fn new(policy: ArchivePolicy) -> Self {
        Self {
            policy,
            _archive: PhantomData,
        }
    }
}

impl<A> Default for ArchiveNeed<A> {
    fn default() -> Self {
        Self::new(ArchivePolicy::default())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ArchiveTree<A> {
    pub root: PathBuf,
    _archive: PhantomData<A>,
}

impl<A> ArchiveTree<A> {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            _archive: PhantomData,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ArchiveEvidence<A> {
    pub root: PathBuf,
    pub entries: u64,
    pub total_bytes: u64,
    pub files: u64,
    pub directories: u64,
    pub symlinks: u64,
    _archive: PhantomData<A>,
}

impl<A> ArchiveEvidence<A> {
    fn empty(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            entries: 0,
            total_bytes: 0,
            files: 0,
            directories: 0,
            symlinks: 0,
            _archive: PhantomData,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
/// An extraction root exclusively owned by the caller for Pulith preparation.
///
/// Preparing an archive clears this path recursively before extraction. The path must not point
/// at shared or independently managed content.
pub struct ExistingExtractRoot {
    pub root: PathBuf,
}

impl ExistingExtractRoot {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ArchivePrepare<A, R = ExistingExtractRoot> {
    pub resources: R,
    _archive: PhantomData<A>,
}

impl<A, R> ArchivePrepare<A, R> {
    pub fn new(resources: R) -> Self {
        Self {
            resources,
            _archive: PhantomData,
        }
    }
}

#[cfg(feature = "zip")]
impl<I, E> PrepareNode<Verified<I, LocalMaterial, E>> for ArchivePrepare<Zip, ExistingExtractRoot> {
    type Need = ArchiveNeed<Zip>;
    type Prepared = ArchiveTree<Zip>;
    type Evidence = ArchiveEvidence<Zip>;
    type Error = PulithError;
    type Output = Prepared<I, ArchiveTree<Zip>, EvidenceChain<E, ArchiveEvidence<Zip>>>;

    fn prepare_node(
        &self,
        node: Verified<I, LocalMaterial, E>,
        need: Self::Need,
    ) -> Result<Self::Output, Self::Error> {
        if node.material.kind != MaterialKind::File {
            return Err(PulithError::ArchiveRequiresFile(node.material.path));
        }

        let root = self.resources.root.clone();
        reset_extract_root(&root)?;
        let evidence = extract_zip(&node.material.path, &root, &need.policy)?;

        Ok(Prepared::from_prepare(
            node.input,
            ArchiveTree::new(root),
            EvidenceChain::new(node.evidence, evidence),
        ))
    }
}

#[cfg(feature = "tar")]
impl<I, E> PrepareNode<Verified<I, LocalMaterial, E>>
    for ArchivePrepare<Tar<Plain>, ExistingExtractRoot>
{
    type Need = ArchiveNeed<Tar<Plain>>;
    type Prepared = ArchiveTree<Tar<Plain>>;
    type Evidence = ArchiveEvidence<Tar<Plain>>;
    type Error = PulithError;
    type Output =
        Prepared<I, ArchiveTree<Tar<Plain>>, EvidenceChain<E, ArchiveEvidence<Tar<Plain>>>>;

    fn prepare_node(
        &self,
        node: Verified<I, LocalMaterial, E>,
        need: Self::Need,
    ) -> Result<Self::Output, Self::Error> {
        prepare_archive(node, &self.resources.root, need.policy, extract_tar_plain)
    }
}

#[cfg(feature = "gzip")]
impl<I, E> PrepareNode<Verified<I, LocalMaterial, E>>
    for ArchivePrepare<Tar<Gzip>, ExistingExtractRoot>
{
    type Need = ArchiveNeed<Tar<Gzip>>;
    type Prepared = ArchiveTree<Tar<Gzip>>;
    type Evidence = ArchiveEvidence<Tar<Gzip>>;
    type Error = PulithError;
    type Output = Prepared<I, ArchiveTree<Tar<Gzip>>, EvidenceChain<E, ArchiveEvidence<Tar<Gzip>>>>;

    fn prepare_node(
        &self,
        node: Verified<I, LocalMaterial, E>,
        need: Self::Need,
    ) -> Result<Self::Output, Self::Error> {
        prepare_archive(node, &self.resources.root, need.policy, extract_tar_gzip)
    }
}

#[cfg(feature = "xz")]
impl<I, E> PrepareNode<Verified<I, LocalMaterial, E>>
    for ArchivePrepare<Tar<Xz>, ExistingExtractRoot>
{
    type Need = ArchiveNeed<Tar<Xz>>;
    type Prepared = ArchiveTree<Tar<Xz>>;
    type Evidence = ArchiveEvidence<Tar<Xz>>;
    type Error = PulithError;
    type Output = Prepared<I, ArchiveTree<Tar<Xz>>, EvidenceChain<E, ArchiveEvidence<Tar<Xz>>>>;

    fn prepare_node(
        &self,
        node: Verified<I, LocalMaterial, E>,
        need: Self::Need,
    ) -> Result<Self::Output, Self::Error> {
        prepare_archive(node, &self.resources.root, need.policy, extract_tar_xz)
    }
}

#[cfg(feature = "zstd")]
impl<I, E> PrepareNode<Verified<I, LocalMaterial, E>>
    for ArchivePrepare<Tar<Zstd>, ExistingExtractRoot>
{
    type Need = ArchiveNeed<Tar<Zstd>>;
    type Prepared = ArchiveTree<Tar<Zstd>>;
    type Evidence = ArchiveEvidence<Tar<Zstd>>;
    type Error = PulithError;
    type Output = Prepared<I, ArchiveTree<Tar<Zstd>>, EvidenceChain<E, ArchiveEvidence<Tar<Zstd>>>>;

    fn prepare_node(
        &self,
        node: Verified<I, LocalMaterial, E>,
        need: Self::Need,
    ) -> Result<Self::Output, Self::Error> {
        prepare_archive(node, &self.resources.root, need.policy, extract_tar_zstd)
    }
}

fn prepare_archive<I, E, A>(
    node: Verified<I, LocalMaterial, E>,
    root: &Path,
    policy: ArchivePolicy,
    extract: fn(&Path, &Path, &ArchivePolicy) -> Result<ArchiveEvidence<A>, PulithError>,
) -> Result<ArchivePrepared<I, E, A>, PulithError> {
    if node.material.kind != MaterialKind::File {
        return Err(PulithError::ArchiveRequiresFile(node.material.path));
    }

    let root = root.to_path_buf();
    reset_extract_root(&root)?;
    let evidence = extract(&node.material.path, &root, &policy)?;

    Ok(Prepared::from_prepare(
        node.input,
        ArchiveTree::new(root),
        EvidenceChain::new(node.evidence, evidence),
    ))
}

#[cfg(feature = "zip")]
fn extract_zip(
    archive_path: &Path,
    root: &Path,
    policy: &ArchivePolicy,
) -> Result<ArchiveEvidence<Zip>, PulithError> {
    let file = File::open(archive_path)
        .map_err(|err| PulithError::io("open zip archive", archive_path, err))?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|err| PulithError::InvalidPreparation(format!("invalid zip archive: {err}")))?;
    let mut evidence = ArchiveEvidence::empty(root);

    for index in 0..archive.len() {
        evidence.entries += 1;
        check_limit("entry-count", evidence.entries, policy.max_entries)?;

        let mut file = archive
            .by_index(index)
            .map_err(|err| PulithError::InvalidPreparation(format!("invalid zip entry: {err}")))?;
        let enclosed = file
            .enclosed_name()
            .ok_or_else(|| PulithError::ArchiveInvalidPath(file.name().to_string()))?;
        let Some(relative) = sanitize_relative(&enclosed, policy.strip_components)? else {
            continue;
        };
        let target = root.join(&relative);
        ensure_under_root(root, &target)?;
        reject_existing_symlink_path(root, &target)?;

        if is_zip_symlink(file.unix_mode()) {
            evidence.symlinks += 1;
            return Err(PulithError::UnsupportedArchiveEntry(relative));
        }

        if file.is_dir() {
            evidence.directories += 1;
            fs::create_dir_all(&target)
                .map_err(|err| PulithError::io("create archive directory", &target, err))?;
            continue;
        }

        let size = file.size();
        evidence.files += 1;
        evidence.total_bytes =
            evidence
                .total_bytes
                .checked_add(size)
                .ok_or(PulithError::ArchiveLimitExceeded {
                    limit: "total-bytes",
                    actual: u64::MAX,
                    max: policy.max_total_bytes.unwrap_or(u64::MAX),
                })?;
        check_limit("total-bytes", evidence.total_bytes, policy.max_total_bytes)?;

        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)
                .map_err(|err| PulithError::io("create archive file parent", parent, err))?;
        }
        let mut output = File::create(&target)
            .map_err(|err| PulithError::io("create archive file", &target, err))?;
        io::copy(&mut file, &mut output)
            .map_err(|err| PulithError::io("extract archive file", &target, err))?;
    }

    Ok(evidence)
}

#[cfg(feature = "tar")]
fn extract_tar_plain(
    archive_path: &Path,
    root: &Path,
    policy: &ArchivePolicy,
) -> Result<ArchiveEvidence<Tar<Plain>>, PulithError> {
    let file = File::open(archive_path)
        .map_err(|err| PulithError::io("open tar archive", archive_path, err))?;
    extract_tar_reader(file, root, policy)
}

#[cfg(feature = "gzip")]
fn extract_tar_gzip(
    archive_path: &Path,
    root: &Path,
    policy: &ArchivePolicy,
) -> Result<ArchiveEvidence<Tar<Gzip>>, PulithError> {
    let file = File::open(archive_path)
        .map_err(|err| PulithError::io("open gzip tar archive", archive_path, err))?;
    extract_tar_reader(flate2::read::GzDecoder::new(file), root, policy)
}

#[cfg(feature = "xz")]
fn extract_tar_xz(
    archive_path: &Path,
    root: &Path,
    policy: &ArchivePolicy,
) -> Result<ArchiveEvidence<Tar<Xz>>, PulithError> {
    let file = File::open(archive_path)
        .map_err(|err| PulithError::io("open xz tar archive", archive_path, err))?;
    extract_tar_reader(xz2::read::XzDecoder::new(file), root, policy)
}

#[cfg(feature = "zstd")]
fn extract_tar_zstd(
    archive_path: &Path,
    root: &Path,
    policy: &ArchivePolicy,
) -> Result<ArchiveEvidence<Tar<Zstd>>, PulithError> {
    let file = File::open(archive_path)
        .map_err(|err| PulithError::io("open zstd tar archive", archive_path, err))?;
    let decoder = zstd::stream::Decoder::new(file)
        .map_err(|err| PulithError::io("open zstd tar decoder", archive_path, err))?;
    extract_tar_reader(decoder, root, policy)
}

#[cfg(feature = "tar")]
fn extract_tar_reader<A, R: Read>(
    reader: R,
    root: &Path,
    policy: &ArchivePolicy,
) -> Result<ArchiveEvidence<A>, PulithError> {
    let mut archive = tar::Archive::new(reader);
    let mut evidence = ArchiveEvidence::empty(root);
    let entries = archive
        .entries()
        .map_err(|err| PulithError::io("read tar archive", root, err))?;

    for entry in entries {
        evidence.entries += 1;
        check_limit("entry-count", evidence.entries, policy.max_entries)?;

        let mut entry = entry.map_err(|err| PulithError::io("read tar entry", root, err))?;
        let raw_path = entry
            .path()
            .map_err(|_| PulithError::ArchiveInvalidPath("invalid tar entry path".into()))?
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
            return Err(PulithError::UnsupportedArchiveEntry(relative));
        }

        if entry_type.is_dir() {
            evidence.directories += 1;
            fs::create_dir_all(&target)
                .map_err(|err| PulithError::io("create archive directory", &target, err))?;
            continue;
        }

        if !entry_type.is_file() {
            return Err(PulithError::UnsupportedArchiveEntry(relative));
        }

        let size = entry.header().size().unwrap_or(0);
        evidence.files += 1;
        evidence.total_bytes =
            evidence
                .total_bytes
                .checked_add(size)
                .ok_or(PulithError::ArchiveLimitExceeded {
                    limit: "total-bytes",
                    actual: u64::MAX,
                    max: policy.max_total_bytes.unwrap_or(u64::MAX),
                })?;
        check_limit("total-bytes", evidence.total_bytes, policy.max_total_bytes)?;

        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)
                .map_err(|err| PulithError::io("create archive file parent", parent, err))?;
        }
        let mut output = File::create(&target)
            .map_err(|err| PulithError::io("create archive file", &target, err))?;
        io::copy(&mut entry, &mut output)
            .map_err(|err| PulithError::io("extract archive file", &target, err))?;
    }

    Ok(evidence)
}

fn check_limit(limit: &'static str, actual: u64, max: Option<u64>) -> Result<(), PulithError> {
    if let Some(max) = max
        && actual > max
    {
        return Err(PulithError::ArchiveLimitExceeded { limit, actual, max });
    }
    Ok(())
}

fn reset_extract_root(root: &Path) -> Result<(), PulithError> {
    if let Ok(metadata) = fs::symlink_metadata(root) {
        let file_type = metadata.file_type();
        if file_type.is_symlink() {
            return Err(PulithError::UnsupportedArchiveEntry(root.to_path_buf()));
        }
        if file_type.is_dir() {
            fs::remove_dir_all(root)
                .map_err(|err| PulithError::io("clear archive root", root, err))?;
        } else {
            fs::remove_file(root)
                .map_err(|err| PulithError::io("clear archive root file", root, err))?;
        }
    }
    fs::create_dir_all(root).map_err(|err| PulithError::io("create archive root", root, err))
}

fn sanitize_relative(path: &Path, strip_components: usize) -> Result<Option<PathBuf>, PulithError> {
    let mut relative = PathBuf::new();
    let mut seen = 0usize;

    for component in path.components() {
        match component {
            Component::Normal(part) => {
                if seen >= strip_components {
                    relative.push(part);
                }
                seen += 1;
            }
            Component::CurDir => {}
            other => return Err(PulithError::ArchiveInvalidPath(format!("{other:?}"))),
        }
    }

    if relative.as_os_str().is_empty() {
        Ok(None)
    } else {
        Ok(Some(relative))
    }
}

fn ensure_under_root(root: &Path, target: &Path) -> Result<(), PulithError> {
    if !target.starts_with(root) {
        return Err(PulithError::ArchiveInvalidPath(
            target.display().to_string(),
        ));
    }
    Ok(())
}

fn reject_existing_symlink_path(root: &Path, target: &Path) -> Result<(), PulithError> {
    let relative = target
        .strip_prefix(root)
        .map_err(|_| PulithError::ArchiveInvalidPath(target.display().to_string()))?;
    let mut cursor = root.to_path_buf();
    for component in relative.components() {
        if let Component::Normal(part) = component {
            cursor.push(part);
            if let Ok(metadata) = fs::symlink_metadata(&cursor)
                && metadata.file_type().is_symlink()
            {
                return Err(PulithError::UnsupportedArchiveEntry(cursor));
            }
        }
    }
    Ok(())
}

#[cfg(feature = "zip")]
fn is_zip_symlink(mode: Option<u32>) -> bool {
    mode.is_some_and(|mode| mode & 0o170000 == 0o120000)
}

#[cfg(all(any(feature = "zip", feature = "tar"), feature = "local"))]
mod local_apply {
    use crate::local::LocalApplied;
    use crate::{
        Applied, ApplyEvidence, ApplyNode, ArchiveTree, Create, CreateOrReplace, EvidenceChain,
        Intent, Item, LocalApply, LocalPrepared, LocalTarget, MaterialKind, Prepared, PulithError,
        Receipt, Replace,
    };

    impl<A, E> ApplyNode<Prepared<Intent<Item, LocalTarget, Create>, ArchiveTree<A>, E>>
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
            node: Prepared<Intent<Item, LocalTarget, Create>, ArchiveTree<A>, E>,
        ) -> Result<Self::Output, Self::Error> {
            apply_archive_tree(node)
        }
    }

    impl<A, E> ApplyNode<Prepared<Intent<Item, LocalTarget, Replace>, ArchiveTree<A>, E>>
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
            node: Prepared<Intent<Item, LocalTarget, Replace>, ArchiveTree<A>, E>,
        ) -> Result<Self::Output, Self::Error> {
            apply_archive_tree(node)
        }
    }

    impl<A, E> ApplyNode<Prepared<Intent<Item, LocalTarget, CreateOrReplace>, ArchiveTree<A>, E>>
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
            node: Prepared<Intent<Item, LocalTarget, CreateOrReplace>, ArchiveTree<A>, E>,
        ) -> Result<Self::Output, Self::Error> {
            apply_archive_tree(node)
        }
    }

    fn apply_archive_tree<A, O, E>(
        node: Prepared<Intent<Item, LocalTarget, O>, ArchiveTree<A>, E>,
    ) -> Result<LocalApplied<O, E>, PulithError>
    where
        LocalApply<O>: ApplyNode<
                Prepared<Intent<Item, LocalTarget, O>, LocalPrepared, E>,
                Receipt = Receipt<O>,
                Evidence = ApplyEvidence,
                Error = PulithError,
                Output = Applied<
                    Intent<Item, LocalTarget, O>,
                    Receipt<O>,
                    EvidenceChain<E, ApplyEvidence>,
                >,
            >,
    {
        let local_node = Prepared::from_prepare(
            node.input,
            LocalPrepared {
                path: node.prepared.root,
                kind: MaterialKind::Directory,
            },
            node.evidence,
        );
        LocalApply::<O>::new().apply_node(local_node)
    }
}

#[cfg(all(test, feature = "zip", feature = "local"))]
mod tests {
    use std::fs::{self, File};
    use std::io::Write;
    use std::path::{Path, PathBuf};

    use crate::{
        AcquireNode, ApplyNode, ArchiveNeed, ArchivePolicy, ArchivePrepare, CreateOrReplace,
        ExistingExtractRoot, Identity, IdentityVerify, Intent, Item, LocalAcquire, LocalApply,
        LocalPath, LocalTarget, PrepareNode, PulithError, VerifyNode, Zip,
    };

    fn temp_root(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "pulith-archive-{name}-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
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

    fn verified_archive(
        root: &Path,
        zip_path: &Path,
        target: &Path,
    ) -> crate::Verified<
        crate::Intent<crate::Item, crate::LocalTarget, crate::CreateOrReplace>,
        crate::LocalMaterial,
        crate::AcquireEvidence,
    > {
        let chosen = Intent::new(Item::new("archive"), LocalTarget::new(target))
            .with_source(LocalPath::new(zip_path))
            .select_first()
            .unwrap();
        let acquired = LocalAcquire.acquire_node(chosen).unwrap();
        assert!(root.exists());
        IdentityVerify.verify_node(acquired, Identity).unwrap()
    }

    #[test]
    fn zip_prepare_extracts_archive_tree() {
        let root = temp_root("extract");
        let zip_path = root.join("payload.zip");
        let extract_root = root.join("extract");
        let target = root.join("target");
        fs::create_dir_all(&root).unwrap();
        write_zip(&zip_path, &[("bin/tool.txt", b"pulith")]);

        let verified = verified_archive(&root, &zip_path, &target);
        let prepared = ArchivePrepare::<Zip>::new(ExistingExtractRoot::new(&extract_root))
            .prepare_node(verified, ArchiveNeed::default())
            .unwrap();

        assert_eq!(prepared.prepared.root, extract_root);
        assert_eq!(prepared.evidence.current.entries, 1);
        assert_eq!(prepared.evidence.current.files, 1);
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

        let verified = verified_archive(&root, &zip_path, &root.join("target"));
        ArchivePrepare::<Zip>::new(ExistingExtractRoot::new(&extract_root))
            .prepare_node(verified, ArchiveNeed::default())
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

        let verified = verified_archive(&root, &zip_path, &root.join("target"));
        let prepared = ArchivePrepare::<Zip>::new(ExistingExtractRoot::new(&extract_root))
            .prepare_node(
                verified,
                ArchiveNeed::new(ArchivePolicy::new().strip_components(1)),
            )
            .unwrap();

        assert_eq!(prepared.evidence.current.entries, 2);
        assert_eq!(prepared.evidence.current.directories, 1);
        assert_eq!(prepared.evidence.current.files, 1);
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

        let verified = verified_archive(&root, &zip_path, &root.join("target"));
        let err = ArchivePrepare::<Zip>::new(ExistingExtractRoot::new(root.join("extract")))
            .prepare_node(
                verified,
                ArchiveNeed::new(ArchivePolicy::new().max_entries(1)),
            )
            .unwrap_err();

        assert!(matches!(
            err,
            PulithError::ArchiveLimitExceeded {
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

        let verified = verified_archive(&root, &zip_path, &root.join("target"));
        let err = ArchivePrepare::<Zip>::new(ExistingExtractRoot::new(root.join("extract")))
            .prepare_node(verified, ArchiveNeed::default())
            .unwrap_err();

        assert!(matches!(err, PulithError::ArchiveInvalidPath(_)));
        assert!(!root.parent().unwrap().join("escape.txt").exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn zip_prepare_rejects_symlink_entry() {
        let root = temp_root("symlink");
        let zip_path = root.join("payload.zip");
        fs::create_dir_all(&root).unwrap();
        write_zip_with_symlink(&zip_path);

        let verified = verified_archive(&root, &zip_path, &root.join("target"));
        let err = ArchivePrepare::<Zip>::new(ExistingExtractRoot::new(root.join("extract")))
            .prepare_node(verified, ArchiveNeed::default())
            .unwrap_err();

        assert!(matches!(err, PulithError::UnsupportedArchiveEntry(_)));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn zip_prepare_rejects_byte_limit() {
        let root = temp_root("byte-limit");
        let zip_path = root.join("payload.zip");
        fs::create_dir_all(&root).unwrap();
        write_zip(&zip_path, &[("a.txt", b"abcd")]);

        let verified = verified_archive(&root, &zip_path, &root.join("target"));
        let err = ArchivePrepare::<Zip>::new(ExistingExtractRoot::new(root.join("extract")))
            .prepare_node(
                verified,
                ArchiveNeed::new(ArchivePolicy::new().max_total_bytes(3)),
            )
            .unwrap_err();

        assert!(matches!(
            err,
            PulithError::ArchiveLimitExceeded {
                limit: "total-bytes",
                ..
            }
        ));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn zip_prepare_flows_into_local_apply() {
        let root = temp_root("apply");
        let zip_path = root.join("payload.zip");
        let extract_root = root.join("extract");
        let target = root.join("target");
        fs::create_dir_all(&root).unwrap();
        write_zip(&zip_path, &[("bin/tool.txt", b"pulith")]);

        let verified = verified_archive(&root, &zip_path, &target);
        let prepared = ArchivePrepare::<Zip>::new(ExistingExtractRoot::new(&extract_root))
            .prepare_node(verified, ArchiveNeed::default())
            .unwrap();
        LocalApply::<CreateOrReplace>::new()
            .apply_node(prepared)
            .unwrap();

        assert_eq!(
            fs::read_to_string(target.join("bin/tool.txt")).unwrap(),
            "pulith"
        );
        fs::remove_dir_all(root).unwrap();
    }
}

#[cfg(all(test, feature = "tar", feature = "local"))]
mod tar_tests {
    use std::fs::{self, File};
    use std::io::{Cursor, Write};
    use std::path::{Path, PathBuf};

    #[cfg(feature = "gzip")]
    use crate::Gzip;
    #[cfg(feature = "xz")]
    use crate::Xz;
    #[cfg(feature = "zstd")]
    use crate::Zstd;
    use crate::{
        AcquireNode, ApplyNode, ArchiveNeed, ArchivePolicy, ArchivePrepare, CreateOrReplace,
        ExistingExtractRoot, Identity, IdentityVerify, Intent, Item, LocalAcquire, LocalApply,
        LocalPath, LocalTarget, PrepareNode, PulithError, Tar, VerifyNode,
    };

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

    fn verified_archive(
        root: &Path,
        tar_path: &Path,
        target: &Path,
    ) -> crate::Verified<
        crate::Intent<crate::Item, crate::LocalTarget, crate::CreateOrReplace>,
        crate::LocalMaterial,
        crate::AcquireEvidence,
    > {
        let chosen = Intent::new(Item::new("archive"), LocalTarget::new(target))
            .with_source(LocalPath::new(tar_path))
            .select_first()
            .unwrap();
        let acquired = LocalAcquire.acquire_node(chosen).unwrap();
        assert!(root.exists());
        IdentityVerify.verify_node(acquired, Identity).unwrap()
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

        let verified = verified_archive(&root, &tar_path, &root.join("target"));
        let prepared = ArchivePrepare::<Tar>::new(ExistingExtractRoot::new(&extract_root))
            .prepare_node(verified, ArchiveNeed::default())
            .unwrap();

        assert_eq!(prepared.evidence.current.entries, 1);
        assert_eq!(prepared.evidence.current.files, 1);
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

        let verified = verified_archive(&root, &tar_path, &root.join("target"));
        let prepared = ArchivePrepare::<Tar>::new(ExistingExtractRoot::new(&extract_root))
            .prepare_node(
                verified,
                ArchiveNeed::new(ArchivePolicy::new().strip_components(1)),
            )
            .unwrap();

        assert_eq!(prepared.evidence.current.entries, 2);
        assert_eq!(prepared.evidence.current.directories, 1);
        assert_eq!(prepared.evidence.current.files, 1);
        assert_eq!(
            fs::read_to_string(extract_root.join("bin/tool.txt")).unwrap(),
            "pulith"
        );
        assert!(!extract_root.join("root/bin/tool.txt").exists());
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

        let verified = verified_archive(&root, &tar_path, &root.join("target"));
        let err = ArchivePrepare::<Tar>::new(ExistingExtractRoot::new(root.join("extract")))
            .prepare_node(
                verified,
                ArchiveNeed::new(ArchivePolicy::new().max_entries(1)),
            )
            .unwrap_err();

        assert!(matches!(
            err,
            PulithError::ArchiveLimitExceeded {
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

        let verified = verified_archive(&root, &tar_path, &root.join("target"));
        let err = ArchivePrepare::<Tar>::new(ExistingExtractRoot::new(root.join("extract")))
            .prepare_node(verified, ArchiveNeed::default())
            .unwrap_err();

        assert!(matches!(err, PulithError::ArchiveInvalidPath(_)));
        assert!(!root.parent().unwrap().join("escape.txt").exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn tar_prepare_rejects_symlink_entry() {
        let root = temp_root("symlink");
        let tar_path = root.join("payload.tar");
        fs::create_dir_all(&root).unwrap();
        write_tar(&tar_path, |builder| {
            append_symlink(builder, "link", "target")
        });

        let verified = verified_archive(&root, &tar_path, &root.join("target"));
        let err = ArchivePrepare::<Tar>::new(ExistingExtractRoot::new(root.join("extract")))
            .prepare_node(verified, ArchiveNeed::default())
            .unwrap_err();

        assert!(matches!(err, PulithError::UnsupportedArchiveEntry(_)));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn tar_prepare_rejects_byte_limit() {
        let root = temp_root("byte-limit");
        let tar_path = root.join("payload.tar");
        fs::create_dir_all(&root).unwrap();
        write_tar(&tar_path, |builder| append_file(builder, "a.txt", b"abcd"));

        let verified = verified_archive(&root, &tar_path, &root.join("target"));
        let err = ArchivePrepare::<Tar>::new(ExistingExtractRoot::new(root.join("extract")))
            .prepare_node(
                verified,
                ArchiveNeed::new(ArchivePolicy::new().max_total_bytes(3)),
            )
            .unwrap_err();

        assert!(matches!(
            err,
            PulithError::ArchiveLimitExceeded {
                limit: "total-bytes",
                ..
            }
        ));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn tar_prepare_flows_into_local_apply() {
        let root = temp_root("apply");
        let tar_path = root.join("payload.tar");
        let extract_root = root.join("extract");
        let target = root.join("target");
        fs::create_dir_all(&root).unwrap();
        write_tar(&tar_path, |builder| {
            append_file(builder, "bin/tool.txt", b"pulith")
        });

        let verified = verified_archive(&root, &tar_path, &target);
        let prepared = ArchivePrepare::<Tar>::new(ExistingExtractRoot::new(&extract_root))
            .prepare_node(verified, ArchiveNeed::default())
            .unwrap();
        LocalApply::<CreateOrReplace>::new()
            .apply_node(prepared)
            .unwrap();

        assert_eq!(
            fs::read_to_string(target.join("bin/tool.txt")).unwrap(),
            "pulith"
        );
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

        let verified = verified_archive(&root, &tar_path, &root.join("target"));
        let prepared = ArchivePrepare::<Tar<Gzip>>::new(ExistingExtractRoot::new(&extract_root))
            .prepare_node(verified, ArchiveNeed::default())
            .unwrap();

        assert_eq!(prepared.evidence.current.entries, 1);
        assert_eq!(prepared.evidence.current.files, 1);
        assert_eq!(
            fs::read_to_string(extract_root.join("bin/tool.txt")).unwrap(),
            "pulith"
        );
        fs::remove_dir_all(root).unwrap();
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

        let verified = verified_archive(&root, &tar_path, &root.join("target"));
        let err = ArchivePrepare::<Tar<Gzip>>::new(ExistingExtractRoot::new(root.join("extract")))
            .prepare_node(verified, ArchiveNeed::default())
            .unwrap_err();

        assert!(matches!(err, PulithError::ArchiveInvalidPath(_)));
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

        let verified = verified_archive(&root, &tar_path, &root.join("target"));
        let err = ArchivePrepare::<Tar<Gzip>>::new(ExistingExtractRoot::new(root.join("extract")))
            .prepare_node(
                verified,
                ArchiveNeed::new(ArchivePolicy::new().max_total_bytes(3)),
            )
            .unwrap_err();

        assert!(matches!(
            err,
            PulithError::ArchiveLimitExceeded {
                limit: "total-bytes",
                ..
            }
        ));
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(feature = "gzip")]
    #[test]
    fn tar_gzip_prepare_flows_into_local_apply() {
        let root = temp_root("gzip-apply");
        let tar_path = root.join("payload.tar.gz");
        let extract_root = root.join("extract");
        let target = root.join("target");
        fs::create_dir_all(&root).unwrap();
        write_tar_gzip(&tar_path, |builder| {
            append_file(builder, "bin/tool.txt", b"pulith")
        });

        let verified = verified_archive(&root, &tar_path, &target);
        let prepared = ArchivePrepare::<Tar<Gzip>>::new(ExistingExtractRoot::new(&extract_root))
            .prepare_node(verified, ArchiveNeed::default())
            .unwrap();
        LocalApply::<CreateOrReplace>::new()
            .apply_node(prepared)
            .unwrap();

        assert_eq!(
            fs::read_to_string(target.join("bin/tool.txt")).unwrap(),
            "pulith"
        );
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

        let verified = verified_archive(&root, &tar_path, &root.join("target"));
        let prepared = ArchivePrepare::<Tar<Xz>>::new(ExistingExtractRoot::new(&extract_root))
            .prepare_node(verified, ArchiveNeed::default())
            .unwrap();

        assert_eq!(prepared.evidence.current.entries, 1);
        assert_eq!(
            fs::read_to_string(extract_root.join("bin/tool.txt")).unwrap(),
            "pulith"
        );
        fs::remove_dir_all(root).unwrap();
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

        let verified = verified_archive(&root, &tar_path, &root.join("target"));
        let prepared = ArchivePrepare::<Tar<Zstd>>::new(ExistingExtractRoot::new(&extract_root))
            .prepare_node(verified, ArchiveNeed::default())
            .unwrap();

        assert_eq!(prepared.evidence.current.entries, 1);
        assert_eq!(
            fs::read_to_string(extract_root.join("bin/tool.txt")).unwrap(),
            "pulith"
        );
        fs::remove_dir_all(root).unwrap();
    }
}
