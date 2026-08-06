#![cfg(feature = "local")]

use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use pulith::local::{
    LocalAcquire, LocalApply, LocalExpectation, LocalInspect, LocalObservation, LocalPath,
    LocalPlacement, LocalPostInspect, LocalReconcile, LocalReconciliation, LocalTarget,
};
use pulith::{Acquire, Apply, Forget, Inspect, Materialize, MaterializeMode, Reconcile};

fn temp_root(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "pulith-public-api-{name}-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ))
}

#[test]
fn materialize_local_file_without_synthetic_transitions() {
    #[derive(Clone, Debug, Eq, PartialEq)]
    struct AppId(&'static str);

    let root = temp_root("materialize");
    let source = root.join("source.txt");
    let target = root.join("target.txt");
    fs::create_dir_all(&root).unwrap();
    fs::write(&source, "pulith").unwrap();

    let request = Materialize::new(
        AppId("demo"),
        LocalPath::new(&source),
        LocalTarget::new(&target),
        MaterializeMode::ReplaceOrCreate,
    );
    let acquired = LocalAcquire.acquire(request).unwrap();
    let applied = LocalApply.apply(acquired).unwrap();

    assert_eq!(fs::read_to_string(&target).unwrap(), "pulith");
    assert_eq!(applied.input.item, AppId("demo"));
    assert_eq!(applied.evidence.current.strategy, LocalPlacement::Copied);

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn create_new_conflict_is_typed_and_non_mutating() {
    let root = temp_root("create-conflict");
    let source = root.join("source.txt");
    let target = root.join("target.txt");
    fs::create_dir_all(&root).unwrap();
    fs::write(&source, "replacement").unwrap();
    fs::write(&target, "winner").unwrap();

    let request = Materialize::new(
        "demo",
        LocalPath::new(&source),
        LocalTarget::new(&target),
        MaterializeMode::CreateNew,
    );
    let error = LocalApply
        .apply(LocalAcquire.acquire(request).unwrap())
        .unwrap_err();

    assert!(matches!(
        error,
        pulith::local::LocalError::ApplyWouldOverwrite(path) if path == target
    ));
    assert_eq!(fs::read_to_string(&target).unwrap(), "winner");
    assert_eq!(fs::read_to_string(&source).unwrap(), "replacement");
    fs::remove_dir_all(root).unwrap();
}

#[test]
#[cfg(feature = "blake3")]
fn verify_then_apply_exact_local_artifact() {
    use pulith::Verify;
    use pulith::hash::{ArtifactDescriptor, Blake3, HashVerify};

    let root = temp_root("descriptor");
    let source = root.join("source.txt");
    let target = root.join("target.txt");
    fs::create_dir_all(&root).unwrap();
    fs::write(&source, "pulith").unwrap();
    let digest = blake3::hash(b"pulith").to_hex().to_string();

    let request = Materialize::new(
        "demo",
        LocalPath::new(&source),
        LocalTarget::new(&target),
        MaterializeMode::CreateNew,
    );
    let acquired = LocalAcquire.acquire(request).unwrap();
    let expected = ArtifactDescriptor::<Blake3>::new(digest, 6);
    let verified = HashVerify::<Blake3>::new()
        .verify(acquired, expected.clone())
        .unwrap();
    assert_eq!(verified.evidence.current.expected, expected);

    let applied = LocalApply.apply(verified).unwrap();
    assert_eq!(fs::read_to_string(&target).unwrap(), "pulith");
    assert_eq!(applied.input.item, "demo");

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn forget_local_target_directly() {
    let root = temp_root("forget");
    let target = root.join("target.txt");
    fs::create_dir_all(&root).unwrap();
    fs::write(&target, "obsolete").unwrap();

    let applied = LocalApply
        .apply(Forget::new("demo", LocalTarget::new(&target)))
        .unwrap();

    assert!(!target.exists());
    assert_eq!(applied.input.target.path, target);
    assert_eq!(applied.evidence.strategy, LocalPlacement::Removed);

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn inspect_and_reconcile_without_mutating_local_target() {
    let root = temp_root("inspect-reconcile");
    let target = root.join("target.txt");
    fs::create_dir_all(&root).unwrap();
    fs::write(&target, "pulith").unwrap();

    let inspected = LocalInspect.inspect(LocalTarget::new(&target)).unwrap();
    assert_eq!(inspected.observation, LocalObservation::File { bytes: 6 });
    assert_eq!(inspected.evidence, pulith::local::LocalInspectEvidence);

    let reconciled = LocalReconcile
        .reconcile(inspected, LocalExpectation::FileSize(6))
        .unwrap();
    assert_eq!(reconciled.reconciliation, LocalReconciliation::Matches);
    assert_eq!(
        reconciled.evidence.current.expected,
        LocalExpectation::FileSize(6)
    );
    assert_eq!(reconciled.input.path, target);
    assert_eq!(fs::read_to_string(&target).unwrap(), "pulith");

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn post_inspect_preserves_materialize_apply_evidence_and_reconciles() {
    let root = temp_root("post-inspect-materialize");
    let source = root.join("source.txt");
    let target = root.join("target.txt");
    fs::create_dir_all(&root).unwrap();
    fs::write(&source, "pulith").unwrap();

    let applied = LocalApply
        .apply(
            LocalAcquire
                .acquire(Materialize::new(
                    "post-inspect",
                    LocalPath::new(&source),
                    LocalTarget::new(&target),
                    MaterializeMode::CreateNew,
                ))
                .unwrap(),
        )
        .unwrap();
    let apply_evidence = applied.evidence.clone();

    let inspected = LocalPostInspect.inspect(applied).unwrap();
    assert_eq!(inspected.input.path, target);
    assert_eq!(inspected.observation, LocalObservation::File { bytes: 6 });
    assert_eq!(inspected.evidence.previous, apply_evidence);
    assert_eq!(
        inspected.evidence.current,
        pulith::local::LocalInspectEvidence
    );

    let reconciled = LocalReconcile
        .reconcile(inspected, LocalExpectation::FileSize(6))
        .unwrap();
    assert_eq!(reconciled.reconciliation, LocalReconciliation::Matches);

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn post_inspect_reports_later_mutation_without_reapplying() {
    let root = temp_root("post-inspect-mutation");
    let source = root.join("source.txt");
    let target = root.join("target.txt");
    fs::create_dir_all(&root).unwrap();
    fs::write(&source, "pulith").unwrap();

    let applied = LocalApply
        .apply(
            LocalAcquire
                .acquire(Materialize::new(
                    "post-inspect-mutation",
                    LocalPath::new(&source),
                    LocalTarget::new(&target),
                    MaterializeMode::CreateNew,
                ))
                .unwrap(),
        )
        .unwrap();
    fs::write(&target, "changed!").unwrap();

    let inspected = LocalPostInspect.inspect(applied).unwrap();
    let reconciled = LocalReconcile
        .reconcile(inspected, LocalExpectation::FileSize(6))
        .unwrap();
    assert_eq!(
        reconciled.reconciliation,
        LocalReconciliation::Modified {
            expected_bytes: 6,
            observed_bytes: 8,
        }
    );
    assert_eq!(fs::read_to_string(&target).unwrap(), "changed!");

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn post_inspect_forget_observes_missing_without_acquisition() {
    let root = temp_root("post-inspect-forget");
    let target = root.join("target.txt");
    fs::create_dir_all(&root).unwrap();
    fs::write(&target, "obsolete").unwrap();

    let applied = LocalApply
        .apply(Forget::new(
            "post-inspect-forget",
            LocalTarget::new(&target),
        ))
        .unwrap();
    let apply_evidence = applied.evidence.clone();

    let inspected = LocalPostInspect.inspect(applied).unwrap();
    assert_eq!(inspected.input.path, target);
    assert_eq!(inspected.observation, LocalObservation::Missing);
    assert_eq!(inspected.evidence.previous, apply_evidence);

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn post_inspect_error_retains_completed_apply_receipt() {
    let root = temp_root("post-inspect-error");
    let source = root.join("source.txt");
    let target = root.join("target.txt");
    fs::create_dir_all(&root).unwrap();
    fs::write(&source, "pulith").unwrap();

    let mut applied = LocalApply
        .apply(
            LocalAcquire
                .acquire(Materialize::new(
                    "post-inspect-error",
                    LocalPath::new(&source),
                    LocalTarget::new(&target),
                    MaterializeMode::CreateNew,
                ))
                .unwrap(),
        )
        .unwrap();
    let original_target = target.clone();
    applied.input.target.path.push("\0");
    let invalid_target = applied.input.target.path.clone();

    let error = LocalPostInspect.inspect(applied).unwrap_err();
    assert_eq!(error.applied.input.target.path, invalid_target);
    assert!(matches!(
        error.cause,
        pulith::local::LocalError::Io { action: "inspect local target", path, .. } if path == invalid_target
    ));
    assert_eq!(fs::read_to_string(&original_target).unwrap(), "pulith");

    fs::remove_dir_all(root).unwrap();
}
