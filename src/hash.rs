//! Typed digest verification and exact local artifact observation/reconciliation.
//!
//! Owns artifact-identity semantics: a caller-supplied digest or exact descriptor (`blake3` or
//! `sha2`) compared against observed bytes. Verification is factual and never applies, adopts, or
//! authorizes; exact inspection/reconciliation is opt-in under `local + blake3`/`local + sha2`.
//! No provenance or authenticity claim is made from a matching digest.
#[cfg(all(feature = "local", any(feature = "blake3", feature = "sha2")))]
use crate::local::{LocalArtifactObservation, LocalMaterial, LocalTarget, OpenedLocalArtifact};
#[cfg(all(feature = "local", any(feature = "blake3", feature = "sha2")))]
use crate::{Inspect, Reconcile, Verify};
#[cfg(all(feature = "local", any(feature = "blake3", feature = "sha2")))]
use std::fs::File;
#[cfg(all(feature = "local", any(feature = "blake3", feature = "sha2")))]
use std::io;
#[cfg(all(feature = "local", any(feature = "blake3", feature = "sha2")))]
use std::io::Read;
#[cfg(all(feature = "local", any(feature = "blake3", feature = "sha2")))]
use std::path::{Path, PathBuf};

/// Errors produced by hash verification and exact local-artifact inspection.
#[cfg(all(feature = "local", any(feature = "blake3", feature = "sha2")))]
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

#[cfg(all(feature = "local", any(feature = "blake3", feature = "sha2")))]
impl HashError {
    fn io(action: &'static str, path: impl AsRef<Path>, source: io::Error) -> Self {
        Self::Io {
            action,
            path: path.as_ref().to_path_buf(),
            source,
        }
    }
}

#[cfg(all(feature = "local", any(feature = "blake3", feature = "sha2")))]
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

#[cfg(all(feature = "local", any(feature = "blake3", feature = "sha2")))]
impl std::error::Error for HashError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::LocalArtifact { source, .. } => Some(source.as_ref()),
            Self::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}
#[cfg(all(feature = "local", any(feature = "blake3", feature = "sha2")))]
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
/// data-driven entries (e.g. materialization) dispatch on it once in core.
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

/// Evidence emitted by exact local artifact inspection.
#[cfg(all(feature = "local", any(feature = "blake3", feature = "sha2")))]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ArtifactInspectEvidence {
    pub algorithm: DigestAlgorithmKind,
}

/// Exact artifact classification against a caller-supplied descriptor.
#[cfg(all(feature = "local", any(feature = "blake3", feature = "sha2")))]
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

/// Evidence preserving the descriptor and observation used by exact reconciliation.
#[cfg(all(feature = "local", any(feature = "blake3", feature = "sha2")))]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ArtifactReconcileEvidence {
    pub expected: ArtifactDescriptor,
    pub observed: LocalArtifactObservation<ArtifactDescriptor>,
}

/// Exact, no-follow inspection selected by a caller-supplied digest algorithm.
#[cfg(all(feature = "local", any(feature = "blake3", feature = "sha2")))]
impl Inspect<DigestAlgorithmKind> for LocalTarget {
    type Error = HashError;
    type Output = (
        LocalArtifactObservation<ArtifactDescriptor>,
        ArtifactInspectEvidence,
    );

    fn inspect(self, algorithm: DigestAlgorithmKind) -> Result<Self::Output, Self::Error> {
        let path = self.as_path().to_path_buf();
        let observation = match crate::local::open_local_artifact(&path).map_err(|source| {
            HashError::LocalArtifact {
                path: path.clone(),
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
                let (digest, size) = digest_opened(algorithm, &mut file, &path)?;
                LocalArtifactObservation::File {
                    attestation: ArtifactDescriptor::new(algorithm, digest, size),
                }
            }
        };
        Ok((observation, ArtifactInspectEvidence { algorithm }))
    }
}

/// Pure exact-artifact reconciliation selected by an `ArtifactDescriptor` expectation.
#[cfg(all(feature = "local", any(feature = "blake3", feature = "sha2")))]
impl Reconcile<ArtifactDescriptor> for LocalArtifactObservation<ArtifactDescriptor> {
    type Error = std::convert::Infallible;
    type Output = (ArtifactReconciliation, ArtifactReconcileEvidence);

    fn reconcile(self, expected: ArtifactDescriptor) -> Result<Self::Output, Self::Error> {
        let reconciliation = match &self {
            LocalArtifactObservation::Missing => ArtifactReconciliation::Missing,
            LocalArtifactObservation::File { attestation } if attestation.size != expected.size => {
                ArtifactReconciliation::SizeMismatch {
                    expected: expected.size,
                    observed: attestation.size,
                }
            }
            LocalArtifactObservation::File { attestation }
                if attestation.digest.as_str() != expected.digest.as_str() =>
            {
                ArtifactReconciliation::DigestMismatch {
                    expected: expected.digest.clone(),
                    observed: attestation.digest.clone(),
                }
            }
            LocalArtifactObservation::File { .. } => ArtifactReconciliation::Matches,
            observation => ArtifactReconciliation::WrongKind {
                observed: observation.kind(),
            },
        };
        Ok((
            reconciliation,
            ArtifactReconcileEvidence {
                expected,
                observed: self,
            },
        ))
    }
}

/// Verify one local regular file against the declared digest. This is the private implementation
/// of `LocalMaterial::verify`; the public behavior entry point is the trait implementation.
#[cfg(all(feature = "local", any(feature = "blake3", feature = "sha2")))]
pub(crate) fn verify_path(
    path: &Path,
    expected: &DigestValue,
) -> Result<DigestEvidence, HashError> {
    let mut file = open_regular_digest_file(path)?;
    let algorithm = expected.algorithm();
    let (observed_digest, _) = digest_opened(algorithm, &mut file, path)?;
    let observed = DigestValue::new(algorithm, observed_digest).expect("observed digest is 64 hex");
    if observed.as_str() != expected.as_str() {
        return Err(HashError::DigestMismatch {
            expected: expected.as_str().to_string(),
            observed: observed.as_str().to_string(),
        });
    }
    Ok(DigestEvidence {
        algorithm,
        expected: expected.clone(),
        observed,
    })
}

/// Verify a local byte material against a caller-declared digest. Directory trees have no byte
/// representation and are rejected; verification never prepares or publishes a target.
#[cfg(all(feature = "local", any(feature = "blake3", feature = "sha2")))]
impl Verify<DigestValue> for LocalMaterial {
    type Error = HashError;
    type Output = (LocalMaterial, DigestEvidence);

    fn verify(self, expected: DigestValue) -> Result<Self::Output, Self::Error> {
        let path = match &self {
            LocalMaterial::Directory { path } => {
                return Err(HashError::UnsupportedDigestMaterial(path.clone()));
            }
            LocalMaterial::File { path } => path.clone(),
            LocalMaterial::StagedFile { path } => path.to_path_buf(),
        };
        let evidence = verify_path(&path, &expected)?;
        Ok((self, evidence))
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

#[cfg(all(feature = "local", any(feature = "blake3", feature = "sha2")))]
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

/// One closed dispatch from the declared kind to the algorithm family (data-driven).
#[cfg(all(feature = "local", any(feature = "blake3", feature = "sha2")))]
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
    use crate::local::{LocalExpectation, LocalObservation, LocalReconciliation, LocalTarget};
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

    #[cfg(feature = "blake3")]
    #[test]
    fn blake3_verify_path_returns_digest_evidence() {
        let root = temp_root("verify-blake3");
        let source = root.join("source.txt");
        fs::create_dir_all(&root).unwrap();
        fs::write(&source, "pulith").unwrap();
        let digest = blake3::hash(b"pulith").to_hex().to_string();
        let expected = DigestValue::new(DigestAlgorithmKind::Blake3, digest.clone()).unwrap();

        let evidence = verify_path(&source, &expected).unwrap();

        assert_eq!(evidence.algorithm, DigestAlgorithmKind::Blake3);
        assert_eq!(evidence.expected, expected);
        assert_eq!(evidence.observed.as_str(), digest);
        assert_eq!(fs::read_to_string(&source).unwrap(), "pulith");

        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(feature = "blake3")]
    #[test]
    fn blake3_verify_path_rejects_digest_mismatch() {
        let root = temp_root("verify-blake3-mismatch");
        let source = root.join("source.txt");
        fs::create_dir_all(&root).unwrap();
        fs::write(&source, "pulith").unwrap();
        let expected = DigestValue::new(DigestAlgorithmKind::Blake3, "00".repeat(32)).unwrap();

        let error = verify_path(&source, &expected).unwrap_err();

        assert!(matches!(error, HashError::DigestMismatch { .. }));
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(feature = "sha2")]
    #[test]
    fn sha256_verify_path_returns_digest_evidence() {
        use sha2::Digest as _;

        let root = temp_root("verify-sha256");
        let source = root.join("source.txt");
        fs::create_dir_all(&root).unwrap();
        fs::write(&source, "pulith").unwrap();
        let mut hasher = sha2::Sha256::new();
        hasher.update(b"pulith");
        let digest = hex::encode(hasher.finalize());
        let expected = DigestValue::new(DigestAlgorithmKind::Sha256, digest.clone()).unwrap();

        let evidence = verify_path(&source, &expected).unwrap();

        assert_eq!(evidence.algorithm, DigestAlgorithmKind::Sha256);
        assert_eq!(evidence.expected, expected);
        assert_eq!(evidence.observed.as_str(), digest);

        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(feature = "sha2")]
    #[test]
    fn sha256_verify_path_rejects_digest_mismatch() {
        let root = temp_root("verify-sha256-mismatch");
        let source = root.join("source.txt");
        fs::create_dir_all(&root).unwrap();
        fs::write(&source, "pulith").unwrap();
        let expected = DigestValue::new(DigestAlgorithmKind::Sha256, "00".repeat(32)).unwrap();

        let error = verify_path(&source, &expected).unwrap_err();

        assert!(matches!(error, HashError::DigestMismatch { .. }));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn local_inspection_and_size_reconciliation_of_digest_material() {
        let root = temp_root("hash-local-observation");
        let file = root.join("source.txt");
        fs::create_dir_all(&root).unwrap();
        fs::write(&file, "pulith").unwrap();

        let (observation, evidence) = LocalTarget::new(file.as_path())
            .unwrap()
            .inspect(())
            .unwrap();
        assert_eq!(observation, LocalObservation::File { bytes: 6 });
        assert_eq!(evidence.path, file);

        let (reconciliation, reconcile_evidence) = observation
            .reconcile(LocalExpectation::FileSize(6))
            .unwrap();
        assert_eq!(reconciliation, LocalReconciliation::Matches);
        assert_eq!(reconcile_evidence.expected, LocalExpectation::FileSize(6));
        assert_eq!(
            reconcile_evidence.observed,
            LocalObservation::File { bytes: 6 }
        );
        assert_eq!(fs::read_to_string(&file).unwrap(), "pulith");

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn local_inspection_reports_missing_digest_material_without_mutation() {
        let root = temp_root("hash-local-missing");
        let missing = root.join("missing.txt");
        fs::create_dir_all(&root).unwrap();

        let (observation, evidence) = LocalTarget::new(missing.as_path())
            .unwrap()
            .inspect(())
            .unwrap();
        assert_eq!(observation, LocalObservation::Missing);
        assert_eq!(evidence.path, missing);
        assert!(!missing.exists());

        fs::remove_dir_all(root).unwrap();
    }
}
