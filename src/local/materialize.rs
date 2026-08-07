//! Local materialization behavior: owns the material-kind law.
//!
//! A local material is either a byte stream (`File`/`StagedFile` — verify its declared digest,
//! sniff for an archive format, prepare or copy) or a directory tree (no byte digest exists —
//! copy as-is, no verification). The caller never reads the material to route the flow; this
//! adapter owns the decision. Archive extraction uses a caller-owned exclusive scratch path
//! (the publication law). Feature-gated on `local` + `hash` + archive codecs.

use std::path::Path;

use crate::archive::{ArchiveError, ArchiveEvidence, ArchivePolicy, prepare, sniff_format};
use crate::hash::{DigestEvidence, DigestValue, HashError, HashVerify};
use crate::local::{ApplyEvidence, LocalApply, LocalError, LocalMaterial, PathBuf};
use crate::{Acquired, Applied, EvidenceChain, Materialize, Verify};

/// What materialization did, attested as data.
///
/// The upstream acquisition evidence is preserved as the chain's previous record. `verified`
/// carries the digest attestation when a byte material was verified (`None` for a copied
/// directory tree, which has no byte digest); `prepared` carries the archive extraction
/// attestation when an archive was detected; `applied` attests the publication.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MaterializeEvidence {
    pub verified: Option<DigestEvidence>,
    pub prepared: Option<ArchiveEvidence>,
    pub applied: ApplyEvidence,
}

/// An applied materialization: the published tree with the materialize evidence record.
pub type Materialized<I, S, E> =
    Applied<Materialize<I, S, PathBuf>, EvidenceChain<E, MaterializeEvidence>>;

/// The materialization behavior: owns the material-kind law end to end.
#[derive(Clone, Copy, Debug, Default)]
pub struct LocalMaterialize;

impl<I, S, E> crate::local::LocalAcquired<I, S, E> {
    /// The materialization behavior as a node method: verify, prepare (or copy), and publish
    /// this acquired material. The caller composes the flow without naming the typestate.
    pub fn materialize(
        self,
        digest: DigestValue,
        workspace: &Path,
        policy: ArchivePolicy,
    ) -> Result<Materialized<I, S, E>, MaterializeError>
    where
        E: Clone,
    {
        LocalMaterialize.materialize(self, digest, workspace, policy)
    }
}

impl LocalMaterialize {
    /// Materialize an acquired local material into its target.
    ///
    /// Directory material is copied as-is (no byte digest exists, no verification). Byte
    /// material is verified against the declared `DigestValue` (algorithm + digest), sniffed
    /// for an archive format, prepared (or copied when not an archive), and published. The
    /// workspace is used only for archive extraction and must be caller-owned exclusive scratch
    /// (the publication law).
    pub fn materialize<I, S, E>(
        &self,
        acquired: crate::local::LocalAcquired<I, S, E>,
        digest: DigestValue,
        workspace: &Path,
        policy: ArchivePolicy,
    ) -> Result<Materialized<I, S, E>, MaterializeError>
    where
        E: Clone,
    {
        let (input, material, evidence) = (acquired.input, acquired.material, acquired.evidence);
        let byte_path = match &material {
            LocalMaterial::Directory { .. } => None,
            LocalMaterial::File { path } => Some(path.clone()),
            LocalMaterial::StagedFile { path } => Some(path.to_path_buf()),
        };
        let acquire_node = Acquired {
            input,
            material,
            evidence,
        };
        match byte_path {
            None => {
                let applied = LocalApply
                    .apply(acquire_node)
                    .map_err(MaterializeError::Apply)?;
                Ok(rebuild::<I, S, E>(applied, None, None))
            }
            Some(path) => {
                let verified = HashVerify::new(digest.algorithm())
                    .verify(acquire_node, digest)
                    .map_err(MaterializeError::Verify)?;
                let acquire_evidence = verified.evidence.previous.clone();
                let digest_evidence = verified.evidence.current.clone();
                let (input, prepared, apply) =
                    match sniff_format(&path).map_err(MaterializeError::Sniff)? {
                        Some(archive_kind) => {
                            let prepared = prepare(verified, workspace, policy, archive_kind)
                                .map_err(MaterializeError::Prepare)?;
                            let applied = LocalApply
                                .apply(prepared)
                                .map_err(MaterializeError::Apply)?;
                            let prepared_evidence = applied.evidence.previous.current;
                            (
                                applied.input,
                                Some(prepared_evidence),
                                applied.evidence.current,
                            )
                        }
                        None => {
                            let applied = LocalApply
                                .apply(verified)
                                .map_err(MaterializeError::Apply)?;
                            (applied.input, None, applied.evidence.current)
                        }
                    };
                Ok(Applied {
                    input,
                    evidence: EvidenceChain {
                        previous: acquire_evidence,
                        current: MaterializeEvidence {
                            verified: Some(digest_evidence),
                            prepared,
                            applied: apply,
                        },
                    },
                })
            }
        }
    }
}

/// Rebuild an applied receipt into the materialize evidence shape: the upstream evidence stays
/// as the previous record; the digest/preparation attestations (if any) and the publication
/// evidence move into one `MaterializeEvidence` record.
fn rebuild<I, S, U>(
    applied: Applied<Materialize<I, S, PathBuf>, EvidenceChain<U, ApplyEvidence>>,
    verified: Option<DigestEvidence>,
    prepared: Option<ArchiveEvidence>,
) -> Applied<Materialize<I, S, PathBuf>, EvidenceChain<U, MaterializeEvidence>> {
    let EvidenceChain {
        previous,
        current: apply,
    } = applied.evidence;
    Applied {
        input: applied.input,
        evidence: EvidenceChain {
            previous,
            current: MaterializeEvidence {
                verified,
                prepared,
                applied: apply,
            },
        },
    }
}

/// Named, actionable failure of materialization.
#[derive(Debug)]
pub enum MaterializeError {
    Sniff(std::io::Error),
    Verify(HashError),
    Prepare(ArchiveError),
    Apply(LocalError),
}

impl std::fmt::Display for MaterializeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Sniff(cause) => write!(f, "material format detection failed: {cause}"),
            Self::Verify(cause) => write!(f, "material verification failed: {cause}"),
            Self::Prepare(cause) => write!(f, "material preparation failed: {cause}"),
            Self::Apply(cause) => write!(f, "material publication failed: {cause}"),
        }
    }
}

impl std::error::Error for MaterializeError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Sniff(cause) => Some(cause),
            Self::Verify(cause) => Some(cause),
            Self::Prepare(cause) => Some(cause),
            Self::Apply(cause) => Some(cause),
        }
    }
}

#[cfg(all(test, feature = "zip", feature = "blake3"))]
mod tests {
    use crate::hash::DigestAlgorithmKind;
    use std::fs;
    use std::io::Write;
    use std::path::PathBuf;

    use super::*;
    use crate::local::LocalAcquire;
    use crate::{Materialize, MaterializeMode};

    fn temp_root(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "pulith-materialize-{name}-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        root
    }

    fn acquire(
        path: &std::path::Path,
        target: &std::path::Path,
    ) -> Acquired<
        Materialize<String, PathBuf, PathBuf>,
        LocalMaterial,
        crate::local::LocalAcquireEvidence,
    > {
        // The landed publication law never creates parents; the caller owns the layout.
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        LocalAcquire
            .acquire(Materialize::new(
                "tool".to_string(),
                path.to_path_buf(),
                target.to_path_buf(),
                MaterializeMode::CreateNew,
            ))
            .unwrap()
    }

    fn write_zip(path: &std::path::Path, entries: &[(&str, &[u8])]) {
        let file = fs::File::create(path).unwrap();
        let mut writer = zip::ZipWriter::new(file);
        for (name, content) in entries {
            writer
                .start_file(name, zip::write::SimpleFileOptions::default())
                .unwrap();
            writer.write_all(content).unwrap();
        }
        writer.finish().unwrap();
    }

    fn blake3_hex(bytes: &[u8]) -> String {
        blake3::hash(bytes).to_hex().to_string()
    }

    #[test]
    fn materialize_copies_a_directory_tree_without_byte_verification() {
        let root = temp_root("directory-copy");
        let source = root.join("tree");
        fs::create_dir_all(source.join("bin")).unwrap();
        fs::write(source.join("bin/tool"), b"dir payload").unwrap();
        let target = root.join("target");
        let acquired = acquire(&source, &target);

        let applied = LocalMaterialize
            .materialize(
                acquired,
                DigestValue::new(DigestAlgorithmKind::Blake3, blake3_hex(b"never-verified"))
                    .unwrap(),
                &root.join("scratch"),
                ArchivePolicy::default(),
            )
            .unwrap();

        assert_eq!(fs::read(target.join("bin/tool")).unwrap(), b"dir payload");
        assert_eq!(applied.evidence.current.verified, None);
        assert_eq!(applied.evidence.current.prepared, None);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn materialize_verifies_and_copies_a_plain_byte_material() {
        let root = temp_root("plain-copy");
        let source = root.join("plain.bin");
        fs::write(&source, b"plain payload").unwrap();
        let target = root.join("target");
        let acquired = acquire(&source, &target);

        let applied = LocalMaterialize
            .materialize(
                acquired,
                DigestValue::new(DigestAlgorithmKind::Blake3, blake3_hex(b"plain payload"))
                    .unwrap(),
                &root.join("scratch"),
                ArchivePolicy::default(),
            )
            .unwrap();

        assert_eq!(fs::read(&target).unwrap(), b"plain payload");
        let evidence = applied.evidence.current;
        assert_eq!(
            evidence.verified.as_ref().unwrap().algorithm,
            DigestAlgorithmKind::Blake3
        );
        assert_eq!(
            evidence.verified.unwrap().observed.as_str(),
            blake3_hex(b"plain payload")
        );
        assert_eq!(evidence.prepared, None);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn materialize_rejects_a_verify_mismatch_without_publishing() {
        let root = temp_root("verify-mismatch");
        let source = root.join("plain.bin");
        fs::write(&source, b"plain payload").unwrap();
        let target = root.join("target");
        let acquired = acquire(&source, &target);

        let error = LocalMaterialize
            .materialize(
                acquired,
                DigestValue::new(
                    DigestAlgorithmKind::Blake3,
                    blake3_hex(b"different payload"),
                )
                .unwrap(),
                &root.join("scratch"),
                ArchivePolicy::default(),
            )
            .unwrap_err();

        assert!(matches!(error, MaterializeError::Verify(_)));
        assert!(!target.exists());
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn materialize_prepares_a_zip_archive_and_attests_it() {
        let root = temp_root("zip-prepare");
        let source = root.join("tool.zip");
        write_zip(&source, &[("bin/tool", b"zip payload")]);
        let target = root.join("target");
        let acquired = acquire(&source, &target);

        let applied = LocalMaterialize
            .materialize(
                acquired,
                DigestValue::new(
                    DigestAlgorithmKind::Blake3,
                    blake3_hex(&fs::read(&source).unwrap()),
                )
                .unwrap(),
                &root.join("scratch"),
                ArchivePolicy::default(),
            )
            .unwrap();

        assert_eq!(fs::read(target.join("bin/tool")).unwrap(), b"zip payload");
        let evidence = applied.evidence.current;
        assert_eq!(
            evidence.verified.as_ref().unwrap().algorithm,
            DigestAlgorithmKind::Blake3
        );
        assert!(evidence.prepared.is_some());
        fs::remove_dir_all(&root).unwrap();
    }
}
