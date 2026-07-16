#![cfg(feature = "local")]

use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use pulith::{
    AcquireNode, ApplyNode, CreateOrReplace, Forget, Identity, IdentityPrepare, IdentityVerify,
    Intent, Item, LocalAcquire, LocalApply, LocalPath, LocalPlacement, LocalTarget, MemoryRemember,
    PrepareNode, RememberNode, VerifyNode,
};

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
fn public_api_materializes_and_remembers_local_file() {
    let root = temp_root("materialize");
    let source = root.join("source.txt");
    let target = root.join("target.txt");
    fs::create_dir_all(&root).unwrap();
    fs::write(&source, "pulith").unwrap();

    let chosen = Intent::new(Item::new("demo"), LocalTarget::new(&target))
        .with_source(LocalPath::new(&source))
        .select_first()
        .unwrap();
    let acquired = LocalAcquire.acquire_node(chosen).unwrap();
    let verified = IdentityVerify.verify_node(acquired, Identity).unwrap();
    let prepared = IdentityPrepare.prepare_node(verified, Identity).unwrap();
    let applied = LocalApply::<CreateOrReplace>::new()
        .apply_node(prepared)
        .unwrap();
    let remembered = MemoryRemember.remember_node(applied).unwrap();

    assert_eq!(fs::read_to_string(&target).unwrap(), "pulith");
    assert_eq!(remembered.receipt().item, "demo");
    assert_eq!(remembered.evidence().current.item, "demo");

    fs::remove_dir_all(root).unwrap();
}

#[test]
#[cfg(feature = "blake3")]
fn public_api_verifies_exact_artifact_descriptor() {
    use pulith::{ArtifactDescriptor, Blake3, DescriptorVerify};

    let root = temp_root("descriptor");
    let source = root.join("source.txt");
    let target = root.join("target.txt");
    fs::create_dir_all(&root).unwrap();
    fs::write(&source, "pulith").unwrap();
    let digest = blake3::hash(b"pulith").to_hex().to_string();

    let chosen = Intent::new(Item::new("demo"), LocalTarget::new(&target))
        .with_source(LocalPath::new(&source))
        .select_first()
        .unwrap();
    let acquired = LocalAcquire.acquire_node(chosen).unwrap();
    let descriptor = ArtifactDescriptor::<Blake3>::new(digest, 6);
    let verified = DescriptorVerify::<Blake3>::new()
        .verify_node(acquired, descriptor.clone())
        .unwrap();

    assert_eq!(verified.evidence().current.expected, descriptor);
    assert_eq!(verified.evidence().current.observed, descriptor);
    assert!(!target.exists());

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn public_api_forgets_local_target_directly() {
    let root = temp_root("forget");
    let target = root.join("target.txt");
    fs::create_dir_all(&root).unwrap();
    fs::write(&target, "obsolete").unwrap();

    let applied = LocalApply::<Forget>::new()
        .apply_node(Intent::new(Item::new("demo"), LocalTarget::new(&target)).op::<Forget>())
        .unwrap();

    assert!(!target.exists());
    assert_eq!(applied.receipt().target, target);
    assert_eq!(applied.evidence().strategy, LocalPlacement::Removed);

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn public_api_inspects_and_reconciles_without_mutating_local_target() {
    use pulith::{
        InspectNode, LocalExpectation, LocalInspect, LocalInspectMethod, LocalObservation,
        LocalReconcile, LocalReconciliation, ReconcileNode,
    };

    let root = temp_root("inspect-reconcile");
    let target = root.join("target.txt");
    fs::create_dir_all(&root).unwrap();
    fs::write(&target, "pulith").unwrap();

    let inspected = LocalInspect
        .inspect_node(LocalTarget::new(&target))
        .unwrap();
    assert_eq!(
        inspected.observation(),
        &LocalObservation::File { bytes: 6 }
    );
    assert_eq!(
        inspected.evidence().method,
        LocalInspectMethod::NoFollowMetadata
    );

    let reconciled = LocalReconcile
        .reconcile_node(inspected, LocalExpectation::FileSize(6))
        .unwrap();
    assert_eq!(reconciled.reconciliation(), &LocalReconciliation::Matches);
    assert_eq!(
        reconciled.evidence().previous.method,
        LocalInspectMethod::NoFollowMetadata
    );
    assert_eq!(
        reconciled.evidence().current.expected,
        LocalExpectation::FileSize(6)
    );
    assert_eq!(reconciled.input().path, target);
    assert_eq!(fs::read_to_string(&target).unwrap(), "pulith");

    fs::remove_dir_all(root).unwrap();
}
