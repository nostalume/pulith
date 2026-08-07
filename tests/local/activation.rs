#![cfg(feature = "local")]

use std::fs;

use crate::common::{publish_tree, receipt_for};
use pulith::local::{LocalActivate, LocalActivateError, LocalObservation};

#[test]
fn activate_exposes_published_tree_without_copying() {
    let root = tempfile::tempdir().unwrap();
    let (source, applied) = publish_tree(root.path(), "1.0.0", b"artifact payload");
    let view = root.path().join("views/demo-tool");
    fs::create_dir_all(view.parent().unwrap()).unwrap();

    let activated = LocalActivate.activate(applied, view.clone()).unwrap();

    assert_eq!(activated.input, view);
    assert_eq!(activated.evidence.current.source, source);
    assert_eq!(
        activated.evidence.current.view_observation,
        Some(LocalObservation::SymlinkToDirectory)
    );
    assert!(
        fs::symlink_metadata(&activated.input)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(
        fs::read(activated.input.join("tool.txt")).unwrap(),
        b"artifact payload"
    );
}

#[test]
fn activate_rejects_existing_view_without_mutation() {
    let root = tempfile::tempdir().unwrap();
    let (_source, applied) = publish_tree(root.path(), "1.0.0", b"artifact payload");
    let view = root.path().join("views/demo-tool");
    fs::create_dir_all(view.parent().unwrap()).unwrap();
    fs::write(&view, b"existing winner").unwrap();

    let error = LocalActivate.activate(applied, view.clone()).unwrap_err();

    assert!(matches!(
        error,
        LocalActivateError::ViewAlreadyExists {
            observed: LocalObservation::File { .. },
            ..
        }
    ));
    assert_eq!(fs::read(&view).unwrap(), b"existing winner");
}

#[test]
fn activate_rejects_missing_or_file_source_without_view() {
    let root = tempfile::tempdir().unwrap();
    let views = root.path().join("views");
    fs::create_dir_all(&views).unwrap();

    let missing = root.path().join("artifacts/missing");
    let missing_view = views.join("missing");
    let missing_error = LocalActivate
        .activate(receipt_for(&missing), missing_view.clone())
        .unwrap_err();
    assert!(matches!(
        missing_error,
        LocalActivateError::SourceNotDirectory {
            observed: LocalObservation::Missing,
            ..
        }
    ));
    assert!(!missing_view.exists());

    let file = root.path().join("artifacts/file");
    fs::create_dir_all(file.parent().unwrap()).unwrap();
    fs::write(&file, b"not a tree").unwrap();
    let file_view = views.join("file");
    let file_error = LocalActivate
        .activate(receipt_for(&file), file_view.clone())
        .unwrap_err();
    assert!(matches!(
        file_error,
        LocalActivateError::SourceNotDirectory {
            observed: LocalObservation::File { .. },
            ..
        }
    ));
    assert!(!file_view.exists());
}

#[test]
fn activate_never_creates_a_view_parent() {
    let root = tempfile::tempdir().unwrap();
    let (_source, applied) = publish_tree(root.path(), "1.0.0", b"artifact payload");
    let parent = root.path().join("missing-views");
    let view = parent.join("demo-tool");

    let error = LocalActivate.activate(applied, view.clone()).unwrap_err();

    assert!(matches!(error, LocalActivateError::BeforeActivation { .. }));
    assert!(!parent.exists());
    assert!(!view.exists());
}
