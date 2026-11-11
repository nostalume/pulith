//! Ambient proxy environment as a test-owned input.
//!
//! Proxy variables are process-global, so this is a dedicated test binary with exactly one test:
//! installing and restoring them cannot reach any other test. `cargo test` runs it like every
//! other target. No external service, fixed port, startup delay, or CI-only job is involved.
#![cfg(feature = "http-ureq")]

use std::ffi::OsString;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use pulith::Inspect;
use pulith::net::RemoteUrl;
use pulith::net::UreqResources;

const PROXY_KEYS: [&str; 6] = [
    "HTTP_PROXY",
    "http_proxy",
    "HTTPS_PROXY",
    "https_proxy",
    "ALL_PROXY",
    "all_proxy",
];
const BYPASS_KEYS: [&str; 2] = ["NO_PROXY", "no_proxy"];

/// Binds a loopback listener and returns its base URL. No thread starts here, so the ambient
/// environment can be installed before any other thread exists in this test binary.
fn bind() -> (TcpListener, String) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    listener.set_nonblocking(true).unwrap();
    (listener, url)
}

/// In-process HTTP responder: listener, port, and response bytes belong to this test.
struct Responder {
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl Responder {
    fn start(listener: TcpListener, status: &'static str) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&stop);
        let handle = thread::spawn(move || {
            while !flag.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        // On Windows an accepted socket inherits the listener's nonblocking mode.
                        stream.set_nonblocking(false).unwrap();
                        stream
                            .set_read_timeout(Some(Duration::from_secs(5)))
                            .unwrap();
                        // Drain the headers: an unread receive buffer makes a TCP close abortive.
                        let mut reader = BufReader::new(stream.try_clone().unwrap());
                        let mut line = String::new();
                        loop {
                            line.clear();
                            if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                                break;
                            }
                        }
                        let response = format!(
                            "HTTP/1.1 {status}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                        );
                        let _ = stream.write_all(response.as_bytes());
                        let _ = stream.flush();
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(5));
                    }
                    Err(_) => break,
                }
            }
        });
        Self {
            stop,
            handle: Some(handle),
        }
    }
}

impl Drop for Responder {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

/// Installs a hostile ambient proxy and restores every previous value on drop.
struct AmbientProxy {
    saved: Vec<(&'static str, Option<OsString>)>,
}

impl AmbientProxy {
    fn install(url: &str) -> Self {
        let saved = PROXY_KEYS
            .iter()
            .chain(BYPASS_KEYS.iter())
            .copied()
            .map(|key| (key, std::env::var_os(key)))
            .collect();
        // SAFETY: this is the only test in this binary, and no thread exists yet. The guard
        // restores every key on drop, after all responder threads have been joined.
        unsafe {
            for key in PROXY_KEYS {
                std::env::set_var(key, url);
            }
            for key in BYPASS_KEYS {
                std::env::remove_var(key);
            }
        }
        Self { saved }
    }
}

impl Drop for AmbientProxy {
    fn drop(&mut self) {
        // SAFETY: all responder threads have been joined before this guard is dropped.
        unsafe {
            for (key, value) in self.saved.drain(..) {
                match value {
                    Some(value) => std::env::set_var(key, value),
                    None => std::env::remove_var(key),
                }
            }
        }
    }
}

#[test]
fn default_resources_ignore_ambient_proxy_environment() {
    let (hostile_listener, hostile_url) = bind();
    let (origin_listener, origin_url) = bind();
    let _env = AmbientProxy::install(&hostile_url);
    // Held for their Drop: each responder joins its thread before the environment is restored.
    let _hostile = Responder::start(hostile_listener, "502 Bad Gateway");
    let _origin = Responder::start(origin_listener, "200 OK");
    let url = RemoteUrl::parse(&format!("{origin_url}/artifact")).unwrap();

    let (observation, _) = Inspect::inspect(url.clone(), ()).unwrap();
    assert_eq!(
        observation.status, 200,
        "default ureq resources followed the ambient proxy"
    );

    // Control: the same URL through an explicitly injected proxy must not reach the origin.
    let config = ureq::Agent::config_builder()
        .proxy(Some(ureq::Proxy::new(&hostile_url).unwrap()))
        .build();
    let result = Inspect::inspect(
        url.with_ureq(UreqResources::from_agent(config.new_agent())),
        (),
    );
    assert!(
        !matches!(&result, Ok((observation, _)) if observation.status == 200),
        "an explicitly injected ureq proxy did not divert the request: {result:?}"
    );
}
