//! Typed digest verification and exact local artifact observation/reconciliation.
//!
//! Owns artifact-identity semantics: a caller-supplied digest or exact descriptor (`blake3` or
//! `sha2`) compared against observed bytes. Verification is factual and never applies, adopts, or
//! authorizes; exact inspection/reconciliation is opt-in under `local + blake3`/`local + sha2`.
//! No provenance or authenticity claim is made from a matching digest.
#[cfg(all(feature = "local", any(feature = "blake3", feature = "sha2")))]
use std::fs::File;
#[cfg(feature = "local")]
use std::io;
#[cfg(all(feature = "local", any(feature = "blake3", feature = "sha2")))]
use std::io::Read;
#[cfg(feature = "local")]
use std::path::{Path, PathBuf};

#[cfg(feature = "local")]
use crate::local::LocalArtifactObservation;
#[cfg(feature = "local")]
use crate::{
    Acquired, Applied, EvidenceChain, Inspect, Inspected, Materialize, Reconcile, Reconciled,
    Verified, Verify,
};

/// Errors produced by hash verification and exact local-artifact inspection.
#[cfg(feature = "local")]
#[non_exhaustive]
#[derive(Debug)]
pub enum HashError {
    DigestMismatch {
        expected: String,
        observed: String,
    },
    ArtifactSizeMismatch {
        expected: u64,
        observed: u64,
    },
    UnsupportedDigestMaterial(PathBuf),
    /// Local artifact inspection failed before hashing could begin.
    LocalArtifact {
        path: PathBuf,
        source: Box<crate::local::LocalError>,
    },
    Io {
        action: &'static str,
        path: PathBuf,
        source: io::Error,
    },
}

#[cfg(feature = "local")]
impl HashError {
    fn io(action: &'static str, path: impl AsRef<Path>, source: io::Error) -> Self {
        Self::Io {
            action,
            path: path.as_ref().to_path_buf(),
            source,
        }
    }
}

#[cfg(feature = "local")]
impl std::fmt::Display for HashError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DigestMismatch { expected, observed } => write!(
                f,
                "digest mismatch: expected {expected}, observed {observed}"
            ),
            Self::ArtifactSizeMismatch { expected, observed } => write!(
                f,
                "artifact size mismatch: expected {expected}, observed {observed}"
            ),
            Self::UnsupportedDigestMaterial(path) => {
                write!(f, "digest verification requires a file: {}", path.display())
            }
            Self::LocalArtifact { path, source } => write!(
                f,
                "failed to inspect digest material {}: {source}",
                path.display()
            ),
            Self::Io {
                action,
                path,
                source,
            } => write!(f, "failed to {action} {}: {source}", path.display()),
        }
    }
}

#[cfg(feature = "local")]
impl std::error::Error for HashError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::LocalArtifact { source, .. } => Some(source.as_ref()),
            Self::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}
#[cfg(feature = "local")]
/// Digest algorithm family: implementors attest digests the same way `Blake3`/`Sha256` do.
///
/// `DigestAlgorithm` is public vocabulary so callers can write one generic flow over the
/// algorithm instead of duplicating it per hash. Implementors hash an already-opened regular
/// file (`file`) whose path is `path` and return the hex digest string together with the byte
/// count hashed.
pub trait DigestAlgorithm {
    fn digest_opened_file_with_size(
        file: &mut File,
        path: &Path,
    ) -> Result<(String, u64), HashError>;
}

#[cfg(feature = "local")]
type DigestVerified<I, E> =
    Verified<I, crate::local::LocalMaterial, EvidenceChain<E, DigestEvidence>>;

#[cfg(feature = "local")]
type DescriptorVerified<I, E> =
    Verified<I, crate::local::LocalMaterial, EvidenceChain<E, DescriptorEvidence>>;

#[cfg(feature = "blake3")]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Blake3;

#[cfg(feature = "sha2")]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Sha256;

/// The declared digest algorithm, as data.
///
/// `DigestAlgorithmKind` is the canonical data representation of a digest algorithm (mirrors
/// `archive::ArchiveKind`): manifests and callers declare the algorithm as this value, and
/// data-driven entries (e.g. materialization) dispatch on it once in core. Callers holding a
/// statically known algorithm keep using the typed `HashVerify<A>` path.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DigestAlgorithmKind {
    Blake3,
    Sha256,
}

/// A digest value packed with its algorithm: the single data carried by verify/materialize
/// (no (kind, hex) pair in callers). The value law — exactly 64 hex digits — is enforced here.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DigestValue {
    algorithm: DigestAlgorithmKind,
    value: String,
}

impl DigestValue {
    /// Construct a digest value; the 64-hex law is enforced here.
    pub fn new(algorithm: DigestAlgorithmKind, value: impl Into<String>) -> Result<Self, String> {
        let value = value.into();
        if value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            Ok(Self {
                algorithm,
                value: normalize_hex(&value),
            })
        } else {
            Err(format!("digest value {value:?} must be 64 hex digits"))
        }
    }

    pub fn algorithm(&self) -> DigestAlgorithmKind {
        self.algorithm
    }

    pub fn as_str(&self) -> &str {
        &self.value
    }

    #[cfg(feature = "local")]
    fn into_string(self) -> String {
        self.value
    }
}

/// Optional serde deserialization (feature `serde`): consumes the manifest table
/// `kind = "blake3"|"sha2"` + `hex = "…"`. The kind mapping and the 64-hex law stay here.
#[cfg(feature = "serde")]
impl<'de> serde::Deserialize<'de> for DigestValue {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct Visitor;
        impl<'de> serde::de::Visitor<'de> for Visitor {
            type Value = DigestValue;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("a table with `kind` and `hex`")
            }

            fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
            where
                A: serde::de::MapAccess<'de>,
            {
                let mut kind: Option<String> = None;
                let mut hex: Option<String> = None;
                while let Some(key) = map.next_key::<String>()? {
                    match key.as_str() {
                        "kind" => kind = Some(map.next_value()?),
                        "hex" => hex = Some(map.next_value()?),
                        other => {
                            return Err(serde::de::Error::unknown_field(other, &["kind", "hex"]));
                        }
                    }
                }
                let kind = kind.ok_or_else(|| serde::de::Error::missing_field("kind"))?;
                let hex = hex.ok_or_else(|| serde::de::Error::missing_field("hex"))?;
                let algorithm = match kind.as_str() {
                    "blake3" => DigestAlgorithmKind::Blake3,
                    "sha2" => DigestAlgorithmKind::Sha256,
                    other => {
                        return Err(serde::de::Error::custom(format!(
                            "unknown digest kind {other:?} (expected blake3 or sha2)"
                        )));
                    }
                };
                DigestValue::new(algorithm, hex).map_err(serde::de::Error::custom)
            }
        }
        deserializer.deserialize_map(Visitor)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DigestEvidence {
    pub algorithm: DigestAlgorithmKind,
    pub expected: DigestValue,
    pub observed: DigestValue,
}

/// Source-independent identity for one exact raw artifact representation.
///
/// The digest proves byte equality with the supplied expectation; it does not authenticate the
/// expectation's publisher or provenance.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ArtifactDescriptor {
    pub digest: DigestValue,
    pub size: u64,
}

impl ArtifactDescriptor {
    pub fn new(algorithm: DigestAlgorithmKind, digest: impl Into<String>, size: u64) -> Self {
        Self {
            digest: DigestValue::new(algorithm, digest).expect("valid descriptor digest"),
            size,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DescriptorEvidence {
    pub algorithm: DigestAlgorithmKind,
    pub expected: ArtifactDescriptor,
    pub observed: ArtifactDescriptor,
}

#[derive(Clone, Copy, Debug)]
pub struct HashVerify {
    kind: DigestAlgorithmKind,
}

impl HashVerify {
    /// Creates a verify adapter for the declared `kind`.
    pub fn new(kind: DigestAlgorithmKind) -> Self {
        Self { kind }
    }
}

#[cfg(feature = "local")]
/// Evidence that a selected hash adapter produced an exact local artifact observation.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ArtifactInspectEvidence;

#[cfg(feature = "local")]
/// Opt-in full-read inspector for a local regular-file target.
///
/// Only the final path component is opened without following links; parent directories must be
/// trusted. The observed byte stream is not an atomic snapshot under concurrent in-place writes.
#[derive(Clone, Copy, Debug)]
pub struct HashInspect {
    kind: DigestAlgorithmKind,
}

#[cfg(feature = "local")]
impl HashInspect {
    /// Creates an exact local artifact inspector for the declared `kind`.
    pub const fn new(kind: DigestAlgorithmKind) -> Self {
        Self { kind }
    }
}

#[cfg(feature = "local")]
/// Exact local artifact inspection performed after a completed file materialization.
///
/// # Contract
///
/// Input is [`Applied`]`<Materialize<_, _, PathBuf>, E>`; it has no additional policy need.
/// Output carries the final [`LocalArtifactObservation`] and
/// `EvidenceChain<E, ArtifactInspectEvidence<A>>`, retaining the earlier materialization receipt
/// evidence. A local-admission or hash-read failure returns [`HashMaterializeInspectError`], which
/// retains the completed receipt. The behavior is read-only: it neither publishes nor repairs.
///
/// Local owns final-component no-follow admission and entry-kind classification. This adapter owns
/// the regular-file digest attestation. Callers supply an expected descriptor to
/// [`ArtifactReconcile`], and own durable records, retry, repair, and rollback. The observation is
/// later than publication, not an atomic publication proof or a continuing-integrity guarantee.
///
/// This adapter composes [`HashInspect`] with the completed receipt rather than duplicating its
/// local admission or digest logic.
#[derive(Clone, Copy, Debug)]
pub struct HashMaterializeInspect {
    kind: DigestAlgorithmKind,
}

#[cfg(feature = "local")]
impl HashMaterializeInspect {
    /// Creates a post-materialization exact inspector for the declared `kind`.
    pub fn new(kind: DigestAlgorithmKind) -> Self {
        Self { kind }
    }
}

#[cfg(feature = "local")]
/// An unavailable exact final-target observation together with its completed materialization.
#[derive(Debug)]
pub struct HashMaterializeInspectError<N, E> {
    pub applied: Applied<N, E>,
    pub cause: HashError,
}

#[cfg(feature = "local")]
impl<N, E> std::fmt::Display for HashMaterializeInspectError<N, E> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "post-materialize exact inspection failed: {}",
            self.cause
        )
    }
}

#[cfg(feature = "local")]
impl<N: std::fmt::Debug, E: std::fmt::Debug> std::error::Error
    for HashMaterializeInspectError<N, E>
{
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.cause)
    }
}
#[cfg(feature = "local")]
impl Inspect<std::path::PathBuf> for HashInspect {
    type Error = HashError;
    type Output = Inspected<
        std::path::PathBuf,
        LocalArtifactObservation<ArtifactDescriptor>,
        ArtifactInspectEvidence,
    >;

    fn inspect(&self, node: std::path::PathBuf) -> Result<Self::Output, Self::Error> {
        use crate::local::OpenedLocalArtifact;
        let observation = match crate::local::open_local_artifact(&node).map_err(|source| {
            HashError::LocalArtifact {
                path: node.clone(),
                source: Box::new(source),
            }
        })? {
            OpenedLocalArtifact::Missing => LocalArtifactObservation::Missing,
            OpenedLocalArtifact::Directory => LocalArtifactObservation::Directory,
            OpenedLocalArtifact::Symlink => LocalArtifactObservation::Symlink,
            #[cfg(windows)]
            OpenedLocalArtifact::Reparse => LocalArtifactObservation::Reparse,
            OpenedLocalArtifact::Other => LocalArtifactObservation::Other,
            OpenedLocalArtifact::File(mut file) => {
                let (digest, size) = digest_opened(self.kind, &mut file, &node)?;
                LocalArtifactObservation::File {
                    attestation: ArtifactDescriptor::new(self.kind, digest, size),
                }
            }
        };
        Ok(Inspected {
            input: node,
            observation,
            evidence: ArtifactInspectEvidence,
        })
    }
}

#[cfg(feature = "local")]
impl<I, S, E> Inspect<Applied<Materialize<I, S, std::path::PathBuf>, E>>
    for HashMaterializeInspect
{
    type Error = HashMaterializeInspectError<Materialize<I, S, std::path::PathBuf>, E>;
    type Output = Inspected<
        std::path::PathBuf,
        LocalArtifactObservation<ArtifactDescriptor>,
        EvidenceChain<E, ArtifactInspectEvidence>,
    >;

    fn inspect(
        &self,
        applied: Applied<Materialize<I, S, std::path::PathBuf>, E>,
    ) -> Result<Self::Output, Self::Error> {
        let target = applied.input.target.clone();
        match HashInspect::new(self.kind).inspect(target) {
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
            Err(cause) => Err(HashMaterializeInspectError { applied, cause }),
        }
    }
}
#[cfg(feature = "local")]
/// Pure expected-descriptor versus observed-artifact classification.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ArtifactReconciliation {
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
        expected: DigestValue,
        observed: DigestValue,
    },
}

#[cfg(feature = "local")]
/// Evidence preserving the caller expectation and exact observation used by reconciliation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ArtifactReconcileEvidence {
    pub expected: ArtifactDescriptor,
    pub observed: LocalArtifactObservation<ArtifactDescriptor>,
}

#[cfg(feature = "local")]
/// Pure reconciler for an expected exact regular-file descriptor.
#[derive(Clone, Copy, Debug, Default)]
pub struct ArtifactReconcile;

#[cfg(feature = "local")]
impl<E>
    Reconcile<
        Inspected<std::path::PathBuf, LocalArtifactObservation<ArtifactDescriptor>, E>,
        ArtifactDescriptor,
    > for ArtifactReconcile
{
    type Error = std::convert::Infallible;
    type Output = Reconciled<
        std::path::PathBuf,
        ArtifactReconciliation,
        EvidenceChain<E, ArtifactReconcileEvidence>,
    >;

    fn reconcile(
        &self,
        node: Inspected<std::path::PathBuf, LocalArtifactObservation<ArtifactDescriptor>, E>,
        expected: ArtifactDescriptor,
    ) -> Result<Self::Output, Self::Error> {
        let reconciliation = match &node.observation {
            LocalArtifactObservation::Missing => ArtifactReconciliation::Missing,
            LocalArtifactObservation::File {
                attestation: descriptor,
            } if descriptor.size != expected.size => ArtifactReconciliation::SizeMismatch {
                expected: expected.size,
                observed: descriptor.size,
            },
            LocalArtifactObservation::File {
                attestation: descriptor,
            } if descriptor.digest.as_str() != expected.digest.as_str() => {
                ArtifactReconciliation::DigestMismatch {
                    expected: DigestValue::new(
                        expected.digest.algorithm(),
                        expected.digest.as_str(),
                    )
                    .unwrap(),
                    observed: DigestValue::new(
                        descriptor.digest.algorithm(),
                        descriptor.digest.as_str(),
                    )
                    .unwrap(),
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
    ) -> Result<(String, u64), HashError> {
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
    ) -> Result<(String, u64), HashError> {
        use sha2::{Digest, Sha256 as Sha256Hasher};

        let mut hasher = Sha256Hasher::new();
        let bytes = copy_into_hasher(path, file, |bytes| {
            hasher.update(bytes);
        })?;
        Ok((hex::encode(hasher.finalize()), bytes))
    }
}

#[cfg(feature = "local")]
fn verify_digest<I, E>(
    node: Acquired<I, crate::local::LocalMaterial, E>,
    expected: DigestValue,
    kind: DigestAlgorithmKind,
) -> Result<DigestVerified<I, E>, HashError> {
    let path = node.material.path();
    let mut file = open_regular_digest_file(path)?;
    let (observed_digest, _) = digest_opened(kind, &mut file, path)?;
    let observed = DigestValue::new(kind, observed_digest).expect("observed digest is 64 hex");
    if observed.as_str() != expected.as_str() {
        return Err(HashError::DigestMismatch {
            expected: expected.into_string(),
            observed: observed.into_string(),
        });
    }
    Ok(Verified {
        input: node.input,
        material: node.material,
        evidence: EvidenceChain {
            previous: node.evidence,
            current: DigestEvidence {
                algorithm: expected.algorithm(),
                expected,
                observed,
            },
        },
    })
}

/// One closed dispatch from the declared kind to the algorithm family (data-driven).
fn digest_opened(
    kind: DigestAlgorithmKind,
    file: &mut File,
    path: &Path,
) -> Result<(String, u64), HashError> {
    match kind {
        #[cfg(feature = "blake3")]
        DigestAlgorithmKind::Blake3 => Blake3::digest_opened_file_with_size(file, path),
        #[cfg(feature = "sha2")]
        DigestAlgorithmKind::Sha256 => Sha256::digest_opened_file_with_size(file, path),
        #[cfg(not(feature = "blake3"))]
        DigestAlgorithmKind::Blake3 => Err(HashError::io(
            "digest blake3 material",
            path,
            std::io::Error::other("blake3 feature is disabled"),
        )),
        #[cfg(not(feature = "sha2"))]
        DigestAlgorithmKind::Sha256 => Err(HashError::io(
            "digest sha256 material",
            path,
            std::io::Error::other("sha2 feature is disabled"),
        )),
    }
}

#[cfg(feature = "local")]
fn verify_descriptor<I, E>(
    node: Acquired<I, crate::local::LocalMaterial, E>,
    expected: ArtifactDescriptor,
    kind: DigestAlgorithmKind,
) -> Result<DescriptorVerified<I, E>, HashError> {
    let path = node.material.path();
    let mut file = open_regular_digest_file(path)?;
    let metadata = file
        .metadata()
        .map_err(|error| HashError::io("read digest material metadata", path, error))?;
    let metadata_size = metadata.len();
    if metadata_size != expected.size {
        return Err(HashError::ArtifactSizeMismatch {
            expected: expected.size,
            observed: metadata_size,
        });
    }

    let (observed_digest, observed_size) = digest_opened(kind, &mut file, path)?;
    if observed_size != expected.size {
        return Err(HashError::ArtifactSizeMismatch {
            expected: expected.size,
            observed: observed_size,
        });
    }
    let observed =
        ArtifactDescriptor::new(expected.digest.algorithm(), observed_digest, observed_size);
    if observed.digest.as_str() != expected.digest.as_str() {
        return Err(HashError::DigestMismatch {
            expected: expected.digest.into_string(),
            observed: observed.digest.into_string(),
        });
    }

    Ok(Verified {
        input: node.input,
        material: node.material,
        evidence: EvidenceChain {
            previous: node.evidence,
            current: DescriptorEvidence {
                algorithm: expected.digest.algorithm(),
                expected,
                observed,
            },
        },
    })
}

#[cfg(feature = "local")]
impl<I, E> Verify<Acquired<I, crate::local::LocalMaterial, E>, DigestValue> for HashVerify {
    type Error = HashError;
    type Output = Verified<I, crate::local::LocalMaterial, EvidenceChain<E, DigestEvidence>>;

    fn verify(
        &self,
        node: Acquired<I, crate::local::LocalMaterial, E>,
        expected: DigestValue,
    ) -> Result<Self::Output, Self::Error> {
        verify_digest(node, expected, self.kind)
    }
}

#[cfg(feature = "local")]
impl<I, E> Verify<Acquired<I, crate::local::LocalMaterial, E>, ArtifactDescriptor> for HashVerify {
    type Error = HashError;
    type Output = Verified<I, crate::local::LocalMaterial, EvidenceChain<E, DescriptorEvidence>>;

    fn verify(
        &self,
        node: Acquired<I, crate::local::LocalMaterial, E>,
        expected: ArtifactDescriptor,
    ) -> Result<Self::Output, Self::Error> {
        verify_descriptor(node, expected, self.kind)
    }
}

#[cfg(feature = "local")]
fn open_regular_digest_file(path: &Path) -> Result<File, HashError> {
    match crate::local::open_local_artifact(path).map_err(|source| HashError::LocalArtifact {
        path: path.to_path_buf(),
        source: Box::new(source),
    })? {
        crate::local::OpenedLocalArtifact::File(file) => Ok(file),
        crate::local::OpenedLocalArtifact::Missing => Err(HashError::io(
            "open file for digest",
            path,
            std::io::Error::from(std::io::ErrorKind::NotFound),
        )),
        _ => Err(HashError::UnsupportedDigestMaterial(path.to_path_buf())),
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
) -> Result<u64, HashError> {
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
                return Err(HashError::io("read file for digest", path, err));
            }
        }
    }
}

#[cfg(all(test, feature = "local", any(feature = "blake3", feature = "sha2")))]
mod tests {
    use std::fs;

    use super::*;
    use crate::local::{LocalAcquire, LocalAcquireEvidence, LocalMaterial};
    use crate::{Acquired, Materialize, MaterializeMode, Verify};

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
    ) -> Acquired<Materialize<&'static str, PathBuf, PathBuf>, LocalMaterial, LocalAcquireEvidence>
    {
        LocalAcquire
            .acquire(Materialize::new(
                "demo",
                source.to_path_buf(),
                target.to_path_buf(),
                MaterializeMode::ReplaceOrCreate,
            ))
            .unwrap()
    }

    #[cfg(feature = "blake3")]
    #[test]
    fn blake3_verify_is_typed_and_does_not_apply() {
        use super::DigestValue;

        let root = temp_root("typed-blake3");
        let source = root.join("source.txt");
        let target = root.join("target.txt");
        fs::create_dir_all(&root).unwrap();
        fs::write(&source, "pulith").unwrap();
        let digest = blake3::hash(b"pulith").to_hex().to_string();

        let acquired = acquired(&source, &target);
        let verified = HashVerify::new(DigestAlgorithmKind::Blake3)
            .verify(
                acquired,
                DigestValue::new(DigestAlgorithmKind::Blake3, digest.clone()).unwrap(),
            )
            .unwrap();

        assert_eq!(verified.evidence.current.expected.value, digest);
        assert_eq!(verified.evidence.current.observed.value, digest);
        assert!(!target.exists());

        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(feature = "blake3")]
    #[test]
    fn descriptor_verify_proves_digest_and_exact_size() {
        use super::{ArtifactDescriptor, HashVerify};

        let root = temp_root("descriptor-exact");
        let source = root.join("source.txt");
        let target = root.join("target.txt");
        fs::create_dir_all(&root).unwrap();
        fs::write(&source, "pulith").unwrap();
        let digest = blake3::hash(b"pulith").to_hex().to_string();

        let acquired = acquired(&source, &target);
        let descriptor = ArtifactDescriptor::new(DigestAlgorithmKind::Blake3, digest, 6);
        let verified = HashVerify::new(DigestAlgorithmKind::Blake3)
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
        use super::{ArtifactDescriptor, HashVerify};

        let root = temp_root("descriptor-size");
        let source = root.join("source.txt");
        let target = root.join("target.txt");
        fs::create_dir_all(&root).unwrap();
        fs::write(&source, "pulith").unwrap();

        let acquired = acquired(&source, &target);
        let error = HashVerify::new(DigestAlgorithmKind::Blake3)
            .verify(
                acquired,
                ArtifactDescriptor::new(DigestAlgorithmKind::Blake3, "00".repeat(32), 7),
            )
            .unwrap_err();

        assert!(matches!(
            error,
            HashError::ArtifactSizeMismatch {
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
        use super::{ArtifactDescriptor, HashVerify};

        let root = temp_root("descriptor-digest");
        let source = root.join("source.txt");
        let target = root.join("target.txt");
        fs::create_dir_all(&root).unwrap();
        fs::write(&source, "pulith").unwrap();
        let acquired = acquired(&source, &target);
        let error = HashVerify::new(DigestAlgorithmKind::Blake3)
            .verify(
                acquired,
                ArtifactDescriptor::new(DigestAlgorithmKind::Blake3, "00".repeat(32), 6),
            )
            .unwrap_err();

        assert!(matches!(error, HashError::DigestMismatch { .. }));
        assert!(!target.exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(feature = "blake3")]
    #[test]
    fn staged_digest_mismatch_releases_material_custody() {
        use super::DigestValue;

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

        let error = HashVerify::new(DigestAlgorithmKind::Blake3)
            .verify(
                node,
                DigestValue::new(DigestAlgorithmKind::Blake3, "00".repeat(32)).unwrap(),
            )
            .unwrap_err();

        assert!(matches!(error, HashError::DigestMismatch { .. }));
        assert!(!staged_path.exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(feature = "blake3")]
    #[test]
    fn successful_staged_verification_retains_custody_until_state_drop() {
        use super::DigestValue;

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

        let verified = HashVerify::new(DigestAlgorithmKind::Blake3)
            .verify(
                node,
                DigestValue::new(
                    DigestAlgorithmKind::Blake3,
                    blake3::hash(b"pulith").to_hex().to_string(),
                )
                .unwrap(),
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
        use super::DigestValue;

        let root = temp_root("typed-sha2");
        let source = root.join("source.txt");
        let target = root.join("target.txt");
        fs::create_dir_all(&root).unwrap();
        fs::write(&source, "pulith").unwrap();

        let acquired = acquired(&source, &target);

        assert!(
            HashVerify::new(DigestAlgorithmKind::Sha256)
                .verify(
                    acquired,
                    DigestValue::new(DigestAlgorithmKind::Blake3, "00".repeat(32)).unwrap()
                )
                .is_err()
        );
        assert!(!target.exists());

        fs::remove_dir_all(root).unwrap();
    }
}
