//! Byte snapshots and verified local publication.

use std::io::{self, Seek};
use std::path::PathBuf;

use crate::local::{FileSnapshot, LocalError, StagedTree};

#[cfg(any(feature = "blake3", feature = "sha2"))]
use super::apply::{FileProtection, publish_single_file};
#[cfg(any(feature = "blake3", feature = "sha2"))]
use crate::hash::{ArtifactDescriptor, DigestValue, HashError, copy_verified};
#[cfg(any(feature = "blake3", feature = "sha2"))]
use crate::local::{FileAdmission, FilePublication, LocalPath, OpenedFile};

#[cfg(any(feature = "blake3", feature = "sha2"))]
impl OpenedFile {
    /// Captures and verifies the complete stream in one pass. The snapshot owns the verified bytes.
    pub fn snapshot_verified(
        mut self,
        expected: &DigestValue,
    ) -> Result<(FileSnapshot, ArtifactDescriptor), HashError> {
        self.file
            .rewind()
            .map_err(|error| HashError::io("rewind opened file", &self.path, error))?;
        let mut file = tempfile::tempfile()
            .map_err(|error| HashError::io("create byte snapshot", &self.path, error))?;
        let descriptor =
            copy_verified(&mut self.file, &self.path, &mut file, &self.path, expected)?;
        file.rewind()
            .map_err(|error| HashError::io("rewind byte snapshot", &self.path, error))?;
        Ok((
            FileSnapshot {
                file,
                source: self.path,
            },
            descriptor,
        ))
    }
}

#[cfg(any(feature = "blake3", feature = "sha2"))]
impl LocalPath {
    /// Publishes at an absent target after verifying the complete stream read from the source handle.
    ///
    /// The file stage remains private through verification and is discarded on failure.
    pub fn publish_verified_file(
        &self,
        mut source: OpenedFile,
        expected: &DigestValue,
    ) -> Result<(FilePublication, ArtifactDescriptor), HashError> {
        source
            .file
            .rewind()
            .map_err(|error| HashError::io("rewind opened file", &source.path, error))?;
        publish_single_file(
            self.as_path(),
            FileAdmission::Absent,
            FileProtection::Ordinary,
            |error| HashError::local_artifact(self.as_path(), error),
            |output, destination| {
                let descriptor = copy_verified(
                    &mut source.file,
                    &source.path,
                    output,
                    destination,
                    expected,
                )?;
                let permissions = source
                    .metadata()
                    .map_err(|error| HashError::io("read source permissions", &source.path, error))?
                    .permissions();
                output.set_permissions(permissions).map_err(|error| {
                    HashError::io("set staged file permissions", destination, error)
                })?;
                Ok((descriptor.size, descriptor))
            },
        )
    }
}

impl FileSnapshot {
    /// Copies the complete owned byte file into a destination stage.
    pub fn stage_into(mut self, stage: StagedTree) -> Result<StagedTree, LocalError> {
        self.file
            .rewind()
            .map_err(|error| LocalError::io("rewind file snapshot", &self.source, error))?;
        let (mut output, destination) = stage.create_file(PathBuf::new())?;
        io::copy(&mut self.file, &mut output)
            .map_err(|error| LocalError::io("copy file snapshot into stage", destination, error))?;
        Ok(stage)
    }
}

#[cfg(all(test, feature = "blake3"))]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};

    use super::*;
    use crate::hash::{DigestAlgorithmKind, DigestValue, HashError};
    use crate::local::{CommitOutcome, TreeAdmission};

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

    fn open(path: &Path) -> OpenedFile {
        LocalPath::new(path).unwrap().open_file().unwrap().unwrap()
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

        let admitted = LocalPath::new(target.clone()).unwrap();
        let stage = admitted.stage().unwrap();
        let prepared = stage
            .copy_tree(LocalPath::new(&source).unwrap(), PathBuf::new())
            .unwrap();
        prepared.publish(admitted, TreeAdmission::Absent).unwrap();

        assert_eq!(fs::read(target.join("bin/tool")).unwrap(), b"dir payload");
        fs::remove_dir_all(&root).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn occupied_target_releases_the_verified_read_only_stage() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("source");
        let target = root.path().join("target");
        fs::write(&source, b"verified bytes").unwrap();
        fs::write(&target, b"predecessor").unwrap();
        let mut permissions = fs::metadata(&source).unwrap().permissions();
        permissions.set_readonly(true);
        fs::set_permissions(&source, permissions).unwrap();
        let expected = open(&source)
            .digest(DigestAlgorithmKind::Blake3)
            .unwrap()
            .digest;

        let result = LocalPath::new(&target)
            .unwrap()
            .publish_verified_file(open(&source), &expected);
        assert!(matches!(
            result,
            Err(HashError::LocalArtifact { source, .. }) if matches!(*source, LocalError::TargetExists(_))
        ));
        assert_eq!(fs::read(target).unwrap(), b"predecessor");
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 2);
    }

    #[test]
    fn materialize_verifies_and_copies_a_plain_byte_material() {
        let root = temp_root("plain-copy");
        let source = root.join("plain.bin");
        fs::write(&source, b"plain payload").unwrap();
        let target = root.join("target");

        let material = open(&source);
        let (publication, digest_evidence) = LocalPath::new(&target)
            .unwrap()
            .publish_verified_file(
                material,
                &DigestValue::new(DigestAlgorithmKind::Blake3, blake3_hex(b"plain payload"))
                    .unwrap(),
            )
            .unwrap();
        assert_eq!(
            digest_evidence.digest.as_str(),
            blake3_hex(b"plain payload")
        );
        assert_eq!(digest_evidence.size, 13);
        assert_eq!(publication.outcome, CommitOutcome::Created);
        assert_eq!(publication.bytes, 13);
        assert_eq!(fs::read(&target).unwrap(), b"plain payload");
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn materialize_rejects_a_verify_mismatch_without_publishing() {
        let root = temp_root("verify-mismatch");
        let source = root.join("plain.bin");
        fs::write(&source, b"plain payload").unwrap();
        let target = root.join("target");

        let error = LocalPath::new(&target)
            .unwrap()
            .publish_verified_file(
                open(&source),
                &DigestValue::new(
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
