#![cfg(feature = "local")]

use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

#[cfg(any(feature = "blake3", feature = "sha2"))]
use pulith::Verify;
use pulith::archive::ArchivePolicy;
use pulith::local::{
    LocalExpectation, LocalObservation, LocalReconciliation, LocalSource, LocalTarget,
    RemoveEvidence,
};

#[test]
fn staged_tree_writes_generated_bytes_inside_its_boundary() {
    let root = tempfile::tempdir().unwrap();
    let target_path = root.path().join("generated");
    let target = LocalTarget::new(&target_path).unwrap();
    target
        .stage()
        .unwrap()
        .write_file(b"schema = 1\n", "service.toml")
        .unwrap()
        .publish(target)
        .unwrap();
    assert_eq!(
        fs::read(target_path.join("service.toml")).unwrap(),
        b"schema = 1\n"
    );
}
use pulith::{Acquire, Inspect, Reconcile, Remove};

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
    let root = temp_root("materialize");
    let source = root.join("source.txt");
    let target = root.join("target.txt");
    fs::create_dir_all(&root).unwrap();
    fs::write(&source, "pulith").unwrap();

    let material = LocalSource::new(source.clone()).unwrap().acquire().unwrap();
    let target_path = target;
    let target = LocalTarget::new(target_path.clone()).unwrap();
    let stage = target.stage().unwrap();
    let (tree, _) = material.prepare(stage, ArchivePolicy::default()).unwrap();
    let evidence = tree.publish(target).unwrap();

    assert_eq!(fs::read_to_string(&target_path).unwrap(), "pulith");
    assert_eq!(evidence.files, 1);
    assert_eq!(evidence.directories, 0);
    assert_eq!(evidence.bytes, 6);

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

    let material = LocalSource::new(source.clone()).unwrap().acquire().unwrap();
    let admitted = LocalTarget::new(target.clone()).unwrap();
    let stage = admitted.stage().unwrap();
    let (tree, _) = material.prepare(stage, ArchivePolicy::default()).unwrap();
    let error = tree.publish(admitted).unwrap_err();

    assert!(matches!(
        error,
        pulith::local::LocalError::AlreadyPublished(path) if path == target
    ));
    assert_eq!(fs::read_to_string(&target).unwrap(), "winner");
    assert_eq!(fs::read_to_string(&source).unwrap(), "replacement");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn thirty_two_publishers_have_one_complete_winner() {
    let root = temp_root("publish-race");
    let target = root.join("target.txt");
    fs::create_dir_all(&root).unwrap();
    let stages = (0_u8..32)
        .map(|value| {
            let source = root.join(format!("source-{value}"));
            fs::write(&source, [value]).unwrap();
            let target = LocalTarget::new(&target).unwrap();
            target
                .stage()
                .unwrap()
                .copy_file(LocalSource::new(source).unwrap(), PathBuf::new())
                .unwrap()
        })
        .collect::<Vec<_>>();
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(stages.len()));
    let results = stages
        .into_iter()
        .map(|stage| {
            let barrier = barrier.clone();
            let target = target.clone();
            std::thread::spawn(move || {
                barrier.wait();
                stage.publish(LocalTarget::new(target).unwrap())
            })
        })
        .collect::<Vec<_>>()
        .into_iter()
        .map(|thread| thread.join().unwrap())
        .collect::<Vec<_>>();

    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    assert_eq!(
        results
            .iter()
            .filter(|result| matches!(result, Err(pulith::local::LocalError::AlreadyPublished(path)) if path == &target))
            .count(),
        31
    );
    assert_eq!(fs::read(&target).unwrap().len(), 1);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn stage_rejects_escaping_destinations_without_leaking_a_target() {
    let root = temp_root("stage-escape");
    let source = root.join("source");
    let target = root.join("target");
    fs::create_dir_all(&root).unwrap();
    fs::write(&source, b"safe").unwrap();
    let admitted = LocalTarget::new(&target).unwrap();
    let error = admitted
        .stage()
        .unwrap()
        .copy_file(LocalSource::new(source).unwrap(), "../escape")
        .unwrap_err();
    assert!(matches!(
        error,
        pulith::local::LocalError::InvalidStagePath(_)
    ));
    assert!(!target.exists());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn copy_tree_rejects_a_source_link_without_leaking_a_target() {
    let root = temp_root("copy-tree-link");
    let source = root.join("source");
    let outside = root.join("outside");
    let target = root.join("target");
    fs::create_dir_all(&source).unwrap();
    fs::write(&outside, b"outside").unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(&outside, source.join("link")).unwrap();
    #[cfg(windows)]
    if let Err(error) = std::os::windows::fs::symlink_file(&outside, source.join("link")) {
        assert_eq!(error.kind(), std::io::ErrorKind::PermissionDenied);
        fs::remove_dir_all(root).unwrap();
        return;
    }

    let admitted = LocalTarget::new(&target).unwrap();
    let error = admitted
        .stage()
        .unwrap()
        .copy_tree(LocalSource::new(source).unwrap(), PathBuf::new())
        .unwrap_err();
    assert!(matches!(
        error,
        pulith::local::LocalError::UnsupportedLocalEntry(_)
    ));
    assert!(!target.exists());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn precommit_rescan_rejects_an_injected_link() {
    let root = temp_root("stage-link-injection");
    let source = root.join("source");
    let outside = root.join("outside");
    let target = root.join("target");
    fs::create_dir_all(&source).unwrap();
    fs::write(source.join("file"), b"safe").unwrap();
    fs::write(&outside, b"outside").unwrap();
    let admitted = LocalTarget::new(&target).unwrap();
    let stage = admitted
        .stage()
        .unwrap()
        .copy_tree(LocalSource::new(source).unwrap(), PathBuf::new())
        .unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(&outside, stage.root().join("injected")).unwrap();
    #[cfg(windows)]
    if let Err(error) = std::os::windows::fs::symlink_file(&outside, stage.root().join("injected"))
    {
        assert_eq!(error.kind(), std::io::ErrorKind::PermissionDenied);
        drop(stage);
        fs::remove_dir_all(root).unwrap();
        return;
    }
    let error = stage.publish(admitted).unwrap_err();
    assert!(matches!(
        error,
        pulith::local::LocalError::UnsupportedLocalEntry(_)
    ));
    assert!(!target.exists());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn hard_link_input_is_normalized_into_independent_stage_bytes() {
    let root = temp_root("stage-hard-link");
    let source = root.join("source");
    let alias = root.join("alias");
    let target = root.join("target");
    fs::create_dir_all(&root).unwrap();
    fs::write(&source, b"before").unwrap();
    fs::hard_link(&source, &alias).unwrap();
    let admitted = LocalTarget::new(&target).unwrap();
    let stage = admitted
        .stage()
        .unwrap()
        .copy_file(LocalSource::new(alias).unwrap(), PathBuf::new())
        .unwrap();
    fs::write(source, b"after").unwrap();
    stage.publish(admitted).unwrap();
    assert_eq!(fs::read(&target).unwrap(), b"before");
    fs::remove_dir_all(root).unwrap();
}

#[test]
#[cfg(feature = "blake3")]
fn verify_then_apply_exact_local_artifact() {
    use pulith::hash::{DigestAlgorithmKind, DigestValue};

    let root = temp_root("descriptor");
    let source = root.join("source.txt");
    let target = root.join("target.txt");
    fs::create_dir_all(&root).unwrap();
    fs::write(&source, "pulith").unwrap();
    let digest = blake3::hash(b"pulith").to_hex().to_string();
    let expected = DigestValue::new(DigestAlgorithmKind::Blake3, digest).unwrap();

    let material = LocalSource::new(source.clone()).unwrap().acquire().unwrap();
    let (material, evidence) = material.verify(expected.clone()).unwrap();
    assert_eq!(evidence.expected, expected);

    let admitted = LocalTarget::new(target.clone()).unwrap();
    let stage = admitted.stage().unwrap();
    let (tree, _) = material.prepare(stage, ArchivePolicy::default()).unwrap();
    tree.publish(admitted).unwrap();
    assert_eq!(fs::read_to_string(&target).unwrap(), "pulith");

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn remove_local_target_reports_changed_and_unchanged() {
    let root = temp_root("forget");
    let target = root.join("target.txt");
    fs::create_dir_all(&root).unwrap();
    fs::write(&target, "obsolete").unwrap();

    let removed = LocalTarget::new(target.clone()).unwrap().remove().unwrap();

    assert!(!target.exists());
    assert_eq!(removed, RemoveEvidence::Removed);
    let unchanged = LocalTarget::new(target.clone()).unwrap().remove().unwrap();
    assert_eq!(unchanged, RemoveEvidence::Unchanged);

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn inspect_and_reconcile_without_mutating_local_target() {
    let root = temp_root("inspect-reconcile");
    let target = root.join("target.txt");
    fs::create_dir_all(&root).unwrap();
    fs::write(&target, "pulith").unwrap();

    let (observation, evidence) = LocalTarget::new(target.clone())
        .unwrap()
        .inspect(())
        .unwrap();
    assert_eq!(observation, LocalObservation::File { bytes: 6 });
    assert_eq!(evidence.path, target);

    let (reconciliation, reconcile_evidence) = observation
        .reconcile(LocalExpectation::FileSize(6))
        .unwrap();
    assert_eq!(reconciliation, LocalReconciliation::Matches);
    assert_eq!(reconcile_evidence.expected, LocalExpectation::FileSize(6));
    assert_eq!(
        reconcile_evidence.observed,
        LocalObservation::File { bytes: 6 }
    );
    assert_eq!(fs::read_to_string(&target).unwrap(), "pulith");

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn post_inspect_preserves_apply_evidence_and_reconciles() {
    let root = temp_root("post-inspect-materialize");
    let source = root.join("source.txt");
    let target = root.join("target.txt");
    fs::create_dir_all(&root).unwrap();
    fs::write(&source, "pulith").unwrap();

    let material = LocalSource::new(source.clone()).unwrap().acquire().unwrap();
    let admitted = LocalTarget::new(target.clone()).unwrap();
    let stage = admitted.stage().unwrap();
    let (tree, _) = material.prepare(stage, ArchivePolicy::default()).unwrap();
    let apply_evidence = tree.publish(admitted).unwrap();

    let (observation, _) = LocalTarget::new(target.clone())
        .unwrap()
        .inspect(())
        .unwrap();
    assert_eq!(observation, LocalObservation::File { bytes: 6 });

    let (reconciliation, _) = observation
        .reconcile(LocalExpectation::FileSize(6))
        .unwrap();
    assert_eq!(reconciliation, LocalReconciliation::Matches);
    let _ = apply_evidence;

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn post_inspect_reports_later_mutation_without_reapplying() {
    let root = temp_root("post-inspect-mutation");
    let source = root.join("source.txt");
    let target = root.join("target.txt");
    fs::create_dir_all(&root).unwrap();
    fs::write(&source, "pulith").unwrap();

    let material = LocalSource::new(source.clone()).unwrap().acquire().unwrap();
    let admitted = LocalTarget::new(target.clone()).unwrap();
    let stage = admitted.stage().unwrap();
    let (tree, _) = material.prepare(stage, ArchivePolicy::default()).unwrap();
    tree.publish(admitted).unwrap();
    fs::write(&target, "changed!").unwrap();

    let (observation, _) = LocalTarget::new(target.clone())
        .unwrap()
        .inspect(())
        .unwrap();
    let (reconciliation, _) = observation
        .reconcile(LocalExpectation::FileSize(6))
        .unwrap();
    assert_eq!(
        reconciliation,
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

    LocalTarget::new(target.clone()).unwrap().remove().unwrap();

    let (observation, _) = LocalTarget::new(target.clone())
        .unwrap()
        .inspect(())
        .unwrap();
    assert_eq!(observation, LocalObservation::Missing);

    fs::remove_dir_all(root).unwrap();
}
