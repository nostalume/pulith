use super::*;
use std::path::Path;
#[test]
fn root_is_explicit_and_absolute() {
    assert!(Command::parse(["env"].map(OsString::from)).is_err());
    assert!(Command::parse(["env", "--root", "relative"].map(OsString::from)).is_err());
    let root = std::env::temp_dir();
    assert!(
        Command::parse([
            OsString::from("unknown"),
            "--root".into(),
            root.into_os_string()
        ])
        .is_err()
    );
}

#[test]
fn service_verbs_are_orthogonal_and_require_one_declaration() {
    let root = std::env::temp_dir();
    for verb in [
        "install", "rebind", "enable", "start", "restart", "status", "stop", "disable", "remove",
    ] {
        let command = Command::parse([
            OsString::from("service"),
            OsString::from(verb),
            OsString::from("--root"),
            root.clone().into_os_string(),
            OsString::from("service.toml"),
        ])
        .unwrap();
        assert!(matches!(command, Command::Service(_, _, _)));
    }
    assert!(Command::parse(["service", "install"].map(OsString::from)).is_err());
    assert!(Command::parse(["service", "unknown", "--root"].map(OsString::from)).is_err());
}

#[test]
fn non_unicode_verb_is_usage_error() {
    #[cfg(unix)]
    let invalid = {
        use std::os::unix::ffi::OsStringExt;
        OsString::from_vec(vec![0xff])
    };
    #[cfg(windows)]
    let invalid = {
        use std::os::windows::ffi::OsStringExt;
        OsString::from_wide(&[0xd800])
    };
    assert!(Command::parse([invalid]).is_err());
}
#[test]
fn environment_plan_is_data() {
    let root = Path::new(if cfg!(windows) {
        r"C:\a root&x"
    } else {
        "/tmp/a root&x"
    });
    let plan = EnvironmentPlan::new(root.to_path_buf());
    assert_eq!(plan.home, root);
    assert_eq!(plan.path_prepend, root.join("current/shims"));
}

#[test]
fn environment_plan_changes_only_its_child() {
    let root = tempfile::Builder::new()
        .prefix("toolhost root & ")
        .tempdir()
        .unwrap();
    let child = root.path().join(recipe::executable_name("env-child"));
    let input = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/examples/toolhost/fixtures/environment_probe.rs");
    assert!(
        std::process::Command::new("rustc")
            .args([input.as_os_str(), "-o".as_ref(), child.as_os_str()])
            .status()
            .unwrap()
            .success()
    );
    let output = root.path().join("child env.txt");
    let before = std::env::var_os("TOOLHOST_HOME");
    let plan = EnvironmentPlan::new(root.path().to_path_buf());
    let prepend = plan.path_prepend.to_string_lossy().to_string();
    assert!(
        plan.run(child.into_os_string(), vec![output.as_os_str().to_owned()])
            .unwrap()
            .success()
    );
    assert_eq!(std::env::var_os("TOOLHOST_HOME"), before);
    let observed = std::fs::read_to_string(output).unwrap();
    assert!(observed.starts_with(&format!("{}\n", root.path().display())));
    assert!(observed.contains(&prepend));
}

#[test]
fn activation_selects_one_release_or_reports_capability_unavailable() {
    let root = tempfile::tempdir().unwrap();
    let first = root.path().join("installs/tool/1");
    let second = root.path().join("installs/tool/2");
    std::fs::create_dir_all(&first).unwrap();
    std::fs::create_dir_all(&second).unwrap();
    let current = root.path().join("current");
    let activated = LocalTarget::new(&first).unwrap().link_root(&current);
    match activated {
        Ok(_) => {
            assert_eq!(
                current.canonicalize().unwrap(),
                first.canonicalize().unwrap()
            );
            LocalTarget::new(&second)
                .unwrap()
                .link_root(&current)
                .unwrap();
            assert_eq!(
                current.canonicalize().unwrap(),
                second.canonicalize().unwrap()
            );
        }
        #[cfg(windows)]
        Err(pulith::local::LinkError::CapabilityUnavailable { .. }) => {
            assert!(!current.exists())
        }
        Err(error) => panic!("unexpected activation result: {error}"),
    }
}
