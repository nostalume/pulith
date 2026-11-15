#![cfg(feature = "local")]

use std::fs;

use crate::common::publish_tree;
use pulith::local::{
    LinkChange, LinkError, LinkEvidence, LocalObservation, LocalPath, UnlinkChange, ViewEntry,
    ViewStatus,
};

#[test]
fn link_root_creates_a_directory_symlink_view_to_the_published_tree() {
    let root = tempfile::tempdir().unwrap();
    let (target, _evidence) = publish_tree(root.path(), "1.0.0", b"artifact payload");
    let view = root.path().join("views/demo-tool");

    let evidence = LocalPath::new(target.as_path())
        .unwrap()
        .link_view(view.as_path(), None)
        .unwrap();

    assert_eq!(evidence.change, LinkChange::Created);
    assert_eq!(
        evidence,
        LinkEvidence {
            source: target.clone(),
            view: view.clone(),
            change: LinkChange::Created,
        }
    );
    // The view is a directory symlink to the published target, and no bytes were copied.
    assert!(
        fs::symlink_metadata(&view)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(
        fs::canonicalize(&view).unwrap(),
        fs::canonicalize(&target).unwrap()
    );
    assert_eq!(
        fs::read(view.join("tool.txt")).unwrap(),
        b"artifact payload"
    );
}

#[test]
fn entry_and_view_inspection_report_distinct_facts() {
    let root = tempfile::tempdir().unwrap();
    let (target, _) = publish_tree(root.path(), "1.0.0", b"artifact payload");
    let view = root.path().join("views/demo-tool");

    let target = LocalPath::new(target).unwrap();
    assert_eq!(
        target.inspect_view(&view, None).unwrap(),
        ViewStatus::Missing
    );
    target.link_view(&view, None).unwrap();

    let observation = LocalPath::new(&view).unwrap().inspect_entry().unwrap();
    assert_eq!(observation, LocalObservation::Symlink);
    assert_eq!(
        target.inspect_view(&view, None).unwrap(),
        ViewStatus::Matches
    );
    let other = root.path().join("other");
    fs::create_dir(&other).unwrap();
    assert_eq!(
        LocalPath::new(other)
            .unwrap()
            .inspect_view(&view, None)
            .unwrap(),
        ViewStatus::WrongTarget
    );
    fs::remove_dir_all(target.as_path()).unwrap();
    assert_eq!(
        target.inspect_view(&view, None).unwrap(),
        ViewStatus::Dangling
    );
}

#[test]
fn link_root_creates_the_view_parent() {
    let root = tempfile::tempdir().unwrap();
    let (target, _) = publish_tree(root.path(), "1.0.0", b"artifact payload");
    let view = root.path().join("missing-views/demo-tool");

    let evidence = LocalPath::new(target.as_path())
        .unwrap()
        .link_view(view.as_path(), None)
        .unwrap();

    assert_eq!(evidence.change, LinkChange::Created);
    assert!(view.parent().unwrap().is_dir());
    assert!(
        fs::symlink_metadata(&view)
            .unwrap()
            .file_type()
            .is_symlink()
    );
}

#[test]
fn link_root_replaces_an_existing_active_view() {
    let root = tempfile::tempdir().unwrap();
    let (target, _) = publish_tree(root.path(), "1.0.0", b"first artifact");
    let view = root.path().join("views/demo-tool");

    let first = LocalPath::new(target.as_path())
        .unwrap()
        .link_view(view.as_path(), None)
        .unwrap();
    assert_eq!(first.change, LinkChange::Created);

    let second = LocalPath::new(target.as_path())
        .unwrap()
        .link_view(view.as_path(), None)
        .unwrap();
    assert_eq!(second.change, LinkChange::Replaced);
    assert_eq!(
        fs::canonicalize(&view).unwrap(),
        fs::canonicalize(&target).unwrap()
    );
    assert_eq!(fs::read(view.join("tool.txt")).unwrap(), b"first artifact");
}

#[test]
fn link_root_rejects_a_non_directory_occupant_without_mutation() {
    let root = tempfile::tempdir().unwrap();
    let (target, _) = publish_tree(root.path(), "1.0.0", b"artifact payload");
    let view = root.path().join("views/demo-tool");
    fs::create_dir_all(view.parent().unwrap()).unwrap();
    fs::write(&view, b"existing winner").unwrap();

    let error = LocalPath::new(target.as_path())
        .unwrap()
        .link_view(view.as_path(), None)
        .unwrap_err();

    assert!(matches!(
        error,
        LinkError::ViewConflict {
            view: ref conflict,
            observed: ViewEntry::Occupied(LocalObservation::File { .. }),
        } if conflict == &view
    ));
    assert_eq!(fs::read(&view).unwrap(), b"existing winner");
}

#[test]
fn link_root_rejects_a_missing_or_file_source_without_a_view() {
    let root = tempfile::tempdir().unwrap();
    let views = root.path().join("views");
    fs::create_dir_all(&views).unwrap();

    let missing = root.path().join("artifacts/missing");
    let missing_view = views.join("missing");
    let missing_error = LocalPath::new(missing.as_path())
        .unwrap()
        .link_view(missing_view.as_path(), None)
        .unwrap_err();
    assert!(matches!(
        missing_error,
        LinkError::ExposeNotDirectory {
            path: ref source,
            observed: LocalObservation::Missing,
        } if source == &missing
    ));
    assert!(!missing_view.exists());

    let file = root.path().join("artifacts/file");
    fs::create_dir_all(file.parent().unwrap()).unwrap();
    fs::write(&file, b"not a tree").unwrap();
    let file_view = views.join("file");
    let file_error = LocalPath::new(file.as_path())
        .unwrap()
        .link_view(file_view.as_path(), None)
        .unwrap_err();
    assert!(matches!(
        file_error,
        LinkError::ExposeNotDirectory {
            path: ref source,
            observed: LocalObservation::File { .. },
        } if source == &file
    ));
    assert!(!file_view.exists());
}

#[test]
fn link_root_view_can_be_unlinked_as_an_active_directory_symlink() {
    let root = tempfile::tempdir().unwrap();
    let (target, _) = publish_tree(root.path(), "1.0.0", b"artifact payload");
    let view = root.path().join("views/demo-tool");

    LocalPath::new(target.as_path())
        .unwrap()
        .link_view(view.as_path(), None)
        .unwrap();

    let evidence = LocalPath::new(view.as_path())
        .unwrap()
        .unlink_view()
        .unwrap();

    assert_eq!(evidence.change, UnlinkChange::Removed);
    assert!(!view.exists());
}
