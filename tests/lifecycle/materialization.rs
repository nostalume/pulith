#![cfg(feature = "local")]

use std::path::PathBuf;

#[cfg(any(feature = "http-sync", feature = "http-async"))]
use crate::common::HttpFixture;
#[cfg(any(feature = "http-sync", feature = "http-async", feature = "zip"))]
use std::io::Write;

#[cfg(all(feature = "blake3", feature = "http-sync"))]
use pulith::Verify;
use pulith::archive::{ArchiveKind, ArchivePolicy};
#[cfg(all(feature = "blake3", feature = "http-sync"))]
use pulith::hash::{DigestAlgorithmKind, DigestValue};
use pulith::local::{LocalObservation, LocalSource, LocalTarget, PreparationEvidence};
use pulith::{Acquire, Remove};
#[cfg(all(feature = "blake3", any(feature = "http-sync", feature = "http-async")))]
use pulith::{Inspect, Reconcile};

#[cfg(any(feature = "http-sync", feature = "http-async"))]
use pulith::local::{LocalExpectation, LocalReconciliation};
#[cfg(any(feature = "http-sync", feature = "http-async"))]
use pulith::net::{RemoteSource, RemoteUrl};

#[cfg(all(feature = "blake3", any(feature = "http-sync", feature = "http-async")))]
fn assert_applied_then_reconciled(target: &std::path::Path, body: &[u8]) {
    assert_eq!(std::fs::read(target).unwrap(), body);

    let (observation, _) = LocalTarget::new(target.to_path_buf())
        .unwrap()
        .inspect(())
        .unwrap();
    let (reconciliation, _) = observation
        .reconcile(LocalExpectation::FileSize(body.len() as u64))
        .unwrap();
    assert_eq!(reconciliation, LocalReconciliation::Matches);
}

#[cfg(all(feature = "blake3", feature = "http-sync"))]
#[test]
fn sync_http_acquire_verifies_prepares_and_applies_with_evidence() {
    let body = b"sync artifact";
    let server = HttpFixture::get(body);
    let root = tempfile::tempdir().unwrap();
    let target = root.path().join("sync-artifact");

    let (artifact, evidence) = RemoteSource::new(RemoteUrl::parse(&server.url).unwrap())
        .acquire()
        .unwrap();
    server.join();
    assert_eq!(evidence.status, 200);
    assert_eq!(evidence.bytes, body.len() as u64);

    let expected = DigestValue::new(
        DigestAlgorithmKind::Blake3,
        blake3::hash(body).to_hex().to_string(),
    )
    .unwrap();
    let (material, digest_evidence) = artifact.verify(expected.clone()).unwrap();
    assert_eq!(digest_evidence.expected, expected);
    let admitted = LocalTarget::new(target.clone()).unwrap();
    let stage = admitted.stage().unwrap();
    let (tree, _) = material.prepare(stage, ArchivePolicy::default()).unwrap();
    tree.publish(admitted).unwrap();
    assert_applied_then_reconciled(&target, body);
}

#[cfg(all(feature = "blake3", feature = "http-async"))]
#[test]
fn async_http_acquire_stages_artifact_with_evidence() {
    use pulith::AsyncAcquire;

    let body = b"async artifact";
    let server = HttpFixture::get(body);
    let root = tempfile::tempdir().unwrap();
    let target = root.path().join("async-artifact");

    let (_artifact, evidence) = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(AsyncAcquire::acquire(RemoteSource::new(
            RemoteUrl::parse(&server.url).unwrap(),
        )))
        .unwrap();
    server.join();
    assert!(!target.exists());
    assert_eq!(evidence.status, 200);
    assert_eq!(evidence.bytes, body.len() as u64);
}

#[cfg(feature = "zip")]
#[test]
fn zip_acquire_prepare_apply_keeps_scratch_and_final_authority_separate() {
    let root = tempfile::tempdir().unwrap();
    let archive = root.path().join("artifact.zip");
    let target = root.path().join("final-tree");
    let mut writer = zip::ZipWriter::new(std::fs::File::create(&archive).unwrap());
    writer
        .start_file("bin/tool.txt", zip::write::SimpleFileOptions::default())
        .unwrap();
    writer.write_all(b"zip payload").unwrap();
    writer.finish().unwrap();

    let material = LocalSource::new(archive.clone())
        .unwrap()
        .acquire()
        .unwrap();
    let kind = ArchiveKind::sniff(&archive).unwrap().unwrap();
    let admitted = LocalTarget::new(target.clone()).unwrap();
    let stage = admitted.stage().unwrap();
    let (tree, evidence) = material.prepare(stage, ArchivePolicy::default()).unwrap();
    assert!(!target.exists());
    let PreparationEvidence::Extracted(evidence) = evidence else {
        panic!("expected extraction")
    };
    assert_eq!(evidence.format, kind);
    assert_eq!(evidence.entries, 1);
    assert_eq!(evidence.files, 1);
    assert_eq!(
        std::fs::read(tree.root().join("bin/tool.txt")).unwrap(),
        b"zip payload"
    );

    tree.publish(admitted).unwrap();
    assert_eq!(
        std::fs::read(target.join("bin/tool.txt")).unwrap(),
        b"zip payload"
    );
}

#[cfg(feature = "tar")]
#[test]
fn tar_acquire_prepare_apply_keeps_scratch_and_final_authority_separate() {
    let root = tempfile::tempdir().unwrap();
    let archive = root.path().join("artifact.tar");
    let target = root.path().join("final-tree");
    let mut builder = tar::Builder::new(std::fs::File::create(&archive).unwrap());
    let mut header = tar::Header::new_gnu();
    header.set_entry_type(tar::EntryType::Regular);
    header.set_size(11);
    header.set_mode(0o644);
    header.set_cksum();
    builder
        .append_data(
            &mut header,
            "bin/tool.txt",
            std::io::Cursor::new(b"tar payload"),
        )
        .unwrap();
    builder.finish().unwrap();

    let material = LocalSource::new(archive.clone())
        .unwrap()
        .acquire()
        .unwrap();
    let kind = ArchiveKind::sniff(&archive).unwrap().unwrap();
    let admitted = LocalTarget::new(target.clone()).unwrap();
    let stage = admitted.stage().unwrap();
    let (tree, evidence) = material.prepare(stage, ArchivePolicy::default()).unwrap();
    assert!(!target.exists());
    let PreparationEvidence::Extracted(evidence) = evidence else {
        panic!("expected extraction")
    };
    assert_eq!(evidence.format, kind);
    assert_eq!(evidence.entries, 1);
    assert_eq!(evidence.files, 1);
    assert_eq!(
        std::fs::read(tree.root().join("bin/tool.txt")).unwrap(),
        b"tar payload"
    );

    tree.publish(admitted).unwrap();
    assert_eq!(
        std::fs::read(target.join("bin/tool.txt")).unwrap(),
        b"tar payload"
    );
}

#[cfg(feature = "zip")]
#[test]
fn unsafe_archive_preparation_leaves_no_final_target_or_contaminated_workspace() {
    let root = tempfile::tempdir().unwrap();
    let archive = root.path().join("unsafe.zip");
    let target = root.path().join("final-tree");
    let mut writer = zip::ZipWriter::new(std::fs::File::create(&archive).unwrap());
    writer
        .start_file("../escape.txt", zip::write::SimpleFileOptions::default())
        .unwrap();
    writer.write_all(b"unsafe payload").unwrap();
    writer.finish().unwrap();

    let material = LocalSource::new(archive).unwrap().acquire().unwrap();
    let admitted = LocalTarget::new(target.clone()).unwrap();
    let stage = admitted.stage().unwrap();
    assert!(material.prepare(stage, ArchivePolicy::default()).is_err());

    assert!(!target.exists());
    assert!(!root.path().join("escape.txt").exists());
}

#[test]
fn forget_applies_directly_without_a_synthetic_predecessor() {
    let root = tempfile::tempdir().unwrap();
    let target = root.path().join("obsolete");
    std::fs::write(&target, b"obsolete").unwrap();

    LocalTarget::new(target.clone()).unwrap().remove().unwrap();

    assert!(!target.exists());
}

#[cfg(all(feature = "blake3", any(feature = "http-sync", feature = "http-async")))]
#[test]
fn local_file_materialization_verification_and_reconciliation() {
    let body = b"local artifact";
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source");
    let target = root.path().join("local-artifact");
    std::fs::write(&source, body).unwrap();

    let material = LocalSource::new(source).unwrap().acquire().unwrap();
    let admitted = LocalTarget::new(target.clone()).unwrap();
    let stage = admitted.stage().unwrap();
    let (tree, _) = material.prepare(stage, ArchivePolicy::default()).unwrap();
    tree.publish(admitted).unwrap();

    assert_applied_then_reconciled(&target, body);
}

/// A plain placeholder so the http-async-only build keeps a non-empty target.
#[allow(dead_code)]
fn _unused() {
    let _ = PathBuf::new();
    let _ = LocalObservation::Missing;
    let _ = ArchiveKind::Plain;
}
