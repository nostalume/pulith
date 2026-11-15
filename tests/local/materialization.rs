#![cfg(feature = "local")]

use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

#[cfg(any(feature = "blake3", feature = "sha2"))]
use pulith::local::{
    CommitOutcome, LocalEntryKind, LocalExpectation, LocalObservation, LocalPath,
    LocalReconciliation, RemoveEvidence, TreeAdmission,
};

#[test]
fn publish_file_reports_the_branch_at_commit_time() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("target");
    let target = LocalPath::new(&path).unwrap();

    assert_eq!(target.inspect_entry().unwrap(), LocalObservation::Missing);
    fs::write(&path, b"arrived after observation").unwrap();
    let publication = target
        .publish_file(b"new", TreeAdmission::ReplaceRegularFile)
        .unwrap();
    assert_eq!(publication.outcome, CommitOutcome::Replaced);
    assert_eq!(publication.bytes, 3);
    assert_eq!(fs::read(&path).unwrap(), b"new");

    assert!(matches!(
        target.inspect_entry().unwrap(),
        LocalObservation::File { .. }
    ));
    fs::remove_file(&path).unwrap();
    let publication = target
        .publish_file(b"next", TreeAdmission::ReplaceRegularFile)
        .unwrap();
    assert_eq!(publication.outcome, CommitOutcome::Created);
    assert_eq!(publication.bytes, 4);
    assert_eq!(fs::read(&path).unwrap(), b"next");
}

#[test]
fn publish_file_reports_no_commit_when_another_writer_occupies_the_name() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("target");
    let target = LocalPath::new(&path).unwrap();
    assert_eq!(target.inspect_entry().unwrap(), LocalObservation::Missing);

    let start = std::sync::Barrier::new(2);
    let (first, second) = std::thread::scope(|scope| {
        let first = scope.spawn(|| {
            start.wait();
            target.publish_file(b"first", TreeAdmission::Absent)
        });
        start.wait();
        let second = target.publish_file(b"second", TreeAdmission::Absent);
        (first.join().unwrap(), second)
    });
    let (publication, error, expected) = match (first, second) {
        (Ok(publication), Err(error)) => (publication, error, b"first".as_slice()),
        (Err(error), Ok(publication)) => (publication, error, b"second".as_slice()),
        results => panic!("expected one successful publication, got {results:?}"),
    };
    assert_eq!(publication.outcome, CommitOutcome::Created);
    assert_eq!(publication.bytes, expected.len() as u64);
    assert!(matches!(error, pulith::local::LocalError::TargetExists(found) if found == path));
    assert_eq!(fs::read(&path).unwrap(), expected);
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 1);
}

#[test]
fn staged_tree_writes_generated_bytes_inside_its_boundary() {
    let root = tempfile::tempdir().unwrap();
    let target_path = root.path().join("generated");
    let target = LocalPath::new(&target_path).unwrap();
    target
        .stage()
        .unwrap()
        .write_file(b"schema = 1\n", "service.toml")
        .unwrap()
        .publish(target, pulith::local::TreeAdmission::Absent)
        .unwrap();
    assert_eq!(
        fs::read(target_path.join("service.toml")).unwrap(),
        b"schema = 1\n"
    );
}
/// A file-shaped publication pays for one temporary file, not for a workspace directory holding
/// a tree. The target's neighbours are the observable proof: after a successful write and after
/// a refusal alike, the directory holds what it held before plus the target itself.
#[test]
fn publish_file_publishes_one_file_and_leaves_nothing_beside_it() {
    let root = tempfile::tempdir().unwrap();
    let target_path = root.path().join("nested/deeper/a.txt");
    let target = LocalPath::new(&target_path).unwrap();
    let evidence = target
        .publish_file(b"fresh", TreeAdmission::ReplaceRegularFile)
        .unwrap();
    assert_eq!(evidence.outcome, pulith::local::CommitOutcome::Created);
    assert_eq!(evidence.bytes, 5);
    assert_eq!(fs::read(&target_path).unwrap(), b"fresh");
    let siblings: Vec<_> = fs::read_dir(target_path.parent().unwrap())
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(siblings, vec![std::ffi::OsString::from("a.txt")]);
}

#[test]
fn publish_file_replaces_the_predecessor_bytes() {
    let root = tempfile::tempdir().unwrap();
    let target_path = root.path().join("a.txt");
    fs::write(&target_path, "stale").unwrap();
    let target = LocalPath::new(&target_path).unwrap();
    target
        .publish_file(b"fresh", TreeAdmission::ReplaceRegularFile)
        .unwrap();
    assert_eq!(fs::read(&target_path).unwrap(), b"fresh");
}

/// The immutable admission keeps its promise here too: an existing predecessor is reported, not
/// replaced, and the bytes that were there are still there.
#[test]
fn publish_file_under_absent_admission_refuses_a_predecessor() {
    let root = tempfile::tempdir().unwrap();
    let target_path = root.path().join("a.txt");
    fs::write(&target_path, "keep").unwrap();
    let target = LocalPath::new(&target_path).unwrap();
    let error = target
        .publish_file(b"new", TreeAdmission::Absent)
        .unwrap_err();
    assert!(
        matches!(error, pulith::local::LocalError::TargetExists(_)),
        "{error:?}"
    );
    assert_eq!(fs::read(&target_path).unwrap(), b"keep");
    let siblings: Vec<_> = fs::read_dir(root.path())
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(siblings, vec![std::ffi::OsString::from("a.txt")]);
}

/// A refusal happens before the rename, so the occupant is untouched and no temporary file is
/// left where a later run would trip over it.
#[test]
fn publish_file_refuses_a_directory_without_leaving_a_temporary_file() {
    let root = tempfile::tempdir().unwrap();
    let target_path = root.path().join("a.txt");
    fs::create_dir(&target_path).unwrap();
    let target = LocalPath::new(&target_path).unwrap();
    let error = target
        .publish_file(b"new", TreeAdmission::ReplaceRegularFile)
        .unwrap_err();
    assert!(matches!(
        error,
        pulith::local::LocalError::OccupiedTarget { .. }
    ));
    assert!(target_path.is_dir());
    let siblings: Vec<_> = fs::read_dir(root.path())
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(siblings, vec![std::ffi::OsString::from("a.txt")]);
}

/// Replacing a file must not reformat it: the mode of the file that is already there travels
/// onto the staged file before the rename.
#[cfg(unix)]
#[test]
fn publish_file_keeps_the_mode_of_the_file_it_replaces() {
    use std::os::unix::fs::PermissionsExt;
    let root = tempfile::tempdir().unwrap();
    let target_path = root.path().join("a.txt");
    fs::write(&target_path, "stale").unwrap();
    fs::set_permissions(&target_path, fs::Permissions::from_mode(0o600)).unwrap();
    let target = LocalPath::new(&target_path).unwrap();
    target
        .publish_file(b"fresh", TreeAdmission::ReplaceRegularFile)
        .unwrap();
    let mode = fs::metadata(&target_path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600);
}

fn temp_root(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "pulith-public-api-{name}-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ))
}

#[test]
fn materialize_local_file_without_synthetic_transitions() {
    let root = temp_root("materialize");
    let source = root.join("source.txt");
    let target = root.join("target.txt");
    fs::create_dir_all(&root).unwrap();
    fs::write(&source, "pulith").unwrap();

    let material = LocalPath::new(source.clone()).unwrap();
    let target_path = target;
    let target = LocalPath::new(target_path.clone()).unwrap();
    let stage = target.stage().unwrap();
    let tree = stage
        .copy_file(material, std::path::PathBuf::new())
        .unwrap();
    let evidence = tree
        .publish(target, pulith::local::TreeAdmission::Absent)
        .unwrap();

    assert_eq!(fs::read_to_string(&target_path).unwrap(), "pulith");
    assert_eq!(evidence.files, 1);
    assert_eq!(evidence.directories, 0);
    assert_eq!(evidence.bytes, 6);

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn create_new_conflict_is_typed_and_non_mutating() {
    let root = temp_root("create-conflict");
    let source = root.join("source.txt");
    let target = root.join("target.txt");
    fs::create_dir_all(&root).unwrap();
    fs::write(&source, "replacement").unwrap();
    fs::write(&target, "winner").unwrap();

    let material = LocalPath::new(source.clone()).unwrap();
    let admitted = LocalPath::new(target.clone()).unwrap();
    let stage = admitted.stage().unwrap();
    let tree = stage
        .copy_file(material, std::path::PathBuf::new())
        .unwrap();
    let error = tree
        .publish(admitted, pulith::local::TreeAdmission::Absent)
        .unwrap_err();

    assert!(matches!(
        error,
        pulith::local::LocalError::TargetExists(path) if path == target
    ));
    assert_eq!(fs::read_to_string(&target).unwrap(), "winner");
    assert_eq!(fs::read_to_string(&source).unwrap(), "replacement");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn thirty_two_publishers_have_one_complete_winner() {
    let root = temp_root("publish-race");
    let target = root.join("target.txt");
    fs::create_dir_all(&root).unwrap();
    let stages = (0_u8..32)
        .map(|value| {
            let source = root.join(format!("source-{value}"));
            fs::write(&source, [value]).unwrap();
            let target = LocalPath::new(&target).unwrap();
            target
                .stage()
                .unwrap()
                .copy_file(LocalPath::new(source).unwrap(), PathBuf::new())
                .unwrap()
        })
        .collect::<Vec<_>>();
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(stages.len()));
    let results = stages
        .into_iter()
        .map(|stage| {
            let barrier = barrier.clone();
            let target = target.clone();
            std::thread::spawn(move || {
                barrier.wait();
                stage.publish(
                    LocalPath::new(target).unwrap(),
                    pulith::local::TreeAdmission::Absent,
                )
            })
        })
        .collect::<Vec<_>>()
        .into_iter()
        .map(|thread| thread.join().unwrap())
        .collect::<Vec<_>>();

    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    assert_eq!(
        results
            .iter()
            .filter(|result| matches!(result, Err(pulith::local::LocalError::TargetExists(path)) if path == &target))
            .count(),
        31
    );
    assert_eq!(fs::read(&target).unwrap().len(), 1);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn stage_rejects_escaping_destinations_without_leaking_a_target() {
    let root = temp_root("stage-escape");
    let source = root.join("source");
    let target = root.join("target");
    fs::create_dir_all(&root).unwrap();
    fs::write(&source, b"safe").unwrap();
    let admitted = LocalPath::new(&target).unwrap();
    let error = admitted
        .stage()
        .unwrap()
        .copy_file(LocalPath::new(source).unwrap(), "../escape")
        .unwrap_err();
    assert!(matches!(
        error,
        pulith::local::LocalError::InvalidStagePath(_)
    ));
    assert!(!target.exists());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn copy_tree_rejects_a_source_link_without_leaking_a_target() {
    let root = temp_root("copy-tree-link");
    let source = root.join("source");
    let outside = root.join("outside");
    let target = root.join("target");
    fs::create_dir_all(&source).unwrap();
    fs::write(&outside, b"outside").unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(&outside, source.join("link")).unwrap();
    #[cfg(windows)]
    if let Err(error) = std::os::windows::fs::symlink_file(&outside, source.join("link")) {
        assert_eq!(error.kind(), std::io::ErrorKind::PermissionDenied);
        fs::remove_dir_all(root).unwrap();
        return;
    }

    let admitted = LocalPath::new(&target).unwrap();
    let error = admitted
        .stage()
        .unwrap()
        .copy_tree(LocalPath::new(source).unwrap(), PathBuf::new())
        .unwrap_err();
    assert!(matches!(
        error,
        pulith::local::LocalError::UnsupportedLocalEntry(_)
    ));
    assert!(!target.exists());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn precommit_rescan_rejects_an_injected_link() {
    let root = temp_root("stage-link-injection");
    let source = root.join("source");
    let outside = root.join("outside");
    let target = root.join("target");
    fs::create_dir_all(&source).unwrap();
    fs::write(source.join("file"), b"safe").unwrap();
    fs::write(&outside, b"outside").unwrap();
    let admitted = LocalPath::new(&target).unwrap();
    let stage = admitted
        .stage()
        .unwrap()
        .copy_tree(LocalPath::new(source).unwrap(), PathBuf::new())
        .unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(&outside, stage.root().join("injected")).unwrap();
    #[cfg(windows)]
    if let Err(error) = std::os::windows::fs::symlink_file(&outside, stage.root().join("injected"))
    {
        assert_eq!(error.kind(), std::io::ErrorKind::PermissionDenied);
        drop(stage);
        fs::remove_dir_all(root).unwrap();
        return;
    }
    let error = stage
        .publish(admitted, pulith::local::TreeAdmission::Absent)
        .unwrap_err();
    assert!(matches!(
        error,
        pulith::local::LocalError::UnsupportedLocalEntry(_)
    ));
    assert!(!target.exists());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn hard_link_input_is_normalized_into_independent_stage_bytes() {
    let root = temp_root("stage-hard-link");
    let source = root.join("source");
    let alias = root.join("alias");
    let target = root.join("target");
    fs::create_dir_all(&root).unwrap();
    fs::write(&source, b"before").unwrap();
    fs::hard_link(&source, &alias).unwrap();
    let admitted = LocalPath::new(&target).unwrap();
    let stage = admitted
        .stage()
        .unwrap()
        .copy_file(LocalPath::new(alias).unwrap(), PathBuf::new())
        .unwrap();
    fs::write(source, b"after").unwrap();
    stage
        .publish(admitted, pulith::local::TreeAdmission::Absent)
        .unwrap();
    assert_eq!(fs::read(&target).unwrap(), b"before");
    fs::remove_dir_all(root).unwrap();
}

#[test]
#[cfg(feature = "blake3")]
fn verified_publication_keeps_the_installed_bytes_after_source_changes() {
    use pulith::hash::{DigestAlgorithmKind, DigestValue};

    let root = temp_root("descriptor");
    let source = root.join("source.txt");
    let target = root.join("target.txt");
    fs::create_dir_all(&root).unwrap();
    fs::write(&source, "pulith").unwrap();
    let digest = blake3::hash(b"pulith").to_hex().to_string();
    let expected = DigestValue::new(DigestAlgorithmKind::Blake3, digest).unwrap();

    let material = LocalPath::new(source.clone()).unwrap();
    let admitted = LocalPath::new(target.clone()).unwrap();
    let file = material.open_file().unwrap().unwrap();
    fs::rename(&source, root.join("old.txt")).unwrap();
    fs::write(&source, "replaced").unwrap();
    let (publication, evidence) = admitted.publish_verified_file(file, &expected).unwrap();
    assert_eq!(evidence.digest, expected);
    assert_eq!(publication.bytes, 6);
    assert_eq!(fs::read_to_string(&target).unwrap(), "pulith");

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn remove_local_target_reports_changed_and_unchanged() {
    let root = temp_root("forget");
    let target = root.join("target.txt");
    fs::create_dir_all(&root).unwrap();
    fs::write(&target, "obsolete").unwrap();

    let removed = LocalPath::new(target.clone()).unwrap().remove().unwrap();

    assert!(!target.exists());
    assert_eq!(removed, RemoveEvidence::Removed);
    let unchanged = LocalPath::new(target.clone()).unwrap().remove().unwrap();
    assert_eq!(unchanged, RemoveEvidence::Unchanged);

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn inspect_and_reconcile_without_mutating_local_target() {
    let root = temp_root("inspect-reconcile");
    let target = root.join("target.txt");
    fs::create_dir_all(&root).unwrap();
    fs::write(&target, "pulith").unwrap();

    let observation = LocalPath::new(target.clone())
        .unwrap()
        .inspect_entry()
        .unwrap();
    assert_eq!(observation, LocalObservation::File { bytes: 6 });

    let reconciliation = observation.compare(&LocalExpectation::FileSize(6));
    assert_eq!(reconciliation, LocalReconciliation::Matches);
    assert_eq!(fs::read_to_string(&target).unwrap(), "pulith");

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn post_inspect_preserves_apply_evidence_and_reconciles() {
    let root = temp_root("post-inspect-materialize");
    let source = root.join("source.txt");
    let target = root.join("target.txt");
    fs::create_dir_all(&root).unwrap();
    fs::write(&source, "pulith").unwrap();

    let material = LocalPath::new(source.clone()).unwrap();
    let admitted = LocalPath::new(target.clone()).unwrap();
    let stage = admitted.stage().unwrap();
    let tree = stage
        .copy_file(material, std::path::PathBuf::new())
        .unwrap();
    let apply_evidence = tree
        .publish(admitted, pulith::local::TreeAdmission::Absent)
        .unwrap();

    let observation = LocalPath::new(target.clone())
        .unwrap()
        .inspect_entry()
        .unwrap();
    assert_eq!(observation, LocalObservation::File { bytes: 6 });

    let reconciliation = observation.compare(&LocalExpectation::FileSize(6));
    assert_eq!(reconciliation, LocalReconciliation::Matches);
    let _ = apply_evidence;

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn post_inspect_reports_later_mutation_without_reapplying() {
    let root = temp_root("post-inspect-mutation");
    let source = root.join("source.txt");
    let target = root.join("target.txt");
    fs::create_dir_all(&root).unwrap();
    fs::write(&source, "pulith").unwrap();

    let material = LocalPath::new(source.clone()).unwrap();
    let admitted = LocalPath::new(target.clone()).unwrap();
    let stage = admitted.stage().unwrap();
    let tree = stage
        .copy_file(material, std::path::PathBuf::new())
        .unwrap();
    tree.publish(admitted, pulith::local::TreeAdmission::Absent)
        .unwrap();
    fs::write(&target, "changed!").unwrap();

    let observation = LocalPath::new(target.clone())
        .unwrap()
        .inspect_entry()
        .unwrap();
    let reconciliation = observation.compare(&LocalExpectation::FileSize(6));
    assert_eq!(
        reconciliation,
        LocalReconciliation::Modified {
            expected_bytes: 6,
            observed_bytes: 8,
        }
    );
    assert_eq!(fs::read_to_string(&target).unwrap(), "changed!");

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn post_inspect_forget_observes_missing_without_acquisition() {
    let root = temp_root("post-inspect-forget");
    let target = root.join("target.txt");
    fs::create_dir_all(&root).unwrap();
    fs::write(&target, "obsolete").unwrap();

    LocalPath::new(target.clone()).unwrap().remove().unwrap();

    let observation = LocalPath::new(target.clone())
        .unwrap()
        .inspect_entry()
        .unwrap();
    assert_eq!(observation, LocalObservation::Missing);

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn inspect_entry_distinguishes_regular_file_and_directory() {
    let root = temp_root("occupancy");
    fs::create_dir_all(root.join("directory")).unwrap();
    fs::write(root.join("file.txt"), b"four").unwrap();

    assert_eq!(
        LocalPath::new(root.join("missing.txt"))
            .unwrap()
            .inspect_entry()
            .unwrap(),
        LocalObservation::Missing
    );
    assert_eq!(
        LocalPath::new(root.join("file.txt"))
            .unwrap()
            .inspect_entry()
            .unwrap(),
        LocalObservation::File { bytes: 4 }
    );
    assert!(matches!(
        LocalPath::new(root.join("directory"))
            .unwrap()
            .inspect_entry(),
        Ok(LocalObservation::Directory)
    ));

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn commit_replaces_a_regular_file_and_keeps_its_mode() {
    let root = temp_root("commit-replace");
    fs::create_dir_all(&root).unwrap();
    let target_path = root.join("target.txt");
    fs::write(&target_path, b"before").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&target_path, fs::Permissions::from_mode(0o640)).unwrap();
    }

    let target = LocalPath::new(&target_path).unwrap();
    let evidence = target
        .stage()
        .unwrap()
        .write_file(b"after", PathBuf::new())
        .unwrap()
        .publish(target.clone(), TreeAdmission::ReplaceRegularFile)
        .unwrap();

    assert_eq!(fs::read(&target_path).unwrap(), b"after");
    assert_eq!(evidence.files, 1);
    assert_eq!(evidence.bytes, 5);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(&target_path).unwrap().permissions().mode() & 0o777,
            0o640
        );
    }

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn commit_creates_the_target_when_the_replacing_admission_finds_it_absent() {
    let root = temp_root("commit-create");
    let target_path = root.join("created.txt");
    let target = LocalPath::new(&target_path).unwrap();

    target
        .stage()
        .unwrap()
        .write_file(b"created", PathBuf::new())
        .unwrap()
        .publish(target, TreeAdmission::ReplaceRegularFile)
        .unwrap();

    assert_eq!(fs::read(&target_path).unwrap(), b"created");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn commit_refuses_a_directory_target_without_touching_it() {
    let root = temp_root("commit-directory");
    fs::create_dir_all(root.join("occupied/inner")).unwrap();
    fs::write(root.join("occupied/inner/keep.txt"), b"keep").unwrap();
    let target = LocalPath::new(root.join("occupied")).unwrap();

    let error = target
        .stage()
        .unwrap()
        .write_file(b"replacement", PathBuf::new())
        .unwrap()
        .publish(target, TreeAdmission::ReplaceRegularFile)
        .unwrap_err();

    assert!(matches!(
        error,
        pulith::local::LocalError::OccupiedTarget {
            kind: LocalEntryKind::Directory,
            ..
        }
    ));
    assert_eq!(
        fs::read(root.join("occupied/inner/keep.txt")).unwrap(),
        b"keep"
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn commit_refuses_a_symlink_target_without_following_it() {
    let root = temp_root("commit-symlink");
    fs::create_dir_all(&root).unwrap();
    let real = root.join("real.txt");
    fs::write(&real, b"origin").unwrap();
    let link = root.join("link.txt");
    #[cfg(unix)]
    std::os::unix::fs::symlink(&real, &link).unwrap();
    #[cfg(windows)]
    if let Err(error) = std::os::windows::fs::symlink_file(&real, &link) {
        assert_eq!(error.kind(), std::io::ErrorKind::PermissionDenied);
        fs::remove_dir_all(root).unwrap();
        return;
    }

    let target = LocalPath::new(&link).unwrap();
    let error = target
        .stage()
        .unwrap()
        .write_file(b"replacement", PathBuf::new())
        .unwrap()
        .publish(target, TreeAdmission::ReplaceRegularFile)
        .unwrap_err();

    assert!(matches!(
        error,
        pulith::local::LocalError::OccupiedTarget {
            kind: LocalEntryKind::Symlink,
            ..
        }
    ));
    assert_eq!(fs::read(&real).unwrap(), b"origin");
    assert!(
        fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink()
    );

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn absent_admission_still_refuses_an_existing_regular_file() {
    let root = temp_root("commit-absent-only");
    fs::create_dir_all(&root).unwrap();
    let target_path = root.join("target.txt");
    fs::write(&target_path, b"before").unwrap();
    let target = LocalPath::new(&target_path).unwrap();

    let error = target
        .stage()
        .unwrap()
        .write_file(b"after", PathBuf::new())
        .unwrap()
        .publish(target, TreeAdmission::Absent)
        .unwrap_err();

    assert!(matches!(
        error,
        pulith::local::LocalError::TargetExists(path) if path == target_path
    ));
    assert_eq!(fs::read(&target_path).unwrap(), b"before");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn copy_tree_publishes_an_empty_directory() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source");
    let path = root.path().join("target");
    fs::create_dir(&source).unwrap();
    let target = LocalPath::new(&path).unwrap();
    let tree = target
        .stage()
        .unwrap()
        .copy_tree(LocalPath::new(source).unwrap(), PathBuf::new())
        .unwrap();
    let receipt = tree.publish(target, TreeAdmission::Absent).unwrap();
    assert_eq!(receipt.directories, 1);
    assert_eq!(receipt.files, 0);
    assert_eq!(fs::read_dir(path).unwrap().count(), 0);
}

#[test]
fn copy_file_does_not_follow_an_existing_stage_destination() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source");
    let outside = root.path().join("outside");
    let target = LocalPath::new(root.path().join("target")).unwrap();
    fs::write(&source, b"new bytes").unwrap();
    fs::write(&outside, b"original").unwrap();

    let stage = target.stage().unwrap();
    fs::create_dir(stage.root()).unwrap();
    crate::common::file_symlink(&outside, stage.root().join("occupied")).unwrap();
    assert!(matches!(
        stage.copy_file(LocalPath::new(source).unwrap(), "occupied"),
        Err(pulith::local::LocalError::Io { source, .. }) if source.kind() == std::io::ErrorKind::AlreadyExists
    ));
    assert_eq!(fs::read(outside).unwrap(), b"original");
    assert!(!target.as_path().exists());
}
