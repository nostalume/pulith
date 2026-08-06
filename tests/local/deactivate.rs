#![cfg(feature = "local")]

use std::fs;

use crate::common::{directory_symlink, publish_tree};
use pulith::Activate;
use pulith::local::{
    LocalActivate, LocalDeactivate, LocalDeactivateError, LocalDeactivatePrior, LocalObservation,
};

#[test]
fn deactivate_removes_only_the_active_view_and_preserves_the_tree() {
    let root = tempfile::tempdir().unwrap();
    let (target, applied) = publish_tree(root.path(), "1.0.0", b"artifact payload");
    let view = root.path().join("views/demo-tool");
    fs::create_dir_all(view.parent().unwrap()).unwrap();
    let _activated = LocalActivate
        .activate(applied.clone(), view.clone())
        .unwrap();

    let deactivated = LocalDeactivate.activate(applied, view.clone()).unwrap();

    assert_eq!(deactivated.input, view);
    assert_eq!(deactivated.evidence.current.view, view);
    assert_eq!(
        deactivated.evidence.current.prior,
        LocalDeactivatePrior::DirectorySymlink
    );
    // The view link is gone (Windows: the symlink_dir/junction is removed; the tree stays).
    assert!(fs::symlink_metadata(&view).is_err());
    assert_eq!(
        fs::read(target.join("tool.txt")).unwrap(),
        b"artifact payload"
    );
    assert!(target.is_dir());
}

#[test]
fn deactivate_on_missing_view_is_idempotent_with_missing_prior() {
    let root = tempfile::tempdir().unwrap();
    let (_target, applied) = publish_tree(root.path(), "1.0.0", b"artifact payload");
    let view = root.path().join("views/never-activated");

    let deactivated = LocalDeactivate.activate(applied, view.clone()).unwrap();

    assert_eq!(deactivated.input, view);
    assert_eq!(
        deactivated.evidence.current.prior,
        LocalDeactivatePrior::Missing
    );
    assert!(fs::symlink_metadata(&view).is_err());
}

#[test]
fn deactivate_rejects_a_regular_file_view_without_removal() {
    let root = tempfile::tempdir().unwrap();
    let (_target, applied) = publish_tree(root.path(), "1.0.0", b"artifact payload");
    let view = root.path().join("views/plain-file");
    fs::create_dir_all(view.parent().unwrap()).unwrap();
    fs::write(&view, b"not a view").unwrap();

    let error = LocalDeactivate
        .activate(applied, view.clone())
        .expect_err("a regular file is not an active view");

    match error {
        LocalDeactivateError::NotActiveView {
            view: seen,
            observed,
            ..
        } => {
            assert_eq!(seen, view);
            assert_eq!(observed, LocalObservation::File { bytes: 10 });
        }
        other => panic!("expected NotActiveView, got {other:?}"),
    }
    assert_eq!(fs::read(&view).unwrap(), b"not a view");
}

#[test]
fn deactivate_rejects_directory_and_file_symlink_views_without_removal() {
    let root = tempfile::tempdir().unwrap();
    let (_target, applied) = publish_tree(root.path(), "1.0.0", b"artifact payload");

    // A real directory at the view path is refused and left intact.
    let directory_view = root.path().join("views/plain-directory");
    fs::create_dir_all(&directory_view).unwrap();
    let error = LocalDeactivate
        .activate(applied.clone(), directory_view.clone())
        .expect_err("a directory is not an active view");
    assert!(matches!(
        error,
        LocalDeactivateError::NotActiveView {
            view: _,
            observed: LocalObservation::Directory,
            ..
        }
    ));
    assert!(directory_view.is_dir());

    // A symlink to a FILE is refused: only a directory-symlink view is removable.
    let file = root.path().join("some-file.txt");
    fs::write(&file, b"target file").unwrap();
    let file_symlink_view = root.path().join("views/file-symlink");
    directory_symlink(&file, &file_symlink_view);
    let error = LocalDeactivate
        .activate(applied, file_symlink_view.clone())
        .expect_err("a file symlink is not an active directory view");
    match error {
        LocalDeactivateError::NotActiveView {
            view: seen,
            observed: LocalObservation::File { .. },
            ..
        } => assert_eq!(seen, file_symlink_view),
        other => panic!("expected NotActiveView with the resolved target kind, got {other:?}"),
    }
    assert!(
        fs::symlink_metadata(&file_symlink_view)
            .unwrap()
            .file_type()
            .is_symlink()
    );
}

#[test]
fn deactivate_holds_the_view_state_cycle() {
    let root = tempfile::tempdir().unwrap();
    let (first_target, first_applied) = publish_tree(root.path(), "1.0.0", b"first artifact");
    let (_second_target, second_applied) = publish_tree(root.path(), "2.0.0", b"second artifact");
    let view = root.path().join("views/demo-tool");
    fs::create_dir_all(view.parent().unwrap()).unwrap();

    // activate -> deactivate -> activate -> switch -> deactivate
    let _activated = LocalActivate
        .activate(first_applied.clone(), view.clone())
        .unwrap();
    let deactivated = LocalDeactivate
        .activate(first_applied.clone(), view.clone())
        .unwrap();
    assert_eq!(
        deactivated.evidence.current.prior,
        LocalDeactivatePrior::DirectorySymlink
    );
    assert!(fs::symlink_metadata(&view).is_err());

    let _activated = LocalActivate.activate(first_applied, view.clone()).unwrap();
    let _switched = pulith::local::LocalSwitch
        .activate(second_applied.clone(), view.clone())
        .unwrap();
    assert!(
        fs::symlink_metadata(&view)
            .unwrap()
            .file_type()
            .is_symlink()
    );

    let deactivated = LocalDeactivate
        .activate(second_applied, view.clone())
        .unwrap();
    assert_eq!(
        deactivated.evidence.current.prior,
        LocalDeactivatePrior::DirectorySymlink
    );
    assert!(fs::symlink_metadata(&view).is_err());
    assert_eq!(
        fs::read(first_target.join("tool.txt")).unwrap(),
        b"first artifact"
    );
}
