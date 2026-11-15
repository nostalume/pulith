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
        Self::with_source(label, "source/bin/tool")
    }

    fn with_source(label: &str, source_path: &str) -> Self {
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
            source = root.join(source_path).to_string_lossy().replace('\\', "/"),
        )).unwrap();
        Self { root }
    }

    fn install(&self) -> std::process::Output {
        self.run("install")
    }

    fn run(&self, verb: &str) -> std::process::Output {
        vtool()
            .args([
                verb,
                "--root",
                self.root.to_str().unwrap(),
                self.root.join("manifest.toml").to_str().unwrap(),
            ])
            .output()
            .unwrap()
    }
}

#[test]
fn repair_reports_verified_file_or_preserves_a_conflict() {
    let fixture = Fixture::new("repair-file");
    assert!(fixture.install().status.success());
    let out = fixture.run("repair");
    assert!(out.status.success(), "{out:?}");
    assert_eq!(
        out.stdout,
        b"demo-tool@1.2.0: unchanged; file digest verified\n"
    );

    let target = fixture.root.join("artifacts/demo-tool/1.2.0");
    std::fs::write(&target, b"locally changed").unwrap();
    let out = fixture.run("repair");
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    assert_eq!(out.stdout, b"demo-tool@1.2.0: failed\n");
    assert!(String::from_utf8_lossy(&out.stderr).contains("installed file digest differs"));
    assert_eq!(std::fs::read(target).unwrap(), b"locally changed");
}

#[test]
fn repair_of_an_uninstalled_version_leaves_no_state() {
    let fixture = Fixture::new("not-installed");
    let out = fixture.run("repair");
    assert!(out.status.success(), "{out:?}");
    assert_eq!(out.stdout, b"demo-tool@1.2.0: not installed\n");
    assert!(out.stderr.is_empty());
    assert!(!fixture.root.join(".vtool").exists());
    assert!(!fixture.root.join("artifacts").exists());
}

#[test]
fn repair_reports_scope_for_existing_and_restored_archives() {
    let fixture = Fixture::new("repair-archive");
    let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/examples/vtool/fixtures/tool-win.zip");
    let source = source.to_string_lossy().replace('\\', "/");
    let platform = if cfg!(windows) { "windows" } else { "linux" };
    std::fs::write(fixture.root.join("manifest.toml"), format!(
        "name='demo-tool'\nversion='1.2.0'\n[{platform}.source]\nkind='local'\nformat='zip'\npath='{source}'\n[{platform}.hash]\nkind='sha2'\nhex='2edc44c413a0a47ab1977297f524ee8a87aae99d5db2f3e5d7ee668c33c22076'\n",
    )).unwrap();
    let out = fixture.install();
    assert!(out.status.success(), "{out:?}");
    let target = fixture.root.join("artifacts/demo-tool/1.2.0");
    let file = target.join("bin/tool");
    std::fs::write(&file, b"locally changed").unwrap();
    let out = fixture.run("repair");
    assert!(out.status.success(), "{out:?}");
    assert_eq!(
        out.stdout,
        b"demo-tool@1.2.0: unchanged; archive directory present (contents unchecked)\n"
    );
    assert_eq!(std::fs::read(&file).unwrap(), b"locally changed");

    std::fs::rename(&target, fixture.root.join("old-archive")).unwrap();
    let out = fixture.run("repair");
    assert!(out.status.success(), "{out:?}");
    assert_eq!(
        out.stdout,
        b"demo-tool@1.2.0: repaired; archive input verified\n"
    );
    assert_eq!(std::fs::read(&file).unwrap(), b"win-tool-bytes\n");

    std::fs::rename(&target, fixture.root.join("repaired-archive")).unwrap();
    let view = fixture.root.join("view");
    std::fs::write(&view, b"local view occupant").unwrap();
    let manifest = fixture.root.join("manifest.toml");
    let text = std::fs::read_to_string(&manifest).unwrap();
    let view_path = view.to_string_lossy().replace('\\', "/");
    std::fs::write(manifest, format!("link_at='{view_path}'\n{text}")).unwrap();
    let out = fixture.run("repair");
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    assert_eq!(
        out.stdout,
        b"demo-tool@1.2.0: failed after publication; archive input verified\n"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("nothing replaced"));
    assert_eq!(stderr.lines().count(), 1);
    assert_eq!(std::fs::read(file).unwrap(), b"win-tool-bytes\n");
    assert_eq!(std::fs::read(view).unwrap(), b"local view occupant");
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[test]
fn restart_reads_the_committed_snapshot() {
    let fixture = Fixture::new("restart");
    assert!(fixture.install().status.success());
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
    assert!(fixture.install().status.success());
    assert!(!stage.join("snapshot.json").exists());
    let snapshot =
        std::fs::read_to_string(fixture.root.join(".vtool/state/snapshot.json")).unwrap();
    assert!(snapshot.contains("demo-tool"));
}

#[test]
fn directory_cannot_supply_verified_file_bytes() {
    let fixture = Fixture::with_source("directory", "source");
    let out = fixture.install();
    assert!(!out.status.success(), "{out:?}");
    assert!(!fixture.root.join(".vtool/state/snapshot.json").exists());
    assert_eq!(
        std::fs::read(fixture.root.join("source/bin/tool")).unwrap(),
        b"restart-test bytes"
    );
}
