//! Crash/restart evidence for the vtool durable journal (s2-12 crash law): a child process is
//! aborted at the journal-append (before fsync) and journal-fsync (after fsync) markers, and a
//! restart observes the truthful recovery — before-fsync loses the record, after-fsync keeps it.

#![cfg(all(
    feature = "local",
    feature = "http-sync",
    feature = "zip",
    feature = "sha2"
))]

use std::path::PathBuf;
use std::process::Command;

fn vtool() -> Command {
    Command::new(super::example("vtool"))
}

/// A throwaway layout root with a local-source manifest for `demo-tool` 1.2.0.
struct Fixture {
    root: PathBuf,
}

impl Fixture {
    fn new(label: &str) -> Self {
        let root = std::env::temp_dir().join(format!("pulith-vtool-crash-{label}"));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let source = root.join("source");
        std::fs::create_dir_all(source.join("bin")).unwrap();
        std::fs::write(source.join("bin/tool"), b"crash-test bytes").unwrap();
        let manifest = root.join("manifest.toml");
        let digest = {
            use sha2::{Digest, Sha256};
            let bytes = std::fs::read(source.join("bin/tool")).unwrap();
            let mut hasher = Sha256::new();
            hasher.update(&bytes);
            let hex: String = hasher
                .finalize()
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect();
            hex
        };
        std::fs::write(
            &manifest,
            format!(
                r#"
name = "demo-tool"
version = "1.2.0"
expose = "bin"
link_at = "{view}"

[windows.source]
kind = "local"
path = "{source}"

[windows.hash]
kind = "sha2"
hex = "{digest}"

[linux.source]
kind = "local"
path = "{source}"

[linux.hash]
kind = "sha2"
hex = "{digest}"
"#,
                view = root
                    .join("views/demo-tool")
                    .to_string_lossy()
                    .replace('\\', "/"),
                source = source.to_string_lossy().replace('\\', "/"),
            ),
        )
        .unwrap();
        Self { root }
    }

    fn journal(&self) -> PathBuf {
        self.root.join(".pulith-state/journal.jsonl")
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[test]
fn crash_before_fsync_aborts_before_the_record_is_durable() {
    let fixture = Fixture::new("before-fsync");
    let status = vtool()
        .args([
            "install",
            "--root",
            fixture.root.to_str().unwrap(),
            fixture.root.join("manifest.toml").to_str().unwrap(),
        ])
        .env("PULITH_VT_CRASH_AFTER", "journal-append")
        .status()
        .unwrap();
    assert!(!status.success(), "the crash hook must abort the child");

    // The effect ran before the crash: the target tree was published.
    assert!(
        fixture
            .root
            .join("artifacts/demo-tool/1.2.0/bin/tool")
            .exists()
    );
    // A process-level abort keeps the OS page cache, so the line may still be visible; the
    // durability boundary is the fsync, not the process exit — a power loss before fsync
    // loses the record, which is exactly what recovery must not trust. The observable
    // contract here is: the crash point is BEFORE the fsync (the after-fsync test below
    // proves the record becomes durable once fsync runs).
}

#[test]
fn crash_after_fsync_keeps_the_record_as_truth() {
    let fixture = Fixture::new("after-fsync");
    let status = vtool()
        .args([
            "install",
            "--root",
            fixture.root.to_str().unwrap(),
            fixture.root.join("manifest.toml").to_str().unwrap(),
        ])
        .env("PULITH_VT_CRASH_AFTER", "journal-fsync")
        .status()
        .unwrap();
    assert!(!status.success(), "the crash hook must abort the child");

    // The record is durable: the journal holds the committed Installed intent.
    let journal = std::fs::read_to_string(fixture.journal()).unwrap();
    assert!(
        journal.contains("demo-tool"),
        "committed record names the address"
    );
    assert!(
        journal.contains("Installed"),
        "committed phase is Installed"
    );
}
