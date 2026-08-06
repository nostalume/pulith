#![cfg(all(feature = "local", any(feature = "blake3", feature = "sha2")))]

use pulith::Inspected;
use pulith::hash::{
    ArtifactDescriptor, ArtifactInspectEvidence, ArtifactReconcile, ArtifactReconciliation,
    HashInspect, HashMaterializeInspect,
};
use pulith::local::LocalArtifactObservation;
#[cfg(feature = "blake3")]
use pulith::local::LocalEntryKind;
use pulith::local::LocalTarget;
use pulith::{Inspect, Reconcile};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ExternalEvidence;

struct NonTraitAlgorithm;

#[test]
fn exact_evidence_and_reconcile_do_not_require_marker_traits() {
    fn require_open_evidence<T: Clone + Copy + Default + std::fmt::Debug + Eq>() {}
    fn require_open_adapter<T: Clone + Copy + Default + std::fmt::Debug>() {}
    require_open_evidence::<ArtifactInspectEvidence<NonTraitAlgorithm>>();
    require_open_adapter::<HashInspect<NonTraitAlgorithm>>();

    let inspected = Inspected {
        input: LocalTarget::new("external-target"),
        observation: LocalArtifactObservation::File {
            attestation: ArtifactDescriptor::<NonTraitAlgorithm>::new("observed", 1),
        },
        evidence: ExternalEvidence,
    };
    let reconciled = ArtifactReconcile
        .reconcile(
            inspected,
            ArtifactDescriptor::<NonTraitAlgorithm>::new("expected", 1),
        )
        .unwrap();
    assert!(matches!(
        reconciled.reconciliation,
        ArtifactReconciliation::DigestMismatch { .. }
    ));
}

#[cfg(feature = "blake3")]
fn blake3_descriptor(bytes: &[u8]) -> ArtifactDescriptor<pulith::hash::Blake3> {
    ArtifactDescriptor::new(blake3::hash(bytes).to_hex().to_string(), bytes.len() as u64)
}

#[cfg(feature = "blake3")]
#[test]
fn blake3_inspect_and_reconcile_exact_regular_file() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("artifact");
    std::fs::write(&path, b"pulith").unwrap();
    let expected = blake3_descriptor(b"pulith");

    let inspected = HashInspect::<pulith::hash::Blake3>::new()
        .inspect(LocalTarget::new(&path))
        .unwrap();
    assert_eq!(
        inspected.observation,
        LocalArtifactObservation::File {
            attestation: blake3_descriptor(b"pulith")
        }
    );
    assert_eq!(std::fs::read(&path).unwrap(), b"pulith");
    let reconciled = ArtifactReconcile.reconcile(inspected, expected).unwrap();
    assert_eq!(reconciled.reconciliation, ArtifactReconciliation::Matches);
}

#[cfg(feature = "blake3")]
#[test]
fn same_size_content_drift_is_digest_mismatch() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("artifact");
    std::fs::write(&path, b"PULITH").unwrap();
    let inspected = HashInspect::<pulith::hash::Blake3>::new()
        .inspect(LocalTarget::new(&path))
        .unwrap();

    let reconciled = ArtifactReconcile
        .reconcile(inspected, blake3_descriptor(b"pulith"))
        .unwrap();
    assert!(matches!(
        reconciled.reconciliation,
        ArtifactReconciliation::DigestMismatch { .. }
    ));
}

#[cfg(feature = "blake3")]
#[test]
fn unequal_observed_bytes_is_size_mismatch() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("artifact");
    std::fs::write(&path, b"longer").unwrap();
    let inspected = HashInspect::<pulith::hash::Blake3>::new()
        .inspect(LocalTarget::new(&path))
        .unwrap();

    let reconciled = ArtifactReconcile
        .reconcile(inspected, blake3_descriptor(b"short"))
        .unwrap();
    assert_eq!(
        reconciled.reconciliation,
        ArtifactReconciliation::SizeMismatch {
            expected: 5,
            observed: 6,
        }
    );
}

#[cfg(feature = "blake3")]
#[test]
fn missing_and_directory_are_observations() {
    let root = tempfile::tempdir().unwrap();
    let missing = HashInspect::<pulith::hash::Blake3>::new()
        .inspect(LocalTarget::new(root.path().join("missing")))
        .unwrap();
    assert_eq!(missing.observation, LocalArtifactObservation::Missing);
    let reconciled = ArtifactReconcile
        .reconcile(missing, blake3_descriptor(b"expected"))
        .unwrap();
    assert_eq!(reconciled.reconciliation, ArtifactReconciliation::Missing);

    let directory = HashInspect::<pulith::hash::Blake3>::new()
        .inspect(LocalTarget::new(root.path()))
        .unwrap();
    let reconciled = ArtifactReconcile
        .reconcile(directory, blake3_descriptor(b"expected"))
        .unwrap();
    assert_eq!(
        reconciled.reconciliation,
        ArtifactReconciliation::WrongKind {
            observed: LocalEntryKind::Directory,
        }
    );
}

#[cfg(feature = "blake3")]
#[test]
fn reconcile_preserves_external_inspection_evidence() {
    let observation = LocalArtifactObservation::File {
        attestation: blake3_descriptor(b"pulith"),
    };
    let expected_observation = observation.clone();
    let inspected = Inspected {
        input: LocalTarget::new("external-target"),
        observation,
        evidence: ExternalEvidence,
    };

    let reconciled = ArtifactReconcile
        .reconcile(inspected, blake3_descriptor(b"pulith"))
        .unwrap();
    assert_eq!(reconciled.evidence.previous, ExternalEvidence);
    assert_eq!(
        reconciled.evidence.current.expected,
        blake3_descriptor(b"pulith")
    );
    assert_eq!(reconciled.evidence.current.observed, expected_observation);
}

#[cfg(feature = "blake3")]
#[test]
fn final_symlink_and_dangling_symlink_are_wrong_kind() {
    let root = tempfile::tempdir().unwrap();
    let target = root.path().join("target");
    let link = root.path().join("link");
    let dangling = root.path().join("dangling");
    std::fs::write(&target, b"secret").unwrap();
    crate::common::file_symlink(&target, &link).unwrap();
    crate::common::file_symlink(root.path().join("absent"), &dangling).unwrap();
    for path in [link, dangling] {
        let inspected = HashInspect::<pulith::hash::Blake3>::new()
            .inspect(LocalTarget::new(path))
            .unwrap();
        let reconciled = ArtifactReconcile
            .reconcile(inspected, blake3_descriptor(b"secret"))
            .unwrap();
        assert_eq!(
            reconciled.reconciliation,
            ArtifactReconciliation::WrongKind {
                observed: LocalEntryKind::Symlink
            }
        );
    }
}

#[cfg(feature = "blake3")]
#[test]
fn parent_symlink_is_followed_within_trusted_parent_boundary() {
    let root = tempfile::tempdir().unwrap();
    let real = root.path().join("real");
    let linked = root.path().join("linked");
    std::fs::create_dir(&real).unwrap();
    std::fs::write(real.join("artifact"), b"pulith").unwrap();
    crate::common::dir_symlink(&real, &linked).unwrap();
    let inspected = HashInspect::<pulith::hash::Blake3>::new()
        .inspect(LocalTarget::new(linked.join("artifact")))
        .unwrap();
    assert!(matches!(
        inspected.observation,
        LocalArtifactObservation::File { .. }
    ));
}

#[cfg(all(unix, feature = "blake3"))]
#[test]
fn unix_fifo_and_socket_are_other_without_blocking() {
    use std::os::unix::net::UnixListener;
    use std::process::{Command, Stdio};
    let root = tempfile::tempdir().unwrap();
    let fifo = root.path().join("fifo");
    let socket = root.path().join("socket");
    let status = Command::new("mkfifo")
        .arg(&fifo)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .unwrap();
    assert!(status.success());
    let _listener = UnixListener::bind(&socket).unwrap();
    for path in [fifo, socket] {
        let inspected = HashInspect::<pulith::hash::Blake3>::new()
            .inspect(LocalTarget::new(path))
            .unwrap();
        assert_eq!(inspected.observation, LocalArtifactObservation::Other);
    }
}

#[cfg(all(windows, feature = "blake3"))]
#[test]
fn windows_junction_is_reparse() {
    use std::process::{Command, Stdio};
    let root = tempfile::tempdir().unwrap();
    let target = root.path().join("target");
    let junction = root.path().join("junction");
    std::fs::create_dir(&target).unwrap();
    let status = Command::new("cmd")
        .args(["/d", "/c", "mklink", "/J"])
        .arg(&junction)
        .arg(&target)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .unwrap();
    assert!(status.success());
    let inspected = HashInspect::<pulith::hash::Blake3>::new()
        .inspect(LocalTarget::new(junction))
        .unwrap();
    assert_eq!(inspected.observation, LocalArtifactObservation::Reparse);
}

#[cfg(feature = "sha2")]
#[test]
fn sha256_inspect_and_reconcile_exact_regular_file() {
    use sha2::{Digest, Sha256};

    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("artifact");
    std::fs::write(&path, b"pulith").unwrap();
    let expected =
        ArtifactDescriptor::<pulith::hash::Sha256>::new(hex::encode(Sha256::digest(b"pulith")), 6);

    let inspected = HashInspect::<pulith::hash::Sha256>::new()
        .inspect(LocalTarget::new(&path))
        .unwrap();
    let reconciled = ArtifactReconcile.reconcile(inspected, expected).unwrap();
    assert_eq!(reconciled.reconciliation, ArtifactReconciliation::Matches);
}

#[cfg(feature = "blake3")]
#[test]
fn materialize_inspection_preserves_receipt_and_observes_later_drift() {
    use pulith::local::{LocalAcquire, LocalApply, LocalPath};
    use pulith::{Acquire, Apply, Materialize, MaterializeMode};

    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source");
    let target = root.path().join("target");
    std::fs::write(&source, b"pulith").unwrap();
    let applied = LocalApply
        .apply(
            LocalAcquire
                .acquire(Materialize::new(
                    "hash-materialize-inspect",
                    LocalPath::new(&source),
                    LocalTarget::new(&target),
                    MaterializeMode::CreateNew,
                ))
                .unwrap(),
        )
        .unwrap();
    let apply_evidence = applied.evidence.clone();
    std::fs::write(&target, b"PULITH").unwrap();

    let inspected = HashMaterializeInspect::<pulith::hash::Blake3>::new()
        .inspect(applied)
        .unwrap();
    assert_eq!(inspected.evidence.previous, apply_evidence);
    assert_eq!(
        inspected.observation,
        LocalArtifactObservation::File {
            attestation: blake3_descriptor(b"PULITH"),
        }
    );
    let reconciled = ArtifactReconcile
        .reconcile(inspected, blake3_descriptor(b"pulith"))
        .unwrap();
    assert!(matches!(
        reconciled.reconciliation,
        ArtifactReconciliation::DigestMismatch { .. }
    ));
}

#[cfg(feature = "blake3")]
#[test]
fn materialize_inspection_error_retains_completed_receipt() {
    use pulith::local::{LocalAcquire, LocalApply, LocalPath};
    use pulith::{Acquire, Apply, Materialize, MaterializeMode};

    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source");
    let target = root.path().join("target");
    std::fs::write(&source, b"pulith").unwrap();
    let mut applied = LocalApply
        .apply(
            LocalAcquire
                .acquire(Materialize::new(
                    "hash-materialize-inspect-error",
                    LocalPath::new(&source),
                    LocalTarget::new(&target),
                    MaterializeMode::CreateNew,
                ))
                .unwrap(),
        )
        .unwrap();
    applied.input.target.path.push("\0");
    let invalid_target = applied.input.target.path.clone();

    let error = HashMaterializeInspect::<pulith::hash::Blake3>::new()
        .inspect(applied)
        .unwrap_err();
    assert_eq!(error.applied.input.target.path, invalid_target);
    assert!(matches!(
        error.cause,
        pulith::hash::HashError::LocalArtifact { path, .. } if path == invalid_target
    ));
}
#[cfg(feature = "blake3")]
#[test]
fn materialize_inspection_classifies_later_symlink_without_hashing_it() {
    use pulith::local::{LocalAcquire, LocalApply, LocalPath};
    use pulith::{Acquire, Apply, Materialize, MaterializeMode};

    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source");
    let target = root.path().join("target");
    std::fs::write(&source, b"pulith").unwrap();
    let applied = LocalApply
        .apply(
            LocalAcquire
                .acquire(Materialize::new(
                    "hash-materialize-inspect-symlink",
                    LocalPath::new(&source),
                    LocalTarget::new(&target),
                    MaterializeMode::CreateNew,
                ))
                .unwrap(),
        )
        .unwrap();
    std::fs::remove_file(&target).unwrap();
    crate::common::file_symlink(&source, &target).unwrap();

    let inspected = HashMaterializeInspect::<pulith::hash::Blake3>::new()
        .inspect(applied)
        .unwrap();
    assert_eq!(inspected.observation, LocalArtifactObservation::Symlink);
}

#[cfg(feature = "sha2")]
#[test]
fn sha256_materialize_inspection_attests_final_file() {
    use pulith::local::{LocalAcquire, LocalApply, LocalPath};
    use pulith::{Acquire, Apply, Materialize, MaterializeMode};
    use sha2::{Digest, Sha256};

    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source");
    let target = root.path().join("target");
    std::fs::write(&source, b"pulith").unwrap();
    let applied = LocalApply
        .apply(
            LocalAcquire
                .acquire(Materialize::new(
                    "sha256-hash-materialize-inspect",
                    LocalPath::new(&source),
                    LocalTarget::new(&target),
                    MaterializeMode::CreateNew,
                ))
                .unwrap(),
        )
        .unwrap();

    let inspected = HashMaterializeInspect::<pulith::hash::Sha256>::new()
        .inspect(applied)
        .unwrap();
    assert_eq!(
        inspected.observation,
        LocalArtifactObservation::File {
            attestation: ArtifactDescriptor::new(hex::encode(Sha256::digest(b"pulith")), 6),
        }
    );
}
