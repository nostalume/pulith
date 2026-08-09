#![cfg(all(feature = "local", any(feature = "blake3", feature = "sha2")))]

use pulith::hash::{ArtifactDescriptor, ArtifactReconciliation, DigestAlgorithmKind, DigestValue};
use pulith::local::{
    LocalArtifactObservation, LocalEntryKind, LocalExpectation, LocalObservation, LocalSource,
    LocalTarget,
};
use pulith::{Acquire, Inspect, Reconcile, Verify};

#[cfg(feature = "blake3")]
fn blake3_digest(bytes: &[u8]) -> DigestValue {
    DigestValue::new(
        DigestAlgorithmKind::Blake3,
        blake3::hash(bytes).to_hex().to_string(),
    )
    .unwrap()
}

#[cfg(feature = "sha2")]
fn sha256_digest(bytes: &[u8]) -> DigestValue {
    use sha2::{Digest, Sha256};
    let value = hex::encode(Sha256::digest(bytes));
    DigestValue::new(DigestAlgorithmKind::Sha256, value).unwrap()
}

#[cfg(feature = "blake3")]
#[test]
fn blake3_verify_path_proves_the_declared_digest() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("artifact");
    std::fs::write(&path, b"pulith").unwrap();
    let expected = blake3_digest(b"pulith");

    let (_, evidence) = LocalSource::new(path.clone())
        .unwrap()
        .acquire()
        .unwrap()
        .verify(expected.clone())
        .unwrap();
    assert_eq!(evidence.algorithm, DigestAlgorithmKind::Blake3);
    assert_eq!(evidence.expected, expected);
    assert_eq!(
        evidence.observed.as_str(),
        blake3_digest(b"pulith").as_str()
    );
    assert_eq!(std::fs::read(&path).unwrap(), b"pulith");
}

#[cfg(feature = "blake3")]
#[test]
fn exact_inspection_and_reconciliation_use_the_digest_need() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("artifact");
    std::fs::write(&path, b"pulith").unwrap();
    let expected = ArtifactDescriptor::new(
        DigestAlgorithmKind::Blake3,
        blake3::hash(b"pulith").to_hex().to_string(),
        6,
    );

    let (observation, evidence) = LocalTarget::new(path)
        .unwrap()
        .inspect(DigestAlgorithmKind::Blake3)
        .unwrap();
    assert_eq!(evidence.algorithm, DigestAlgorithmKind::Blake3);
    let (reconciliation, receipt) = observation.reconcile(expected.clone()).unwrap();
    assert_eq!(reconciliation, ArtifactReconciliation::Matches);
    assert_eq!(receipt.expected, expected);
}

#[cfg(feature = "blake3")]
#[test]
fn exact_inspection_classifies_same_size_content_drift() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("artifact");
    std::fs::write(&path, b"PULITH").unwrap();

    let (observation, _) = LocalTarget::new(path)
        .unwrap()
        .inspect(DigestAlgorithmKind::Blake3)
        .unwrap();
    let (reconciliation, _) = observation
        .reconcile(ArtifactDescriptor::new(
            DigestAlgorithmKind::Blake3,
            blake3::hash(b"pulith").to_hex().to_string(),
            6,
        ))
        .unwrap();
    assert!(matches!(
        reconciliation,
        ArtifactReconciliation::DigestMismatch { .. }
    ));
}

#[cfg(feature = "blake3")]
#[test]
fn exact_inspection_reconciles_unequal_bytes_as_size_mismatch() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("artifact");
    std::fs::write(&path, b"longer").unwrap();

    let (observation, _) = LocalTarget::new(path)
        .unwrap()
        .inspect(DigestAlgorithmKind::Blake3)
        .unwrap();
    let (reconciliation, _) = observation
        .reconcile(ArtifactDescriptor::new(
            DigestAlgorithmKind::Blake3,
            blake3::hash(b"short").to_hex().to_string(),
            5,
        ))
        .unwrap();
    assert_eq!(
        reconciliation,
        ArtifactReconciliation::SizeMismatch {
            expected: 5,
            observed: 6,
        }
    );
}

#[cfg(feature = "blake3")]
#[test]
fn exact_inspection_reports_missing_and_directory() {
    let root = tempfile::tempdir().unwrap();
    let expected = ArtifactDescriptor::new(
        DigestAlgorithmKind::Blake3,
        blake3::hash(b"expected").to_hex().to_string(),
        8,
    );

    let (missing, _) = LocalTarget::new(root.path().join("missing"))
        .unwrap()
        .inspect(DigestAlgorithmKind::Blake3)
        .unwrap();
    assert_eq!(missing, LocalArtifactObservation::Missing);
    assert_eq!(
        missing.reconcile(expected.clone()).unwrap().0,
        ArtifactReconciliation::Missing
    );

    let (directory, _) = LocalTarget::new(root.path().to_path_buf())
        .unwrap()
        .inspect(DigestAlgorithmKind::Blake3)
        .unwrap();
    assert_eq!(
        directory.reconcile(expected).unwrap().0,
        ArtifactReconciliation::WrongKind {
            observed: LocalEntryKind::Directory,
        }
    );
}

#[cfg(feature = "blake3")]
#[test]
fn exact_inspection_does_not_follow_final_symlinks() {
    let root = tempfile::tempdir().unwrap();
    let target = root.path().join("target");
    let link = root.path().join("link");
    std::fs::write(&target, b"secret").unwrap();
    crate::common::file_symlink(&target, &link).unwrap();

    let (observation, _) = LocalTarget::new(link)
        .unwrap()
        .inspect(DigestAlgorithmKind::Blake3)
        .unwrap();
    assert_eq!(observation, LocalArtifactObservation::Symlink);
    assert_eq!(
        observation
            .reconcile(ArtifactDescriptor::new(
                DigestAlgorithmKind::Blake3,
                blake3::hash(b"secret").to_hex().to_string(),
                6,
            ))
            .unwrap()
            .0,
        ArtifactReconciliation::WrongKind {
            observed: LocalEntryKind::Symlink,
        }
    );
}

#[cfg(feature = "blake3")]
#[test]
fn blake3_verify_path_rejects_content_drift() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("artifact");
    std::fs::write(&path, b"PULITH").unwrap();

    let error = LocalSource::new(path)
        .unwrap()
        .acquire()
        .unwrap()
        .verify(blake3_digest(b"pulith"))
        .unwrap_err();
    assert!(matches!(
        error,
        pulith::hash::HashError::DigestMismatch { .. }
    ));
}

#[cfg(feature = "sha2")]
#[test]
fn sha256_verify_path_proves_the_declared_digest() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("artifact");
    std::fs::write(&path, b"pulith").unwrap();
    let expected = sha256_digest(b"pulith");

    let (_, evidence) = LocalSource::new(path)
        .unwrap()
        .acquire()
        .unwrap()
        .verify(expected)
        .unwrap();
    assert_eq!(evidence.algorithm, DigestAlgorithmKind::Sha256);
    assert_eq!(
        evidence.observed.as_str(),
        sha256_digest(b"pulith").as_str()
    );
}

#[cfg(feature = "sha2")]
#[test]
fn sha256_exact_inspection_uses_the_explicit_digest_need() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("artifact");
    std::fs::write(&path, b"pulith").unwrap();
    let expected = ArtifactDescriptor::new(
        DigestAlgorithmKind::Sha256,
        sha256_digest(b"pulith").as_str(),
        6,
    );

    let (observation, evidence) = LocalTarget::new(path)
        .unwrap()
        .inspect(DigestAlgorithmKind::Sha256)
        .unwrap();
    assert_eq!(evidence.algorithm, DigestAlgorithmKind::Sha256);
    assert_eq!(
        observation.reconcile(expected).unwrap().0,
        ArtifactReconciliation::Matches
    );
}

#[test]
fn local_inspection_and_size_reconciliation_of_digest_material() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("artifact");
    std::fs::write(&path, b"pulith").unwrap();

    let (observation, evidence) = LocalTarget::new(path.clone()).unwrap().inspect(()).unwrap();
    assert_eq!(observation, LocalObservation::File { bytes: 6 });
    assert_eq!(evidence.path, path);

    let (reconciliation, _) = observation
        .reconcile(LocalExpectation::FileSize(6))
        .unwrap();
    assert_eq!(reconciliation, pulith::local::LocalReconciliation::Matches);
    assert_eq!(std::fs::read(&path).unwrap(), b"pulith");
}

#[test]
fn local_inspection_reports_missing_and_directory_without_mutation() {
    let root = tempfile::tempdir().unwrap();
    let missing = root.path().join("missing");
    let (observation, _) = LocalTarget::new(missing.clone())
        .unwrap()
        .inspect(())
        .unwrap();
    assert_eq!(observation, LocalObservation::Missing);

    let (observation, _) = LocalTarget::new(root.path().to_path_buf())
        .unwrap()
        .inspect(())
        .unwrap();
    assert_eq!(observation, LocalObservation::Directory);
}

#[cfg(unix)]
#[test]
fn symlink_is_classified_by_resolved_target() {
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("file");
    let directory = root.path().join("directory");
    std::fs::write(&file, b"secret").unwrap();
    std::fs::create_dir(&directory).unwrap();
    crate::common::file_symlink(&file, root.path().join("file-link")).unwrap();
    crate::common::dir_symlink(&directory, root.path().join("dir-link")).unwrap();

    let (observation, _) = LocalTarget::new(root.path().join("file-link"))
        .unwrap()
        .inspect(())
        .unwrap();
    assert_eq!(observation, LocalObservation::SymlinkToFile);

    let (observation, _) = LocalTarget::new(root.path().join("dir-link"))
        .unwrap()
        .inspect(())
        .unwrap();
    assert_eq!(observation, LocalObservation::SymlinkToDirectory);
}
