#![cfg(feature = "local")]

use std::fs;
use std::path::{Path, PathBuf};

use pulith::local::{LinkChange, LinkError, LocalPath};

/// Publish a versioned tree with a `bin/tool` subpath and return the target path.
fn publish_tree_with_bin(root: &Path, version: &str, bytes: &[u8]) -> PathBuf {
    let source = root.join(format!("source-{version}"));
    let target = root.join(format!("artifacts/demo-tool/{version}"));
    fs::create_dir_all(source.join("bin")).unwrap();
    fs::create_dir_all(target.parent().unwrap()).unwrap();
    fs::write(source.join("bin/tool"), bytes).unwrap();

    let material = LocalPath::new(source.clone()).unwrap();
    let admitted = LocalPath::new(target.clone()).unwrap();
    let stage = admitted.stage().unwrap();
    let tree = stage
        .copy_tree(material, std::path::PathBuf::new())
        .unwrap();
    tree.publish(admitted, pulith::local::TreeAdmission::Absent)
        .unwrap();
    target
}

#[test]
fn link_creates_the_view_parent_and_exposes_the_subpath() {
    let root = tempfile::tempdir().unwrap();
    let target = publish_tree_with_bin(root.path(), "1.0.0", b"tool-bytes\n");
    let view = root.path().join("views/demo-tool");

    // No views/ parent exists: the link law creates it.
    let evidence = LocalPath::new(target.clone())
        .unwrap()
        .link_view(&view, Some(Path::new("bin")))
        .unwrap();

    assert_eq!(evidence.change, LinkChange::Created);
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
    let target = publish_tree_with_bin(root.path(), "1.0.0", b"tool-bytes\n");
    let view = root.path().join("views/demo-tool");

    let evidence = LocalPath::new(target.clone())
        .unwrap()
        .link_view(&view, None)
        .unwrap();

    assert_eq!(evidence.change, LinkChange::Created);
    assert_eq!(fs::read_link(&view).unwrap(), target);
}

#[test]
fn link_auto_switches_an_occupied_directory_symlink_view() {
    let root = tempfile::tempdir().unwrap();
    let view = root.path().join("views/demo-tool");
    let first = publish_tree_with_bin(root.path(), "1.0.0", b"v1\n");
    LocalPath::new(first.clone())
        .unwrap()
        .link_view(&view, Some(Path::new("bin")))
        .unwrap();

    let second = publish_tree_with_bin(root.path(), "2.0.0", b"v2\n");
    let evidence = LocalPath::new(second.clone())
        .unwrap()
        .link_view(&view, Some(Path::new("bin")))
        .unwrap();

    assert_eq!(evidence.change, LinkChange::Replaced);
    assert_eq!(fs::read_link(&view).unwrap(), second.join("bin"));
    assert_eq!(fs::read(view.join("tool")).unwrap(), b"v2\n");
    // The first version's tree is untouched (retention is caller-owned).
    assert!(root.path().join("artifacts/demo-tool/1.0.0").is_dir());
}

#[test]
fn link_rejects_an_escaping_expose() {
    let root = tempfile::tempdir().unwrap();
    let target = publish_tree_with_bin(root.path(), "1.0.0", b"tool-bytes\n");
    let view = root.path().join("views/demo-tool");

    let error = LocalPath::new(target)
        .unwrap()
        .link_view(&view, Some(Path::new("..")))
        .unwrap_err();
    assert!(matches!(error, LinkError::InvalidExpose { .. }));
    assert!(!view.exists());
}

#[test]
fn link_rejects_a_missing_expose_and_creates_no_view() {
    let root = tempfile::tempdir().unwrap();
    let target = publish_tree_with_bin(root.path(), "1.0.0", b"tool-bytes\n");
    let view = root.path().join("views/demo-tool");

    let error = LocalPath::new(target)
        .unwrap()
        .link_view(&view, Some(Path::new("missing")))
        .unwrap_err();

    assert!(matches!(error, LinkError::ExposeNotDirectory { .. }));
    assert!(!view.exists());
}

#[test]
fn link_rejects_an_occupied_non_symlink_entry() {
    let root = tempfile::tempdir().unwrap();
    let target = publish_tree_with_bin(root.path(), "1.0.0", b"tool-bytes\n");
    let view = root.path().join("views/demo-tool");
    fs::create_dir_all(view.parent().unwrap()).unwrap();
    fs::write(&view, b"existing winner").unwrap();

    let error = LocalPath::new(target)
        .unwrap()
        .link_view(&view, Some(Path::new("bin")))
        .unwrap_err();

    assert!(matches!(
        error,
        LinkError::ViewConflict {
            observed: pulith::local::ViewEntry::Occupied(
                pulith::local::LocalObservation::File { .. }
            ),
            ..
        }
    ));
    assert_eq!(fs::read(&view).unwrap(), b"existing winner");
}
