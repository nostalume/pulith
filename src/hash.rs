#[cfg(any(feature = "blake3", feature = "sha2"))]
use std::fs::File;
#[cfg(any(feature = "blake3", feature = "sha2"))]
use std::io::{self, Read};
use std::marker::PhantomData;
use std::path::Path;

use crate::PulithError;
#[cfg(feature = "local")]
use crate::{Acquired, EvidenceChain, Verified, VerifyNode};

pub trait DigestAlgorithm {
    const NAME: &'static str;

    fn digest_file_with_size(path: &Path) -> Result<(String, u64), PulithError>;

    fn digest_file(path: &Path) -> Result<String, PulithError> {
        Self::digest_file_with_size(path).map(|(digest, _)| digest)
    }
}

#[cfg(feature = "blake3")]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Blake3;

#[cfg(feature = "sha2")]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Sha256;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DigestValue<A> {
    value: String,
    _algorithm: PhantomData<A>,
}

impl<A> DigestValue<A> {
    pub fn new(value: impl Into<String>) -> Self {
        Self {
            value: normalize_hex(&value.into()),
            _algorithm: PhantomData,
        }
    }

    pub fn as_str(&self) -> &str {
        &self.value
    }

    pub fn into_string(self) -> String {
        self.value
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DigestNeed<A> {
    pub expected: DigestValue<A>,
}

impl<A> DigestNeed<A> {
    pub fn new(expected: impl Into<String>) -> Self {
        Self {
            expected: DigestValue::new(expected),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DigestEvidence<A> {
    pub expected: DigestValue<A>,
    pub observed: DigestValue<A>,
}

/// Source-independent identity for one exact raw artifact representation.
///
/// The digest proves byte equality with the supplied expectation; it does not authenticate the
/// expectation's publisher or provenance.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ArtifactDescriptor<A> {
    pub digest: DigestValue<A>,
    pub size: u64,
}

impl<A> ArtifactDescriptor<A> {
    pub fn new(digest: impl Into<String>, size: u64) -> Self {
        Self {
            digest: DigestValue::new(digest),
            size,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DescriptorEvidence<A> {
    pub expected: ArtifactDescriptor<A>,
    pub observed: ArtifactDescriptor<A>,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct DescriptorVerify<A> {
    _algorithm: PhantomData<A>,
}

impl<A> DescriptorVerify<A> {
    pub fn new() -> Self {
        Self {
            _algorithm: PhantomData,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct NoHashResource;

#[derive(Clone, Debug, Default)]
pub struct HashVerify<A, R = NoHashResource> {
    pub resources: R,
    _algorithm: PhantomData<A>,
}

impl<A> HashVerify<A, NoHashResource> {
    pub fn new() -> Self {
        Self {
            resources: NoHashResource,
            _algorithm: PhantomData,
        }
    }
}

#[cfg(feature = "blake3")]
impl DigestAlgorithm for Blake3 {
    const NAME: &'static str = "blake3";

    fn digest_file_with_size(path: &Path) -> Result<(String, u64), PulithError> {
        let mut file = open_digest_file(path)?;
        let mut hasher = blake3::Hasher::new();
        let bytes = copy_into_hasher(path, &mut file, |bytes| {
            hasher.update(bytes);
        })?;
        Ok((hasher.finalize().to_hex().to_string(), bytes))
    }
}

#[cfg(feature = "sha2")]
impl DigestAlgorithm for Sha256 {
    const NAME: &'static str = "sha256";

    fn digest_file_with_size(path: &Path) -> Result<(String, u64), PulithError> {
        use sha2::{Digest, Sha256 as Sha256Hasher};

        let mut file = open_digest_file(path)?;
        let mut hasher = Sha256Hasher::new();
        let bytes = copy_into_hasher(path, &mut file, |bytes| {
            hasher.update(bytes);
        })?;
        Ok((hex::encode(hasher.finalize()), bytes))
    }
}

#[cfg(feature = "local")]
impl<I, E, A, R> VerifyNode<Acquired<I, crate::local::LocalMaterial, E>> for HashVerify<A, R>
where
    A: DigestAlgorithm,
{
    type Need = DigestNeed<A>;
    type Evidence = DigestEvidence<A>;
    type Error = PulithError;
    type Output = Verified<I, crate::local::LocalMaterial, EvidenceChain<E, DigestEvidence<A>>>;

    fn verify_node(
        &self,
        node: Acquired<I, crate::local::LocalMaterial, E>,
        need: Self::Need,
    ) -> Result<Self::Output, Self::Error> {
        require_regular_digest_file(&node.material.path)?;

        let observed = DigestValue::<A>::new(A::digest_file(&node.material.path)?);
        if observed.as_str() != need.expected.as_str() {
            return Err(PulithError::DigestMismatch {
                expected: need.expected.into_string(),
                observed: observed.into_string(),
            });
        }

        let evidence = DigestEvidence {
            expected: need.expected,
            observed,
        };
        Ok(Verified::from_verify(
            node.input,
            node.material,
            EvidenceChain::new(node.evidence, evidence),
        ))
    }
}

#[cfg(feature = "local")]
impl<I, E, A> VerifyNode<Acquired<I, crate::local::LocalMaterial, E>> for DescriptorVerify<A>
where
    A: DigestAlgorithm,
{
    type Need = ArtifactDescriptor<A>;
    type Evidence = DescriptorEvidence<A>;
    type Error = PulithError;
    type Output = Verified<I, crate::local::LocalMaterial, EvidenceChain<E, DescriptorEvidence<A>>>;

    fn verify_node(
        &self,
        node: Acquired<I, crate::local::LocalMaterial, E>,
        expected: Self::Need,
    ) -> Result<Self::Output, Self::Error> {
        let metadata = require_regular_digest_file(&node.material.path)?;
        let metadata_size = metadata.len();
        if metadata_size != expected.size {
            return Err(PulithError::ArtifactSizeMismatch {
                expected: expected.size,
                observed: metadata_size,
            });
        }

        let (observed_digest, observed_size) = A::digest_file_with_size(&node.material.path)?;
        if observed_size != expected.size {
            return Err(PulithError::ArtifactSizeMismatch {
                expected: expected.size,
                observed: observed_size,
            });
        }
        let observed = ArtifactDescriptor::new(observed_digest, observed_size);
        if observed.digest.as_str() != expected.digest.as_str() {
            return Err(PulithError::DigestMismatch {
                expected: expected.digest.into_string(),
                observed: observed.digest.into_string(),
            });
        }

        Ok(Verified::from_verify(
            node.input,
            node.material,
            EvidenceChain::new(node.evidence, DescriptorEvidence { expected, observed }),
        ))
    }
}

#[cfg(feature = "local")]
fn require_regular_digest_file(path: &Path) -> Result<std::fs::Metadata, PulithError> {
    let metadata = std::fs::symlink_metadata(path)
        .map_err(|err| PulithError::io("read digest material metadata", path, err))?;
    let file_type = metadata.file_type();
    if file_type.is_symlink() || !file_type.is_file() {
        return Err(PulithError::UnsupportedDigestMaterial(path.to_path_buf()));
    }
    Ok(metadata)
}

fn normalize_hex(value: &str) -> String {
    value
        .chars()
        .filter(|ch| !ch.is_ascii_whitespace())
        .flat_map(char::to_lowercase)
        .collect()
}

#[cfg(any(feature = "blake3", feature = "sha2"))]
fn open_digest_file(path: &Path) -> Result<File, PulithError> {
    File::open(path).map_err(|err| PulithError::io("open file for digest", path, err))
}

#[cfg(any(feature = "blake3", feature = "sha2"))]
fn copy_into_hasher(
    path: &Path,
    reader: &mut impl Read,
    mut update: impl FnMut(&[u8]),
) -> Result<u64, PulithError> {
    let mut buffer = [0; 16 * 1024];
    let mut observed = 0_u64;
    loop {
        match reader.read(&mut buffer) {
            Ok(0) => return Ok(observed),
            Ok(n) => {
                update(&buffer[..n]);
                observed = observed.saturating_add(n as u64);
            }
            Err(err) if err.kind() == io::ErrorKind::Interrupted => {}
            Err(err) => {
                return Err(PulithError::io("read file for digest", path, err));
            }
        }
    }
}

#[cfg(all(test, feature = "local", any(feature = "blake3", feature = "sha2")))]
mod tests {
    use std::fs;

    use crate::{
        AcquireNode, HashVerify, Intent, Item, LocalAcquire, LocalPath, LocalTarget, VerifyNode,
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

    #[cfg(feature = "blake3")]
    #[test]
    fn blake3_verify_is_typed_and_does_not_apply() {
        use crate::{Blake3, DigestNeed};

        let root = temp_root("typed-blake3");
        let source = root.join("source.txt");
        let target = root.join("target.txt");
        fs::create_dir_all(&root).unwrap();
        fs::write(&source, "pulith").unwrap();
        let digest = blake3::hash(b"pulith").to_hex().to_string();

        let chosen = Intent::new(Item::new("demo"), LocalTarget::new(&target))
            .with_source(LocalPath::new(&source))
            .select_first()
            .unwrap();
        let acquired = LocalAcquire.acquire_node(chosen).unwrap();
        let verified = HashVerify::<Blake3>::new()
            .verify_node(acquired, DigestNeed::<Blake3>::new(digest.clone()))
            .unwrap();

        assert_eq!(verified.evidence.current.expected.value, digest);
        assert_eq!(verified.evidence.current.observed.value, digest);
        assert!(!target.exists());

        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(feature = "blake3")]
    #[test]
    fn descriptor_verify_proves_digest_and_exact_size() {
        use crate::{ArtifactDescriptor, Blake3, DescriptorVerify};

        let root = temp_root("descriptor-exact");
        let source = root.join("source.txt");
        let target = root.join("target.txt");
        fs::create_dir_all(&root).unwrap();
        fs::write(&source, "pulith").unwrap();
        let digest = blake3::hash(b"pulith").to_hex().to_string();

        let chosen = Intent::new(Item::new("demo"), LocalTarget::new(&target))
            .with_source(LocalPath::new(&source))
            .select_first()
            .unwrap();
        let acquired = LocalAcquire.acquire_node(chosen).unwrap();
        let descriptor = ArtifactDescriptor::<Blake3>::new(digest, 6);
        let verified = DescriptorVerify::<Blake3>::new()
            .verify_node(acquired, descriptor.clone())
            .unwrap();

        assert_eq!(verified.evidence.current.expected, descriptor);
        assert_eq!(verified.evidence.current.observed, descriptor);
        assert!(!target.exists());

        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(feature = "blake3")]
    #[test]
    fn descriptor_verify_rejects_size_before_digest() {
        use crate::{ArtifactDescriptor, Blake3, DescriptorVerify};

        let root = temp_root("descriptor-size");
        let source = root.join("source.txt");
        let target = root.join("target.txt");
        fs::create_dir_all(&root).unwrap();
        fs::write(&source, "pulith").unwrap();

        let chosen = Intent::new(Item::new("demo"), LocalTarget::new(&target))
            .with_source(LocalPath::new(&source))
            .select_first()
            .unwrap();
        let acquired = LocalAcquire.acquire_node(chosen).unwrap();
        let error = DescriptorVerify::<Blake3>::new()
            .verify_node(acquired, ArtifactDescriptor::new("00".repeat(32), 7))
            .unwrap_err();

        assert!(matches!(
            error,
            crate::PulithError::ArtifactSizeMismatch {
                expected: 7,
                observed: 6
            }
        ));
        assert!(!target.exists());

        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(feature = "blake3")]
    #[test]
    fn descriptor_verify_rejects_digest_when_size_matches() {
        use crate::{ArtifactDescriptor, Blake3, DescriptorVerify};

        let root = temp_root("descriptor-digest");
        let source = root.join("source.txt");
        let target = root.join("target.txt");
        fs::create_dir_all(&root).unwrap();
        fs::write(&source, "pulith").unwrap();
        let chosen = Intent::new(Item::new("demo"), LocalTarget::new(&target))
            .with_source(LocalPath::new(&source))
            .select_first()
            .unwrap();

        let acquired = LocalAcquire.acquire_node(chosen).unwrap();
        let error = DescriptorVerify::<Blake3>::new()
            .verify_node(acquired, ArtifactDescriptor::new("00".repeat(32), 6))
            .unwrap_err();

        assert!(matches!(error, crate::PulithError::DigestMismatch { .. }));
        assert!(!target.exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(feature = "sha2")]
    #[test]
    fn sha256_verify_rejects_mismatch_before_apply() {
        use crate::{DigestNeed, Sha256};

        let root = temp_root("typed-sha2");
        let source = root.join("source.txt");
        let target = root.join("target.txt");
        fs::create_dir_all(&root).unwrap();
        fs::write(&source, "pulith").unwrap();

        let chosen = Intent::new(Item::new("demo"), LocalTarget::new(&target))
            .with_source(LocalPath::new(&source))
            .select_first()
            .unwrap();
        let acquired = LocalAcquire.acquire_node(chosen).unwrap();

        assert!(
            HashVerify::<Sha256>::new()
                .verify_node(acquired, DigestNeed::<Sha256>::new("00".repeat(32)))
                .is_err()
        );
        assert!(!target.exists());

        fs::remove_dir_all(root).unwrap();
    }
}
