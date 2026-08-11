//! Local materialization behaviors: the material-kind law split into independent steps.
//!
//! A byte material (`File`/`StagedFile`) or directory tree is prepared into guarded custody. Digest
//! verification is a separate behavior owned by `crate::hash`; callers compose either ordering
//! explicitly. Preparation is feature-gated on `local` + archive codecs.

use std::path::PathBuf;

use crate::archive::{ArchiveError, ArchiveEvidence, ArchiveKind, ArchivePolicy};
use crate::local::{LocalError, LocalMaterial, LocalSource, StagedTree};

#[derive(Clone, Debug, Eq, PartialEq)]
/// Evidence distinguishing a copied tree from a decoded archive tree.
pub enum PreparationEvidence {
    /// The copied outcome.
    Copied,
    /// The extracted outcome.
    Extracted(ArchiveEvidence),
}

/// Errors produced while verifying or preparing one local material.
#[derive(Debug)]
pub enum MaterializeError {
    /// The sniff outcome.
    Sniff(std::io::Error),
    /// The prepare outcome.
    Prepare(ArchiveError),
    /// The copy outcome.
    Copy(LocalError),
}

impl std::fmt::Display for MaterializeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Sniff(cause) => write!(f, "material format detection failed: {cause}"),
            Self::Prepare(cause) => write!(f, "material preparation failed: {cause}"),
            Self::Copy(cause) => write!(f, "material custody copy failed: {cause}"),
        }
    }
}

impl std::error::Error for MaterializeError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Sniff(cause) => Some(cause),
            Self::Prepare(cause) => Some(cause),
            Self::Copy(cause) => Some(cause),
        }
    }
}

impl LocalMaterial {
    /// Copies or decodes this local material into caller-selected staged-tree custody.
    pub fn prepare(
        self,
        stage: StagedTree,
        policy: ArchivePolicy,
    ) -> Result<(StagedTree, PreparationEvidence), MaterializeError> {
        match self {
            Self::Directory { path } => stage
                .copy_tree(
                    LocalSource::new(path).map_err(MaterializeError::Copy)?,
                    PathBuf::new(),
                )
                .map(|stage| (stage, PreparationEvidence::Copied))
                .map_err(MaterializeError::Copy),
            Self::File { path } => stage.prepare_file(path, policy),
            Self::StagedFile { path } => stage.prepare_file(path.to_path_buf(), policy),
        }
    }
}

impl StagedTree {
    fn prepare_file(
        self,
        path: PathBuf,
        policy: ArchivePolicy,
    ) -> Result<(Self, PreparationEvidence), MaterializeError> {
        match ArchiveKind::sniff(&path).map_err(MaterializeError::Sniff)? {
            Some(kind) => kind
                .prepare(&path, self.root(), policy)
                .map(|evidence| (self, PreparationEvidence::Extracted(evidence)))
                .map_err(MaterializeError::Prepare),
            None => self
                .copy_file(
                    LocalSource::new(path).map_err(MaterializeError::Copy)?,
                    PathBuf::new(),
                )
                .map(|stage| (stage, PreparationEvidence::Copied))
                .map_err(MaterializeError::Copy),
        }
    }
}

#[cfg(all(test, feature = "zip", feature = "blake3"))]
mod tests {
    use std::fs;
    use std::io::Write;
    use std::path::{Path, PathBuf};

    use super::*;
    use crate::archive::ArchiveKind;
    use crate::hash::{DigestAlgorithmKind, DigestValue, HashError};
    use crate::local::LocalSource;
    use crate::local::LocalTarget;
    use crate::{Acquire, Verify};

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

    fn acquire(path: &Path) -> LocalMaterial {
        // The landed publication law never creates parents; the caller owns the layout.
        LocalSource::new(path.to_path_buf())
            .unwrap()
            .acquire()
            .unwrap()
    }

    fn write_zip(path: &Path, entries: &[(&str, &[u8])]) {
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

        // A directory tree has no byte digest: verification is refused, so the caller skips it.
        let error = acquire(&source)
            .verify(
                DigestValue::new(DigestAlgorithmKind::Blake3, blake3_hex(b"never-verified"))
                    .unwrap(),
            )
            .unwrap_err();
        assert!(matches!(error, HashError::UnsupportedDigestMaterial(path) if path == source));

        let admitted = LocalTarget::new(target.clone()).unwrap();
        let stage = admitted.stage().unwrap();
        let (prepared, evidence) = acquire(&source)
            .prepare(stage, ArchivePolicy::default())
            .unwrap();
        assert_eq!(evidence, PreparationEvidence::Copied);
        prepared.publish(admitted).unwrap();

        assert_eq!(fs::read(target.join("bin/tool")).unwrap(), b"dir payload");
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn materialize_verifies_and_copies_a_plain_byte_material() {
        let root = temp_root("plain-copy");
        let source = root.join("plain.bin");
        fs::write(&source, b"plain payload").unwrap();
        let target = root.join("target");

        let material = acquire(&source);
        let (material, digest_evidence) = material
            .verify(
                DigestValue::new(DigestAlgorithmKind::Blake3, blake3_hex(b"plain payload"))
                    .unwrap(),
            )
            .unwrap();
        assert_eq!(digest_evidence.algorithm, DigestAlgorithmKind::Blake3);
        assert_eq!(
            digest_evidence.observed.as_str(),
            blake3_hex(b"plain payload")
        );

        let admitted = LocalTarget::new(target.clone()).unwrap();
        let stage = admitted.stage().unwrap();
        let (prepared, evidence) = material.prepare(stage, ArchivePolicy::default()).unwrap();
        assert_eq!(evidence, PreparationEvidence::Copied);
        prepared.publish(admitted).unwrap();
        assert_eq!(fs::read(&target).unwrap(), b"plain payload");
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn materialize_rejects_a_verify_mismatch_without_publishing() {
        let root = temp_root("verify-mismatch");
        let source = root.join("plain.bin");
        fs::write(&source, b"plain payload").unwrap();
        let target = root.join("target");

        let error = acquire(&source)
            .verify(
                DigestValue::new(
                    DigestAlgorithmKind::Blake3,
                    blake3_hex(b"different payload"),
                )
                .unwrap(),
            )
            .unwrap_err();

        assert!(matches!(error, HashError::DigestMismatch { .. }));
        assert!(!target.exists());
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn materialize_prepares_a_zip_archive_and_attests_it() {
        let root = temp_root("zip-prepare");
        let source = root.join("tool.zip");
        write_zip(&source, &[("bin/tool", b"zip payload")]);
        let target = root.join("target");

        let material = acquire(&source);
        let (material, digest_evidence) = material
            .verify(
                DigestValue::new(
                    DigestAlgorithmKind::Blake3,
                    blake3_hex(&fs::read(&source).unwrap()),
                )
                .unwrap(),
            )
            .unwrap();
        assert_eq!(digest_evidence.algorithm, DigestAlgorithmKind::Blake3);

        let admitted = LocalTarget::new(target.clone()).unwrap();
        let stage = admitted.stage().unwrap();
        let (prepared, evidence) = material.prepare(stage, ArchivePolicy::default()).unwrap();
        let PreparationEvidence::Extracted(archive_evidence) = evidence else {
            panic!("expected extraction evidence")
        };
        assert_eq!(archive_evidence.format, ArchiveKind::Zip);
        assert_eq!(archive_evidence.files, 1);
        prepared.publish(admitted).unwrap();
        assert_eq!(fs::read(target.join("bin/tool")).unwrap(), b"zip payload");
        fs::remove_dir_all(&root).unwrap();
    }
}
