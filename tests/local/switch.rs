#![cfg(feature = "local")]

use std::fs;

use crate::common::publish_tree;
use pulith::Link;
use pulith::local::LinkError;
use pulith::local::{LinkChange, LocalObservation, LocalTarget};

#[test]
fn switch_replaces_only_the_active_view_and_records_the_outcome() {
    let root = tempfile::tempdir().unwrap();
    let (first_target, _) = publish_tree(root.path(), "1.0.0", b"first artifact");
    let (second_target, _) = publish_tree(root.path(), "2.0.0", b"second artifact");
    let view = root.path().join("views/demo-tool");
    fs::create_dir_all(view.parent().unwrap()).unwrap();
    LocalTarget::new(first_target.clone())
        .unwrap()
        .link_root(&view)
        .unwrap();

    let evidence = LocalTarget::new(second_target.clone())
        .unwrap()
        .link_root(&view)
        .unwrap();

    assert_eq!(evidence.change, LinkChange::Replaced);
    assert_eq!(evidence.source, second_target);
    assert_eq!(evidence.view, view);
    assert!(
        fs::symlink_metadata(&view)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(fs::read(view.join("tool.txt")).unwrap(), b"second artifact");
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
    let (_, first_applied) = publish_tree(root.path(), "1.0.0", b"first artifact");
    let (second_target, _) = publish_tree(root.path(), "2.0.0", b"second artifact");
    let view = root.path().join("views/demo-tool");
    fs::create_dir_all(view.parent().unwrap()).unwrap();
    fs::write(&view, b"existing winner").unwrap();

    let error = LocalTarget::new(second_target)
        .unwrap()
        .link_root(&view)
        .unwrap_err();

    assert!(matches!(
        error,
        LinkError::ViewConflict {
            observed: LocalObservation::File { .. },
            ..
        }
    ));
    assert_eq!(fs::read(&view).unwrap(), b"existing winner");
    let _ = first_applied;
}

#[test]
fn switch_rejects_directory_views_without_mutation() {
    let root = tempfile::tempdir().unwrap();
    let (_, first_applied) = publish_tree(root.path(), "1.0.0", b"first artifact");
    let (second_target, _) = publish_tree(root.path(), "2.0.0", b"second artifact");
    let view = root.path().join("views/demo-tool");
    fs::create_dir_all(&view).unwrap();

    let error = LocalTarget::new(second_target)
        .unwrap()
        .link_root(&view)
        .unwrap_err();
    assert!(matches!(
        error,
        LinkError::ViewConflict {
            observed: LocalObservation::Directory,
            ..
        }
    ));
    assert!(view.is_dir());
    let _ = first_applied;
}

#[test]
fn switch_rejects_non_directory_sources_without_changing_the_old_view() {
    let root = tempfile::tempdir().unwrap();
    let (first_target, _) = publish_tree(root.path(), "1.0.0", b"first artifact");
    let view = root.path().join("views/demo-tool");
    fs::create_dir_all(view.parent().unwrap()).unwrap();
    LocalTarget::new(first_target.clone())
        .unwrap()
        .link_root(&view)
        .unwrap();

    let missing = root.path().join("missing");
    let missing_error = LocalTarget::new(missing)
        .unwrap()
        .link_root(&view)
        .unwrap_err();
    assert!(matches!(
        missing_error,
        LinkError::ExposeNotDirectory {
            observed: LocalObservation::Missing,
            ..
        }
    ));

    let file = root.path().join("file");
    fs::write(&file, b"not a directory").unwrap();
    let file_error = LocalTarget::new(file)
        .unwrap()
        .link_root(&view)
        .unwrap_err();
    assert!(matches!(
        file_error,
        LinkError::ExposeNotDirectory {
            observed: LocalObservation::File { .. },
            ..
        }
    ));

    assert_eq!(fs::read(view.join("tool.txt")).unwrap(), b"first artifact");
    assert_eq!(fs::read_link(&view).unwrap(), first_target);
}

#[test]
fn switch_retries_an_occupied_staged_name_without_deleting_it() {
    let root = tempfile::tempdir().unwrap();
    let (first_target, _) = publish_tree(root.path(), "1.0.0", b"first artifact");
    let (second_target, _) = publish_tree(root.path(), "2.0.0", b"second artifact");
    let view = root.path().join("views/demo-tool");
    let parent = view.parent().unwrap();
    fs::create_dir_all(parent).unwrap();
    LocalTarget::new(first_target.clone())
        .unwrap()
        .link_root(&view)
        .unwrap();
    let occupied = parent.join(format!(".pulith-switch-{}-0", std::process::id()));
    fs::write(&occupied, b"caller-owned collision").unwrap();

    let evidence = LocalTarget::new(second_target.clone())
        .unwrap()
        .link_root(&view)
        .unwrap();

    assert_eq!(evidence.change, LinkChange::Replaced);
    assert_eq!(fs::read(view.join("tool.txt")).unwrap(), b"second artifact");
    assert_eq!(fs::read(&occupied).unwrap(), b"caller-owned collision");
    assert_eq!(fs::read_link(&view).unwrap(), second_target);
}
