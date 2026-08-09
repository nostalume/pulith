use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::Once;

mod vtool_crash;

static BUILD: Once = Once::new();

fn example(name: &str) -> PathBuf {
    BUILD.call_once(|| {
        assert!(
            Command::new(env!("CARGO"))
                .args([
                    "build",
                    "--all-features",
                    "--example",
                    "toolhost",
                    "--example",
                    "toolhost-shim",
                    "--example",
                    "toolhost-service",
                    "--example",
                    "vtool",
                ])
                .status()
                .unwrap()
                .success()
        );
    });
    let suffix = std::env::consts::EXE_SUFFIX;
    std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("target"))
        .join("debug/examples")
        .join(format!("{name}{suffix}"))
}

#[test]
fn service_host_is_a_packaged_executable() {
    assert!(example("toolhost-service").is_file());
}

fn run(name: &str, args: impl IntoIterator<Item = OsString>) -> Output {
    Command::new(example(name)).args(args).output().unwrap()
}

fn assert_output(output: Output, code: i32, stdout: &[u8], stderr: &[u8]) {
    assert_eq!(output.status.code(), Some(code));
    assert_eq!(output.stdout, stdout);
    assert_eq!(output.stderr, stderr);
}

#[test]
fn toolhost_process_contract_is_exact() {
    assert_output(run("toolhost", []), 2, b"", b"toolhost: missing verb\n");
    let root = tempfile::tempdir().unwrap();
    let root_arg = root.path().as_os_str().to_owned();
    assert_output(
        run(
            "toolhost",
            ["unknown".into(), "--root".into(), root_arg.clone()],
        ),
        2,
        b"",
        b"toolhost: unknown verb: unknown\n",
    );
    assert_output(
        run(
            "toolhost",
            ["env".into(), "--root".into(), "relative".into()],
        ),
        2,
        b"",
        b"toolhost: --root must be absolute\n",
    );

    let no_build = root.path().join("no-build.toml");
    std::fs::write(
        &no_build,
        "name='tool'\nversion='1'\n[outputs]\nbinary='missing'",
    )
    .unwrap();
    assert_output(
        run(
            "toolhost",
            [
                "install".into(),
                "--root".into(),
                root_arg.clone(),
                no_build.into_os_string(),
            ],
        ),
        0,
        b"no-build\n",
        b"",
    );

    let invalid = root.path().join("invalid.toml");
    std::fs::write(
        &invalid,
        "name='bad/name'\nversion='1'\n[outputs]\nbinary='missing'",
    )
    .unwrap();
    assert_output(
        run(
            "toolhost",
            [
                "install".into(),
                "--root".into(),
                root_arg.clone(),
                invalid.into_os_string(),
            ],
        ),
        1,
        b"",
        b"toolhost: name must be one normal path component\n",
    );

    let env = run(
        "toolhost",
        ["env".into(), "--root".into(), root_arg.clone()],
    );
    let expected = format!(
        "TOOLHOST_HOME={}\nPATH_PREPEND={}\n",
        root.path().display(),
        root.path().join("current/shims").display()
    );
    assert_output(env, 0, expected.as_bytes(), b"");

    #[cfg(windows)]
    let child = ("cmd.exe", ["/C", "exit"]);
    #[cfg(unix)]
    let child = ("sh", ["-c", "exit"]);
    for code in [0, 7] {
        let output = run(
            "toolhost",
            [
                "run".into(),
                "--root".into(),
                root_arg.clone(),
                "--".into(),
                child.0.into(),
                child.1[0].into(),
                format!("{} {code}", child.1[1]).into(),
            ],
        );
        assert_output(output, code, b"", b"");
    }
}

#[test]
fn service_exposure_rejects_an_untrusted_root_before_manager_mutation() {
    let root = tempfile::tempdir().unwrap();
    let declaration = root.path().join("service.toml");
    std::fs::write(
        &declaration,
        "schema=1\nid='demo'\npayload='demo'\nargs=[]\n",
    )
    .unwrap();
    for verb in ["install", "rebind", "enable", "start", "restart"] {
        let output = run(
            "toolhost",
            [
                "service".into(),
                verb.into(),
                "--root".into(),
                root.path().as_os_str().to_owned(),
                declaration.as_os_str().to_owned(),
            ],
        );
        assert_eq!(output.status.code(), Some(1), "{verb}");
        assert!(output.stdout.is_empty(), "{verb}");
        assert!(
            output.stderr.starts_with(b"toolhost: "),
            "{verb}: {:?}",
            output.stderr
        );
    }
}

#[test]
fn service_observation_and_containment_do_not_require_a_trusted_root() {
    let root = tempfile::tempdir().unwrap();
    let declaration = root.path().join("service.toml");
    std::fs::write(
        &declaration,
        "schema=1\nid='pulith-missing-containment-probe'\npayload='demo'\nargs=[]\n",
    )
    .unwrap();
    for verb in ["status", "stop", "disable", "remove"] {
        let output = run(
            "toolhost",
            [
                "service".into(),
                verb.into(),
                "--root".into(),
                root.path().as_os_str().to_owned(),
                declaration.as_os_str().to_owned(),
            ],
        );
        assert!(
            !output
                .stderr
                .windows(b"service root owner".len())
                .any(|window| window == b"service root owner"),
            "{verb}: {:?}",
            output.stderr
        );
    }
}

#[test]
fn toolhost_packaged_vertical_builds_verifies_publishes_and_dispatches() {
    let fixture = tempfile::tempdir().unwrap();
    let runtime = fixture.path().join("runtime");
    std::fs::create_dir(&runtime).unwrap();
    let library = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/examples/toolhost/fixtures/runtime_library.rs");
    let library_name = if cfg!(windows) {
        "expected.dll"
    } else {
        "libexpected.so"
    };
    assert!(
        Command::new("rustc")
            .args([
                "--edition",
                "2024",
                "--crate-type",
                "cdylib",
                library.to_str().unwrap(),
                "-o",
                runtime.join(library_name).to_str().unwrap(),
            ])
            .status()
            .unwrap()
            .success()
    );

    #[cfg(windows)]
    let loader = "runtime_loader_windows.rs";
    #[cfg(not(windows))]
    let loader = "runtime_loader_unix.rs";
    let loader = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/examples/toolhost/fixtures")
        .join(loader)
        .to_string_lossy()
        .replace('\\', "/");
    let binary = format!("built{}", std::env::consts::EXE_SUFFIX);
    let arguments = [
        "--edition".to_string(),
        "2024".to_string(),
        loader,
        "-o".to_string(),
        binary.clone(),
    ];
    #[cfg(not(windows))]
    let arguments = arguments
        .into_iter()
        .chain([
            "-C".into(),
            "link-arg=-Wl,-rpath,$ORIGIN/../private-runtime".into(),
        ])
        .collect::<Vec<_>>();
    let args = arguments
        .iter()
        .map(|argument| format!("{argument:?}"))
        .collect::<Vec<_>>()
        .join(",");
    let recipe = fixture.path().join("toolhost.toml");
    std::fs::write(&recipe, format!(
        "name='tool'\nversion='1'\n[build]\ncommand='rustc'\nargs=[{args}]\ntimeout_seconds=30\n[outputs]\nbinary='{binary}'\nruntime='runtime'\n[[verify]]\nloaded_runtime={{identity='runtime/1',path='{library_name}'}}\n"
    )).unwrap();
    let root = fixture.path().join("root");
    let output = run(
        "toolhost",
        [
            "install".into(),
            "--root".into(),
            root.as_os_str().to_owned(),
            recipe.into_os_string(),
        ],
    );
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    assert!(output.stdout.starts_with(b"published="));
    let release = root.join("installs/tool/1");
    assert!(
        release
            .join("service")
            .join(format!("tool{}", std::env::consts::EXE_SUFFIX))
            .is_file()
    );
    assert!(release.join("private-runtime").join(library_name).is_file());
    let dispatched = Command::new(
        release
            .join("shims")
            .join(format!("tool{}", std::env::consts::EXE_SUFFIX)),
    )
    .output()
    .unwrap();
    assert_output(
        dispatched,
        0,
        format!(
            "identity=runtime/1\norigin={}\n",
            release
                .join("private-runtime")
                .join(library_name)
                .canonicalize()
                .unwrap()
                .display()
        )
        .as_bytes(),
        b"",
    );
}

#[test]
fn clis_reject_non_unicode_verbs() {
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
    assert_output(
        run("toolhost", [invalid.clone()]),
        2,
        b"",
        b"toolhost: verb is not Unicode\n",
    );
    assert_output(
        run("vtool", [invalid]),
        2,
        b"",
        b"vtool: verb is not Unicode\n",
    );
}

#[test]
fn vtool_process_contract_separates_streams_and_status() {
    assert_output(run("vtool", []), 2, b"", b"vtool: missing verb\n");
    let root = tempfile::tempdir().unwrap();
    let root_arg = root.path().as_os_str().to_owned();
    assert_output(
        run(
            "vtool",
            [
                "unknown".into(),
                "--root".into(),
                root_arg.clone(),
                "x".into(),
            ],
        ),
        2,
        b"",
        b"vtool: unknown verb: unknown\n",
    );
    assert_output(
        run(
            "vtool",
            [
                "plan".into(),
                "--root".into(),
                "relative".into(),
                "x".into(),
            ],
        ),
        2,
        b"",
        b"vtool: --root must be absolute\n",
    );

    let source = root.path().join("source");
    let manifest = root.path().join("vtool.toml");
    let path = source.to_string_lossy().replace('\\', "/");
    std::fs::write(
        &manifest,
        format!(
            r#"
name = "tool"
version = "1"
[windows.source]
kind = "local"
path = "{path}"
[windows.hash]
kind = "sha2"
hex = "{hash}"
[linux.source]
kind = "local"
path = "{path}"
[linux.hash]
kind = "sha2"
hex = "{hash}"
"#,
            hash = "0".repeat(64)
        ),
    )
    .unwrap();
    let output = run(
        "vtool",
        [
            "plan".into(),
            "--root".into(),
            root_arg,
            manifest.into_os_string(),
        ],
    );
    let expected = format!(
        "plan: tool@1\nsource={}\ntarget={}\n",
        path,
        root.path()
            .join("artifacts")
            .join("tool")
            .join("1")
            .display()
    );
    assert_output(output, 0, expected.as_bytes(), b"");
}
