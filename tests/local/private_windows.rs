//! Windows private-file acceptance fixture for two ordinary account tokens.

use std::fs::{self, File, OpenOptions};
use std::io::{ErrorKind, Read};
use std::path::Path;

use pulith::local::{CommitOutcome, LocalError, LocalPath, TreeAdmission};

#[test]
#[ignore = "requires a writable path on unsupported storage"]
fn windows_private_volume_refuses_before_creating_parent() {
    let root =
        std::env::var_os("PULITH_TEST_UNSUPPORTED_VOLUME").expect("unsupported fixture root");
    let scratch = tempfile::Builder::new()
        .prefix("pulith-acl-")
        .tempdir_in(root)
        .unwrap();
    let control = scratch.path().join("control");
    fs::write(&control, b"ordinary write").unwrap();
    assert_eq!(fs::read(control).unwrap(), b"ordinary write");

    let parent = scratch.path().join("new-parent");
    let target = LocalPath::new(parent.join("private")).unwrap();
    assert!(matches!(
        target.publish_private_file(b"fixture", TreeAdmission::Absent),
        Err(LocalError::PrivatePublicationUnavailable(_))
    ));
    assert!(!parent.exists());
    println!("ordinary write succeeded; private publication refused before creating its parent");
}

#[test]
#[ignore = "requires a shared NTFS fixture and two ordinary Windows accounts"]
fn windows_private_publication_two_accounts() {
    let root = std::env::var_os("PULITH_PRIVATE_TEST_ROOT").expect("shared fixture directory");
    let phase = std::env::var("PULITH_PRIVATE_TEST_PHASE").expect("create, read or remove");
    let root = Path::new(&root);
    let control = root.join("control");
    let old = root.join("read-only");
    let first = root.join("created");
    let second = root.join("replaced");
    let owner = root.join("owner");
    let sid = account_sid();
    match phase.as_str() {
        "create" => {
            assert!(root.is_dir());
            assert_eq!(fs::read_dir(root).unwrap().count(), 0);
            fs::write(&control, b"public fixture").unwrap();
            fs::write(&owner, sid).unwrap();
            fs::write(&second, b"public predecessor").unwrap();
            fs::write(&old, b"read-only predecessor").unwrap();
            let mut permissions = fs::metadata(&old).unwrap().permissions();
            permissions.set_readonly(true);
            fs::set_permissions(&old, permissions).unwrap();
            let target = LocalPath::new(&first).unwrap();
            let created = target
                .publish_private_file(b"created fixture", TreeAdmission::Absent)
                .unwrap();
            assert_eq!(created.outcome, CommitOutcome::Created);
            assert!(matches!(
                target.publish_private_file(b"conflict", TreeAdmission::Absent),
                Err(LocalError::TargetExists(_))
            ));
            let target = LocalPath::new(&second).unwrap();
            let replaced = target
                .publish_private_file(b"replaced fixture", TreeAdmission::ReplaceRegularFile)
                .unwrap();
            assert_eq!(replaced.outcome, CommitOutcome::Replaced);
            assert_eq!(fs::read(first).unwrap(), b"created fixture");
            assert_eq!(fs::read(second).unwrap(), b"replaced fixture");
            let target = LocalPath::new(&old).unwrap();
            assert!(matches!(
                target.publish_private_file(b"must not replace", TreeAdmission::ReplaceRegularFile),
                Err(LocalError::Io { .. })
            ));
            assert_eq!(fs::read(&old).unwrap(), b"read-only predecessor");
            assert_eq!(fs::read_dir(root).unwrap().count(), 5);
            println!(
                "publisher read the created and replaced bytes; both refusals kept the predecessor"
            );
        }
        "read" => {
            assert_eq!(fs::read(control).unwrap(), b"public fixture");
            assert_ne!(
                fs::read_to_string(owner).unwrap(),
                sid,
                "use a distinct reader account"
            );
            assert_denied(&first);
            assert_denied(&second);
            println!(
                "distinct reader opened the control; both published files denied read and write"
            );
        }
        "remove" => {
            assert_eq!(fs::read_to_string(&owner).unwrap(), sid);
            fs::set_permissions(&old, fs::metadata(&control).unwrap().permissions()).unwrap();
            for path in [control, first, second, owner, old] {
                fs::remove_file(path).unwrap();
            }
            assert!(fs::read_dir(root).unwrap().next().is_none());
            println!("publisher removed the fixture contents");
        }
        _ => panic!("use create, read or remove"),
    }
}

fn assert_denied(path: &Path) {
    let result = File::open(path).and_then(|mut file| file.read(&mut [0]));
    assert_eq!(
        result.unwrap_err().kind(),
        ErrorKind::PermissionDenied,
        "{}",
        path.display()
    );
    assert_eq!(
        OpenOptions::new()
            .write(true)
            .open(path)
            .unwrap_err()
            .kind(),
        ErrorKind::PermissionDenied,
        "{}",
        path.display()
    );
}

fn account_sid() -> String {
    let result = std::process::Command::new("whoami")
        .args(["/user", "/fo", "csv", "/nh"])
        .output()
        .unwrap();
    assert!(result.status.success());
    let output = String::from_utf8_lossy(&result.stdout);
    output
        .trim()
        .rsplit('"')
        .nth(1)
        .expect("account SID")
        .to_owned()
}
