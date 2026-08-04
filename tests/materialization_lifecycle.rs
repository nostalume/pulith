#![cfg(feature = "local")]

#[cfg(any(feature = "http-sync", feature = "http-async"))]
use std::io::Read;
#[cfg(any(feature = "http-sync", feature = "http-async", feature = "zip"))]
use std::io::Write;
#[cfg(any(feature = "http-sync", feature = "http-async"))]
use std::net::TcpListener;
#[cfg(any(feature = "http-sync", feature = "http-async"))]
use std::thread;

#[cfg(any(feature = "http-sync", feature = "zip", feature = "tar"))]
use pulith::Acquire;
#[cfg(any(feature = "zip", feature = "tar"))]
use pulith::Prepare;
#[cfg(all(feature = "blake3", any(feature = "http-sync", feature = "http-async")))]
use pulith::Verify;
#[cfg(all(feature = "blake3", any(feature = "http-sync", feature = "http-async")))]
use pulith::hash::{ArtifactDescriptor, Blake3, HashVerify};
#[cfg(any(feature = "zip", feature = "tar"))]
use pulith::local::{LocalAcquire, LocalPath};
use pulith::local::{LocalApply, LocalTarget};
#[cfg(any(feature = "http-sync", feature = "http-async"))]
use pulith::local::{LocalExpectation, LocalInspect, LocalReconcile, LocalReconciliation};
#[cfg(any(feature = "http-sync", feature = "http-async"))]
use pulith::net::{RemoteSource, RemoteUrl};
use pulith::{Apply, Forget};
#[cfg(any(feature = "http-sync", feature = "http-async"))]
use pulith::{Inspect, Reconcile};
#[cfg(any(
    feature = "http-sync",
    feature = "http-async",
    feature = "zip",
    feature = "tar"
))]
use pulith::{Materialize, MaterializeMode};

#[cfg(any(feature = "http-sync", feature = "http-async"))]
struct HttpFixture {
    url: String,
    handle: thread::JoinHandle<()>,
}

#[cfg(any(feature = "http-sync", feature = "http-async"))]
impl HttpFixture {
    fn get(body: &'static [u8]) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0_u8; 1_024];
            let _ = stream.read(&mut request).unwrap();
            stream
                .write_all(
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    )
                    .as_bytes(),
                )
                .unwrap();
            stream.write_all(body).unwrap();
            stream.flush().unwrap();
        });
        Self {
            url: format!("http://{address}/artifact"),
            handle,
        }
    }

    fn join(self) {
        self.handle.join().unwrap();
    }
}

#[cfg(all(feature = "blake3", any(feature = "http-sync", feature = "http-async")))]
fn descriptor(body: &[u8]) -> ArtifactDescriptor<Blake3> {
    ArtifactDescriptor::new(blake3::hash(body).to_hex().to_string(), body.len() as u64)
}

#[cfg(all(feature = "blake3", any(feature = "http-sync", feature = "http-async")))]
fn assert_applied_then_reconciled(target: &std::path::Path, body: &[u8]) {
    assert_eq!(std::fs::read(target).unwrap(), body);

    let inspected = LocalInspect.inspect(LocalTarget::new(target)).unwrap();
    let reconciled = LocalReconcile
        .reconcile(inspected, LocalExpectation::FileSize(body.len() as u64))
        .unwrap();
    assert_eq!(reconciled.reconciliation, LocalReconciliation::Matches);
    assert_eq!(reconciled.input.path, target);
}

#[cfg(any(feature = "zip", feature = "tar"))]
fn acquire_local_archive(
    archive: &std::path::Path,
    target: &std::path::Path,
) -> pulith::Acquired<
    Materialize<&'static str, LocalPath, LocalTarget>,
    pulith::local::LocalMaterial,
    pulith::local::LocalAcquireEvidence,
> {
    LocalAcquire
        .acquire(Materialize::new(
            "archive-flow",
            LocalPath::new(archive),
            LocalTarget::new(target),
            MaterializeMode::ReplaceOrCreate,
        ))
        .unwrap()
}

#[cfg(all(feature = "blake3", feature = "http-sync"))]
#[test]
fn sync_http_materialization_verification_and_reconciliation() {
    use pulith::net::SyncHttpAcquire;

    let body = b"sync artifact";
    let server = HttpFixture::get(body);
    let root = tempfile::tempdir().unwrap();
    let target = root.path().join("sync-artifact");
    let request = Materialize::new(
        "sync-flow",
        RemoteSource::new(RemoteUrl::parse(&server.url).unwrap()),
        LocalTarget::new(&target),
        MaterializeMode::CreateNew,
    );

    let acquired = SyncHttpAcquire::default().acquire(request).unwrap();
    server.join();
    assert!(!target.exists());
    assert_eq!(acquired.evidence.status, 200);
    assert_eq!(acquired.evidence.bytes, body.len() as u64);

    let verified = HashVerify::<Blake3>::new()
        .verify(acquired, descriptor(body))
        .unwrap();
    assert_eq!(verified.evidence.previous.bytes, body.len() as u64);
    assert_eq!(verified.evidence.current.expected, descriptor(body));
    let applied = LocalApply.apply(verified).unwrap();

    assert_eq!(applied.input.item, "sync-flow");
    assert_applied_then_reconciled(&target, body);
}

#[cfg(all(feature = "blake3", feature = "http-async"))]
#[test]
fn async_http_materialization_verification_and_reconciliation() {
    use pulith::AsyncAcquire;
    use pulith::net::AsyncHttpAcquire;

    let body = b"async artifact";
    let server = HttpFixture::get(body);
    let root = tempfile::tempdir().unwrap();
    let target = root.path().join("async-artifact");
    let request = Materialize::new(
        "async-flow",
        RemoteSource::new(RemoteUrl::parse(&server.url).unwrap()),
        LocalTarget::new(&target),
        MaterializeMode::CreateNew,
    );

    let acquired = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(AsyncHttpAcquire::default().acquire_async(request))
        .unwrap();
    server.join();
    assert!(!target.exists());
    assert_eq!(acquired.evidence.status, 200);
    assert_eq!(acquired.evidence.bytes, body.len() as u64);

    let verified = HashVerify::<Blake3>::new()
        .verify(acquired, descriptor(body))
        .unwrap();
    assert_eq!(verified.evidence.previous.bytes, body.len() as u64);
    assert_eq!(verified.evidence.current.expected, descriptor(body));
    let applied = LocalApply.apply(verified).unwrap();

    assert_eq!(applied.input.item, "async-flow");
    assert_applied_then_reconciled(&target, body);
}

#[cfg(all(feature = "blake3", feature = "http-sync"))]
#[test]
fn failed_http_verification_never_publishes_the_adapter_owned_stage() {
    use pulith::net::SyncHttpAcquire;

    let body = b"untrusted artifact";
    let server = HttpFixture::get(body);
    let root = tempfile::tempdir().unwrap();
    let target = root.path().join("unpublished-artifact");
    let request = Materialize::new(
        "verify-failure",
        RemoteSource::new(RemoteUrl::parse(&server.url).unwrap()),
        LocalTarget::new(&target),
        MaterializeMode::CreateNew,
    );

    let acquired = SyncHttpAcquire::default().acquire(request).unwrap();
    server.join();
    let wrong = descriptor(b"different artifact");
    assert!(HashVerify::<Blake3>::new().verify(acquired, wrong).is_err());

    assert!(!target.exists());
}

#[cfg(feature = "zip")]
#[test]
fn zip_acquire_prepare_apply_keeps_scratch_and_final_authority_separate() {
    use pulith::archive::{ArchivePolicy, ArchivePrepare, ExtractWorkspace, Zip};

    let root = tempfile::tempdir().unwrap();
    let archive = root.path().join("artifact.zip");
    let scratch = root.path().join("scratch");
    let target = root.path().join("final-tree");
    let mut writer = zip::ZipWriter::new(std::fs::File::create(&archive).unwrap());
    writer
        .start_file("bin/tool.txt", zip::write::SimpleFileOptions::default())
        .unwrap();
    writer.write_all(b"zip payload").unwrap();
    writer.finish().unwrap();

    let prepared = ArchivePrepare::<Zip>::new(ExtractWorkspace::new(&scratch))
        .prepare(
            acquire_local_archive(&archive, &target),
            ArchivePolicy::default(),
        )
        .unwrap();
    assert!(!target.exists());
    assert_eq!(prepared.evidence.current.entries, 1);
    assert_eq!(prepared.evidence.current.files, 1);
    assert_eq!(
        std::fs::read(prepared.prepared.root().join("bin/tool.txt")).unwrap(),
        b"zip payload"
    );

    LocalApply.apply(prepared).unwrap();
    assert_eq!(
        std::fs::read(target.join("bin/tool.txt")).unwrap(),
        b"zip payload"
    );
}

#[cfg(feature = "tar")]
#[test]
fn tar_acquire_prepare_apply_keeps_scratch_and_final_authority_separate() {
    use pulith::archive::{ArchivePolicy, ArchivePrepare, ExtractWorkspace, Tar};

    let root = tempfile::tempdir().unwrap();
    let archive = root.path().join("artifact.tar");
    let scratch = root.path().join("scratch");
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

    let prepared = ArchivePrepare::<Tar>::new(ExtractWorkspace::new(&scratch))
        .prepare(
            acquire_local_archive(&archive, &target),
            ArchivePolicy::default(),
        )
        .unwrap();
    assert!(!target.exists());
    assert_eq!(prepared.evidence.current.entries, 1);
    assert_eq!(prepared.evidence.current.files, 1);
    assert_eq!(
        std::fs::read(prepared.prepared.root().join("bin/tool.txt")).unwrap(),
        b"tar payload"
    );

    LocalApply.apply(prepared).unwrap();
    assert_eq!(
        std::fs::read(target.join("bin/tool.txt")).unwrap(),
        b"tar payload"
    );
}

#[cfg(feature = "zip")]
#[test]
fn unsafe_archive_preparation_leaves_no_final_target_or_contaminated_workspace() {
    use pulith::archive::{ArchivePolicy, ArchivePrepare, ExtractWorkspace, Zip};

    let root = tempfile::tempdir().unwrap();
    let archive = root.path().join("unsafe.zip");
    let scratch = root.path().join("scratch");
    let target = root.path().join("final-tree");
    let mut writer = zip::ZipWriter::new(std::fs::File::create(&archive).unwrap());
    writer
        .start_file("../escape.txt", zip::write::SimpleFileOptions::default())
        .unwrap();
    writer.write_all(b"unsafe payload").unwrap();
    writer.finish().unwrap();

    assert!(
        ArchivePrepare::<Zip>::new(ExtractWorkspace::new(&scratch))
            .prepare(
                acquire_local_archive(&archive, &target),
                ArchivePolicy::default(),
            )
            .is_err()
    );

    assert!(!target.exists());
    assert_eq!(std::fs::read_dir(&scratch).unwrap().count(), 0);
    assert!(!root.path().join("escape.txt").exists());
}

#[test]
fn forget_applies_directly_without_a_synthetic_predecessor() {
    let root = tempfile::tempdir().unwrap();
    let target = root.path().join("obsolete");
    std::fs::write(&target, b"obsolete").unwrap();

    let applied = LocalApply
        .apply(Forget::new("forget-flow", LocalTarget::new(&target)))
        .unwrap();

    assert!(!target.exists());
    assert_eq!(applied.input.item, "forget-flow");
}
