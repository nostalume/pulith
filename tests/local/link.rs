#![cfg(feature = "local")]

use std::fs;
use std::path::Path;
use std::path::PathBuf;

use pulith::local::{
    ApplyEvidence, LinkError, LinkOutcome, LocalAcquire, LocalAcquireEvidence, LocalActivate,
    LocalApply, LocalObservation, OccupiedViewPolicy,
};
use pulith::{Applied, EvidenceChain, Materialize, MaterializeMode};

type AppliedTree = Applied<
    Materialize<&'static str, PathBuf, PathBuf>,
    EvidenceChain<LocalAcquireEvidence, ApplyEvidence>,
>;

/// Publish a versioned tree with a `bin/tool` subpath and return the applied receipt.
fn publish_tree_with_bin(root: &Path, version: &str, bytes: &[u8]) -> AppliedTree {
    let source = root.join(format!("source-{version}"));
    let target = root.join(format!("artifacts/demo-tool/{version}"));
    fs::create_dir_all(source.join("bin")).unwrap();
    fs::create_dir_all(target.parent().unwrap()).unwrap();
    fs::write(source.join("bin/tool"), bytes).unwrap();

    LocalApply
        .apply(
            LocalAcquire
                .acquire(Materialize::new(
                    "demo-tool",
                    source.clone(),
                    target.clone(),
                    MaterializeMode::CreateNew,
                ))
                .unwrap(),
        )
        .unwrap()
}

#[test]
fn link_creates_the_view_parent_and_exposes_the_subpath() {
    let root = tempfile::tempdir().unwrap();
    let applied = publish_tree_with_bin(root.path(), "1.0.0", b"tool-bytes\n");
    let view = root.path().join("views/demo-tool");

    // No views/ parent exists: the link law creates it.
    let outcome = LocalActivate
        .link(
            applied,
            &view,
            Path::new("bin"),
            OccupiedViewPolicy::AutoSwitch,
        )
        .unwrap();

    assert_eq!(outcome, LinkOutcome::Activated);
    let target = root.path().join("artifacts/demo-tool/1.0.0");
    assert!(
        fs::symlink_metadata(&view)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(fs::read_link(&view).unwrap(), target.join("bin"));
    assert_eq!(fs::read(view.join("tool")).unwrap(), b"tool-bytes\n");
}

#[test]
fn link_root_exposes_the_tree_root() {
    let root = tempfile::tempdir().unwrap();
    let applied = publish_tree_with_bin(root.path(), "1.0.0", b"tool-bytes\n");
    let view = root.path().join("views/demo-tool");

    let outcome = LocalActivate
        .link_root(applied, &view, OccupiedViewPolicy::AutoSwitch)
        .unwrap();

    assert_eq!(outcome, LinkOutcome::Activated);
    let target = root.path().join("artifacts/demo-tool/1.0.0");
    assert_eq!(fs::read_link(&view).unwrap(), target);
}

#[test]
fn link_auto_switches_an_occupied_directory_symlink_view() {
    let root = tempfile::tempdir().unwrap();
    let view = root.path().join("views/demo-tool");
    let first = publish_tree_with_bin(root.path(), "1.0.0", b"v1\n");
    LocalActivate
        .link(
            first,
            &view,
            Path::new("bin"),
            OccupiedViewPolicy::AutoSwitch,
        )
        .unwrap();

    let second = publish_tree_with_bin(root.path(), "2.0.0", b"v2\n");
    let outcome = LocalActivate
        .link(
            second,
            &view,
            Path::new("bin"),
            OccupiedViewPolicy::AutoSwitch,
        )
        .unwrap();

    assert_eq!(outcome, LinkOutcome::Switched);
    let second_target = root.path().join("artifacts/demo-tool/2.0.0");
    assert_eq!(fs::read_link(&view).unwrap(), second_target.join("bin"));
    assert_eq!(fs::read(view.join("tool")).unwrap(), b"v2\n");
    // The first version's tree is untouched (retention is caller-owned).
    assert!(root.path().join("artifacts/demo-tool/1.0.0").is_dir());
}

#[test]
fn link_refuses_an_occupied_view_under_the_refuse_policy() {
    let root = tempfile::tempdir().unwrap();
    let view = root.path().join("views/demo-tool");
    let first = publish_tree_with_bin(root.path(), "1.0.0", b"v1\n");
    LocalActivate
        .link(
            first,
            &view,
            Path::new("bin"),
            OccupiedViewPolicy::AutoSwitch,
        )
        .unwrap();

    let second = publish_tree_with_bin(root.path(), "2.0.0", b"v2\n");
    let error = LocalActivate
        .link(second, &view, Path::new("bin"), OccupiedViewPolicy::Refuse)
        .unwrap_err();

    assert!(matches!(
        error,
        LinkError::ViewConflict {
            view: ref v,
            observed: LocalObservation::SymlinkToDirectory,
            ..
        } if *v == view
    ));
    // Nothing was replaced: the view still points at the first tree.
    let first_target = root.path().join("artifacts/demo-tool/1.0.0");
    assert_eq!(fs::read_link(&view).unwrap(), first_target.join("bin"));
}

#[test]
fn link_rejects_an_escaping_expose() {
    let root = tempfile::tempdir().unwrap();
    let applied = publish_tree_with_bin(root.path(), "1.0.0", b"tool-bytes\n");
    let view = root.path().join("views/demo-tool");

    let error = LocalActivate
        .link(
            applied,
            &view,
            Path::new(".."),
            OccupiedViewPolicy::AutoSwitch,
        )
        .unwrap_err();
    assert!(matches!(error, LinkError::InvalidExpose { .. }));
    assert!(!view.exists());
}

#[test]
fn link_rejects_a_missing_expose_and_creates_no_view() {
    let root = tempfile::tempdir().unwrap();
    let applied = publish_tree_with_bin(root.path(), "1.0.0", b"tool-bytes\n");
    let view = root.path().join("views/demo-tool");

    let error = LocalActivate
        .link(
            applied,
            &view,
            Path::new("missing"),
            OccupiedViewPolicy::AutoSwitch,
        )
        .unwrap_err();

    assert!(matches!(error, LinkError::ExposeNotDirectory { .. }));
    assert!(!view.exists());
}

#[test]
fn link_rejects_an_occupied_non_symlink_entry() {
    let root = tempfile::tempdir().unwrap();
    let applied = publish_tree_with_bin(root.path(), "1.0.0", b"tool-bytes\n");
    let view = root.path().join("views/demo-tool");
    fs::create_dir_all(view.parent().unwrap()).unwrap();
    fs::write(&view, b"existing winner").unwrap();

    let error = LocalActivate
        .link(
            applied,
            &view,
            Path::new("bin"),
            OccupiedViewPolicy::AutoSwitch,
        )
        .unwrap_err();

    assert!(matches!(
        error,
        LinkError::ViewConflict {
            observed: LocalObservation::File { .. },
            ..
        }
    ));
    assert_eq!(fs::read(&view).unwrap(), b"existing winner");
}
