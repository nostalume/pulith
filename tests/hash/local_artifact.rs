#![cfg(all(feature = "local", any(feature = "blake3", feature = "sha2")))]

use pulith::hash::{ArtifactDescriptor, ArtifactReconciliation, DigestAlgorithmKind, DigestValue};
#[cfg(feature = "blake3")]
use pulith::local::{LocalEntryKind, LocalError};
use pulith::local::{LocalExpectation, LocalObservation, LocalPath};

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
fn blake3_snapshot_verified_proves_the_declared_digest() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("artifact");
    std::fs::write(&path, b"pulith").unwrap();
    let expected = blake3_digest(b"pulith");

    let (_, evidence) = LocalPath::new(path.clone())
        .unwrap()
        .open_file()
        .unwrap()
        .unwrap()
        .snapshot_verified(&expected)
        .unwrap();
    assert_eq!(evidence.digest.algorithm(), DigestAlgorithmKind::Blake3);
    assert_eq!(evidence.digest, expected);
    assert_eq!(evidence.digest.as_str(), blake3_digest(b"pulith").as_str());
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
    )
    .unwrap();

    let observation = LocalPath::new(path)
        .unwrap()
        .open_file()
        .unwrap()
        .unwrap()
        .digest(DigestAlgorithmKind::Blake3)
        .unwrap();
    let reconciliation = observation.compare(&expected);
    assert_eq!(reconciliation, ArtifactReconciliation::Matches);
}

#[cfg(feature = "blake3")]
#[test]
fn exact_inspection_classifies_same_size_content_drift() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("artifact");
    std::fs::write(&path, b"PULITH").unwrap();

    let observation = LocalPath::new(path)
        .unwrap()
        .open_file()
        .unwrap()
        .unwrap()
        .digest(DigestAlgorithmKind::Blake3)
        .unwrap();
    let reconciliation = observation.compare(
        &ArtifactDescriptor::new(
            DigestAlgorithmKind::Blake3,
            blake3::hash(b"pulith").to_hex().to_string(),
            6,
        )
        .unwrap(),
    );
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

    let observation = LocalPath::new(path)
        .unwrap()
        .open_file()
        .unwrap()
        .unwrap()
        .digest(DigestAlgorithmKind::Blake3)
        .unwrap();
    let reconciliation = observation.compare(
        &ArtifactDescriptor::new(
            DigestAlgorithmKind::Blake3,
            blake3::hash(b"short").to_hex().to_string(),
            5,
        )
        .unwrap(),
    );
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
fn open_distinguishes_missing_directory_and_symlink_before_digest() {
    let root = tempfile::tempdir().unwrap();
    let missing = LocalPath::new(root.path().join("missing")).unwrap();
    assert!(missing.open_file().unwrap().is_none());
    assert!(matches!(
        LocalPath::new(root.path()).unwrap().open_file(),
        Err(LocalError::NotRegularFile {
            kind: LocalEntryKind::Directory,
            ..
        })
    ));

    let target = root.path().join("target");
    let link = root.path().join("link");
    std::fs::write(&target, b"secret").unwrap();
    crate::common::file_symlink(&target, &link).unwrap();
    assert!(matches!(
        LocalPath::new(link).unwrap().open_file(),
        Err(LocalError::NotRegularFile {
            kind: LocalEntryKind::Symlink,
            ..
        })
    ));
}

#[cfg(feature = "blake3")]
#[test]
fn blake3_snapshot_verified_rejects_content_drift() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("artifact");
    std::fs::write(&path, b"PULITH").unwrap();

    let error = LocalPath::new(path)
        .unwrap()
        .open_file()
        .unwrap()
        .unwrap()
        .snapshot_verified(&blake3_digest(b"pulith"))
        .unwrap_err();
    assert!(matches!(
        error,
        pulith::hash::HashError::DigestMismatch { .. }
    ));
}

#[cfg(feature = "sha2")]
#[test]
fn sha256_snapshot_verified_proves_the_declared_digest() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("artifact");
    std::fs::write(&path, b"pulith").unwrap();
    let expected = sha256_digest(b"pulith");

    let (_, evidence) = LocalPath::new(path)
        .unwrap()
        .open_file()
        .unwrap()
        .unwrap()
        .snapshot_verified(&expected)
        .unwrap();
    assert_eq!(evidence.digest.algorithm(), DigestAlgorithmKind::Sha256);
    assert_eq!(evidence.digest.as_str(), sha256_digest(b"pulith").as_str());
}

#[cfg(all(feature = "blake3", feature = "sha2"))]
#[test]
fn exact_reconciliation_compares_algorithm_as_well_as_hex() {
    let observed = ArtifactDescriptor::new(DigestAlgorithmKind::Blake3, "a".repeat(64), 7).unwrap();
    let expected = ArtifactDescriptor::new(DigestAlgorithmKind::Sha256, "a".repeat(64), 7).unwrap();
    assert!(matches!(
        observed.compare(&expected),
        ArtifactReconciliation::DigestMismatch { .. }
    ));
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
    )
    .unwrap();

    let observation = LocalPath::new(path)
        .unwrap()
        .open_file()
        .unwrap()
        .unwrap()
        .digest(DigestAlgorithmKind::Sha256)
        .unwrap();
    assert_eq!(
        observation.compare(&expected),
        ArtifactReconciliation::Matches
    );
}

#[test]
fn local_inspection_and_size_reconciliation_of_digest_material() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("artifact");
    std::fs::write(&path, b"pulith").unwrap();

    let observation = LocalPath::new(path.clone())
        .unwrap()
        .inspect_entry()
        .unwrap();
    assert_eq!(observation, LocalObservation::File { bytes: 6 });

    let reconciliation = observation.compare(&LocalExpectation::FileSize(6));
    assert_eq!(reconciliation, pulith::local::LocalReconciliation::Matches);
    assert_eq!(std::fs::read(&path).unwrap(), b"pulith");
}

#[test]
fn local_inspection_reports_missing_and_directory_without_mutation() {
    let root = tempfile::tempdir().unwrap();
    let missing = root.path().join("missing");
    let observation = LocalPath::new(missing.clone())
        .unwrap()
        .inspect_entry()
        .unwrap();
    assert_eq!(observation, LocalObservation::Missing);

    let observation = LocalPath::new(root.path().to_path_buf())
        .unwrap()
        .inspect_entry()
        .unwrap();
    assert_eq!(observation, LocalObservation::Directory);
}

#[cfg(unix)]
#[test]
fn entry_inspection_classifies_symlinks_without_following_them() {
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("file");
    let directory = root.path().join("directory");
    std::fs::write(&file, b"secret").unwrap();
    std::fs::create_dir(&directory).unwrap();
    crate::common::file_symlink(&file, root.path().join("file-link")).unwrap();
    crate::common::dir_symlink(&directory, root.path().join("dir-link")).unwrap();

    let observation = LocalPath::new(root.path().join("file-link"))
        .unwrap()
        .inspect_entry()
        .unwrap();
    assert_eq!(observation, LocalObservation::Symlink);

    let observation = LocalPath::new(root.path().join("dir-link"))
        .unwrap()
        .inspect_entry()
        .unwrap();
    assert_eq!(observation, LocalObservation::Symlink);
}

#[test]
fn artifact_descriptor_rejects_invalid_digest_without_panicking() {
    let error = ArtifactDescriptor::new(DigestAlgorithmKind::Blake3, "not-a-digest", 0)
        .expect_err("invalid digest must be rejected");
    assert!(error.contains("64 hex digits"));
}
