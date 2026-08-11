use pulith::local::{RecordError, RecordLimit, RecordObservation, RecordStore};
use std::io::Cursor;

#[test]
fn record_store_serializes_writers_and_keeps_inspection_read_only() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let store = RecordStore::new(&root).unwrap();
    let limit = RecordLimit::new(16).unwrap();

    assert!(matches!(
        store.inspect("state", limit).unwrap().0,
        RecordObservation::Missing
    ));
    assert!(!root.join("lock").exists());
    assert!(!root.join("stage").exists());

    let edit = store.edit().unwrap();
    assert!(matches!(
        RecordStore::new(&root).unwrap().edit().unwrap_err(),
        RecordError::Busy { .. }
    ));
    drop(edit);
    RecordStore::new(&root).unwrap().edit().unwrap();
}

#[test]
fn record_store_streams_create_replace_and_remove_under_one_limit() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let limit = RecordLimit::new(5).unwrap();
    let mut edit = RecordStore::new(&root).unwrap().edit().unwrap();

    edit.create_from("state", limit, Cursor::new(b"old"))
        .unwrap();
    assert_eq!(
        edit.inspect("state", limit).unwrap().0,
        RecordObservation::Present(b"old".to_vec())
    );
    edit.replace_from("state", limit, Cursor::new(b"newer"))
        .unwrap();
    assert_eq!(
        edit.inspect("state", limit).unwrap().0,
        RecordObservation::Present(b"newer".to_vec())
    );
    assert!(matches!(
        edit.replace_from("state", limit, Cursor::new(b"excess")),
        Err(RecordError::TooLarge { .. })
    ));
    assert_eq!(std::fs::read(root.join("state")).unwrap(), b"newer");
    edit.remove("state").unwrap();
    assert!(matches!(
        edit.inspect("state", limit).unwrap().0,
        RecordObservation::Missing
    ));
}

#[test]
fn record_store_rejects_invalid_roots_names_and_conflicting_transitions() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let limit = RecordLimit::new(8).unwrap();

    assert!(matches!(
        RecordLimit::new(0),
        Err(RecordError::InvalidLimit)
    ));
    assert!(matches!(
        RecordStore::new("relative"),
        Err(RecordError::InvalidStore(_))
    ));
    assert!(matches!(
        RecordStore::new(root.join("missing")),
        Err(RecordError::InvalidStore(_))
    ));
    assert!(matches!(
        RecordStore::new(&root).unwrap().inspect("../state", limit),
        Err(RecordError::InvalidName(_))
    ));
    assert!(matches!(
        RecordStore::new(&root).unwrap().inspect("lock", limit),
        Err(RecordError::InvalidName(_))
    ));

    let mut edit = RecordStore::new(&root).unwrap().edit().unwrap();
    assert!(matches!(
        edit.replace_from("state", limit, Cursor::new(b"new")),
        Err(RecordError::Conflict { .. })
    ));
    edit.create_from("state", limit, Cursor::new(b"old"))
        .unwrap();
    assert!(matches!(
        edit.create_from("state", limit, Cursor::new(b"new")),
        Err(RecordError::Conflict { .. })
    ));
}

#[test]
fn record_store_reclaims_bounded_stage_residue() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let limit = RecordLimit::new(4).unwrap();
    let mut edit = RecordStore::new(&root).unwrap().edit().unwrap();

    assert!(matches!(
        edit.create_from("state", limit, Cursor::new(b"excess")),
        Err(RecordError::TooLarge { .. })
    ));
    assert_eq!(
        std::fs::metadata(root.join("stage/state")).unwrap().len(),
        4
    );
    edit.create_from("state", limit, Cursor::new(b"good"))
        .unwrap();
    assert_eq!(std::fs::read(root.join("state")).unwrap(), b"good");
}
