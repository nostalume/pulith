#![cfg(feature = "net")]

use pulith::net::{RemoteUrl, RemoteUrlError};

#[cfg(any(feature = "http-ureq", feature = "http-reqwest"))]
use std::net::TcpListener;
#[cfg(any(feature = "http-ureq", feature = "http-reqwest"))]
use std::num::NonZeroU32;
#[cfg(any(feature = "http-ureq", feature = "http-reqwest"))]
use std::time::Duration;

#[cfg(feature = "http-ureq")]
use pulith::{Acquire, Inspect};
#[cfg(feature = "http-reqwest")]
use pulith::{AsyncAcquire, AsyncInspect};

#[cfg(any(feature = "http-ureq", feature = "http-reqwest"))]
use pulith::local::LocalMaterial;
#[cfg(feature = "http-reqwest")]
use pulith::net::ReqwestResources;
#[cfg(any(feature = "http-ureq", feature = "http-reqwest"))]
use pulith::net::{
    AcquireError, AcquirePolicy, AttemptOutcome, AttemptRate, HttpInspectError, HttpInspectPolicy,
    RateAdmission, RemoteAcquireEvidence, RemoteInspectEvidence, RemoteObservation, RemoteSource,
    RetryPolicy,
};
#[cfg(feature = "http-ureq")]
use pulith::net::{TransportPhase, UreqResources};

#[test]
fn remote_url_uses_resource_specific_errors() {
    assert!(matches!(
        RemoteUrl::parse("file:///tmp/pulith"),
        Err(RemoteUrlError::UnsupportedScheme { .. })
    ));
    assert!(matches!(
        RemoteUrl::parse("not a URL"),
        Err(RemoteUrlError::Invalid { .. })
    ));
    assert!(RemoteUrl::parse("http://example.com/pulith").is_ok());
}

#[cfg(feature = "http-ureq")]
#[test]
fn remote_source_implements_sync_acquire_contract() {
    fn assert_acquire<
        T: Acquire<Error = AcquireError, Output = (LocalMaterial, RemoteAcquireEvidence)>,
    >() {
    }
    assert_acquire::<pulith::net::PreparedRemote>();

    let source = RemoteSource::new(RemoteUrl::parse("http://example.com/resource").unwrap());
    assert_eq!(source.url().as_str(), "http://example.com/resource");
}

#[cfg(feature = "http-ureq")]
#[test]
fn remote_source_acquire_continues_as_local_material() {
    fn assert_acquire<
        T: Acquire<Error = AcquireError, Output = (LocalMaterial, RemoteAcquireEvidence)>,
    >() {
    }

    assert_acquire::<pulith::net::PreparedRemote>();
}

#[cfg(feature = "http-reqwest")]
#[test]
fn remote_source_implements_async_acquire_contract() {
    fn assert_acquire<
        T: AsyncAcquire<Error = AcquireError, Output = (LocalMaterial, RemoteAcquireEvidence)>,
    >() {
    }
    assert_acquire::<pulith::net::PreparedRemote>();

    fn assert_send(_: impl Send) {}
    let prepared = RemoteSource::new(RemoteUrl::parse("http://example.com/resource").unwrap())
        .prepare()
        .unwrap();
    assert_send(AsyncAcquire::acquire(prepared));
}

#[cfg(feature = "http-ureq")]
#[test]
fn remote_url_implements_sync_inspect_contract() {
    fn assert_inspect<
        T: Inspect<(), Error = HttpInspectError, Output = (RemoteObservation, RemoteInspectEvidence)>,
    >() {
    }
    assert_inspect::<RemoteUrl>();
}

#[cfg(feature = "http-reqwest")]
#[test]
fn remote_url_implements_async_inspect_contract() {
    fn assert_inspect<
        T: AsyncInspect<Error = HttpInspectError, Output = (RemoteObservation, RemoteInspectEvidence)>,
    >() {
    }
    assert_inspect::<RemoteUrl>();
}

#[cfg(feature = "http-ureq")]
#[test]
fn sync_acquire_stages_artifact_with_evidence() {
    let body = b"sync acquire body";
    let server = serve_once(200, body, &[]);
    let url = server.url.clone();
    let source = RemoteSource::from_url_str(&server.url).unwrap();

    let (artifact, evidence) = Acquire::acquire(source.prepare().unwrap()).unwrap();
    let request = server.next_request();
    server.join();

    assert!(request.starts_with("GET /artifact.bin "));
    let LocalMaterial::StagedFile { path } = artifact else {
        panic!("remote acquisition must keep staged-file custody")
    };
    assert_eq!(std::fs::read(&path).unwrap(), body);
    assert_eq!(evidence.url.as_str(), url);
    assert_eq!(evidence.status, 200);
    assert_eq!(evidence.bytes, body.len() as u64);
    assert_eq!(evidence.content_length, Some(body.len() as u64));
    assert_eq!(evidence.resume, None);
    assert_eq!(evidence.attempts.len(), 1);
    assert_eq!(evidence.attempts[0].outcome, AttemptOutcome::Success);
}

#[cfg(feature = "http-reqwest")]
#[test]
fn async_acquire_stages_artifact_with_evidence() {
    let body = b"async acquire body";
    let server = serve_once(200, body, &[]);
    let url = server.url.clone();
    let source = RemoteSource::new(RemoteUrl::parse(&server.url).unwrap());

    let (artifact, evidence) = block_on(AsyncAcquire::acquire(source.prepare().unwrap())).unwrap();
    let request = server.next_request();
    server.join();

    assert!(request.starts_with("GET /artifact.bin "));
    let LocalMaterial::StagedFile { path } = artifact else {
        panic!("remote acquisition must keep staged-file custody")
    };
    assert_eq!(std::fs::read(&path).unwrap(), body);
    assert_eq!(evidence.url.as_str(), url);
    assert_eq!(evidence.status, 200);
    assert_eq!(evidence.bytes, body.len() as u64);
    assert_eq!(evidence.content_length, Some(body.len() as u64));
    assert_eq!(evidence.resume, None);
    assert_eq!(evidence.attempts.len(), 1);
    assert_eq!(evidence.attempts[0].outcome, AttemptOutcome::Success);
}

#[cfg(feature = "http-ureq")]
#[test]
fn sync_inspect_reports_observation_with_evidence() {
    let body = b"body not materialized by inspect";
    let server = serve_once(200, body, &[]);
    let url = server.url.clone();

    let (observation, evidence) =
        Inspect::inspect(RemoteUrl::parse(&server.url).unwrap(), ()).unwrap();
    let request = server.next_request();
    server.join();

    assert!(request.starts_with("HEAD /artifact.bin "));
    assert_eq!(observation.status, 200);
    assert_eq!(observation.declared_content_length, Some(body.len() as u64));
    assert_eq!(evidence.requested_url.as_str(), url);
    assert_eq!(evidence.requested_url, evidence.final_url);
    assert_eq!(evidence.attempts.len(), 1);
    assert_eq!(evidence.attempts[0].status, Some(200));
}

#[cfg(feature = "http-reqwest")]
#[test]
fn async_inspect_reports_observation_with_evidence() {
    let body = b"body not materialized by inspect";
    let server = serve_once(200, body, &[]);
    let url = server.url.clone();

    let (observation, evidence) = block_on(AsyncInspect::inspect(
        RemoteUrl::parse(&server.url).unwrap(),
    ))
    .unwrap();
    let request = server.next_request();
    server.join();

    assert!(request.starts_with("HEAD /artifact.bin "));
    assert_eq!(observation.status, 200);
    assert_eq!(observation.declared_content_length, Some(body.len() as u64));
    assert_eq!(evidence.requested_url.as_str(), url);
    assert_eq!(evidence.requested_url, evidence.final_url);
    assert_eq!(evidence.attempts.len(), 1);
    assert_eq!(evidence.attempts[0].status, Some(200));
}

#[cfg(feature = "http-ureq")]
#[test]
fn sync_acquire_retries_retryable_status_then_succeeds() {
    let body = b"retried body";
    let server = serve_sequence(vec![(503, b"", &[]), (200, body, &[])]);
    let policy = AcquirePolicy::default().retry(RetryPolicy::exponential(1, Duration::ZERO));
    let source = RemoteSource::new(RemoteUrl::parse(&server.url).unwrap()).policy(policy);

    let (artifact, evidence) = Acquire::acquire(source.prepare().unwrap()).unwrap();
    assert!(server.next_request().starts_with("GET "));
    assert!(server.next_request().starts_with("GET "));
    server.join();

    let LocalMaterial::StagedFile { path } = artifact else {
        panic!("remote acquisition must keep staged-file custody")
    };
    assert_eq!(std::fs::read(&path).unwrap(), body);
    assert_eq!(evidence.status, 200);
    assert_eq!(evidence.attempts.len(), 2);
    assert_eq!(evidence.attempts[0].status, Some(503));
    assert_eq!(
        evidence.attempts[0].outcome,
        AttemptOutcome::RetryableStatus
    );
    assert_eq!(evidence.attempts[1].status, Some(200));
    assert_eq!(evidence.attempts[1].outcome, AttemptOutcome::Success);
}

#[cfg(feature = "http-reqwest")]
#[test]
fn async_acquire_retries_retryable_status_then_succeeds() {
    let body = b"retried body";
    let server = serve_sequence(vec![(503, b"", &[]), (200, body, &[])]);
    let policy = AcquirePolicy::default().retry(RetryPolicy::exponential(1, Duration::ZERO));
    let source = RemoteSource::new(RemoteUrl::parse(&server.url).unwrap()).policy(policy);

    let (artifact, evidence) = block_on(AsyncAcquire::acquire(source.prepare().unwrap())).unwrap();
    assert!(server.next_request().starts_with("GET "));
    assert!(server.next_request().starts_with("GET "));
    server.join();

    let LocalMaterial::StagedFile { path } = artifact else {
        panic!("remote acquisition must keep staged-file custody")
    };
    assert_eq!(std::fs::read(&path).unwrap(), body);
    assert_eq!(evidence.status, 200);
    assert_eq!(evidence.attempts.len(), 2);
    assert_eq!(evidence.attempts[0].status, Some(503));
    assert_eq!(
        evidence.attempts[0].outcome,
        AttemptOutcome::RetryableStatus
    );
    assert_eq!(evidence.attempts[1].status, Some(200));
    assert_eq!(evidence.attempts[1].outcome, AttemptOutcome::Success);
}

#[cfg(feature = "http-ureq")]
#[test]
fn sync_inspect_retries_status_and_returns_final_observation() {
    let server = serve_sequence(vec![(503, b"", &[]), (200, b"final", &[])]);
    let policy = HttpInspectPolicy::default().retry(RetryPolicy::exponential(1, Duration::ZERO));

    let (observation, evidence) = Inspect::inspect(
        RemoteUrl::parse(&server.url)
            .unwrap()
            .with_inspect_policy(policy),
        (),
    )
    .unwrap();
    assert!(server.next_request().starts_with("HEAD "));
    assert!(server.next_request().starts_with("HEAD "));
    server.join();

    assert_eq!(observation.status, 200);
    assert_eq!(evidence.attempts.len(), 2);
    assert_eq!(evidence.attempts[0].status, Some(503));
    assert_eq!(evidence.attempts[0].planned_delay, Some(Duration::ZERO));
    assert_eq!(evidence.attempts[1].status, Some(200));
    assert_eq!(evidence.attempts[1].planned_delay, None);
}

#[cfg(feature = "http-reqwest")]
#[test]
fn async_inspect_retries_status_and_returns_final_observation() {
    let server = serve_sequence(vec![(503, b"", &[]), (200, b"final", &[])]);
    let policy = HttpInspectPolicy::default().retry(RetryPolicy::exponential(1, Duration::ZERO));

    let (observation, evidence) = block_on(AsyncInspect::inspect(
        RemoteUrl::parse(&server.url)
            .unwrap()
            .with_inspect_policy(policy),
    ))
    .unwrap();
    assert!(server.next_request().starts_with("HEAD "));
    assert!(server.next_request().starts_with("HEAD "));
    server.join();

    assert_eq!(observation.status, 200);
    assert_eq!(evidence.attempts.len(), 2);
    assert_eq!(evidence.attempts[0].status, Some(503));
    assert_eq!(evidence.attempts[0].planned_delay, Some(Duration::ZERO));
    assert_eq!(evidence.attempts[1].status, Some(200));
    assert_eq!(evidence.attempts[1].planned_delay, None);
}

#[cfg(feature = "http-ureq")]
#[test]
fn sync_acquire_reports_non_retryable_http_status() {
    let server = serve_once(404, b"not found", &[]);
    let source = RemoteSource::new(RemoteUrl::parse(&server.url).unwrap());

    let error = Acquire::acquire(source.prepare().unwrap()).unwrap_err();
    server.join();

    let attempts = match error {
        AcquireError::HttpStatus {
            status: 404,
            retryable: false,
            attempts,
            ..
        } => attempts,
        other => panic!("expected 404 status error, got {other:?}"),
    };
    assert_eq!(attempts.len(), 1);
    assert_eq!(attempts[0].status, Some(404));
    assert_eq!(attempts[0].outcome, AttemptOutcome::NonRetryableStatus);
}

#[cfg(feature = "http-reqwest")]
#[test]
fn async_acquire_reports_non_retryable_http_status() {
    let server = serve_once(404, b"not found", &[]);
    let source = RemoteSource::new(RemoteUrl::parse(&server.url).unwrap());

    let error = block_on(AsyncAcquire::acquire(source.prepare().unwrap())).unwrap_err();
    server.join();

    let attempts = match error {
        AcquireError::HttpStatus {
            status: 404,
            retryable: false,
            attempts,
            ..
        } => attempts,
        other => panic!("expected 404 status error, got {other:?}"),
    };
    assert_eq!(attempts.len(), 1);
    assert_eq!(attempts[0].status, Some(404));
    assert_eq!(attempts[0].outcome, AttemptOutcome::NonRetryableStatus);
}

#[cfg(feature = "http-ureq")]
#[test]
fn sync_acquire_reports_transport_error_with_attempt_evidence() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    drop(listener);

    let error = Acquire::acquire(
        RemoteSource::new(RemoteUrl::parse(&format!("http://{address}/resource")).unwrap())
            .prepare()
            .unwrap(),
    )
    .unwrap_err();

    match error {
        AcquireError::Transport {
            phase: TransportPhase::SendRequest,
            attempts,
            ..
        } => {
            assert_eq!(attempts.len(), 1);
            assert_eq!(attempts[0].status, None);
            assert_eq!(attempts[0].outcome, AttemptOutcome::RetryableNetworkError);
        }
        other => panic!("expected transport error, got {other:?}"),
    }
}

#[cfg(feature = "http-reqwest")]
#[test]
fn async_inspect_reports_transport_error_with_attempt_evidence() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    drop(listener);

    let error = block_on(AsyncInspect::inspect(
        RemoteUrl::parse(&format!("http://{address}/resource")).unwrap(),
    ))
    .unwrap_err();

    match error {
        HttpInspectError::Transport { attempts, .. } => {
            assert_eq!(attempts.len(), 1);
            assert_eq!(attempts[0].status, None);
        }
        other => panic!("expected transport error, got {other:?}"),
    }
}

#[cfg(feature = "http-ureq")]
#[test]
fn sync_acquire_waits_on_shared_rate_admission() {
    let admission = RateAdmission::new(AttemptRate::new(
        NonZeroU32::new(1).unwrap(),
        NonZeroU32::new(1).unwrap(),
    ));
    let resources = UreqResources::default().with_admission(std::sync::Arc::new(admission));
    let server = serve_sequence(vec![(200, b"first", &[]), (200, b"second", &[])]);
    let url = RemoteUrl::parse(&server.url).unwrap();

    let (first, first_evidence) = Acquire::acquire(
        RemoteSource::new(url.clone())
            .with_ureq(resources.clone())
            .prepare()
            .unwrap(),
    )
    .unwrap();
    let (second, second_evidence) = Acquire::acquire(
        RemoteSource::new(url)
            .with_ureq(resources)
            .prepare()
            .unwrap(),
    )
    .unwrap();
    server.join();

    let LocalMaterial::StagedFile { path: first_path } = first else {
        panic!("remote acquisition must keep staged-file custody")
    };
    let LocalMaterial::StagedFile { path: second_path } = second else {
        panic!("remote acquisition must keep staged-file custody")
    };
    assert_eq!(std::fs::read(&first_path).unwrap(), b"first");
    assert_eq!(std::fs::read(&second_path).unwrap(), b"second");
    assert_eq!(
        first_evidence.attempts[0].admission_wait,
        Some(Duration::ZERO)
    );
    assert!(second_evidence.attempts[0].admission_wait.unwrap() > Duration::ZERO);
}

#[cfg(feature = "http-reqwest")]
#[test]
fn async_acquire_waits_on_shared_rate_admission() {
    let admission = RateAdmission::new(AttemptRate::new(
        NonZeroU32::new(1).unwrap(),
        NonZeroU32::new(1).unwrap(),
    ));
    let resources = ReqwestResources::default().with_admission(std::sync::Arc::new(admission));
    let server = serve_sequence(vec![(200, b"first", &[]), (200, b"second", &[])]);
    let url = RemoteUrl::parse(&server.url).unwrap();

    let (first, first_evidence) = block_on(AsyncAcquire::acquire(
        RemoteSource::new(url.clone())
            .with_reqwest(resources.clone())
            .prepare()
            .unwrap(),
    ))
    .unwrap();
    let (second, second_evidence) = block_on(AsyncAcquire::acquire(
        RemoteSource::new(url)
            .with_reqwest(resources)
            .prepare()
            .unwrap(),
    ))
    .unwrap();
    server.join();

    let LocalMaterial::StagedFile { path: first_path } = first else {
        panic!("remote acquisition must keep staged-file custody")
    };
    let LocalMaterial::StagedFile { path: second_path } = second else {
        panic!("remote acquisition must keep staged-file custody")
    };
    assert_eq!(std::fs::read(&first_path).unwrap(), b"first");
    assert_eq!(std::fs::read(&second_path).unwrap(), b"second");
    assert_eq!(
        first_evidence.attempts[0].admission_wait,
        Some(Duration::ZERO)
    );
    assert!(second_evidence.attempts[0].admission_wait.unwrap() > Duration::ZERO);
}

#[cfg(feature = "http-reqwest")]
fn block_on<F: std::future::Future>(future: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(future)
}

#[cfg(any(feature = "http-ureq", feature = "http-reqwest"))]
type TestResponse = (u16, &'static [u8], &'static [(&'static str, &'static str)]);

#[cfg(any(feature = "http-ureq", feature = "http-reqwest"))]
struct TestServer {
    url: String,
    handle: std::thread::JoinHandle<()>,
    requests: std::sync::mpsc::Receiver<String>,
}

#[cfg(any(feature = "http-ureq", feature = "http-reqwest"))]
impl TestServer {
    fn next_request(&self) -> String {
        self.requests.recv().unwrap()
    }

    fn join(self) {
        self.handle.join().unwrap();
    }
}

#[cfg(any(feature = "http-ureq", feature = "http-reqwest"))]
fn serve_once(
    status: u16,
    body: &'static [u8],
    headers: &'static [(&'static str, &'static str)],
) -> TestServer {
    serve_sequence(vec![(status, body, headers)])
}

#[cfg(any(feature = "http-ureq", feature = "http-reqwest"))]
fn serve_sequence(responses: Vec<TestResponse>) -> TestServer {
    use std::net::TcpListener;
    use std::sync::mpsc;

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let (requests, request_rx) = mpsc::channel();
    let handle = std::thread::spawn(move || {
        for (status, body, headers) in responses {
            let (stream, _) = listener.accept().unwrap();
            handle_request(stream, status, body, headers, &requests);
        }
    });
    TestServer {
        url: format!("http://{address}/artifact.bin"),
        handle,
        requests: request_rx,
    }
}

#[cfg(any(feature = "http-ureq", feature = "http-reqwest"))]
fn handle_request(
    mut stream: std::net::TcpStream,
    status: u16,
    body: &[u8],
    headers: &[(&str, &str)],
    requests: &std::sync::mpsc::Sender<String>,
) {
    use std::io::{BufRead, BufReader, Write};

    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let mut line = String::new();
    let mut request = String::new();
    loop {
        line.clear();
        reader.read_line(&mut line).unwrap();
        request.push_str(&line);
        if line == "\r\n" || line.is_empty() {
            break;
        }
    }
    requests.send(request).unwrap();

    let reason = match status {
        200 => "OK",
        404 => "Not Found",
        503 => "Service Unavailable",
        _ => "Status",
    };
    write!(
        stream,
        "HTTP/1.1 {status} {reason}\r\nContent-Length: {}\r\nConnection: close\r\n",
        body.len()
    )
    .unwrap();
    for (name, value) in headers {
        write!(stream, "{name}: {value}\r\n").unwrap();
    }
    write!(stream, "\r\n").unwrap();
    stream.write_all(body).unwrap();
    stream.flush().unwrap();
}
