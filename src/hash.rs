#[cfg(all(feature = "local", any(feature = "blake3", feature = "sha2")))]
use std::fs::File;
#[cfg(all(feature = "local", any(feature = "blake3", feature = "sha2")))]
use std::io::{self, Read};
use std::marker::PhantomData;
#[cfg(feature = "local")]
use std::path::Path;

#[cfg(feature = "local")]
use crate::PulithError;
#[cfg(feature = "local")]
use crate::{Acquired, EvidenceChain, Inspect, Inspected, Reconcile, Reconciled, Verified, Verify};

#[cfg(feature = "local")]
trait DigestAlgorithm {
    fn digest_opened_file_with_size(
        file: &mut File,
        path: &Path,
    ) -> Result<(String, u64), PulithError>;
}

#[cfg(feature = "local")]
type DigestVerified<I, E, A> =
    Verified<I, crate::local::LocalMaterial, EvidenceChain<E, DigestEvidence<A>>>;

#[cfg(feature = "local")]
type DescriptorVerified<I, E, A> =
    Verified<I, crate::local::LocalMaterial, EvidenceChain<E, DescriptorEvidence<A>>>;

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

    #[cfg(feature = "local")]
    fn into_string(self) -> String {
        self.value
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
pub struct HashVerify<A> {
    _algorithm: PhantomData<A>,
}

impl<A> HashVerify<A> {
    pub fn new() -> Self {
        Self {
            _algorithm: PhantomData,
        }
    }
}

#[cfg(feature = "local")]
/// Exact, hash-backed observation of one local artifact target.
///
/// File descriptors contain bytes counted by the same read loop as the digest. Other variants are
/// valid read-only observations; open, attribute-query, and read failures remain errors. Entry-kind
/// observations can become stale immediately after their metadata or handle query.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LocalArtifactObservation<A> {
    Missing,
    File { descriptor: ArtifactDescriptor<A> },
    Directory,
    Symlink,
    Reparse,
    Other,
}

#[cfg(feature = "local")]
impl<A> LocalArtifactObservation<A> {
    fn kind(&self) -> crate::local::LocalEntryKind {
        match self {
            Self::Missing => crate::local::LocalEntryKind::Missing,
            Self::File { .. } => crate::local::LocalEntryKind::File,
            Self::Directory => crate::local::LocalEntryKind::Directory,
            Self::Symlink => crate::local::LocalEntryKind::Symlink,
            Self::Reparse => crate::local::LocalEntryKind::Reparse,
            Self::Other => crate::local::LocalEntryKind::Other,
        }
    }
}

#[cfg(feature = "local")]
/// Evidence that a selected hash adapter produced an exact local artifact observation.
pub struct ArtifactInspectEvidence<A> {
    _algorithm: PhantomData<A>,
}

#[cfg(feature = "local")]
impl<A> Clone for ArtifactInspectEvidence<A> {
    fn clone(&self) -> Self {
        *self
    }
}

#[cfg(feature = "local")]
impl<A> Copy for ArtifactInspectEvidence<A> {}

#[cfg(feature = "local")]
impl<A> Default for ArtifactInspectEvidence<A> {
    fn default() -> Self {
        Self {
            _algorithm: PhantomData,
        }
    }
}

#[cfg(feature = "local")]
impl<A> std::fmt::Debug for ArtifactInspectEvidence<A> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("ArtifactInspectEvidence")
    }
}

#[cfg(feature = "local")]
impl<A> PartialEq for ArtifactInspectEvidence<A> {
    fn eq(&self, _other: &Self) -> bool {
        true
    }
}

#[cfg(feature = "local")]
impl<A> Eq for ArtifactInspectEvidence<A> {}

#[cfg(feature = "local")]
/// Opt-in full-read inspector for a local regular-file target using algorithm `A`.
///
/// Only the final path component is opened without following links; parent directories must be
/// trusted. The observed byte stream is not an atomic snapshot under concurrent in-place writes.
pub struct HashInspect<A> {
    _algorithm: PhantomData<A>,
}

#[cfg(feature = "local")]
impl<A> Clone for HashInspect<A> {
    fn clone(&self) -> Self {
        *self
    }
}

#[cfg(feature = "local")]
impl<A> Copy for HashInspect<A> {}

#[cfg(feature = "local")]
impl<A> Default for HashInspect<A> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(feature = "local")]
impl<A> std::fmt::Debug for HashInspect<A> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("HashInspect")
    }
}

#[cfg(feature = "local")]
impl<A> HashInspect<A> {
    /// Creates an exact local artifact inspector for algorithm `A`.
    pub fn new() -> Self {
        Self {
            _algorithm: PhantomData,
        }
    }
}

#[cfg(feature = "local")]
impl<A: DigestAlgorithm> Inspect<crate::local::LocalTarget> for HashInspect<A> {
    type Error = PulithError;
    type Output = Inspected<
        crate::local::LocalTarget,
        LocalArtifactObservation<A>,
        ArtifactInspectEvidence<A>,
    >;

    fn inspect(&self, node: crate::local::LocalTarget) -> Result<Self::Output, Self::Error> {
        use crate::local::OpenedLocalArtifact;
        let observation = match crate::local::open_local_artifact(&node.path)? {
            OpenedLocalArtifact::Missing => LocalArtifactObservation::Missing,
            OpenedLocalArtifact::Directory => LocalArtifactObservation::Directory,
            OpenedLocalArtifact::Symlink => LocalArtifactObservation::Symlink,
            #[cfg(windows)]
            OpenedLocalArtifact::Reparse => LocalArtifactObservation::Reparse,
            OpenedLocalArtifact::Other => LocalArtifactObservation::Other,
            OpenedLocalArtifact::File(mut file) => {
                let (digest, size) = A::digest_opened_file_with_size(&mut file, &node.path)?;
                LocalArtifactObservation::File {
                    descriptor: ArtifactDescriptor::new(digest, size),
                }
            }
        };
        Ok(Inspected {
            input: node,
            observation,
            evidence: ArtifactInspectEvidence {
                _algorithm: PhantomData,
            },
        })
    }
}

#[cfg(feature = "local")]
/// Pure expected-descriptor versus observed-artifact classification.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ArtifactReconciliation<A> {
    Matches,
    Missing,
    WrongKind {
        observed: crate::local::LocalEntryKind,
    },
    SizeMismatch {
        expected: u64,
        observed: u64,
    },
    DigestMismatch {
        expected: DigestValue<A>,
        observed: DigestValue<A>,
    },
}

#[cfg(feature = "local")]
/// Evidence preserving the caller expectation and exact observation used by reconciliation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ArtifactReconcileEvidence<A> {
    pub expected: ArtifactDescriptor<A>,
    pub observed: LocalArtifactObservation<A>,
}

#[cfg(feature = "local")]
/// Pure reconciler for an expected exact regular-file descriptor.
#[derive(Clone, Copy, Debug, Default)]
pub struct ArtifactReconcile;

#[cfg(feature = "local")]
impl<A, E>
    Reconcile<
        Inspected<crate::local::LocalTarget, LocalArtifactObservation<A>, E>,
        ArtifactDescriptor<A>,
    > for ArtifactReconcile
{
    type Error = std::convert::Infallible;
    type Output = Reconciled<
        crate::local::LocalTarget,
        ArtifactReconciliation<A>,
        EvidenceChain<E, ArtifactReconcileEvidence<A>>,
    >;

    fn reconcile(
        &self,
        node: Inspected<crate::local::LocalTarget, LocalArtifactObservation<A>, E>,
        expected: ArtifactDescriptor<A>,
    ) -> Result<Self::Output, Self::Error> {
        let reconciliation = match &node.observation {
            LocalArtifactObservation::Missing => ArtifactReconciliation::Missing,
            LocalArtifactObservation::File { descriptor } if descriptor.size != expected.size => {
                ArtifactReconciliation::SizeMismatch {
                    expected: expected.size,
                    observed: descriptor.size,
                }
            }
            LocalArtifactObservation::File { descriptor }
                if descriptor.digest.as_str() != expected.digest.as_str() =>
            {
                ArtifactReconciliation::DigestMismatch {
                    expected: DigestValue::new(expected.digest.as_str()),
                    observed: DigestValue::new(descriptor.digest.as_str()),
                }
            }
            LocalArtifactObservation::File { .. } => ArtifactReconciliation::Matches,
            observation => ArtifactReconciliation::WrongKind {
                observed: observation.kind(),
            },
        };
        Ok(Reconciled {
            input: node.input,
            reconciliation,
            evidence: EvidenceChain {
                previous: node.evidence,
                current: ArtifactReconcileEvidence {
                    expected,
                    observed: node.observation,
                },
            },
        })
    }
}

#[cfg(all(feature = "local", feature = "blake3"))]
impl DigestAlgorithm for Blake3 {
    fn digest_opened_file_with_size(
        file: &mut File,
        path: &Path,
    ) -> Result<(String, u64), PulithError> {
        let mut hasher = blake3::Hasher::new();
        let bytes = copy_into_hasher(path, file, |bytes| {
            hasher.update(bytes);
        })?;
        Ok((hasher.finalize().to_hex().to_string(), bytes))
    }
}

#[cfg(all(feature = "local", feature = "sha2"))]
impl DigestAlgorithm for Sha256 {
    fn digest_opened_file_with_size(
        file: &mut File,
        path: &Path,
    ) -> Result<(String, u64), PulithError> {
        use sha2::{Digest, Sha256 as Sha256Hasher};

        let mut hasher = Sha256Hasher::new();
        let bytes = copy_into_hasher(path, file, |bytes| {
            hasher.update(bytes);
        })?;
        Ok((hex::encode(hasher.finalize()), bytes))
    }
}

#[cfg(feature = "local")]
fn verify_digest<I, E, A: DigestAlgorithm>(
    node: Acquired<I, crate::local::LocalMaterial, E>,
    expected: DigestValue<A>,
) -> Result<DigestVerified<I, E, A>, PulithError> {
    let path = node.material.path();
    let mut file = open_regular_digest_file(path)?;
    let (observed_digest, _) = A::digest_opened_file_with_size(&mut file, path)?;
    let observed = DigestValue::<A>::new(observed_digest);
    if observed.as_str() != expected.as_str() {
        return Err(PulithError::DigestMismatch {
            expected: expected.into_string(),
            observed: observed.into_string(),
        });
    }
    Ok(Verified {
        input: node.input,
        material: node.material,
        evidence: EvidenceChain {
            previous: node.evidence,
            current: DigestEvidence { expected, observed },
        },
    })
}

#[cfg(feature = "local")]
fn verify_descriptor<I, E, A: DigestAlgorithm>(
    node: Acquired<I, crate::local::LocalMaterial, E>,
    expected: ArtifactDescriptor<A>,
) -> Result<DescriptorVerified<I, E, A>, PulithError> {
    let path = node.material.path();
    let mut file = open_regular_digest_file(path)?;
    let metadata = file
        .metadata()
        .map_err(|error| PulithError::io("read digest material metadata", path, error))?;
    let metadata_size = metadata.len();
    if metadata_size != expected.size {
        return Err(PulithError::ArtifactSizeMismatch {
            expected: expected.size,
            observed: metadata_size,
        });
    }

    let (observed_digest, observed_size) = A::digest_opened_file_with_size(&mut file, path)?;
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

    Ok(Verified {
        input: node.input,
        material: node.material,
        evidence: EvidenceChain {
            previous: node.evidence,
            current: DescriptorEvidence { expected, observed },
        },
    })
}

#[cfg(feature = "local")]
macro_rules! impl_hash_verify {
    ($algorithm:ty) => {
        impl<I, E> Verify<Acquired<I, crate::local::LocalMaterial, E>, DigestValue<$algorithm>>
            for HashVerify<$algorithm>
        {
            type Error = PulithError;
            type Output = Verified<
                I,
                crate::local::LocalMaterial,
                EvidenceChain<E, DigestEvidence<$algorithm>>,
            >;

            fn verify(
                &self,
                node: Acquired<I, crate::local::LocalMaterial, E>,
                expected: DigestValue<$algorithm>,
            ) -> Result<Self::Output, Self::Error> {
                verify_digest(node, expected)
            }
        }

        impl<I, E>
            Verify<Acquired<I, crate::local::LocalMaterial, E>, ArtifactDescriptor<$algorithm>>
            for HashVerify<$algorithm>
        {
            type Error = PulithError;
            type Output = Verified<
                I,
                crate::local::LocalMaterial,
                EvidenceChain<E, DescriptorEvidence<$algorithm>>,
            >;

            fn verify(
                &self,
                node: Acquired<I, crate::local::LocalMaterial, E>,
                expected: ArtifactDescriptor<$algorithm>,
            ) -> Result<Self::Output, Self::Error> {
                verify_descriptor(node, expected)
            }
        }
    };
}

#[cfg(all(feature = "local", feature = "blake3"))]
impl_hash_verify!(Blake3);
#[cfg(all(feature = "local", feature = "sha2"))]
impl_hash_verify!(Sha256);

#[cfg(feature = "local")]
fn open_regular_digest_file(path: &Path) -> Result<File, PulithError> {
    match crate::local::open_local_artifact(path)? {
        crate::local::OpenedLocalArtifact::File(file) => Ok(file),
        crate::local::OpenedLocalArtifact::Missing => Err(PulithError::io(
            "open file for digest",
            path,
            std::io::Error::from(std::io::ErrorKind::NotFound),
        )),
        _ => Err(PulithError::UnsupportedDigestMaterial(path.to_path_buf())),
    }
}

fn normalize_hex(value: &str) -> String {
    value
        .chars()
        .filter(|ch| !ch.is_ascii_whitespace())
        .flat_map(char::to_lowercase)
        .collect()
}

#[cfg(all(feature = "local", any(feature = "blake3", feature = "sha2")))]
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

    use super::*;
    use crate::local::{LocalAcquire, LocalAcquireEvidence, LocalMaterial, LocalPath, LocalTarget};
    use crate::{Acquire, Acquired, Materialize, MaterializeMode, Verify};

    fn temp_root(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "pulith-{name}-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    fn acquired(
        source: &std::path::Path,
        target: &std::path::Path,
    ) -> Acquired<
        Materialize<&'static str, LocalPath, LocalTarget>,
        LocalMaterial,
        LocalAcquireEvidence,
    > {
        LocalAcquire
            .acquire(Materialize::new(
                "demo",
                LocalPath::new(source),
                LocalTarget::new(target),
                MaterializeMode::ReplaceOrCreate,
            ))
            .unwrap()
    }

    #[cfg(feature = "blake3")]
    #[test]
    fn blake3_verify_is_typed_and_does_not_apply() {
        use super::{Blake3, DigestValue};

        let root = temp_root("typed-blake3");
        let source = root.join("source.txt");
        let target = root.join("target.txt");
        fs::create_dir_all(&root).unwrap();
        fs::write(&source, "pulith").unwrap();
        let digest = blake3::hash(b"pulith").to_hex().to_string();

        let acquired = acquired(&source, &target);
        let verified = HashVerify::<Blake3>::new()
            .verify(acquired, DigestValue::<Blake3>::new(digest.clone()))
            .unwrap();

        assert_eq!(verified.evidence.current.expected.value, digest);
        assert_eq!(verified.evidence.current.observed.value, digest);
        assert!(!target.exists());

        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(feature = "blake3")]
    #[test]
    fn descriptor_verify_proves_digest_and_exact_size() {
        use super::{ArtifactDescriptor, Blake3, HashVerify};

        let root = temp_root("descriptor-exact");
        let source = root.join("source.txt");
        let target = root.join("target.txt");
        fs::create_dir_all(&root).unwrap();
        fs::write(&source, "pulith").unwrap();
        let digest = blake3::hash(b"pulith").to_hex().to_string();

        let acquired = acquired(&source, &target);
        let descriptor = ArtifactDescriptor::<Blake3>::new(digest, 6);
        let verified = HashVerify::<Blake3>::new()
            .verify(acquired, descriptor.clone())
            .unwrap();

        assert_eq!(verified.evidence.current.expected, descriptor);
        assert_eq!(verified.evidence.current.observed, descriptor);
        assert!(!target.exists());

        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(feature = "blake3")]
    #[test]
    fn descriptor_verify_rejects_size_before_digest() {
        use super::{ArtifactDescriptor, Blake3, HashVerify};

        let root = temp_root("descriptor-size");
        let source = root.join("source.txt");
        let target = root.join("target.txt");
        fs::create_dir_all(&root).unwrap();
        fs::write(&source, "pulith").unwrap();

        let acquired = acquired(&source, &target);
        let error = HashVerify::<Blake3>::new()
            .verify(acquired, ArtifactDescriptor::new("00".repeat(32), 7))
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
        use super::{ArtifactDescriptor, Blake3, HashVerify};

        let root = temp_root("descriptor-digest");
        let source = root.join("source.txt");
        let target = root.join("target.txt");
        fs::create_dir_all(&root).unwrap();
        fs::write(&source, "pulith").unwrap();
        let acquired = acquired(&source, &target);
        let error = HashVerify::<Blake3>::new()
            .verify(acquired, ArtifactDescriptor::new("00".repeat(32), 6))
            .unwrap_err();

        assert!(matches!(error, crate::PulithError::DigestMismatch { .. }));
        assert!(!target.exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(feature = "blake3")]
    #[test]
    fn staged_digest_mismatch_releases_material_custody() {
        use super::{Blake3, DigestValue};

        let root = temp_root("staged-digest-mismatch");
        fs::create_dir_all(&root).unwrap();
        let staged = tempfile::NamedTempFile::new_in(&root).unwrap();
        fs::write(staged.path(), b"pulith").unwrap();
        let staged_path = staged.path().to_path_buf();
        let node = Acquired {
            input: (),
            material: LocalMaterial::StagedFile {
                path: staged.into_temp_path(),
            },
            evidence: (),
        };

        let error = HashVerify::<Blake3>::new()
            .verify(node, DigestValue::<Blake3>::new("00".repeat(32)))
            .unwrap_err();

        assert!(matches!(error, crate::PulithError::DigestMismatch { .. }));
        assert!(!staged_path.exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(feature = "blake3")]
    #[test]
    fn successful_staged_verification_retains_custody_until_state_drop() {
        use super::{Blake3, DigestValue};

        let root = temp_root("staged-digest-success");
        fs::create_dir_all(&root).unwrap();
        let staged = tempfile::NamedTempFile::new_in(&root).unwrap();
        fs::write(staged.path(), b"pulith").unwrap();
        let staged_path = staged.path().to_path_buf();
        let node = Acquired {
            input: (),
            material: LocalMaterial::StagedFile {
                path: staged.into_temp_path(),
            },
            evidence: (),
        };

        let verified = HashVerify::<Blake3>::new()
            .verify(
                node,
                DigestValue::<Blake3>::new(blake3::hash(b"pulith").to_hex().to_string()),
            )
            .unwrap();

        assert!(staged_path.exists());
        drop(verified);
        assert!(!staged_path.exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(feature = "sha2")]
    #[test]
    fn sha256_verify_rejects_mismatch_before_apply() {
        use super::{DigestValue, Sha256};

        let root = temp_root("typed-sha2");
        let source = root.join("source.txt");
        let target = root.join("target.txt");
        fs::create_dir_all(&root).unwrap();
        fs::write(&source, "pulith").unwrap();

        let acquired = acquired(&source, &target);

        assert!(
            HashVerify::<Sha256>::new()
                .verify(acquired, DigestValue::<Sha256>::new("00".repeat(32)))
                .is_err()
        );
        assert!(!target.exists());

        fs::remove_dir_all(root).unwrap();
    }
}
