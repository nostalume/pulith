#![cfg(feature = "zip")]

use std::fs;
use std::io::Write;

use pulith::archive::{ArchivePolicy, ArchivePrepare, ExtractWorkspace, Zip};
use pulith::local::{
    LocalAcquire, LocalApply, LocalError, LocalExpectation, LocalObservation, LocalPath,
    LocalPlacement, LocalPostInspect, LocalReconcile, LocalReconciliation, LocalTarget,
};
use pulith::{Acquire, Apply, Inspect, Materialize, MaterializeMode, Prepare, Reconcile};

#[derive(Clone, Debug, Eq, PartialEq)]
struct ArtifactId {
    name: &'static str,
    version: &'static str,
}

fn write_archive(path: &std::path::Path) {
    let mut writer = zip::ZipWriter::new(fs::File::create(path).unwrap());
    writer
        .start_file("bin/tool.txt", zip::write::SimpleFileOptions::default())
        .unwrap();
    writer.write_all(b"artifact payload").unwrap();
    writer.finish().unwrap();
}

fn request(
    archive: &std::path::Path,
    target: &std::path::Path,
) -> Materialize<ArtifactId, LocalPath, LocalTarget> {
    Materialize::new(
        ArtifactId {
            name: "demo-tool",
            version: "1.0.0",
        },
        LocalPath::new(archive),
        LocalTarget::new(target),
        MaterializeMode::CreateNew,
    )
}

#[test]
fn unlinked_artifact_tree_composes_existing_behaviors() {
    let root = tempfile::tempdir().unwrap();
    let archive = root.path().join("demo-tool-1.0.0.zip");
    let scratch = root.path().join("scratch");
    let target = root.path().join("artifacts/demo-tool/1.0.0");
    let active_view = root.path().join("bin/demo-tool");
    write_archive(&archive);
    fs::create_dir_all(target.parent().unwrap()).unwrap();

    let acquired = LocalAcquire.acquire(request(&archive, &target)).unwrap();
    let prepared = ArchivePrepare::<Zip>::new(ExtractWorkspace::new(&scratch))
        .prepare(acquired, ArchivePolicy::default())
        .unwrap();
    assert!(!target.exists());
    assert_eq!(prepared.evidence.previous.path, archive);
    assert_eq!(prepared.evidence.current.entries, 1);
    assert_eq!(prepared.evidence.current.files, 1);

    let applied = LocalApply.apply(prepared).unwrap();
    let apply_evidence = applied.evidence.clone();
    assert_eq!(applied.input.item.name, "demo-tool");
    assert_eq!(applied.input.item.version, "1.0.0");
    assert_eq!(applied.evidence.current.strategy, LocalPlacement::Copied);
    assert_eq!(
        fs::read(target.join("bin/tool.txt")).unwrap(),
        b"artifact payload"
    );
    assert!(!active_view.exists());

    let inspected = LocalPostInspect.inspect(applied).unwrap();
    assert_eq!(inspected.input.path, target);
    assert_eq!(inspected.observation, LocalObservation::Directory);
    assert_eq!(inspected.evidence.previous, apply_evidence);

    let reconciled = LocalReconcile
        .reconcile(inspected, LocalExpectation::Directory)
        .unwrap();
    assert_eq!(reconciled.reconciliation, LocalReconciliation::Matches);
}

#[test]
fn unlinked_artifact_tree_rejects_quiescent_existing_target() {
    let root = tempfile::tempdir().unwrap();
    let archive = root.path().join("demo-tool-1.0.0.zip");
    let scratch = root.path().join("scratch");
    let target = root.path().join("artifacts/demo-tool/1.0.0");
    let sentinel = target.join("retained.txt");
    write_archive(&archive);
    fs::create_dir_all(&target).unwrap();
    fs::write(&sentinel, b"retain me").unwrap();

    let acquired = LocalAcquire.acquire(request(&archive, &target)).unwrap();
    let prepared = ArchivePrepare::<Zip>::new(ExtractWorkspace::new(&scratch))
        .prepare(acquired, ArchivePolicy::default())
        .unwrap();
    let error = LocalApply.apply(prepared).unwrap_err();

    assert!(matches!(
        error,
        LocalError::ApplyWouldOverwrite(path) if path == target
    ));
    assert_eq!(fs::read(&sentinel).unwrap(), b"retain me");
    assert!(!target.join("bin/tool.txt").exists());
}
