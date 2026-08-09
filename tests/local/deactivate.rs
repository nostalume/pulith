#![cfg(feature = "local")]
//! The unlink law removes exactly one active directory-symlink view, is idempotent when missing,
//! and refuses any other entry without removing it.
//! The published tree is never touched.

use std::fs;
use std::path::{Path, PathBuf};

use pulith::archive::ArchivePolicy;
use pulith::local::{
    LinkChange, LocalObservation, LocalSource, LocalTarget, UnlinkChange, UnlinkError,
};
use pulith::{Acquire, Link, Unlink};

/// Publish one versioned tree at `root/artifacts/demo-tool/<version>` via the trait-only local
/// chain (acquire -> prepare -> apply) and return the target path.
fn publish_tree(root: &Path, version: &'static str, contents: &'static [u8]) -> PathBuf {
    let source = root.join(format!("source-{version}"));
    let target = root.join(format!("artifacts/demo-tool/{version}"));
    fs::create_dir_all(&source).unwrap();
    fs::create_dir_all(target.parent().unwrap()).unwrap();
    fs::write(source.join("tool.txt"), contents).unwrap();

    let material = LocalSource::new(source).unwrap().acquire().unwrap();
    let admitted = LocalTarget::new(target.clone()).unwrap();
    let stage = admitted.stage().unwrap();
    let (tree, _) = material.prepare(stage, ArchivePolicy::default()).unwrap();
    tree.publish(admitted).unwrap();
    target
}

/// Link the published tree's root at `view` (occupied views auto-switch) and return the outcome.
fn activate_root(target: &Path, view: &Path) -> LinkChange {
    LocalTarget::new(target)
        .unwrap()
        .link_root(view)
        .unwrap()
        .change
}

#[cfg(unix)]
fn file_symlink(original: &Path, link: &Path) {
    std::os::unix::fs::symlink(original, link).unwrap();
}

#[cfg(windows)]
fn file_symlink(original: &Path, link: &Path) {
    std::os::windows::fs::symlink_file(original, link).unwrap();
}

#[test]
fn unlink_removes_only_the_active_view_and_preserves_the_tree() {
    let root = tempfile::tempdir().unwrap();
    let target = publish_tree(root.path(), "1.0.0", b"artifact payload");
    let view = root.path().join("views/demo-tool");
    assert_eq!(activate_root(&target, &view), LinkChange::Created);

    let evidence = LocalTarget::new(&view).unwrap().unlink().unwrap();

    assert_eq!(evidence.view, view);
    assert_eq!(evidence.change, UnlinkChange::Removed);
    // The view link is gone (Windows: the symlink_dir/junction is removed; the tree stays).
    assert!(fs::symlink_metadata(&view).is_err());
    assert_eq!(
        fs::read(target.join("tool.txt")).unwrap(),
        b"artifact payload"
    );
    assert!(target.is_dir());
}

#[test]
fn unlink_on_missing_view_is_idempotent_with_missing_prior() {
    let root = tempfile::tempdir().unwrap();
    let view = root.path().join("views/never-activated");

    let evidence = LocalTarget::new(&view).unwrap().unlink().unwrap();

    assert_eq!(evidence.view, view);
    assert_eq!(evidence.change, UnlinkChange::Unchanged);
    assert!(fs::symlink_metadata(&view).is_err());
}

#[test]
fn unlink_rejects_a_regular_file_view_without_removal() {
    let root = tempfile::tempdir().unwrap();
    let view = root.path().join("views/plain-file");
    fs::create_dir_all(view.parent().unwrap()).unwrap();
    fs::write(&view, b"not a view").unwrap();

    let error = LocalTarget::new(&view)
        .unwrap()
        .unlink()
        .expect_err("a regular file is not an active view");

    match error {
        UnlinkError::NotActiveView {
            view: seen,
            observed,
        } => {
            assert_eq!(seen, view);
            assert_eq!(observed, LocalObservation::File { bytes: 10 });
        }
        other => panic!("expected UnlinkNotActiveView, got {other:?}"),
    }
    assert_eq!(fs::read(&view).unwrap(), b"not a view");
}

#[test]
fn unlink_rejects_directory_and_file_symlink_views_without_removal() {
    let root = tempfile::tempdir().unwrap();

    // A real directory at the view path is refused and left intact.
    let directory_view = root.path().join("views/plain-directory");
    fs::create_dir_all(&directory_view).unwrap();
    let error = LocalTarget::new(&directory_view)
        .unwrap()
        .unlink()
        .expect_err("a directory is not an active view");
    assert!(matches!(
        error,
        UnlinkError::NotActiveView {
            view: _,
            observed: LocalObservation::Directory,
        }
    ));
    assert!(directory_view.is_dir());

    // A symlink to a FILE is refused: only a directory-symlink view is removable.
    let file = root.path().join("some-file.txt");
    fs::write(&file, b"target file").unwrap();
    let file_symlink_view = root.path().join("views/file-symlink");
    fs::create_dir_all(file_symlink_view.parent().unwrap()).unwrap();
    file_symlink(&file, &file_symlink_view);
    let error = LocalTarget::new(&file_symlink_view)
        .unwrap()
        .unlink()
        .expect_err("a file symlink is not an active directory view");
    match error {
        UnlinkError::NotActiveView {
            view: seen,
            observed,
        } => {
            assert_eq!(seen, file_symlink_view);
            assert_eq!(observed, LocalObservation::SymlinkToFile);
        }
        other => panic!("expected UnlinkNotActiveView with the link-target kind, got {other:?}"),
    }
    assert!(
        fs::symlink_metadata(&file_symlink_view)
            .unwrap()
            .file_type()
            .is_symlink()
    );
}

#[test]
fn unlink_holds_the_view_state_cycle() {
    let root = tempfile::tempdir().unwrap();
    let first_target = publish_tree(root.path(), "1.0.0", b"first artifact");
    let second_target = publish_tree(root.path(), "2.0.0", b"second artifact");
    let view = root.path().join("views/demo-tool");

    // link_root -> unlink -> link_root -> link_root (auto-switch) -> unlink
    assert_eq!(activate_root(&first_target, &view), LinkChange::Created);
    let evidence = LocalTarget::new(&view).unwrap().unlink().unwrap();
    assert_eq!(evidence.change, UnlinkChange::Removed);
    assert!(fs::symlink_metadata(&view).is_err());

    assert_eq!(activate_root(&first_target, &view), LinkChange::Created);
    assert_eq!(activate_root(&second_target, &view), LinkChange::Replaced);
    assert!(
        fs::symlink_metadata(&view)
            .unwrap()
            .file_type()
            .is_symlink()
    );

    let evidence = LocalTarget::new(&view).unwrap().unlink().unwrap();
    assert_eq!(evidence.change, UnlinkChange::Removed);
    assert_eq!(evidence.view, view);
    assert!(fs::symlink_metadata(&view).is_err());
    assert_eq!(
        fs::read(first_target.join("tool.txt")).unwrap(),
        b"first artifact"
    );
}
