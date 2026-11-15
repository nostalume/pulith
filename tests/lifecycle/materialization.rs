#![cfg(feature = "local")]

#[cfg(feature = "http-ureq")]
use crate::common::HttpFixture;
#[cfg(any(feature = "http-ureq", feature = "zip"))]
use std::io::Write;

#[cfg(all(feature = "blake3", feature = "http-ureq"))]
use pulith::archive::ArchivePolicy;
#[cfg(all(feature = "blake3", feature = "http-ureq"))]
use pulith::hash::{DigestAlgorithmKind, DigestValue};
use pulith::local::LocalPath;
#[cfg(feature = "http-ureq")]
use pulith::local::{LocalExpectation, LocalReconciliation};
#[cfg(feature = "http-ureq")]
use pulith::net::{RemoteSource, RemoteUrl, UreqResources};

#[cfg(all(feature = "blake3", feature = "http-ureq"))]
fn assert_applied_then_reconciled(target: &std::path::Path, body: &[u8]) {
    assert_eq!(std::fs::read(target).unwrap(), body);

    let observation = LocalPath::new(target.to_path_buf())
        .unwrap()
        .inspect_entry()
        .unwrap();
    let reconciliation = observation.compare(&LocalExpectation::FileSize(body.len() as u64));
    assert_eq!(reconciliation, LocalReconciliation::Matches);
}

#[cfg(all(feature = "blake3", feature = "http-ureq"))]
#[test]
fn sync_http_acquire_verifies_prepares_and_applies_with_evidence() {
    let body = b"sync artifact";
    let server = HttpFixture::get(body);
    let root = tempfile::tempdir().unwrap();
    let target = root.path().join("sync-artifact");

    let (artifact, evidence) = UreqResources::default()
        .download(RemoteSource::new(RemoteUrl::parse(&server.url).unwrap()))
        .unwrap();
    server.join();
    assert_eq!(evidence.status, 200);
    assert_eq!(evidence.bytes, body.len() as u64);

    let expected = DigestValue::new(
        DigestAlgorithmKind::Blake3,
        blake3::hash(body).to_hex().to_string(),
    )
    .unwrap();
    let (material, digest_evidence) = artifact.verify_digest(&expected).unwrap();
    assert_eq!(digest_evidence.digest, expected);
    let admitted = LocalPath::new(target.clone()).unwrap();
    let stage = admitted.stage().unwrap();
    let tree = material.stage_into(stage).unwrap();
    tree.publish(admitted, pulith::local::TreeAdmission::Absent)
        .unwrap();
    assert_applied_then_reconciled(&target, body);
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

    let admitted = LocalPath::new(target.clone()).unwrap();
    let stage = admitted.stage().unwrap();
    let evidence = pulith::archive::extract_zip(
        std::fs::File::open(&archive).unwrap(),
        stage.root(),
        ArchivePolicy::default(),
    )
    .unwrap();
    assert!(!target.exists());
    assert_eq!(evidence.entries, 1);
    assert_eq!(evidence.files, 1);
    assert_eq!(
        std::fs::read(stage.root().join("bin/tool.txt")).unwrap(),
        b"zip payload"
    );

    stage
        .publish(admitted, pulith::local::TreeAdmission::Absent)
        .unwrap();
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

    let admitted = LocalPath::new(target.clone()).unwrap();
    let stage = admitted.stage().unwrap();
    let evidence = pulith::archive::extract_tar(
        std::fs::File::open(&archive).unwrap(),
        stage.root(),
        ArchivePolicy::default(),
    )
    .unwrap();
    assert!(!target.exists());
    assert_eq!(evidence.entries, 1);
    assert_eq!(evidence.files, 1);
    assert_eq!(
        std::fs::read(stage.root().join("bin/tool.txt")).unwrap(),
        b"tar payload"
    );

    stage
        .publish(admitted, pulith::local::TreeAdmission::Absent)
        .unwrap();
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

    let admitted = LocalPath::new(target.clone()).unwrap();
    let stage = admitted.stage().unwrap();
    assert!(
        pulith::archive::extract_zip(
            std::fs::File::open(&archive).unwrap(),
            stage.root(),
            ArchivePolicy::default()
        )
        .is_err()
    );

    assert!(!target.exists());
    assert!(!root.path().join("escape.txt").exists());
}

#[test]
fn forget_applies_directly_without_a_synthetic_predecessor() {
    let root = tempfile::tempdir().unwrap();
    let target = root.path().join("obsolete");
    std::fs::write(&target, b"obsolete").unwrap();

    LocalPath::new(target.clone()).unwrap().remove().unwrap();

    assert!(!target.exists());
}

#[cfg(all(feature = "blake3", feature = "http-ureq"))]
#[test]
fn local_file_materialization_verification_and_reconciliation() {
    let body = b"local artifact";
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source");
    let target = root.path().join("local-artifact");
    std::fs::write(&source, body).unwrap();

    let expected = DigestValue::new(
        DigestAlgorithmKind::Blake3,
        blake3::hash(body).to_hex().to_string(),
    )
    .unwrap();
    let input = LocalPath::new(source)
        .unwrap()
        .open_file()
        .unwrap()
        .unwrap();
    let destination = LocalPath::new(&target).unwrap();
    let (publication, observed) = destination.publish_verified_file(input, &expected).unwrap();
    assert_eq!(publication.bytes, body.len() as u64);
    assert_eq!(observed.digest, expected);

    assert_applied_then_reconciled(&target, body);
}
