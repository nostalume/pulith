//! Typed digest verification and exact byte descriptors.
//!
//! Owns artifact-identity semantics: a caller-supplied digest or exact descriptor (`blake3` or
//! `sha2`) compared against observed bytes. Verification is factual and never applies, adopts, or
//! authorizes; exact inspection/reconciliation is opt-in under `local + blake3`/`local + sha2`.
//! No provenance or authenticity claim is made from a matching digest.
#[cfg(all(feature = "local", any(feature = "blake3", feature = "sha2")))]
use crate::local::{FileSnapshot, OpenedFile};
#[cfg(all(feature = "local", any(feature = "blake3", feature = "sha2")))]
use std::fs::File;
#[cfg(all(feature = "local", any(feature = "blake3", feature = "sha2")))]
use std::io;
#[cfg(all(feature = "local", any(feature = "blake3", feature = "sha2")))]
use std::io::{Read, Seek, Write};
#[cfg(all(feature = "local", any(feature = "blake3", feature = "sha2")))]
use std::path::{Path, PathBuf};

/// Errors produced by hash verification and exact local-artifact inspection.
#[cfg(all(feature = "local", any(feature = "blake3", feature = "sha2")))]
#[non_exhaustive]
#[derive(Debug)]
pub enum HashError {
    /// The digest mismatch outcome.
    DigestMismatch {
        /// The expected value.
        expected: String,
        /// The observed value.
        observed: String,
    },
    /// A local operation required by a digest operation failed.
    LocalArtifact {
        /// The path value.
        path: PathBuf,
        /// The source value.
        source: Box<crate::local::LocalError>,
    },
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

#[cfg(all(feature = "local", any(feature = "blake3", feature = "sha2")))]
impl HashError {
    pub(crate) fn local_artifact(path: &Path, source: crate::local::LocalError) -> Self {
        Self::LocalArtifact {
            path: path.to_path_buf(),
            source: Box::new(source),
        }
    }

    pub(crate) fn io(action: &'static str, path: impl AsRef<Path>, source: io::Error) -> Self {
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
            Self::LocalArtifact { path, source } => write!(
                f,
                "digest operation failed for {}: {source}",
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
/// The declared digest algorithm, as data.
///
/// `DigestAlgorithmKind` is the canonical data representation of a digest algorithm: manifests
/// declare the algorithm as this value, and data-driven entries (e.g. materialization) dispatch on
/// it once in core. The value is declared, never inferred from bytes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DigestAlgorithmKind {
    /// The blake3 outcome.
    Blake3,
    /// The sha256 outcome.
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
    /// Describes the SHA-256 digest of bytes already held by the caller.
    #[cfg(feature = "sha2")]
    pub fn sha256(bytes: &[u8]) -> Self {
        use sha2::{Digest, Sha256};

        Self {
            algorithm: DigestAlgorithmKind::Sha256,
            value: hex::encode(Sha256::digest(bytes)),
        }
    }

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

    /// Returns the algorithm carried by this digest value.
    pub fn algorithm(&self) -> DigestAlgorithmKind {
        self.algorithm
    }

    /// Returns the normalized lowercase hexadecimal digest.
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

/// Source-independent identity for one exact raw artifact representation.
///
/// The digest proves byte equality with the supplied expectation; it does not authenticate the
/// expectation's publisher or provenance.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ArtifactDescriptor {
    /// The digest value.
    pub digest: DigestValue,
    /// The size value.
    pub size: u64,
}

impl ArtifactDescriptor {
    /// Constructs an exact artifact descriptor from digest algorithm, value, and byte size.
    ///
    /// Invalid digest text is returned as an error instead of panicking.
    pub fn new(
        algorithm: DigestAlgorithmKind,
        digest: impl Into<String>,
        size: u64,
    ) -> Result<Self, String> {
        Ok(Self {
            digest: DigestValue::new(algorithm, digest)?,
            size,
        })
    }

    #[cfg(all(feature = "local", any(feature = "blake3", feature = "sha2")))]
    fn from_digest(digest: DigestValue, size: u64) -> Self {
        Self { digest, size }
    }
}

/// Exact artifact classification against a caller-supplied descriptor.
#[cfg(all(feature = "local", any(feature = "blake3", feature = "sha2")))]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ArtifactReconciliation {
    /// The matches outcome.
    Matches,
    /// The size mismatch outcome.
    SizeMismatch {
        /// The expected value.
        expected: u64,
        /// The observed value.
        observed: u64,
    },
    /// The digest mismatch outcome.
    DigestMismatch {
        /// The expected value.
        expected: DigestValue,
        /// The observed value.
        observed: DigestValue,
    },
}

#[cfg(all(feature = "local", any(feature = "blake3", feature = "sha2")))]
impl OpenedFile {
    /// Reads the complete file through this handle and describes the stream read.
    pub fn digest(
        mut self,
        algorithm: DigestAlgorithmKind,
    ) -> Result<ArtifactDescriptor, HashError> {
        self.file
            .rewind()
            .map_err(|error| HashError::io("rewind opened file", &self.path, error))?;
        describe(algorithm, &mut self.file, &self.path, |_| Ok(()))
    }
}

#[cfg(any(feature = "blake3", feature = "sha2"))]
impl ArtifactDescriptor {
    /// Compares the size and digest with a caller-supplied descriptor.
    pub fn compare(&self, expected: &Self) -> ArtifactReconciliation {
        if self.size != expected.size {
            ArtifactReconciliation::SizeMismatch {
                expected: expected.size,
                observed: self.size,
            }
        } else if self.digest != expected.digest {
            ArtifactReconciliation::DigestMismatch {
                expected: expected.digest.clone(),
                observed: self.digest.clone(),
            }
        } else {
            ArtifactReconciliation::Matches
        }
    }
}

#[cfg(all(feature = "local", any(feature = "blake3", feature = "sha2")))]
pub(crate) fn copy_verified(
    reader: &mut impl Read,
    path: &Path,
    writer: &mut File,
    destination: &Path,
    expected: &DigestValue,
) -> Result<ArtifactDescriptor, HashError> {
    verify_stream(reader, path, expected, |bytes| {
        writer
            .write_all(bytes)
            .map_err(|error| HashError::io("write verified stage", destination, error))
    })
}

#[cfg(all(feature = "local", any(feature = "blake3", feature = "sha2")))]
fn verify_stream(
    reader: &mut impl Read,
    path: &Path,
    expected: &DigestValue,
    consume: impl FnMut(&[u8]) -> Result<(), HashError>,
) -> Result<ArtifactDescriptor, HashError> {
    let observed = describe(expected.algorithm(), reader, path, consume)?;
    if &observed.digest != expected {
        return Err(HashError::DigestMismatch {
            expected: expected.as_str().to_string(),
            observed: observed.digest.as_str().to_string(),
        });
    }
    Ok(observed)
}

#[cfg(all(feature = "local", any(feature = "blake3", feature = "sha2")))]
fn describe(
    algorithm: DigestAlgorithmKind,
    reader: &mut impl Read,
    path: &Path,
    consume: impl FnMut(&[u8]) -> Result<(), HashError>,
) -> Result<ArtifactDescriptor, HashError> {
    let (digest, size) = digest_stream(algorithm, reader, path, consume)?;
    let digest = DigestValue::new(algorithm, digest).expect("observed digest is 64 hex");
    Ok(ArtifactDescriptor::from_digest(digest, size))
}

#[cfg(all(feature = "local", any(feature = "blake3", feature = "sha2")))]
impl FileSnapshot {
    /// Verifies the complete snapshot and returns it positioned for consumption.
    pub fn verify_digest(
        mut self,
        expected: &DigestValue,
    ) -> Result<(Self, ArtifactDescriptor), HashError> {
        let rewind_error = |error| HashError::io("rewind file snapshot", &self.source, error);
        self.file.rewind().map_err(rewind_error)?;
        let evidence = verify_stream(&mut self.file, &self.source, expected, |_| Ok(()))?;
        self.file.rewind().map_err(rewind_error)?;
        Ok((self, evidence))
    }
}

#[cfg(all(feature = "local", feature = "blake3"))]
fn digest_blake3(
    reader: &mut impl Read,
    path: &Path,
    mut consume: impl FnMut(&[u8]) -> Result<(), HashError>,
) -> Result<(String, u64), HashError> {
    let mut hasher = blake3::Hasher::new();
    let bytes = copy_into_hasher(path, reader, |bytes| {
        consume(bytes)?;
        hasher.update(bytes);
        Ok(())
    })?;
    Ok((hasher.finalize().to_hex().to_string(), bytes))
}

#[cfg(all(feature = "local", feature = "sha2"))]
fn digest_sha256(
    reader: &mut impl Read,
    path: &Path,
    mut consume: impl FnMut(&[u8]) -> Result<(), HashError>,
) -> Result<(String, u64), HashError> {
    use sha2::{Digest, Sha256 as Sha256Hasher};
    let mut hasher = Sha256Hasher::new();
    let bytes = copy_into_hasher(path, reader, |bytes| {
        consume(bytes)?;
        hasher.update(bytes);
        Ok(())
    })?;
    Ok((hex::encode(hasher.finalize()), bytes))
}

/// One closed dispatch from the declared kind to the digest implementation.
#[cfg(all(feature = "local", any(feature = "blake3", feature = "sha2")))]
fn digest_stream(
    kind: DigestAlgorithmKind,
    reader: &mut impl Read,
    path: &Path,
    consume: impl FnMut(&[u8]) -> Result<(), HashError>,
) -> Result<(String, u64), HashError> {
    match kind {
        #[cfg(feature = "blake3")]
        DigestAlgorithmKind::Blake3 => digest_blake3(reader, path, consume),
        #[cfg(feature = "sha2")]
        DigestAlgorithmKind::Sha256 => digest_sha256(reader, path, consume),
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
    mut update: impl FnMut(&[u8]) -> Result<(), HashError>,
) -> Result<u64, HashError> {
    let mut buffer = [0; 16 * 1024];
    let mut observed = 0_u64;
    loop {
        match reader.read(&mut buffer) {
            Ok(0) => return Ok(observed),
            Ok(n) => {
                update(&buffer[..n])?;
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

    use crate::local::{LocalExpectation, LocalObservation, LocalPath, LocalReconciliation};

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
    fn local_inspection_and_size_reconciliation_of_digest_material() {
        let root = temp_root("hash-local-observation");
        let file = root.join("source.txt");
        fs::create_dir_all(&root).unwrap();
        fs::write(&file, "pulith").unwrap();

        let observation = LocalPath::new(file.as_path())
            .unwrap()
            .inspect_entry()
            .unwrap();
        assert_eq!(observation, LocalObservation::File { bytes: 6 });

        let reconciliation = observation.compare(&LocalExpectation::FileSize(6));
        assert_eq!(reconciliation, LocalReconciliation::Matches);
        assert_eq!(fs::read_to_string(&file).unwrap(), "pulith");

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn local_inspection_reports_missing_digest_material_without_mutation() {
        let root = temp_root("hash-local-missing");
        let missing = root.join("missing.txt");
        fs::create_dir_all(&root).unwrap();

        let observation = LocalPath::new(missing.as_path())
            .unwrap()
            .inspect_entry()
            .unwrap();
        assert_eq!(observation, LocalObservation::Missing);
        assert!(!missing.exists());

        fs::remove_dir_all(root).unwrap();
    }
}
