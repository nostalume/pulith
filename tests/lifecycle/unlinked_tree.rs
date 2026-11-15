#![cfg(feature = "zip")]

use std::fs;
use std::io::Write;
use std::path::PathBuf;

use pulith::archive::{ArchivePolicy, extract_zip};
use pulith::local::{
    LocalError, LocalExpectation, LocalObservation, LocalPath, LocalReconciliation,
};

fn write_archive(path: &std::path::Path) {
    let mut writer = zip::ZipWriter::new(fs::File::create(path).unwrap());
    writer
        .start_file("bin/tool.txt", zip::write::SimpleFileOptions::default())
        .unwrap();
    writer.write_all(b"artifact payload").unwrap();
    writer.finish().unwrap();
}

/// The unlinked-artifact flow: acquire the zip, decode it into custody, apply, observe, reconcile.
/// Decoding is the caller's composition: the stream is handed in, nothing is guessed.
fn flow(root: &std::path::Path) -> (PathBuf, pulith::local::TreePublication) {
    let archive = root.join("demo-tool-1.0.0.zip");
    let target = root.join("artifacts/demo-tool/1.0.0");
    write_archive(&archive);
    fs::create_dir_all(target.parent().unwrap()).unwrap();

    let material = LocalPath::new(archive).unwrap();
    let admitted = LocalPath::new(target.clone()).unwrap();
    let stage = admitted.stage().unwrap();
    let evidence = extract_zip(
        material.open_file().unwrap().unwrap(),
        stage.root(),
        ArchivePolicy::default(),
    )
    .unwrap();
    assert_eq!(evidence.entries, 1);
    assert_eq!(evidence.files, 1);
    assert!(!target.exists());

    let apply_evidence = stage
        .publish(admitted, pulith::local::TreeAdmission::Absent)
        .unwrap();
    assert_eq!(apply_evidence.files, 1);
    assert_eq!(apply_evidence.bytes, 16);
    assert_eq!(
        fs::read(target.join("bin/tool.txt")).unwrap(),
        b"artifact payload"
    );
    (target, apply_evidence)
}

#[test]
fn unlinked_artifact_tree_composes_existing_behaviors() {
    let root = tempfile::tempdir().unwrap();
    let (target, apply_evidence) = flow(root.path());

    let observation = LocalPath::new(target.clone())
        .unwrap()
        .inspect_entry()
        .unwrap();
    assert_eq!(observation, LocalObservation::Directory);

    let reconciliation = observation.compare(&LocalExpectation::Directory);
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

    let material = LocalPath::new(archive).unwrap();
    let admitted = LocalPath::new(target.clone()).unwrap();
    let stage = admitted.stage().unwrap();
    let tree = stage.copy_file(material, PathBuf::new()).unwrap();
    let error = tree
        .publish(admitted, pulith::local::TreeAdmission::Absent)
        .unwrap_err();

    assert!(matches!(
        error,
        LocalError::TargetExists(path) if path == target
    ));
    assert_eq!(fs::read(&sentinel).unwrap(), b"retain me");
    assert!(!target.join("bin/tool.txt").exists());
}
