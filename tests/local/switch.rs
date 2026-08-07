#![cfg(feature = "local")]

use std::fs;

use crate::common::{directory_symlink, publish_tree, receipt_for};
use pulith::local::{
    LocalActivate, LocalObservation, LocalSwitch, LocalSwitchBackend, LocalSwitchError,
};
#[test]
fn switch_replaces_only_the_active_view_and_records_the_backend() {
    let root = tempfile::tempdir().unwrap();
    let (first_target, first_applied) = publish_tree(root.path(), "1.0.0", b"first artifact");
    let (second_target, second_applied) = publish_tree(root.path(), "2.0.0", b"second artifact");
    let view = root.path().join("views/demo-tool");
    fs::create_dir_all(view.parent().unwrap()).unwrap();
    LocalActivate.activate(first_applied, view.clone()).unwrap();

    let switched = LocalSwitch.activate(second_applied, view.clone()).unwrap();

    assert_eq!(switched.input, view);
    assert_eq!(switched.evidence.current.previous_source, first_target);
    assert_eq!(switched.evidence.current.current_source, second_target);
    assert_eq!(
        switched.evidence.current.view_observation,
        Some(LocalObservation::SymlinkToDirectory)
    );
    #[cfg(unix)]
    assert_eq!(
        switched.evidence.current.backend,
        LocalSwitchBackend::UnixRename
    );
    #[cfg(windows)]
    assert_eq!(
        switched.evidence.current.backend,
        LocalSwitchBackend::WindowsFileRenameInfoExPosix
    );
    assert!(
        fs::symlink_metadata(&switched.input)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(
        fs::read(switched.input.join("tool.txt")).unwrap(),
        b"second artifact"
    );
    assert_eq!(
        fs::read(first_target.join("tool.txt")).unwrap(),
        b"first artifact"
    );
    assert_eq!(
        fs::read(second_target.join("tool.txt")).unwrap(),
        b"second artifact"
    );
}

#[test]
fn switch_rejects_a_non_symlink_view_without_mutation() {
    let root = tempfile::tempdir().unwrap();
    let (_first_target, first_applied) = publish_tree(root.path(), "1.0.0", b"first artifact");
    let (_second_target, second_applied) = publish_tree(root.path(), "2.0.0", b"second artifact");
    let view = root.path().join("views/demo-tool");
    fs::create_dir_all(view.parent().unwrap()).unwrap();
    fs::write(&view, b"existing winner").unwrap();

    let error = LocalSwitch
        .activate(second_applied, view.clone())
        .unwrap_err();

    assert!(matches!(
        error,
        LocalSwitchError::ViewNotSymlink {
            observed: LocalObservation::File { .. },
            ..
        }
    ));
    assert_eq!(fs::read(&view).unwrap(), b"existing winner");
    drop(first_applied);
}
#[test]
fn switch_rejects_missing_and_directory_views_without_mutation() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("artifact");
    let views = root.path().join("views");
    fs::create_dir_all(&source).unwrap();
    fs::create_dir_all(&views).unwrap();

    let missing = views.join("missing");
    let missing_error = LocalSwitch
        .activate(receipt_for(&source), missing.clone())
        .unwrap_err();
    assert!(matches!(
        missing_error,
        LocalSwitchError::ViewNotSymlink {
            observed: LocalObservation::Missing,
            ..
        }
    ));
    assert!(!missing.exists());

    let directory = views.join("directory");
    fs::create_dir(&directory).unwrap();
    let directory_error = LocalSwitch
        .activate(receipt_for(&source), directory.clone())
        .unwrap_err();
    assert!(matches!(
        directory_error,
        LocalSwitchError::ViewNotSymlink {
            observed: LocalObservation::Directory,
            ..
        }
    ));
    assert!(directory.is_dir());
}

#[test]
fn switch_rejects_non_directory_sources_without_changing_the_old_view() {
    let root = tempfile::tempdir().unwrap();
    let (first_target, first_applied) = publish_tree(root.path(), "1.0.0", b"first artifact");
    let view = root.path().join("views/demo-tool");
    fs::create_dir_all(view.parent().unwrap()).unwrap();
    LocalActivate.activate(first_applied, view.clone()).unwrap();

    let missing = root.path().join("missing");
    let missing_error = LocalSwitch
        .activate(receipt_for(&missing), view.clone())
        .unwrap_err();
    assert!(matches!(
        missing_error,
        LocalSwitchError::SourceNotDirectory {
            observed: LocalObservation::Missing,
            ..
        }
    ));

    let file = root.path().join("file");
    fs::write(&file, b"not a directory").unwrap();
    let file_error = LocalSwitch
        .activate(receipt_for(&file), view.clone())
        .unwrap_err();
    assert!(matches!(
        file_error,
        LocalSwitchError::SourceNotDirectory {
            observed: LocalObservation::File { .. },
            ..
        }
    ));

    let linked_directory = root.path().join("linked-directory");
    directory_symlink(&first_target, &linked_directory);
    let symlink_error = LocalSwitch
        .activate(receipt_for(&linked_directory), view.clone())
        .unwrap_err();
    assert!(matches!(
        symlink_error,
        LocalSwitchError::SourceNotDirectory {
            observed: LocalObservation::SymlinkToDirectory,
            ..
        }
    ));

    assert_eq!(fs::read(view.join("tool.txt")).unwrap(), b"first artifact");
    assert_eq!(fs::read_link(&view).unwrap(), first_target);
}

#[test]
fn switch_retries_an_occupied_staged_name_without_deleting_it() {
    let root = tempfile::tempdir().unwrap();
    let (_first_target, first_applied) = publish_tree(root.path(), "1.0.0", b"first artifact");
    let (_second_target, second_applied) = publish_tree(root.path(), "2.0.0", b"second artifact");
    let view = root.path().join("views/demo-tool");
    let parent = view.parent().unwrap();
    fs::create_dir_all(parent).unwrap();
    LocalActivate.activate(first_applied, view.clone()).unwrap();
    let occupied = parent.join(format!(".pulith-switch-{}-0", std::process::id()));
    fs::write(&occupied, b"caller-owned collision").unwrap();

    let switched = LocalSwitch.activate(second_applied, view.clone()).unwrap();

    assert_eq!(
        fs::read(switched.input.join("tool.txt")).unwrap(),
        b"second artifact"
    );
    assert_eq!(fs::read(&occupied).unwrap(), b"caller-owned collision");
    assert_eq!(fs::read_link(&view).unwrap(), _second_target);
}

#[test]
fn switch_never_creates_a_missing_view_parent() {
    let root = tempfile::tempdir().unwrap();
    let (first_target, first_applied) = publish_tree(root.path(), "1.0.0", b"first artifact");
    let view = root.path().join("missing-views/demo-tool");

    let error = LocalSwitch
        .activate(first_applied, view.clone())
        .unwrap_err();

    assert!(matches!(error, LocalSwitchError::BeforeSwitch { .. }));
    assert!(!view.parent().unwrap().exists());
    assert!(first_target.exists());
}
