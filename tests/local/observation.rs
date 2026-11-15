#![cfg(feature = "local")]

use pulith::local::{LocalEntryKind, LocalError, LocalObservation, LocalPath};

#[test]
fn open_file_reports_missing_and_directory() {
    let root = tempfile::tempdir().unwrap();
    let missing = LocalPath::new(root.path().join("missing")).unwrap();
    let directory = LocalPath::new(root.path()).unwrap();

    assert!(missing.open_file().unwrap().is_none());
    assert!(matches!(
        directory.open_file(),
        Err(LocalError::NotRegularFile {
            kind: LocalEntryKind::Directory,
            ..
        })
    ));
}

#[test]
fn open_file_keeps_bytes_and_metadata_on_the_same_handle() {
    use std::fs;
    use std::io::Read;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("artifact");
    fs::write(&path, [0, 255]).unwrap();
    #[cfg(unix)]
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    let target = LocalPath::new(&path).unwrap();
    let mut file = target.open_file().unwrap().unwrap();

    fs::rename(&path, root.path().join("old")).unwrap();
    fs::write(&path, []).unwrap();
    #[cfg(unix)]
    fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();

    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).unwrap();
    assert_eq!(bytes, [0, 255]);
    let metadata = file.metadata().unwrap();
    assert_eq!(metadata.len(), 2);
    #[cfg(unix)]
    assert_eq!(metadata.permissions().mode() & 0o777, 0o600);
    assert_eq!(fs::metadata(&path).unwrap().len(), 0);
}

#[cfg(unix)]
#[test]
fn open_file_refuses_symlinks_and_fifos() {
    use rustix::fs::{CWD, Mode, mkfifoat};
    use std::os::unix::fs::symlink;

    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("file");
    let link = root.path().join("link");
    let fifo = root.path().join("fifo");
    std::fs::write(&path, b"original").unwrap();
    symlink(&path, &link).unwrap();
    mkfifoat(CWD, &fifo, Mode::RUSR | Mode::WUSR).unwrap();
    let open_kind = |path| match LocalPath::new(path).unwrap().open_file().unwrap_err() {
        LocalError::NotRegularFile { kind, .. } => kind,
        error => panic!("{error}"),
    };
    assert_eq!(open_kind(link), LocalEntryKind::Symlink);
    assert_eq!(open_kind(fifo), LocalEntryKind::Other);
    assert_eq!(std::fs::read(path).unwrap(), b"original");
}

#[cfg(windows)]
#[test]
fn open_file_reports_a_junction_without_following_it() {
    let root = tempfile::tempdir().unwrap();
    let directory = root.path().join("directory");
    let junction = root.path().join("junction");
    std::fs::create_dir(&directory).unwrap();
    let result = std::process::Command::new("cmd")
        .args(["/c", "mklink", "/J"])
        .arg(&junction)
        .arg(&directory)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );

    let target = LocalPath::new(&junction).unwrap();
    assert!(matches!(
        target.clone().open_file(),
        Err(LocalError::NotRegularFile {
            kind: LocalEntryKind::Reparse,
            ..
        })
    ));
    assert!(matches!(
        target.inspect_entry(),
        Ok(LocalObservation::Reparse)
    ));
    assert_eq!(std::fs::read_dir(directory).unwrap().count(), 0);
}

#[test]
fn cloned_local_target_observes_the_same_facts_without_mutating_the_target() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("artifact.txt");
    std::fs::write(&path, b"pulith").unwrap();

    let original = LocalPath::new(path.as_path()).unwrap();
    let clone = original.clone();

    let original_observation = original.inspect_entry().unwrap();
    let clone_observation = clone.inspect_entry().unwrap();

    assert_eq!(original_observation, LocalObservation::File { bytes: 6 });
    assert_eq!(original_observation, clone_observation);
    assert_eq!(std::fs::read(&path).unwrap(), b"pulith".to_vec());
}

#[cfg(all(windows, not(feature = "windows-private-preview")))]
#[test]
fn private_publication_refuses_before_creating_a_parent() {
    let root = tempfile::tempdir().unwrap();
    let parent = root.path().join("absent");
    let path = parent.join("private");
    let target = LocalPath::new(&path).unwrap();
    let error = target
        .publish_private_file(b"secret", pulith::local::TreeAdmission::Absent)
        .unwrap_err();
    assert!(matches!(error, LocalError::PrivatePublicationUnavailable(seen) if seen == path));
    assert!(!parent.exists());
}

#[cfg(all(windows, feature = "windows-private-preview"))]
#[test]
fn private_publication_checks_acl_and_preserves_create_only_collisions() {
    use pulith::local::{CommitOutcome, TreeAdmission};
    use std::fs;

    let root = tempfile::tempdir().unwrap();
    let parent = root.path().join("absent");
    let path = parent.join("private");
    let target = LocalPath::new(&path).unwrap();
    target.check_private_publication().unwrap();
    assert!(!parent.exists());

    let created = target
        .publish_private_file(b"first", TreeAdmission::Absent)
        .unwrap();
    assert_eq!(created.outcome, CommitOutcome::Created);
    let file = target.clone().open_file().unwrap().unwrap();
    assert!(file.is_owner_only().unwrap());
    drop(file);
    assert!(matches!(
        target.publish_private_file(b"conflict", TreeAdmission::Absent),
        Err(LocalError::TargetExists(seen)) if seen == path
    ));
    assert_eq!(fs::read(&path).unwrap(), b"first");

    fs::remove_file(&path).unwrap();
    fs::write(&path, b"old").unwrap();
    let file = target.clone().open_file().unwrap().unwrap();
    assert!(!file.is_owner_only().unwrap());
    drop(file);
    let replaced = target
        .publish_private_file(b"second", TreeAdmission::ReplaceRegularFile)
        .unwrap();
    assert_eq!(replaced.outcome, CommitOutcome::Replaced);
    assert!(
        target
            .open_file()
            .unwrap()
            .unwrap()
            .is_owner_only()
            .unwrap()
    );
    assert_eq!(fs::read(&path).unwrap(), b"second");
    assert_eq!(fs::read_dir(parent).unwrap().count(), 1);
}

#[cfg(all(windows, feature = "windows-private-preview"))]
#[test]
fn private_publication_refuses_a_named_stream() {
    use pulith::local::TreeAdmission;

    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("target");
    std::fs::write(&path, b"original").unwrap();
    let stream = root.path().join("target:private");
    let target = LocalPath::new(&stream).unwrap();
    assert!(matches!(
        target.publish_private_file(b"fixture", TreeAdmission::Absent),
        Err(LocalError::InvalidPath(seen)) if seen == stream
    ));
    assert_eq!(std::fs::read(path).unwrap(), b"original");
    assert!(!stream.exists());
}
