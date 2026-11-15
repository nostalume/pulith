use super::*;
use std::error::Error;
use std::fs;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::thread;

const WIN_ZIP_HEX: &str = "2edc44c413a0a47ab1977297f524ee8a87aae99d5db2f3e5d7ee668c33c22076";
const TAR_HEX: &str = "0c472076cda1af8189518b102682fa80058902d8454885ca1ccca758992fc07e";
const PLAIN_BIN_HEX: &str = "83cb532b46738b2a268f95dac473a5642b6eea7497fc3b149be7fe7d9f5190fe";
#[cfg(unix)]
const SYMLINK_TAR_HEX: &str = "7e23e73d324c41c54134791ad98c6075d1331c3f83dcd16cf71ed7657f26c693";
const WRAPPED_ZIP_HEX: &str = "b3422d3baa6aa950529e9b11e71d9d98f0a2ce17bff2a22ebc2b3f012d668138";

fn fixture(name: &str) -> String {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/examples/vtool/fixtures")
        .join(name)
        .to_string_lossy()
        .into_owned()
}

/// Escape a path for embedding in a TOML string (backslash and quote).
fn toml_string(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

fn recipe_text(
    name: &str,
    version: &str,
    fixture: &str,
    hex: &str,
    expose: Option<&str>,
    link_at: Option<&Path>,
) -> String {
    let mut text = format!("name = \"{name}\"\nversion = \"{version}\"\n");
    if let Some(expose) = expose {
        text.push_str(&format!("expose = \"{expose}\"\n"));
    }
    if let Some(link_at) = link_at {
        text.push_str(&format!(
            "link_at = \"{}\"\n",
            toml_string(&link_at.display().to_string())
        ));
    }
    let format = match Path::new(fixture)
        .extension()
        .and_then(|extension| extension.to_str())
    {
        Some("zip") => "zip",
        Some("tar") => "tar",
        _ => "plain",
    };
    if cfg!(windows) {
        text.push_str(&format!(
            "[windows.source]\nkind = \"local\"\nformat = \"{format}\"\npath = \"{}\"\n\n\
                 [windows.hash]\nkind = \"sha2\"\nhex = \"{hex}\"\n\n",
            toml_string(fixture)
        ));
    } else {
        text.push_str(&format!(
            "[linux.source]\nkind = \"local\"\nformat = \"{format}\"\npath = \"{}\"\n\n\
                 [linux.hash]\nkind = \"sha2\"\nhex = \"{hex}\"\n\n",
            toml_string(fixture)
        ));
    }
    text
}

fn install_manifest(
    text: &str,
    layout_root: &Path,
) -> Result<Option<LinkChange>, Box<dyn Error + Send + Sync>> {
    let manifest = crate::manifest::Manifest::parse(text).expect("parse manifest");
    let resolved = manifest.resolve(layout_root).expect("resolve");
    let activate = resolved.view.is_some();
    resolved.install(layout_root)?;
    if activate {
        let resolved = crate::manifest::Manifest::parse(text)
            .expect("parse manifest")
            .resolve(layout_root)
            .expect("resolve");
        Ok(Some(resolved.activate(layout_root)?))
    } else {
        Ok(None)
    }
}

#[test]
fn verified_archive_uses_captured_bytes_after_source_changes() {
    use pulith::archive::{ArchivePolicy, extract_zip};
    use pulith::hash::{DigestAlgorithmKind, DigestValue};
    use pulith::local::LocalPath;

    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source.zip");
    fs::copy(fixture("tool-win.zip"), &source).unwrap();
    let expected = DigestValue::new(DigestAlgorithmKind::Sha256, WIN_ZIP_HEX).unwrap();
    let (snapshot, _) = LocalPath::new(&source)
        .unwrap()
        .open_file()
        .unwrap()
        .unwrap()
        .snapshot_verified(&expected)
        .unwrap();

    fs::write(&source, "changed in place").unwrap();
    fs::rename(&source, root.path().join("old.zip")).unwrap();
    fs::write(&source, "replacement").unwrap();

    let target_path = root.path().join("installed");
    let target = LocalPath::new(&target_path).unwrap();
    let stage = target.stage().unwrap();
    extract_zip(snapshot, stage.root(), ArchivePolicy::new()).unwrap();
    stage
        .publish(target, pulith::local::TreeAdmission::Absent)
        .unwrap();
    assert_eq!(
        fs::read(target_path.join("bin/tool")).unwrap(),
        b"win-tool-bytes\n"
    );
}

fn make_directory_symlink(target: &Path, view: &Path) {
    #[cfg(windows)]
    std::os::windows::fs::symlink_dir(target, view).unwrap();
    #[cfg(not(windows))]
    std::os::unix::fs::symlink(target, view).unwrap();
}

/// A state seed for a repair address (the committed intent repair reconciles against).
fn seed_intent(root: &Path, name: &str, version: &str, phase: Phase) {
    State::open(root)
        .unwrap()
        .commit(name, version, phase)
        .unwrap();
}

#[test]
fn install_materializes_and_activates_the_exposed_view() {
    let layout_root = tempfile::tempdir().unwrap();
    let link_at = layout_root.path().join("views/demo-tool");
    let (fixture_name, hex, tool_bytes) = if cfg!(windows) {
        ("tool-win.zip", WIN_ZIP_HEX, b"win-tool-bytes\n".as_slice())
    } else {
        ("tool.tar", TAR_HEX, b"linux-tool-bytes\n".as_slice())
    };
    let recipe = recipe_text(
        "demo-tool",
        "1.2.0",
        &fixture(fixture_name),
        hex,
        Some("bin"),
        Some(&link_at),
    );

    let outcome = install_manifest(&recipe, layout_root.path()).unwrap();

    let target = layout_root.path().join("artifacts/demo-tool/1.2.0");
    assert_eq!(outcome, Some(LinkChange::Created));
    assert_eq!(fs::read(target.join("bin/tool")).unwrap(), tool_bytes);
    // The view is a directory symlink pointing at the EXPOSED subpath.
    assert!(
        fs::symlink_metadata(&link_at)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(
        fs::canonicalize(&link_at).unwrap(),
        fs::canonicalize(target.join("bin")).unwrap()
    );
    assert_eq!(fs::read(link_at.join("tool")).unwrap(), tool_bytes);
}

#[test]
fn install_materializes_a_url_source_through_the_linear_chain() {
    use sha2::{Digest, Sha256};

    let body = b"remote tool bytes\n";
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = [0_u8; 1024];
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
    });
    let root = tempfile::tempdir().unwrap();
    let platform = if cfg!(windows) { "windows" } else { "linux" };
    let recipe = format!(
        "name = \"remote-tool\"\nversion = \"1.0.0\"\n\n[{platform}.source]\nkind = \"url\"\nurl = \"{url}\"\n\n[{platform}.hash]\nkind = \"sha2\"\nhex = \"{}\"\n",
        hex::encode(Sha256::digest(body)),
    );

    let outcome = install_manifest(&recipe, root.path()).unwrap();
    server.join().unwrap();

    assert_eq!(outcome, None);
    assert_eq!(
        fs::read(root.path().join("artifacts/remote-tool/1.0.0")).unwrap(),
        body
    );
}

#[test]
fn install_uses_the_declared_raw_tar_format() {
    let layout_root = tempfile::tempdir().unwrap();
    let recipe = recipe_text(
        "raw-tar-tool",
        "1.0.0",
        &fixture("tool.tar"),
        TAR_HEX,
        Some("bin"),
        None,
    );

    let outcome = install_manifest(&recipe, layout_root.path()).unwrap();
    let target = layout_root.path().join("artifacts/raw-tar-tool/1.0.0");
    assert_eq!(
        fs::read(target.join("bin/tool")).unwrap(),
        b"linux-tool-bytes\n"
    );
    assert_eq!(outcome, None);
}

#[test]
fn install_copies_a_plain_file_source() {
    let layout_root = tempfile::tempdir().unwrap();
    let recipe = recipe_text(
        "plain-tool",
        "1.0.0",
        &fixture("plain.bin"),
        PLAIN_BIN_HEX,
        None,
        None,
    );

    let outcome = install_manifest(&recipe, layout_root.path()).unwrap();
    assert_eq!(outcome, None);
    let target = layout_root.path().join("artifacts/plain-tool/1.0.0");
    assert_eq!(fs::read(&target).unwrap(), b"plain-file-bytes\n");
}

#[test]
fn install_refuses_a_local_directory_for_a_byte_digest() {
    let layout_root = tempfile::tempdir().unwrap();
    let recipe = recipe_text(
        "dir-tool",
        "1.0.0",
        &fixture("plain-dir"),
        "0000000000000000000000000000000000000000000000000000000000000000",
        None,
        None,
    );

    assert!(install_manifest(&recipe, layout_root.path()).is_err());
    let target = layout_root.path().join("artifacts/dir-tool/1.0.0");
    assert!(!target.exists());
}

#[cfg(unix)]
#[test]
fn install_rejects_an_archive_with_symlink_entries() {
    let layout_root = tempfile::tempdir().unwrap();
    let recipe = recipe_text(
        "symlink-tool",
        "1.0.0",
        &fixture("tool-symlink.tar"),
        SYMLINK_TAR_HEX,
        None,
        None,
    );

    let error = install_manifest(&recipe, layout_root.path()).unwrap_err();
    assert!(
        error
            .downcast_ref::<pulith::archive::ArchiveError>()
            .is_some(),
        "{error:?}"
    );
    // The versioned tree was never published and the private workspace was cleaned.
    assert!(
        !layout_root
            .path()
            .join("artifacts/symlink-tool/1.0.0")
            .exists()
    );
    assert!(
        !layout_root
            .path()
            .join(".pulith-work/symlink-tool-1.0.0")
            .exists()
    );
}

#[test]
fn install_aborts_on_hash_mismatch_and_leaves_no_target_or_workspace() {
    let layout_root = tempfile::tempdir().unwrap();
    let recipe = recipe_text(
        "bad-hash-tool",
        "1.0.0",
        &fixture("tool-win.zip"),
        "0000000000000000000000000000000000000000000000000000000000000000",
        None,
        None,
    );

    let error = install_manifest(&recipe, layout_root.path()).unwrap_err();
    assert!(matches!(
        error
            .downcast_ref::<pulith::hash::HashError>()
            .expect("expected a hash error"),
        pulith::hash::HashError::DigestMismatch { .. }
    ));
    // The versioned tree was never published and the private workspace was cleaned; the
    // artifacts/<name> layout parent is legitimately pre-created by the vertical.
    assert!(
        !layout_root
            .path()
            .join("artifacts/bad-hash-tool/1.0.0")
            .exists()
    );
    assert!(
        !layout_root
            .path()
            .join(".pulith-work/bad-hash-tool-1.0.0")
            .exists()
    );
}

#[test]
fn install_auto_switches_an_occupied_directory_symlink_view() {
    let layout_root = tempfile::tempdir().unwrap();
    let link_at = layout_root.path().join("views/demo-tool");
    let (fixture_name, hex) = if cfg!(windows) {
        ("tool-win.zip", WIN_ZIP_HEX)
    } else {
        ("tool.tar", TAR_HEX)
    };

    let first = recipe_text(
        "demo-tool",
        "1.0.0",
        &fixture(fixture_name),
        hex,
        Some("bin"),
        Some(&link_at),
    );
    install_manifest(&first, layout_root.path()).unwrap();

    let second = recipe_text(
        "demo-tool",
        "2.0.0",
        &fixture(fixture_name),
        hex,
        Some("bin"),
        Some(&link_at),
    );
    let outcome = install_manifest(&second, layout_root.path()).unwrap();

    assert_eq!(outcome, Some(LinkChange::Replaced));
    let second_target = layout_root.path().join("artifacts/demo-tool/2.0.0");
    assert_eq!(
        fs::canonicalize(&link_at).unwrap(),
        fs::canonicalize(second_target.join("bin")).unwrap()
    );
    // The first version's tree is untouched (retention is caller-owned).
    assert!(
        layout_root
            .path()
            .join("artifacts/demo-tool/1.0.0")
            .is_dir()
    );
}

#[test]
fn install_fails_when_expose_is_not_a_directory_and_creates_no_view() {
    let layout_root = tempfile::tempdir().unwrap();
    let link_at = layout_root.path().join("views/demo-tool");
    let recipe = recipe_text(
        "demo-tool",
        "1.0.0",
        &fixture("tool-win.zip"),
        WIN_ZIP_HEX,
        Some("missing"),
        Some(&link_at),
    );

    let error = install_manifest(&recipe, layout_root.path()).unwrap_err();
    assert!(
        error.to_string().contains("is not a directory"),
        "unexpected error: {error}"
    );
    assert!(!link_at.exists());
}

#[test]
fn install_keeps_a_wrapped_tree_without_magic_unwrap() {
    let layout_root = tempfile::tempdir().unwrap();
    // The archive has entries under tool-1.2.0/; expose must name the real shape (D8).
    let recipe = recipe_text(
        "wrapped-tool",
        "1.2.0",
        &fixture("tool-wrapped.zip"),
        WRAPPED_ZIP_HEX,
        Some("tool-1.2.0/bin"),
        None,
    );

    let outcome = install_manifest(&recipe, layout_root.path()).unwrap();
    let target = layout_root.path().join("artifacts/wrapped-tool/1.2.0");
    assert_eq!(
        fs::read(target.join("tool-1.2.0/bin/tool")).unwrap(),
        b"wrapped-tool-bytes\n"
    );
    assert_eq!(outcome, None);

    // An inert expose (declared but no view linked) is not validated: the D7 law applies
    // only when a view is linked. Use a fresh version to avoid colliding on the target.
    let inert_recipe = recipe_text(
        "wrapped-tool",
        "1.2.1",
        &fixture("tool-wrapped.zip"),
        WRAPPED_ZIP_HEX,
        Some("bin"),
        None,
    );
    let outcome = install_manifest(&inert_recipe, layout_root.path()).unwrap();
    assert_eq!(outcome, None);
}

// --- repair controller tests ---

fn resolve_manifest(text: &str, layout_root: &Path) -> Resolved {
    crate::manifest::Manifest::parse(text)
        .expect("parse manifest")
        .resolve(layout_root)
        .expect("resolve")
}

fn archive_recipe(name: &str, link_at: Option<&Path>) -> String {
    recipe_text(
        name,
        "1.0.0",
        &fixture("tool-win.zip"),
        WIN_ZIP_HEX,
        Some("bin"),
        link_at,
    )
}

#[test]
fn repair_reports_directory_and_view_without_claiming_content_integrity() {
    let root = tempfile::tempdir().unwrap();
    let target = root.path().join("artifacts/demo/1.0.0");
    let view = root.path().join("views/demo");
    fs::create_dir_all(view.parent().unwrap()).unwrap();

    let recipe = archive_recipe("demo", Some(&view));
    install_manifest(&recipe, root.path()).unwrap();
    fs::write(target.join("bin/tool"), b"locally changed").unwrap();
    let resolved = resolve_manifest(&recipe, root.path());
    let report = repair(&resolved, root.path(), 3, Duration::from_millis(1)).unwrap();
    assert_eq!(
        report.outcome,
        RepairOutcome::Unchanged(RepairEvidence::ArchiveDirectory)
    );
    assert_eq!(fs::read(view.join("tool")).unwrap(), b"locally changed");
}

#[test]
fn repair_switches_a_view_pointing_at_the_wrong_or_missing_directory() {
    for dangling in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let view = root.path().join("view");
        let recipe = recipe_text(
            "demo-tool",
            "1.0.0",
            &fixture("tool-win.zip"),
            WIN_ZIP_HEX,
            Some("bin"),
            Some(&view),
        );
        let resolved = resolve_manifest(&recipe, root.path());
        fs::create_dir_all(resolved.target.join("bin")).unwrap();
        seed_intent(root.path(), "demo-tool", "1.0.0", Phase::Installed);
        let other = root.path().join("other");
        if !dangling {
            fs::create_dir(&other).unwrap();
        }
        make_directory_symlink(&other, &view);

        let report = repair(&resolved, root.path(), 1, Duration::ZERO).unwrap();
        assert_eq!(
            report.outcome,
            RepairOutcome::Repaired(RepairEvidence::ArchiveDirectory)
        );
        assert_eq!(
            fs::canonicalize(&view).unwrap(),
            fs::canonicalize(resolved.target.join("bin")).unwrap()
        );
    }
}

#[test]
fn repair_restores_a_missing_view_and_commits_the_next_generation() {
    let root = tempfile::tempdir().unwrap();
    let target = root.path().join("artifacts/demo/1.0.0");
    fs::create_dir_all(target.join("bin")).unwrap();
    fs::write(target.join("bin/tool"), b"repair me").unwrap();
    let view = root.path().join("views/demo");
    fs::create_dir_all(view.parent().unwrap()).unwrap();

    let recipe = archive_recipe("demo", Some(&view));
    let resolved = resolve_manifest(&recipe, root.path());
    seed_intent(root.path(), "demo", "1.0.0", Phase::Installed);
    let report = repair(&resolved, root.path(), 3, Duration::from_millis(1)).unwrap();
    assert_eq!(
        report.outcome,
        RepairOutcome::Repaired(RepairEvidence::ArchiveDirectory),
        "{report:?}"
    );
    assert!(view.is_symlink() || view.join("tool").exists());
    // The repair committed the next generation (supersede: latest record wins).
    let records = State::open(root.path()).unwrap().read().unwrap();
    let latest = records.iter().max_by_key(|r| r.generation).unwrap();
    assert_eq!(latest.generation, 2);
}

#[test]
fn repair_does_not_resurrect_a_deactivated_address() {
    let root = tempfile::tempdir().unwrap();
    let recipe = archive_recipe("demo", None);
    let resolved = resolve_manifest(&recipe, root.path());
    seed_intent(root.path(), "demo", "1.0.0", Phase::Deactivated);
    let report = repair(&resolved, root.path(), 3, Duration::from_millis(1)).unwrap();
    assert_eq!(report.outcome, RepairOutcome::Deactivated);
}

#[test]
fn repair_budget_exhaustion_reports_and_stops() {
    let root = tempfile::tempdir().unwrap();
    // The resolved source points at a missing path → the fresh cycle must fail every attempt.
    let recipe = archive_recipe("demo", None);
    let mut resolved = resolve_manifest(&recipe, root.path());
    resolved.source = Source::Local {
        path: root.path().join("missing-source"),
        format: crate::manifest::ArchiveFormat::Plain,
    };
    seed_intent(root.path(), "demo", "1.0.0", Phase::Installed);
    let report = repair(&resolved, root.path(), 2, Duration::from_millis(1)).unwrap();
    assert_eq!(report.outcome, RepairOutcome::Failed { published: None });
    assert_eq!(report.diagnostics.len(), 2, "each attempt is reported");
}

#[test]
fn repair_accepts_an_installed_plain_file_and_refuses_content_drift() {
    let root = tempfile::tempdir().unwrap();
    let recipe = recipe_text(
        "plain-tool",
        "1.0.0",
        &fixture("plain.bin"),
        PLAIN_BIN_HEX,
        None,
        None,
    );
    install_manifest(&recipe, root.path()).unwrap();
    let resolved = resolve_manifest(&recipe, root.path());

    let report = repair(&resolved, root.path(), 1, Duration::ZERO).unwrap();
    assert_eq!(
        report.outcome,
        RepairOutcome::Unchanged(RepairEvidence::FileDigest)
    );

    fs::write(&resolved.target, b"locally changed").unwrap();
    let report = repair(&resolved, root.path(), 1, Duration::ZERO).unwrap();
    assert_eq!(report.outcome, RepairOutcome::Failed { published: None });
    assert!(report.diagnostics[0].contains("installed file digest differs"));
    assert_eq!(fs::read(&resolved.target).unwrap(), b"locally changed");
}
