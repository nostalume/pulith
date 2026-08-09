//! Shared integration-test harness (named-file style; included per test file as `mod common;`).
//!
//! Owns the side-effect fixture frames so tests/ files do not duplicate them: the process fixture
//! harness (real `/bin/sh` on unix, PowerShell on windows), the local publish/receipt helpers, the
//! zip/tar writers, and the mock HTTP server. Platform handling is cfg-split here, exactly as the
//! per-file harnesses used to be. This module is not a test target: it declares no `#[test]`.
#![allow(dead_code)]

#[cfg(feature = "process")]
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
#[cfg(feature = "process")]
use std::time::Duration;

#[cfg(feature = "process")]
use pulith::process::{Arg, EnvVars, OutputPath, OutputProcess};

use pulith::Acquire;
use pulith::archive::ArchivePolicy;
use pulith::local::{LocalSource, LocalTarget};

/// Runs one real process fixture: a hand-rolled local server or child process script.
#[derive(Clone, Copy)]
pub enum Fixture {
    Success,
    Nonzero,
    MissingOutput,
    FileOutput,
    Sleeps,
    SpawnsDescendant,
    CopiesInputEnv,
    CopiesInputArg,
}

/// The unified temp-dir idiom: auto-cleaned `tempfile::tempdir()`.
pub fn temp_dir() -> tempfile::TempDir {
    tempfile::tempdir().unwrap()
}

// ---------------------------------------------------------------------------
// Local publish/receipt helpers (previously duplicated in activation.rs and switch.rs)
// ---------------------------------------------------------------------------

/// Publishes one artifact tree at `root/artifacts/demo-tool/<version>` and returns target + receipt.
pub fn publish_tree(
    root: &Path,
    version: &'static str,
    contents: &'static [u8],
) -> (PathBuf, pulith::local::ApplyEvidence) {
    let source = root.join(format!("source-{version}"));
    let target = root.join(format!("artifacts/demo-tool/{version}"));
    fs::create_dir_all(&source).unwrap();
    fs::create_dir_all(target.parent().unwrap()).unwrap();
    fs::write(source.join("tool.txt"), contents).unwrap();

    let material = LocalSource::new(source).unwrap().acquire().unwrap();
    let admitted = LocalTarget::new(target.clone()).unwrap();
    let stage = admitted.stage().unwrap();
    let (tree, _) = material.prepare(stage, ArchivePolicy::default()).unwrap();
    let evidence = tree.publish(admitted).unwrap();

    (target, evidence)
}

#[cfg(unix)]
pub fn file_symlink(original: impl AsRef<Path>, link: impl AsRef<Path>) -> std::io::Result<()> {
    std::os::unix::fs::symlink(original, link)
}

#[cfg(windows)]
pub fn file_symlink(original: impl AsRef<Path>, link: impl AsRef<Path>) -> std::io::Result<()> {
    std::os::windows::fs::symlink_file(original, link)
}

#[cfg(unix)]
pub fn dir_symlink(original: impl AsRef<Path>, link: impl AsRef<Path>) -> std::io::Result<()> {
    std::os::unix::fs::symlink(original, link)
}

#[cfg(windows)]
pub fn dir_symlink(original: impl AsRef<Path>, link: impl AsRef<Path>) -> std::io::Result<()> {
    std::os::windows::fs::symlink_dir(original, link)
}

#[cfg(unix)]
pub fn directory_symlink(source: &Path, view: &Path) {
    std::os::unix::fs::symlink(source, view).unwrap();
}

#[cfg(windows)]
pub fn directory_symlink(source: &Path, view: &Path) {
    std::os::windows::fs::symlink_dir(source, view).unwrap();
}

// ---------------------------------------------------------------------------
// Process fixture harness (previously duplicated in process.rs and process_async.rs)
// ---------------------------------------------------------------------------

#[cfg(feature = "process")]
pub fn absolute_program() -> PathBuf {
    #[cfg(unix)]
    {
        PathBuf::from("/bin/sh")
    }
    #[cfg(windows)]
    {
        PathBuf::from(std::env::var_os("SystemRoot").unwrap())
            .join("System32/WindowsPowerShell/v1.0/powershell.exe")
    }
}

/// Builds the `MARKER` (+ `LOOP_SCRIPT` on windows) environment for a descendant-loop fixture.
#[cfg(feature = "process")]
pub fn marker_environment(marker: &Path) -> EnvVars {
    #[cfg(unix)]
    {
        EnvVars::new([(OsString::from("MARKER"), marker.as_os_str().to_os_string())]).unwrap()
    }
    #[cfg(windows)]
    {
        let loop_script = marker.with_extension("ps1");
        std::fs::write(
            &loop_script,
            "while($true){[IO.File]::AppendAllText($env:MARKER,'x'); Start-Sleep -Milliseconds 50}",
        )
        .unwrap();
        EnvVars::new([
            (
                OsString::from("SystemRoot"),
                std::env::var_os("SystemRoot").unwrap(),
            ),
            (OsString::from("MARKER"), marker.as_os_str().to_os_string()),
            (
                OsString::from("LOOP_SCRIPT"),
                loop_script.as_os_str().to_os_string(),
            ),
        ])
        .unwrap()
    }
}

#[cfg(feature = "process")]
pub fn fixture_process(fixture: Fixture, output: &str, timeout: Duration) -> OutputProcess {
    #[cfg(unix)]
    let script = match fixture {
        Fixture::Success => {
            "echo out-line; echo more-output; echo err-line >&2; mkdir -p \"$PULITH_OUTPUT_ROOT/bin\" && printf pulith > \"$PULITH_OUTPUT_ROOT/bin/tool\""
        }
        Fixture::Nonzero => "echo dying-message; exit 7",
        Fixture::MissingOutput => "exit 0",
        Fixture::FileOutput => "echo warn; printf file > \"$PULITH_OUTPUT_ROOT\"",
        Fixture::Sleeps => "sleep 1",
        Fixture::SpawnsDescendant => {
            "echo spawned; sh -c 'while :; do printf x >> \"$MARKER\"; sleep 0.05; done' & wait"
        }
        Fixture::CopiesInputEnv => {
            "mkdir -p \"$PULITH_OUTPUT_ROOT\"; cp \"$PULITH_INPUT_ROOT/input.txt\" \"$PULITH_OUTPUT_ROOT/file.txt\""
        }
        Fixture::CopiesInputArg => {
            "mkdir -p \"$PULITH_OUTPUT_ROOT\"; cp \"$1/inputs/input.txt\" \"$PULITH_OUTPUT_ROOT/arg-file.txt\""
        }
    };
    #[cfg(windows)]
    let script = match fixture {
        Fixture::Success => {
            "Write-Output 'out-line'; Write-Output 'more-output'; [Console]::Error.WriteLine('err-line'); $bin = Join-Path $env:PULITH_OUTPUT_ROOT 'bin'; New-Item -ItemType Directory -Force -Path $bin | Out-Null; [IO.File]::WriteAllText((Join-Path $bin 'tool'), 'pulith')"
        }
        Fixture::Nonzero => "Write-Output 'dying-message'; exit 7",
        Fixture::MissingOutput => "exit 0",
        Fixture::FileOutput => {
            "Write-Output 'warn'; [IO.File]::WriteAllText($env:PULITH_OUTPUT_ROOT, 'file')"
        }
        Fixture::Sleeps => "Start-Sleep -Seconds 1",
        Fixture::SpawnsDescendant => {
            "Write-Output 'spawned'; [Console]::Out.Flush(); Start-Process -FilePath (Join-Path $PSHOME 'powershell.exe') -ArgumentList '-NoProfile','-ExecutionPolicy','Bypass','-File',$env:LOOP_SCRIPT -NoNewWindow -PassThru | ForEach-Object { $_.WaitForExit() }"
        }
        Fixture::CopiesInputEnv => {
            "$out = $env:PULITH_OUTPUT_ROOT; New-Item -ItemType Directory -Force -Path $out | Out-Null; Copy-Item -Path (Join-Path $env:PULITH_INPUT_ROOT 'input.txt') -Destination (Join-Path $out 'file.txt')"
        }
        Fixture::CopiesInputArg => {
            "$out = $env:PULITH_OUTPUT_ROOT; New-Item -ItemType Directory -Force -Path $out | Out-Null; Copy-Item -Path (Join-Path $src 'inputs/input.txt') -Destination (Join-Path $out 'arg-file.txt')"
        }
    };

    #[cfg(unix)]
    let arguments: Vec<Arg> = match fixture {
        Fixture::CopiesInputArg => [
            Arg::Literal(OsString::from("-c")),
            Arg::Literal(OsString::from(script)),
            Arg::Literal(OsString::from("-")),
            Arg::WorkspaceRoot,
        ]
        .into_iter()
        .collect::<Vec<Arg>>(),
        _ => [
            Arg::Literal(OsString::from("-c")),
            Arg::Literal(OsString::from(script)),
        ]
        .into_iter()
        .collect::<Vec<Arg>>(),
    };
    #[cfg(windows)]
    let arguments: Vec<Arg> = {
        let base = [
            Arg::Literal(OsString::from("-NoProfile")),
            Arg::Literal(OsString::from("-NonInteractive")),
            Arg::Literal(OsString::from("-Command")),
        ];
        if matches!(fixture, Fixture::CopiesInputArg) {
            [
                base.as_slice(),
                &[
                    Arg::Literal(OsString::from(format!("& {{ param($src) {script} }}"))),
                    Arg::WorkspaceRoot,
                ][..],
            ]
            .concat()
        } else {
            [base.as_slice(), &[Arg::Literal(OsString::from(script))][..]].concat()
        }
    };

    let action = OutputProcess::new(
        absolute_program(),
        OutputPath::new(output).unwrap(),
        timeout,
    )
    .unwrap()
    .with_arguments(arguments);
    #[cfg(windows)]
    let action = action.with_environment(
        EnvVars::new([(
            OsString::from("SystemRoot"),
            std::env::var_os("SystemRoot").unwrap(),
        )])
        .unwrap(),
    );
    action
}

/// Asserts that `result` is the expected failure and the final target stays untouched.
#[cfg(feature = "process")]
pub fn assert_failure_keeps_target_missing(
    fixture: Fixture,
    timeout: Duration,
    is_expected: impl FnOnce(&pulith::process::RunError) -> bool,
) {
    let root = temp_dir();
    let target = root.path().join("published");
    let result = fixture_process(fixture, "tree", timeout).acquire();

    assert!(
        matches!(&result, Err(error) if is_expected(error)),
        "unexpected result: {result:?}"
    );
    assert!(!target.exists());
}

pub fn contains_bytes(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

#[cfg(feature = "process")]
pub fn captured_contains(diagnostics: &pulith::process::Diagnostics, needle: &[u8]) -> bool {
    diagnostics
        .stdout
        .as_deref()
        .is_some_and(|stdout| contains_bytes(stdout, needle))
}

// ---------------------------------------------------------------------------
// Mock HTTP server (previously HttpFixture in lifecycle/materialization.rs).
//
// Binds `127.0.0.1` and serves one connection. Ambient `http_proxy`/`https_proxy` environment
// variables must not intercept the request: run local HTTP tests with the proxy unset (or a
// `no_proxy` entry the async client honors exactly, e.g. `127.0.0.1,localhost`), otherwise a proxy
// can answer the mock-server request with its own status (observed: 502 from a local proxy because
// reqwest does not match a `127.*` no_proxy glob).
// ---------------------------------------------------------------------------

#[cfg(any(feature = "http-sync", feature = "http-async"))]
pub struct HttpFixture {
    pub url: String,
    handle: std::thread::JoinHandle<()>,
}

#[cfg(any(feature = "http-sync", feature = "http-async"))]
impl HttpFixture {
    pub fn get(body: &'static [u8]) -> Self {
        use std::io::{Read, Write};
        use std::net::TcpListener;
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let handle = std::thread::spawn(move || {
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

    pub fn join(self) {
        self.handle.join().unwrap();
    }
}

/// Writes a zip archive with the given (name, bytes) entries for archive fixtures.
#[cfg(feature = "zip")]
pub fn write_zip(path: &Path, entries: &[(&str, &[u8])]) {
    use std::io::Write;
    let mut writer = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
    for (name, bytes) in entries {
        writer
            .start_file(name, zip::write::SimpleFileOptions::default())
            .unwrap();
        writer.write_all(bytes).unwrap();
    }
    writer.finish().unwrap();
}
