#![cfg(feature = "zip")]

use std::fs;
use std::io::Write;
use std::path::PathBuf;

use pulith::archive::{ArchiveKind, ArchivePolicy};
use pulith::local::{
    LocalError, LocalExpectation, LocalObservation, LocalPlacement, LocalReconciliation,
    LocalSource, LocalTarget, PreparationEvidence,
};
use pulith::{Acquire, Inspect, Reconcile};

fn write_archive(path: &std::path::Path) {
    let mut writer = zip::ZipWriter::new(fs::File::create(path).unwrap());
    writer
        .start_file("bin/tool.txt", zip::write::SimpleFileOptions::default())
        .unwrap();
    writer.write_all(b"artifact payload").unwrap();
    writer.finish().unwrap();
}

/// The unlinked-artifact flow: acquire the zip, verify is skipped (the artifact is byte
/// material — here we just prepare), prepare, apply, observe, reconcile.
fn flow(root: &std::path::Path) -> (PathBuf, pulith::local::ApplyEvidence) {
    let archive = root.join("demo-tool-1.0.0.zip");
    let target = root.join("artifacts/demo-tool/1.0.0");
    write_archive(&archive);
    fs::create_dir_all(target.parent().unwrap()).unwrap();

    let material = LocalSource::new(archive).unwrap().acquire().unwrap();
    let kind = ArchiveKind::sniff(material_path(&material))
        .unwrap()
        .unwrap();
    let admitted = LocalTarget::new(target.clone()).unwrap();
    let stage = admitted.stage().unwrap();
    let (tree, evidence) = material.prepare(stage, ArchivePolicy::default()).unwrap();
    let PreparationEvidence::Extracted(evidence) = evidence else {
        panic!("expected extraction")
    };
    assert_eq!(evidence.format, kind);
    assert_eq!(evidence.entries, 1);
    assert_eq!(evidence.files, 1);
    assert!(!target.exists());

    let apply_evidence = tree.publish(admitted).unwrap();
    assert_eq!(apply_evidence.strategy, LocalPlacement::Moved);
    assert_eq!(
        fs::read(target.join("bin/tool.txt")).unwrap(),
        b"artifact payload"
    );
    (target, apply_evidence)
}

fn material_path(material: &pulith::local::LocalMaterial) -> &std::path::Path {
    match material {
        pulith::local::LocalMaterial::File { path } => path,
        pulith::local::LocalMaterial::Directory { path } => path,
        pulith::local::LocalMaterial::StagedFile { path } => path.as_ref(),
    }
}

#[test]
fn unlinked_artifact_tree_composes_existing_behaviors() {
    let root = tempfile::tempdir().unwrap();
    let (target, apply_evidence) = flow(root.path());

    let (observation, _) = LocalTarget::new(target.clone())
        .unwrap()
        .inspect(())
        .unwrap();
    assert_eq!(observation, LocalObservation::Directory);

    let (reconciliation, _) = observation.reconcile(LocalExpectation::Directory).unwrap();
    assert_eq!(reconciliation, LocalReconciliation::Matches);
    let _ = apply_evidence;
}

#[test]
fn unlinked_artifact_tree_rejects_quiescent_existing_target() {
    let root = tempfile::tempdir().unwrap();
    let archive = root.path().join("demo-tool-1.0.0.zip");
    let target = root.path().join("artifacts/demo-tool/1.0.0");
    let sentinel = target.join("retained.txt");
    write_archive(&archive);
    fs::create_dir_all(&target).unwrap();
    fs::write(&sentinel, b"retain me").unwrap();

    let material = LocalSource::new(archive).unwrap().acquire().unwrap();
    let admitted = LocalTarget::new(target.clone()).unwrap();
    let stage = admitted.stage().unwrap();
    let (tree, _) = material.prepare(stage, ArchivePolicy::default()).unwrap();
    let error = tree.publish(admitted).unwrap_err();

    assert!(matches!(
        error,
        LocalError::AlreadyPublished(path) if path == target
    ));
    assert_eq!(fs::read(&sentinel).unwrap(), b"retain me");
    assert!(!target.join("bin/tool.txt").exists());
}
