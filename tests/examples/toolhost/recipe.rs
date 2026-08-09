use super::*;

fn compile(fixture: &str, output: &Path) {
    let input = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/examples/toolhost/fixtures")
        .join(fixture);
    assert!(
        std::process::Command::new("rustc")
            .args([input.as_os_str(), "-o".as_ref(), output.as_os_str()])
            .status()
            .unwrap()
            .success()
    );
}

fn build(
    binary: PathBuf,
    runtime: Option<PathBuf>,
    verifications: Vec<Verification>,
) -> ResolvedBuild {
    let worktree = binary.parent().unwrap().canonicalize().unwrap();
    ResolvedBuild {
        identity: ("tool".into(), "1".into()),
        process: WorktreeProcess::new(&binary, worktree, Duration::from_secs(10)).unwrap(),
        outputs: Outputs { binary, runtime },
        verifications,
    }
}

#[test]
fn rejects_unknown_fields_and_zero_timeout() {
    assert!(Recipe::parse("name='x'\nversion='1'\nunknown=1\n[outputs]\nbinary='x'").is_err());
    let recipe = Recipe::parse(
        "name='x'\nversion='1'\n[build]\ncommand='x'\ntimeout_seconds=0\n[outputs]\nbinary='x'",
    )
    .unwrap();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("toolhost.toml");
    std::fs::write(&path, "").unwrap();
    assert!(
        recipe
            .resolve(&path)
            .unwrap_err()
            .to_string()
            .contains("positive")
    );
}

#[test]
fn absent_build_resolves_without_observing_recipe_parent_or_outputs() {
    let recipe = Recipe::parse("name='x'\nversion='1'\n[outputs]\nbinary='missing'").unwrap();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("missing/toolhost.toml");
    assert!(matches!(recipe.resolve(&path).unwrap(), Resolved::NoBuild));
}

#[test]
fn recipe_relative_command_and_outputs_resolve_under_worktree() {
    let dir = tempfile::tempdir().unwrap();
    let driver = dir.path().join(if cfg!(windows) {
        "driver.exe"
    } else {
        "driver"
    });
    std::fs::write(&driver, b"driver").unwrap();
    let path = dir.path().join("toolhost.toml");
    std::fs::write(&path, "").unwrap();
    let command = driver.file_name().unwrap().to_string_lossy();
    let text = format!(
        "name='x'\nversion='1'\n[build]\ncommand='./{command}'\ntimeout_seconds=1\n[outputs]\nbinary='out/tool'"
    );
    let Resolved::Build(build) = Recipe::parse(&text).unwrap().resolve(&path).unwrap() else {
        panic!()
    };
    let ResolvedBuild { outputs, .. } = *build;
    assert_eq!(
        outputs.binary,
        dir.path().canonicalize().unwrap().join("out/tool")
    );
}

#[test]
fn runtime_and_verification_must_make_one_exact_claim() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("toolhost.toml");
    std::fs::write(&path, "").unwrap();
    let driver = dir.path().join(if cfg!(windows) {
        "driver.exe"
    } else {
        "driver"
    });
    std::fs::write(&driver, b"driver").unwrap();
    let command = driver.file_name().unwrap().to_string_lossy();
    let text = format!(
        "name='x'\nversion='1'\n[build]\ncommand='./{command}'\ntimeout_seconds=1\n[outputs]\nbinary='x'\nruntime='runtime'"
    );
    let error = Recipe::parse(&text).unwrap().resolve(&path).unwrap_err();
    assert!(error.to_string().contains("exactly one loaded_runtime"));
}

#[test]
fn cli_command_name_resolves_to_one_absolute_program() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("toolhost.toml");
    std::fs::write(&path, "").unwrap();
    let text = "name='x'\nversion='1'\n[build]\ncommand='rustc'\ntimeout_seconds=1\n[outputs]\nbinary='out/tool'";
    assert!(matches!(
        Recipe::parse(text).unwrap().resolve(&path).unwrap(),
        Resolved::Build(_)
    ));
}

#[test]
fn install_harvests_declared_artifacts_verifies_and_publishes() {
    let fixture = tempfile::tempdir().unwrap();
    let binary = fixture.path().join(executable_name("built"));
    let shim = fixture.path().join(executable_name("shim"));
    let runtime = fixture.path().join("runtime");
    compile("version_tool.rs", &binary);
    std::fs::write(&shim, b"shim").unwrap();
    std::fs::create_dir_all(runtime.join("nested")).unwrap();
    std::fs::write(runtime.join("nested/library"), b"runtime").unwrap();
    std::fs::write(fixture.path().join("sibling.dll"), b"decoy").unwrap();
    let checks = vec![Verification::Stdout {
        args: vec!["--version".into()],
        stdout: "tool/1\n".into(),
    }];
    let root = fixture.path().join("root");
    let InstallOutcome::Published { release, .. } = build(binary, Some(runtime), checks)
        .install(&root, companions(shim))
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(
        std::fs::read(release.join("private-runtime/nested/library")).unwrap(),
        b"runtime"
    );
    assert!(!release.join("private-runtime/sibling.dll").exists());
}

#[test]
fn verification_failure_publishes_nothing() {
    let fixture = tempfile::tempdir().unwrap();
    let binary = fixture.path().join(executable_name("built"));
    let shim = fixture.path().join(executable_name("shim"));
    compile("failure_tool.rs", &binary);
    std::fs::write(&shim, b"shim").unwrap();
    let checks = vec![Verification::Stdout {
        args: vec!["--version".into()],
        stdout: "expected\n".into(),
    }];
    let root = fixture.path().join("root");
    assert!(
        build(binary, None, checks)
            .install(&root, companions(shim))
            .is_err()
    );
    assert!(!root.join("installs/tool/1").exists());
}

#[test]
fn post_verification_shape_check_rejects_a_deleted_dispatcher() {
    let fixture = tempfile::tempdir().unwrap();
    let binary = fixture.path().join(executable_name("built"));
    let shim = fixture.path().join(executable_name("shim"));
    compile("delete_shim.rs", &binary);
    std::fs::write(&shim, b"shim").unwrap();
    let checks = vec![Verification::Stdout {
        args: Vec::new(),
        stdout: String::new(),
    }];
    let root = fixture.path().join("root");
    assert!(
        build(binary, None, checks)
            .install(&root, companions(shim))
            .is_err()
    );
    assert!(!root.join("installs/tool/1").exists());
}

fn companions(path: PathBuf) -> Companions {
    Companions {
        dispatcher: LocalSource::new(&path).unwrap(),
        service_host: LocalSource::new(path).unwrap(),
    }
}

#[test]
fn loaded_runtime_witness_requires_staged_identity_and_origin() {
    let root = tempfile::tempdir().unwrap();
    let stage = root.path().join("stage");
    let runtime = stage.join("private-runtime");
    std::fs::create_dir_all(stage.join("bin")).unwrap();
    std::fs::create_dir_all(&runtime).unwrap();
    let expected = runtime.join(if cfg!(windows) {
        "expected.dll"
    } else {
        "libexpected.so"
    });
    let library = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/examples/toolhost/fixtures/runtime_library.rs");
    assert!(
        std::process::Command::new("rustc")
            .args([
                "--edition".as_ref(),
                "2024".as_ref(),
                "--crate-type".as_ref(),
                "cdylib".as_ref(),
                library.as_os_str(),
                "-o".as_ref(),
                expected.as_os_str()
            ])
            .status()
            .unwrap()
            .success()
    );
    let binary = stage.join("bin").join(executable_name("loader"));
    #[cfg(windows)]
    let loader = "runtime_loader_windows.rs";
    #[cfg(not(windows))]
    let loader = "runtime_loader_unix.rs";
    let input = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/examples/toolhost/fixtures")
        .join(loader);
    let mut command = std::process::Command::new("rustc");
    command.args([
        "--edition".as_ref(),
        "2024".as_ref(),
        input.as_os_str(),
        "-o".as_ref(),
        binary.as_os_str(),
    ]);
    #[cfg(not(windows))]
    command.args(["-C", "link-arg=-Wl,-rpath,$ORIGIN/../private-runtime"]);
    assert!(command.status().unwrap().success());
    let path = expected.file_name().unwrap().into();
    Verification::LoadedRuntime {
        args: Vec::new(),
        loaded_runtime: LoadedRuntime {
            identity: "runtime/1".into(),
            path,
        },
    }
    .verify((
        binary.canonicalize().unwrap(),
        stage.canonicalize().unwrap(),
    ))
    .unwrap();
}
