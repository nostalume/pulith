//! Restart evidence for vtool's atomic state snapshot.

#![cfg(all(
    feature = "local",
    feature = "http-ureq",
    feature = "zip",
    feature = "sha2"
))]

use std::path::PathBuf;
use std::process::Command;

fn vtool() -> Command {
    Command::new(super::example("vtool"))
}

struct Fixture {
    root: PathBuf,
}

impl Fixture {
    fn new(label: &str) -> Self {
        let temporary = tempfile::Builder::new()
            .prefix(&format!("vtool-{label}-"))
            .tempdir()
            .unwrap();
        let root = temporary.keep();
        let source = root.join("source");
        std::fs::create_dir_all(source.join("bin")).unwrap();
        std::fs::write(source.join("bin/tool"), b"restart-test bytes").unwrap();
        let digest = {
            use sha2::{Digest, Sha256};
            hex::encode(Sha256::digest(b"restart-test bytes"))
        };
        std::fs::write(root.join("manifest.toml"), format!(
            "name = \"demo-tool\"\nversion = \"1.2.0\"\n\n[windows.source]\nkind = \"local\"\npath = \"{source}\"\n\n[windows.hash]\nkind = \"sha2\"\nhex = \"{digest}\"\n\n[linux.source]\nkind = \"local\"\npath = \"{source}\"\n\n[linux.hash]\nkind = \"sha2\"\nhex = \"{digest}\"\n",
            source = source.to_string_lossy().replace('\\', "/"),
        )).unwrap();
        Self { root }
    }

    fn install(&self) -> std::process::ExitStatus {
        vtool()
            .args([
                "install",
                "--root",
                self.root.to_str().unwrap(),
                self.root.join("manifest.toml").to_str().unwrap(),
            ])
            .status()
            .unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[test]
fn restart_reads_the_committed_snapshot() {
    let fixture = Fixture::new("restart");
    assert!(fixture.install().success());
    let snapshot =
        std::fs::read_to_string(fixture.root.join(".vtool/state/snapshot.json")).unwrap();
    assert!(snapshot.contains("demo-tool"));
    assert!(snapshot.contains("Installed"));
}

#[test]
fn restart_reclaims_a_bounded_precommit_residue() {
    let fixture = Fixture::new("residue");
    let stage = fixture.root.join(".vtool/state/stage");
    std::fs::create_dir_all(&stage).unwrap();
    std::fs::write(stage.join("snapshot.json"), b"interrupted").unwrap();
    assert!(fixture.install().success());
    assert!(!stage.join("snapshot.json").exists());
    let snapshot =
        std::fs::read_to_string(fixture.root.join(".vtool/state/snapshot.json")).unwrap();
    assert!(snapshot.contains("demo-tool"));
}
