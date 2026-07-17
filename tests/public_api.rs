#![cfg(feature = "local")]

use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use pulith::local::{
    LocalAcquire, LocalApply, LocalExpectation, LocalInspect, LocalObservation, LocalPath,
    LocalPlacement, LocalReconcile, LocalReconciliation, LocalTarget,
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
fn public_api_materializes_local_file_without_synthetic_transitions() {
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
        MaterializeMode::CreateOrReplace,
    );
    let acquired = LocalAcquire.acquire(request).unwrap();
    let applied = LocalApply.apply(acquired).unwrap();

    assert_eq!(fs::read_to_string(&target).unwrap(), "pulith");
    assert_eq!(applied.input().item, AppId("demo"));
    assert_eq!(applied.evidence().current.strategy, LocalPlacement::Copied);

    fs::remove_dir_all(root).unwrap();
}

#[test]
#[cfg(feature = "blake3")]
fn public_api_verifies_then_applies_exact_artifact() {
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
        MaterializeMode::Create,
    );
    let acquired = LocalAcquire.acquire(request).unwrap();
    let expected = ArtifactDescriptor::<Blake3>::new(digest, 6);
    let verified = HashVerify::<Blake3>::new()
        .verify(acquired, expected.clone())
        .unwrap();
    assert_eq!(verified.evidence().current.expected, expected);

    let applied = LocalApply.apply(verified).unwrap();
    assert_eq!(fs::read_to_string(&target).unwrap(), "pulith");
    assert_eq!(applied.input().item, "demo");

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn public_api_forgets_local_target_directly() {
    let root = temp_root("forget");
    let target = root.join("target.txt");
    fs::create_dir_all(&root).unwrap();
    fs::write(&target, "obsolete").unwrap();

    let applied = LocalApply
        .apply(Forget::new("demo", LocalTarget::new(&target)))
        .unwrap();

    assert!(!target.exists());
    assert_eq!(applied.input().target.path, target);
    assert_eq!(applied.evidence().strategy, LocalPlacement::Removed);

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn public_api_inspects_and_reconciles_without_mutating_local_target() {
    let root = temp_root("inspect-reconcile");
    let target = root.join("target.txt");
    fs::create_dir_all(&root).unwrap();
    fs::write(&target, "pulith").unwrap();

    let inspected = LocalInspect.inspect(LocalTarget::new(&target)).unwrap();
    assert_eq!(
        inspected.observation(),
        &LocalObservation::File { bytes: 6 }
    );
    assert_eq!(inspected.evidence(), &pulith::local::LocalInspectEvidence);

    let reconciled = LocalReconcile
        .reconcile(inspected, LocalExpectation::FileSize(6))
        .unwrap();
    assert_eq!(reconciled.reconciliation(), &LocalReconciliation::Matches);
    assert_eq!(
        reconciled.evidence().current.expected,
        LocalExpectation::FileSize(6)
    );
    assert_eq!(reconciled.input().path, target);
    assert_eq!(fs::read_to_string(&target).unwrap(), "pulith");

    fs::remove_dir_all(root).unwrap();
}
