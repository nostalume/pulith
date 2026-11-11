//! Local materialization: the material-kind law, from material to guarded custody.
//!
//! A byte material (`File`/`StagedFile`) or a directory tree is copied into private custody.
//! Digest verification is a separate behavior owned by `crate::hash`; archive extraction is a
//! separate behavior owned by `crate::archive`, which takes a decoder stream the caller supplies.
//! Neither is guessed here: the caller composes the ordering and the encoding it needs.

use std::path::PathBuf;

use crate::local::{LocalError, LocalMaterial, LocalSource, StagedTree};

impl LocalMaterial {
    /// Copies this local material into caller-selected staged-tree custody.
    pub fn prepare(self, stage: StagedTree) -> Result<StagedTree, LocalError> {
        match self {
            Self::Directory { path } => stage.copy_tree(LocalSource::new(path)?, PathBuf::new()),
            Self::File { path } => stage.copy_file(LocalSource::new(path)?, PathBuf::new()),
            Self::StagedFile { path } => {
                stage.copy_file(LocalSource::new(path.to_path_buf())?, PathBuf::new())
            }
        }
    }
}

#[cfg(all(test, feature = "blake3"))]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};

    use super::*;
    use crate::hash::{DigestAlgorithmKind, DigestValue, HashError};
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
        let prepared = acquire(&source).prepare(stage).unwrap();
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
        let prepared = material.prepare(stage).unwrap();
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
}
