use pulith::local::{RecordChange, RecordError, RecordLimit, RecordStore};
use std::io::Cursor;

#[test]
fn record_store_serializes_writers_and_keeps_inspection_read_only() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let store = RecordStore::new(&root).unwrap();
    let limit = RecordLimit::new(16).unwrap();

    assert!(store.read("state", limit).unwrap().is_none());
    assert!(!root.join("lock").exists());
    assert!(!root.join("stage").exists());

    let mut edit = store.edit().unwrap();
    for alias in ["LOCK", "Stage"] {
        assert!(matches!(
            edit.remove(alias),
            Err(RecordError::InvalidName(_))
        ));
    }
    #[cfg(windows)]
    for alias in ["lock.", "lock ", "lock:stream"] {
        assert!(matches!(
            edit.remove(alias),
            Err(RecordError::InvalidName(_))
        ));
    }
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

    let created = edit
        .publish_from("state", limit, Cursor::new(b"old"))
        .unwrap();
    assert_eq!(created.change, RecordChange::Created);
    assert_eq!(edit.read("state", limit).unwrap(), Some(b"old".to_vec()));
    let replaced = edit
        .publish_from("state", limit, Cursor::new(b"newer"))
        .unwrap();
    assert_eq!(replaced.change, RecordChange::Replaced);
    assert_eq!(edit.read("state", limit).unwrap(), Some(b"newer".to_vec()));
    assert!(matches!(
        edit.replace_from("state", limit, Cursor::new(b"excess")),
        Err(RecordError::TooLarge { .. })
    ));
    assert_eq!(std::fs::read(root.join("state")).unwrap(), b"newer");
    edit.remove("state").unwrap();
    assert!(edit.read("state", limit).unwrap().is_none());
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
        RecordStore::new(&root).unwrap().read("../state", limit),
        Err(RecordError::InvalidName(_))
    ));
    assert!(matches!(
        RecordStore::new(&root).unwrap().read("lock", limit),
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

#[test]
fn record_inspection_refuses_links_and_accepts_the_maximum_limit() {
    let root = tempfile::tempdir().unwrap();
    let store = RecordStore::new(root.path()).unwrap();
    let record = root.path().join("record");
    std::fs::write(&record, b"record").unwrap();
    let limit = RecordLimit::new(u64::MAX).unwrap();
    assert_eq!(
        store.read("record", limit).unwrap(),
        Some(b"record".to_vec())
    );

    let link = root.path().join("link");
    #[cfg(unix)]
    std::os::unix::fs::symlink(&record, &link).unwrap();
    #[cfg(windows)]
    std::os::windows::fs::symlink_file(&record, &link).unwrap();
    assert!(matches!(
        store.read("link", limit),
        Err(RecordError::Conflict { .. })
    ));
}

#[test]
fn record_publication_selects_create_or_replace_without_reading_old_bytes() {
    let root = tempfile::tempdir().unwrap();
    let limit = RecordLimit::new(4).unwrap();
    let store = RecordStore::new(root.path()).unwrap();
    let mut edit = store.edit().unwrap();
    assert_eq!(
        edit.publish_from("state", limit, &b"old"[..])
            .unwrap()
            .bytes,
        3
    );
    std::fs::write(root.path().join("state"), b"above the new limit").unwrap();
    assert_eq!(
        edit.publish_from("state", limit, &b"new"[..])
            .unwrap()
            .bytes,
        3
    );
    assert_eq!(edit.read("state", limit).unwrap(), Some(b"new".to_vec()));
    std::fs::create_dir(root.path().join("occupied")).unwrap();
    assert!(matches!(
        edit.publish_from("occupied", limit, &b"x"[..]),
        Err(RecordError::Conflict { .. })
    ));
}
