#![cfg(feature = "process")]

use std::ffi::OsString;
use std::time::Duration;

mod common;
use common::{
    Fixture, assert_failure_keeps_target_missing, assert_marker_stable, captured_contains,
    fixture_process, marker_environment,
};
use pulith::process::{
    CancelToken, ConfigError, EnvVars, ManagedProcess, OutputProcess, OutputResult, ProcessEnd,
    ProcessObservation, RunError, StagedInput, WorktreeProcess,
};

type ProcessOutput = OutputResult;

fn acquire(root: &std::path::Path, action: OutputProcess) -> Result<ProcessOutput, RunError> {
    let _ = root;
    action.prepare()?.execute()
}

#[test]
fn managed_process_reports_nonzero_exit_as_observation() {
    let _guard = common::process_fixture_guard();
    #[cfg(unix)]
    let arguments = ["-c", "exit 7"].map(OsString::from);
    #[cfg(windows)]
    let arguments = ["-NoProfile", "-NonInteractive", "-Command", "exit 7"].map(OsString::from);

    let end = ManagedProcess::new(common::absolute_program(), std::env::current_dir().unwrap())
        .unwrap()
        .with_arguments(arguments)
        .start()
        .unwrap()
        .wait()
        .unwrap();

    assert!(matches!(
        end,
        ProcessEnd::Exited { status, .. } if status.code() == Some(7)
    ));
}

#[test]
fn managed_process_observes_running_and_stops_explicitly() {
    let _guard = common::process_fixture_guard();
    #[cfg(unix)]
    let arguments = ["-c", "sleep 5"].map(OsString::from);
    #[cfg(windows)]
    let arguments = [
        "-NoProfile",
        "-NonInteractive",
        "-Command",
        "Start-Sleep -Seconds 5",
    ]
    .map(OsString::from);

    let mut session =
        ManagedProcess::new(common::absolute_program(), std::env::current_dir().unwrap())
            .unwrap()
            .with_arguments(arguments)
            .start()
            .unwrap();
    assert!(matches!(
        session.poll_exit().unwrap(),
        ProcessObservation::Running
    ));
    assert!(matches!(
        session.stop_within(Duration::from_secs(2)).unwrap(),
        ProcessEnd::Stopped { .. }
    ));
}

#[test]
fn managed_stop_at_zero_deadline_keeps_the_session_available() {
    let _guard = common::process_fixture_guard();
    #[cfg(unix)]
    let arguments = ["-c", "sleep 5"].map(OsString::from);
    #[cfg(windows)]
    let arguments = [
        "-NoProfile",
        "-NonInteractive",
        "-Command",
        "Start-Sleep -Seconds 5",
    ]
    .map(OsString::from);
    let mut session =
        ManagedProcess::new(common::absolute_program(), std::env::current_dir().unwrap())
            .unwrap()
            .with_arguments(arguments)
            .start()
            .unwrap();
    match session.stop_within(Duration::ZERO) {
        Err(pulith::process::SessionError::StopTimedOut { .. }) => {
            assert!(matches!(session.wait().unwrap(), ProcessEnd::Exited { .. }));
        }
        Ok(ProcessEnd::Stopped { .. }) => {
            assert!(matches!(
                session.poll_exit().unwrap(),
                ProcessObservation::Exited(_)
            ));
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn worktree_process_runs_in_the_existing_worktree_and_returns_evidence() {
    let _guard = common::process_fixture_guard();
    let root = common::temp_dir();
    let worktree = root.path().join("caller-worktree");
    std::fs::create_dir(&worktree).unwrap();

    #[cfg(unix)]
    let arguments = ["-c", "printf ok > worktree-marker; printf worktree"].map(OsString::from);
    #[cfg(windows)]
    let arguments = [
        "-NoProfile",
        "-NonInteractive",
        "-Command",
        "[IO.File]::WriteAllText((Join-Path (Get-Location) 'worktree-marker'), 'ok'); Write-Output 'worktree'",
    ]
    .map(OsString::from);

    let result = WorktreeProcess::new(
        common::absolute_program(),
        worktree.clone(),
        Duration::from_secs(10),
    )
    .unwrap()
    .with_arguments(arguments)
    .execute()
    .unwrap();

    assert_eq!(result.evidence.working_dir, worktree);
    assert_eq!(
        std::fs::read(result.evidence.working_dir.join("worktree-marker")).unwrap(),
        b"ok"
    );
    assert!(captured_contains(&result.diagnostics, b"worktree"));
    assert!(!result.evidence.working_dir.join("output").exists());
}

#[test]
fn worktree_process_rejects_missing_worktree_before_spawn() {
    let _guard = common::process_fixture_guard();
    let root = common::temp_dir();
    let missing = root.path().join("missing-worktree");
    let result = WorktreeProcess::new(
        common::absolute_program(),
        missing.clone(),
        Duration::from_secs(10),
    )
    .unwrap()
    .execute();

    assert!(matches!(result, Err(RunError::WorktreeMissing { path }) if path == missing));
}

fn sleeping_worktree_process(worktree: &std::path::Path, timeout: Duration) -> WorktreeProcess {
    #[cfg(unix)]
    let arguments = ["-c", "sleep 5"].map(OsString::from);
    #[cfg(windows)]
    let arguments = [
        "-NoProfile",
        "-NonInteractive",
        "-Command",
        "Start-Sleep -Seconds 5",
    ]
    .map(OsString::from);
    WorktreeProcess::new(common::absolute_program(), worktree, timeout)
        .unwrap()
        .with_arguments(arguments)
}

fn scripted_worktree_process(
    worktree: &std::path::Path,
    unix_script: &str,
    windows_script: &str,
) -> WorktreeProcess {
    #[cfg(unix)]
    let _ = windows_script;
    #[cfg(windows)]
    let _ = unix_script;
    #[cfg(unix)]
    let arguments = [OsString::from("-c"), OsString::from(unix_script)];
    #[cfg(windows)]
    let arguments = [
        OsString::from("-NoProfile"),
        OsString::from("-NonInteractive"),
        OsString::from("-Command"),
        OsString::from(windows_script),
    ];
    WorktreeProcess::new(
        common::absolute_program(),
        worktree,
        Duration::from_secs(15),
    )
    .unwrap()
    .with_arguments(arguments)
}

#[test]
fn worktree_timeout_is_distinct_from_cancellation() {
    let _guard = common::process_fixture_guard();
    let root = common::temp_dir();
    let result = sleeping_worktree_process(root.path(), Duration::from_millis(100)).execute();
    assert!(matches!(result, Err(RunError::TimedOut { .. })));
}

#[test]
fn worktree_cancellable_stops_a_running_child() {
    let _guard = common::process_fixture_guard();
    let root = common::temp_dir();
    let token = CancelToken::new();
    let canceler = std::thread::spawn({
        let token = token.clone();
        move || {
            std::thread::sleep(Duration::from_millis(100));
            token.cancel();
        }
    });
    let result =
        sleeping_worktree_process(root.path(), Duration::from_secs(5)).execute_cancellable(&token);
    canceler.join().unwrap();
    assert!(matches!(result, Err(RunError::Cancelled { .. })));
}

#[test]
fn worktree_pre_cancelled_fails_before_child_effect() {
    let _guard = common::process_fixture_guard();
    let root = common::temp_dir();
    let token = CancelToken::new();
    token.cancel();
    let result =
        sleeping_worktree_process(root.path(), Duration::from_secs(5)).execute_cancellable(&token);
    match result {
        Err(RunError::Cancelled { diagnostics, .. }) => {
            assert_eq!(diagnostics.cap, 0);
            assert!(diagnostics.stdout.is_none() && diagnostics.stderr.is_none());
        }
        other => panic!("expected Cancelled, got: {other:?}"),
    }
}

#[test]
fn worktree_rejects_file_and_reports_spawn_and_nonzero_distinctly() {
    let _guard = common::process_fixture_guard();
    let root = common::temp_dir();
    let file = root.path().join("not-a-directory");
    std::fs::write(&file, b"x").unwrap();
    let wrong_kind = WorktreeProcess::new(
        common::absolute_program(),
        file.clone(),
        Duration::from_secs(10),
    )
    .unwrap()
    .execute();
    assert!(matches!(wrong_kind, Err(RunError::WorktreeWrongKind { path }) if path == file));

    let missing_program = root.path().join("missing-program.exe");
    let spawn = WorktreeProcess::new(missing_program, root.path(), Duration::from_secs(10))
        .unwrap()
        .execute();
    assert!(matches!(spawn, Err(RunError::Spawn { .. })));

    let nonzero = scripted_worktree_process(
        root.path(),
        "printf failed; exit 7",
        "Write-Output 'failed'; exit 7",
    )
    .execute();
    assert!(
        matches!(nonzero, Err(RunError::ExitedNonZero { diagnostics, .. }) if captured_contains(&diagnostics, b"failed"))
    );
}

#[test]
fn worktree_inherits_environment_and_caps_diagnostics() {
    let _guard = common::process_fixture_guard();
    let root = common::temp_dir();
    let result = scripted_worktree_process(
        root.path(),
        "printf '%s' \"$PATH\"",
        "Write-Output $env:SystemRoot",
    )
    .with_capture_cap(8)
    .execute()
    .unwrap();
    assert_eq!(
        result.diagnostics.stdout.as_deref().map(<[u8]>::len),
        Some(8)
    );
    assert!(result.diagnostics.stdout_truncated);
}

#[test]
fn worktree_env_vars_do_not_inherit_caller_entries() {
    let _guard = common::process_fixture_guard();
    let root = common::temp_dir();
    let key = "PULITH_WORKTREE_AMBIENT_FIXTURE";
    unsafe { std::env::set_var(key, "ambient") };
    let admitted = [(OsString::from("ADMITTED"), OsString::from("yes"))];
    #[cfg(unix)]
    let entries = admitted;
    #[cfg(windows)]
    let entries = {
        let mut entries = common::windows_shell_environment();
        entries.extend(admitted);
        entries
    };
    let environment = EnvVars::new(entries).unwrap();
    let result = scripted_worktree_process(
        root.path(),
        "printf '%s:%s' \"${PULITH_WORKTREE_AMBIENT_FIXTURE-unset}\" \"$ADMITTED\"",
        "if ($null -eq $env:PULITH_WORKTREE_AMBIENT_FIXTURE) {$ambient='unset'} else {$ambient=$env:PULITH_WORKTREE_AMBIENT_FIXTURE}; Write-Output \"${ambient}:$env:ADMITTED\"",
    )
    .execute_in_environment(environment)
    .unwrap();
    unsafe { std::env::remove_var(key) };
    assert!(captured_contains(&result.diagnostics, b"unset:yes"));
}

#[test]
fn worktree_cancellation_stops_descendants() {
    let _guard = common::process_fixture_guard();
    let root = common::temp_dir();
    let marker = root.path().join("descendant-marker");
    #[cfg(windows)]
    std::fs::write(
        root.path().join("loop.ps1"),
        "while($true){[IO.File]::AppendAllText((Join-Path (Get-Location) 'descendant-marker'),'x'); Start-Sleep -Milliseconds 50}",
    )
    .unwrap();
    let action = scripted_worktree_process(
        root.path(),
        "sh -c 'while :; do printf x >> descendant-marker; sleep 0.05; done' & wait",
        "Start-Process -FilePath (Join-Path $PSHOME 'powershell.exe') -ArgumentList '-NoProfile','-NonInteractive','-ExecutionPolicy','Bypass','-File',(Join-Path (Get-Location) 'loop.ps1') -NoNewWindow -PassThru | ForEach-Object { $_.WaitForExit() }",
    );
    let token = CancelToken::new();
    let canceler = std::thread::spawn({
        let marker = marker.clone();
        let token = token.clone();
        move || {
            for _ in 0..500 {
                if std::fs::metadata(&marker).map(|m| m.len()).unwrap_or(0) > 0 {
                    token.cancel();
                    return;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            panic!("descendant never started");
        }
    });
    let result = action.execute_cancellable(&token);
    assert!(
        matches!(result, Err(RunError::Cancelled { .. })),
        "expected Cancelled, got {result:?}"
    );
    canceler.join().unwrap();
    let before = std::fs::metadata(&marker).map(|m| m.len()).unwrap_or(0);
    assert_marker_stable(&marker, before, Duration::from_millis(300));
    let after = std::fs::metadata(&marker).map(|m| m.len()).unwrap_or(0);
    assert!(before > 0);
    assert_eq!(before, after, "descendant survived cancellation");
}

#[test]
fn output_process_stages_tree_before_local_apply() {
    let _guard = common::process_fixture_guard();
    let root = common::temp_dir();
    let target = root.path().join("published");
    let acquired = acquire(
        root.path(),
        fixture_process(Fixture::Success, "tree", Duration::from_secs(10)),
    )
    .unwrap();

    assert!(!target.exists());
    assert_eq!(
        std::fs::read(acquired.tree.root().join("bin/tool")).unwrap(),
        b"pulith"
    );
    acquired
        .tree
        .publish(
            pulith::local::LocalPath::new(target.clone()).unwrap(),
            pulith::local::TreeAdmission::Absent,
        )
        .unwrap();
    assert_eq!(std::fs::read(target.join("bin/tool")).unwrap(), b"pulith");
}

#[test]
fn successful_staged_tree_is_removed_when_not_applied() {
    let _guard = common::process_fixture_guard();
    let root = common::temp_dir();
    let acquired = acquire(
        root.path(),
        fixture_process(Fixture::Success, "tree", Duration::from_secs(10)),
    )
    .unwrap();
    let workspace = acquired
        .tree
        .root()
        .parent()
        .and_then(|path| path.parent())
        .unwrap()
        .to_path_buf();

    drop(acquired);
    assert!(!workspace.exists());
}

#[test]
fn nonzero_exit_leaves_final_target_untouched() {
    let _guard = common::process_fixture_guard();
    assert_failure_keeps_target_missing(Fixture::Nonzero, Duration::from_secs(10), |error| {
        matches!(error, RunError::ExitedNonZero { .. })
    });
}

#[test]
fn nonzero_exit_carries_captured_diagnostics() {
    let _guard = common::process_fixture_guard();
    assert_failure_keeps_target_missing(Fixture::Nonzero, Duration::from_secs(10), |error| {
        matches!(
            error,
            RunError::ExitedNonZero { diagnostics, .. }
                if captured_contains(diagnostics, b"dying-message")
        )
    });
}

#[test]
fn missing_declared_output_leaves_final_target_untouched() {
    let _guard = common::process_fixture_guard();
    assert_failure_keeps_target_missing(Fixture::MissingOutput, Duration::from_secs(10), |error| {
        matches!(error, RunError::OutputMissing { .. })
    });
}

#[test]
fn non_directory_output_leaves_final_target_untouched() {
    let _guard = common::process_fixture_guard();
    assert_failure_keeps_target_missing(Fixture::FileOutput, Duration::from_secs(10), |error| {
        matches!(error, RunError::OutputWrongKind { .. })
    });
}

#[test]
fn wrong_kind_output_carries_captured_diagnostics() {
    let _guard = common::process_fixture_guard();
    assert_failure_keeps_target_missing(Fixture::FileOutput, Duration::from_secs(10), |error| {
        matches!(
            error,
            RunError::OutputWrongKind { diagnostics, .. } if captured_contains(diagnostics, b"warn")
        )
    });
}

#[test]
fn success_capture_preserves_evidence_chain_order() {
    let _guard = common::process_fixture_guard();
    let root = common::temp_dir();
    let acquired = acquire(
        root.path(),
        fixture_process(Fixture::Success, "tree", Duration::from_secs(10)),
    )
    .unwrap();
    let previous = &acquired.evidence;
    let current = &acquired.diagnostics;
    assert_eq!(
        previous.output,
        pulith::process::OutputPath::new("tree").unwrap()
    );
    let stdout = current.stdout.as_deref().expect("captured stdout");
    let stderr = current.stderr.as_deref().expect("captured stderr");
    assert!(captured_contains(current, b"out-line"));
    assert!(captured_contains(current, b"more-output"));
    assert!(common::contains_bytes(stderr, b"err-line"));
    assert!(stdout.windows(8).any(|w| w == b"out-line"));
    assert!(!current.stdout_truncated && !current.stderr_truncated);
}

#[test]
fn capture_truncates_streams_at_the_cap() {
    let _guard = common::process_fixture_guard();
    let root = common::temp_dir();
    let acquired = acquire(
        root.path(),
        fixture_process(Fixture::Success, "tree", Duration::from_secs(10)).with_capture_cap(8),
    )
    .unwrap();
    let current = &acquired.diagnostics;
    assert!(current.stdout_truncated && current.stderr_truncated);
    assert_eq!(current.stdout.as_deref().map(|s| s.len()), Some(8));
    assert_eq!(current.stderr.as_deref().map(|s| s.len()), Some(8));
}

#[test]
fn capture_cap_zero_disables_capture() {
    let _guard = common::process_fixture_guard();
    let root = common::temp_dir();
    let acquired = acquire(
        root.path(),
        fixture_process(Fixture::Success, "tree", Duration::from_secs(10)).with_capture_cap(0),
    )
    .unwrap();
    let current = &acquired.diagnostics;
    assert!(current.stdout.is_none() && current.stderr.is_none());
    assert_eq!(current.cap, 0);
}

#[cfg(unix)]
#[test]
fn capture_reports_the_child_stream_even_if_workspace_names_change() {
    use pulith::process::{Arg, OutputPath, OutputProcess};

    let root = common::temp_dir();
    let script = "printf 'actual stdout'; printf 'actual stderr' >&2; \
        printf 'another file' > stdout.log; printf 'another file' > stderr.log; \
        mkdir -p \"$PULITH_OUTPUT_ROOT\"";
    let process = OutputProcess::new(
        "/bin/sh",
        OutputPath::new("tree").unwrap(),
        Duration::from_secs(10),
    )
    .unwrap()
    .with_arguments([Arg::Literal("-c".into()), Arg::Literal(script.into())]);
    let result = acquire(root.path(), process).unwrap();
    assert_eq!(
        result.diagnostics.stdout.as_deref(),
        Some(b"actual stdout".as_slice())
    );
    assert_eq!(
        result.diagnostics.stderr.as_deref(),
        Some(b"actual stderr".as_slice())
    );
}

#[test]
fn timeout_kills_direct_child_and_leaves_final_target_untouched() {
    let _guard = common::process_fixture_guard();
    assert_failure_keeps_target_missing(Fixture::Sleeps, Duration::from_millis(200), |error| {
        matches!(error, RunError::TimedOut { .. })
    });
}

#[test]
fn timeout_stops_descendant_and_carries_captured_diagnostics() {
    let _guard = common::process_fixture_guard();
    let root = common::temp_dir();
    let marker = root.path().join("descendant-marker");
    let target = root.path().join("published");

    let action = fixture_process(Fixture::SpawnsDescendant, "tree", Duration::from_secs(10))
        .with_environment(marker_environment(&marker));

    let result = action
        .prepare()
        .and_then(pulith::process::PreparedProcess::execute);

    match result {
        Err(RunError::TimedOut { diagnostics, .. }) => {
            let stdout = diagnostics.stdout.as_deref().expect("captured stdout");
            assert!(
                stdout.windows(7).any(|window| window == b"spawned"),
                "expected 'spawned' in captured stdout: {:?}",
                String::from_utf8_lossy(stdout)
            );
        }
        other => panic!("expected TimedOut, got: {other:?}"),
    }

    // The admitted tree was stopped: the descendant marker stops growing.
    let before = std::fs::metadata(&marker)
        .map(|meta| meta.len())
        .unwrap_or(0);
    assert_marker_stable(&marker, before, Duration::from_millis(600));
    let after = std::fs::metadata(&marker)
        .map(|meta| meta.len())
        .unwrap_or(0);
    assert_eq!(
        before, after,
        "descendant marker kept growing after the tree stop"
    );
    assert!(!target.exists());
}

#[test]
fn cancel_stops_descendant_and_returns_cancelled_with_captured_diagnostics() {
    let _guard = common::process_fixture_guard();
    let root = common::temp_dir();
    let marker = root.path().join("descendant-marker");
    let target = root.path().join("published");

    let action = fixture_process(Fixture::SpawnsDescendant, "tree", Duration::from_secs(30))
        .with_environment(marker_environment(&marker));
    let token = CancelToken::new();

    // Cancel from another thread as soon as the admitted tree is demonstrably running.
    let canceler = std::thread::spawn({
        let marker = marker.clone();
        let token = token.clone();
        move || {
            for _ in 0..200 {
                if std::fs::metadata(&marker)
                    .map(|meta| meta.len())
                    .unwrap_or(0)
                    > 0
                {
                    token.cancel();
                    return;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            panic!("descendant marker never appeared");
        }
    });

    let result = action
        .prepare()
        .and_then(|prepared| prepared.execute_cancellable(&token));

    canceler.join().unwrap();

    match result {
        Err(RunError::Cancelled { diagnostics, .. }) => {
            let stdout = diagnostics.stdout.as_deref().expect("captured stdout");
            assert!(
                stdout.windows(7).any(|window| window == b"spawned"),
                "expected 'spawned' in captured stdout: {:?}",
                String::from_utf8_lossy(stdout)
            );
        }
        other => panic!("expected Cancelled, got: {other:?}"),
    }

    // The admitted tree was stopped by the cancellation: the descendant marker stops growing.
    let before = std::fs::metadata(&marker)
        .map(|meta| meta.len())
        .unwrap_or(0);
    assert!(before > 0, "descendant never wrote before cancellation");
    assert_marker_stable(&marker, before, Duration::from_millis(600));
    let after = std::fs::metadata(&marker)
        .map(|meta| meta.len())
        .unwrap_or(0);
    assert_eq!(
        before, after,
        "descendant marker kept growing after cancellation"
    );
    assert!(!target.exists());
}

#[test]
fn pre_cancelled_token_fails_fast_without_spawning() {
    let _guard = common::process_fixture_guard();
    let root = common::temp_dir();
    let target = root.path().join("published");
    let token = CancelToken::new();
    token.cancel();

    let result = fixture_process(Fixture::Success, "tree", Duration::from_secs(30))
        .with_capture_cap(4096)
        .prepare()
        .and_then(|prepared| prepared.execute_cancellable(&token));

    match result {
        Err(RunError::Cancelled { diagnostics, .. }) => {
            // Fail-fast diagnostics are empty: nothing was spawned or captured.
            assert!(diagnostics.stdout.is_none() && diagnostics.stderr.is_none());
            assert_eq!(diagnostics.cap, 0);
        }
        other => panic!("expected Cancelled, got: {other:?}"),
    }
    assert!(!target.exists());
}

#[test]
fn staged_input_is_reachable_with_exact_bytes_via_input_root() {
    let _guard = common::process_fixture_guard();
    let root = common::temp_dir();
    let source = root.path().join("source-input.txt");
    std::fs::write(&source, b"closure-bytes-42").unwrap();
    let target = root.path().join("published");

    let action = fixture_process(Fixture::CopiesInputEnv, "tree", Duration::from_secs(30))
        .with_inputs([StagedInput::new(&source, "input.txt").unwrap()]);

    let acquired = acquire(root.path(), action).unwrap();
    acquired
        .tree
        .publish(
            pulith::local::LocalPath::new(target.clone()).unwrap(),
            pulith::local::TreeAdmission::Absent,
        )
        .unwrap();
    assert_eq!(
        std::fs::read(target.join("file.txt")).unwrap(),
        b"closure-bytes-42"
    );
}

#[test]
fn structured_argument_can_reference_a_staged_input() {
    let _guard = common::process_fixture_guard();
    let root = common::temp_dir();
    let source = root.path().join("source-input.txt");
    std::fs::write(&source, b"arg-referenced-bytes").unwrap();
    let target = root.path().join("published");

    let action = fixture_process(Fixture::CopiesInputArg, "tree", Duration::from_secs(30))
        .with_inputs([StagedInput::new(&source, "input.txt").unwrap()]);

    let acquired = acquire(root.path(), action).unwrap();
    acquired
        .tree
        .publish(
            pulith::local::LocalPath::new(target.clone()).unwrap(),
            pulith::local::TreeAdmission::Absent,
        )
        .unwrap();
    assert_eq!(
        std::fs::read(target.join("arg-file.txt")).unwrap(),
        b"arg-referenced-bytes"
    );
}

#[test]
fn missing_declared_input_fails_pre_spawn_and_keeps_target_missing() {
    let _guard = common::process_fixture_guard();
    let root = common::temp_dir();
    let target = root.path().join("published");
    let missing = root.path().join("nope.txt");

    let action = fixture_process(Fixture::Success, "tree", Duration::from_secs(30))
        .with_inputs([StagedInput::new(&missing, "nope.txt").unwrap()]);

    let result = action
        .prepare()
        .and_then(pulith::process::PreparedProcess::execute);

    match result {
        Err(RunError::InputMissing { path }) => assert_eq!(path, missing),
        other => panic!("expected InputMissing, got: {other:?}"),
    }
    assert!(!target.exists());
}

#[test]
fn non_regular_declared_input_fails_pre_spawn() {
    let _guard = common::process_fixture_guard();
    let root = common::temp_dir();
    let directory = root.path().join("directory");
    std::fs::create_dir(&directory).unwrap();
    let action = fixture_process(Fixture::Success, "tree", Duration::from_secs(30))
        .with_inputs([StagedInput::new(&directory, "input").unwrap()]);
    assert!(matches!(
        action.prepare().and_then(pulith::process::PreparedProcess::execute),
        Err(RunError::InputWrongKind { path }) if path == directory
    ));

    let source = root.path().join("source");
    let link = root.path().join("link");
    std::fs::write(&source, b"bytes").unwrap();
    if common::file_symlink(&source, &link).is_ok() {
        let action = fixture_process(Fixture::Success, "tree", Duration::from_secs(30))
            .with_inputs([StagedInput::new(&link, "input").unwrap()]);
        assert!(matches!(
            action.prepare().and_then(pulith::process::PreparedProcess::execute),
            Err(RunError::InputWrongKind { path }) if path == link
        ));
    }
}

#[test]
fn colliding_staged_names_fail_pre_spawn() {
    let _guard = common::process_fixture_guard();
    let root = common::temp_dir();
    let first = root.path().join("first.txt");
    let second = root.path().join("second.txt");
    std::fs::write(&first, b"a").unwrap();
    std::fs::write(&second, b"b").unwrap();
    let target = root.path().join("published");

    let action = fixture_process(Fixture::Success, "tree", Duration::from_secs(30)).with_inputs([
        StagedInput::new(&first, "same").unwrap(),
        StagedInput::new(&second, "same").unwrap(),
    ]);

    let result = action
        .prepare()
        .and_then(pulith::process::PreparedProcess::execute);

    match result {
        Err(RunError::InputCollision { name }) => assert_eq!(name, "same"),
        other => panic!("expected InputCollision, got: {other:?}"),
    }
    assert!(!target.exists());
}

#[test]
fn staged_input_admission_rejects_invalid_name_and_relative_source() {
    let _guard = common::process_fixture_guard();
    let root = common::temp_dir();
    let source = root.path().join("source.txt");
    std::fs::write(&source, b"x").unwrap();
    for invalid in ["a/b", "..", ""] {
        assert!(StagedInput::new(&source, invalid).is_err());
    }
    assert!(StagedInput::new("relative.txt", "input.txt").is_err());
}

#[test]
fn input_root_environment_key_is_reserved() {
    let _guard = common::process_fixture_guard();
    let error =
        EnvVars::new([(OsString::from("PULITH_INPUT_ROOT"), OsString::from("x"))]).unwrap_err();
    assert!(matches!(error, ConfigError::ReservedEnvironmentKey(_)));
}
