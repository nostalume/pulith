#![cfg(feature = "process")]

use std::ffi::OsString;
use std::time::Duration;

use pulith::local::{LocalApply, LocalTarget};
mod common;
use common::{
    Fixture, assert_failure_keeps_target_missing, captured_contains, fixture_action,
    marker_environment,
};
use pulith::process::{
    CancellationToken, Cooperative, ExplicitEnvironment, InputSpec, ProcessAcquire, ProcessAction,
    ProcessConfigError, ProcessDiagnostics, ProcessError,
};
use pulith::{Acquire, Apply, Materialize, MaterializeMode};

type ProcessMaterialize = Materialize<&'static str, ProcessAction<Cooperative>, LocalTarget>;
type ProcessOutput = pulith::Acquired<
    ProcessMaterialize,
    pulith::local::StagedTree,
    pulith::EvidenceChain<pulith::process::ProcessEvidence<Cooperative>, ProcessDiagnostics>,
>;

fn acquire(
    root: &std::path::Path,
    action: ProcessAction<Cooperative>,
) -> Result<ProcessOutput, ProcessError> {
    ProcessAcquire::<Cooperative>::new().acquire(Materialize::new(
        "process-fixture",
        action,
        LocalTarget::new(root.join("published")),
        MaterializeMode::CreateNew,
    ))
}

#[test]
fn cooperative_process_stages_tree_before_local_apply() {
    let root = common::temp_dir();
    let target = root.path().join("published");
    let acquired = acquire(
        root.path(),
        fixture_action(Fixture::Success, "tree", Duration::from_secs(2)),
    )
    .unwrap();

    assert!(!target.exists());
    assert_eq!(
        std::fs::read(acquired.material.root().join("bin/tool")).unwrap(),
        b"pulith"
    );
    let applied = LocalApply.apply(acquired).unwrap();
    assert_eq!(applied.input.item, "process-fixture");
    assert_eq!(std::fs::read(target.join("bin/tool")).unwrap(), b"pulith");
}

#[test]
fn successful_staged_tree_is_removed_when_not_applied() {
    let root = common::temp_dir();
    let acquired = acquire(
        root.path(),
        fixture_action(Fixture::Success, "tree", Duration::from_secs(2)),
    )
    .unwrap();
    let workspace = acquired
        .material
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
    assert_failure_keeps_target_missing(Fixture::Nonzero, Duration::from_secs(2), |error| {
        matches!(error, ProcessError::ExitedNonZero { .. })
    });
}

#[test]
fn nonzero_exit_carries_captured_diagnostics() {
    assert_failure_keeps_target_missing(Fixture::Nonzero, Duration::from_secs(2), |error| {
        matches!(
            error,
            ProcessError::ExitedNonZero { diagnostics, .. }
                if captured_contains(diagnostics, b"dying-message")
        )
    });
}

#[test]
fn missing_declared_output_leaves_final_target_untouched() {
    assert_failure_keeps_target_missing(Fixture::MissingOutput, Duration::from_secs(2), |error| {
        matches!(error, ProcessError::OutputMissing { .. })
    });
}

#[test]
fn non_directory_output_leaves_final_target_untouched() {
    assert_failure_keeps_target_missing(Fixture::FileOutput, Duration::from_secs(2), |error| {
        matches!(error, ProcessError::OutputWrongKind { .. })
    });
}

#[test]
fn wrong_kind_output_carries_captured_diagnostics() {
    assert_failure_keeps_target_missing(Fixture::FileOutput, Duration::from_secs(2), |error| {
        matches!(
            error,
            ProcessError::OutputWrongKind { diagnostics, .. } if captured_contains(diagnostics, b"warn")
        )
    });
}

#[test]
fn success_capture_preserves_evidence_chain_order() {
    let root = common::temp_dir();
    let acquired = acquire(
        root.path(),
        fixture_action(Fixture::Success, "tree", Duration::from_secs(2)),
    )
    .unwrap();
    let pulith::EvidenceChain { previous, current } = &acquired.evidence;
    assert_eq!(
        previous.output,
        pulith::process::WorkspaceRelativePath::new("tree").unwrap()
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
    let root = common::temp_dir();
    let acquired = acquire(
        root.path(),
        fixture_action(Fixture::Success, "tree", Duration::from_secs(2)).with_capture_cap(8),
    )
    .unwrap();
    let current = &acquired.evidence.current;
    assert!(current.stdout_truncated && current.stderr_truncated);
    assert_eq!(current.stdout.as_deref().map(|s| s.len()), Some(8));
    assert_eq!(current.stderr.as_deref().map(|s| s.len()), Some(8));
}

#[test]
fn capture_cap_zero_disables_capture() {
    let root = common::temp_dir();
    let acquired = acquire(
        root.path(),
        fixture_action(Fixture::Success, "tree", Duration::from_secs(2)).with_capture_cap(0),
    )
    .unwrap();
    let current = &acquired.evidence.current;
    assert!(current.stdout.is_none() && current.stderr.is_none());
    assert_eq!(current.cap, 0);
}

#[test]
fn timeout_kills_direct_child_and_leaves_final_target_untouched() {
    assert_failure_keeps_target_missing(Fixture::Sleeps, Duration::from_millis(200), |error| {
        matches!(error, ProcessError::TimedOut { .. })
    });
}

#[test]
fn timeout_stops_descendant_and_carries_captured_diagnostics() {
    let root = common::temp_dir();
    let marker = root.path().join("descendant-marker");
    let target = root.path().join("published");

    let action = fixture_action(Fixture::SpawnsDescendant, "tree", Duration::from_secs(2))
        .with_environment(marker_environment(&marker));

    let result = ProcessAcquire::<Cooperative>::new().acquire(Materialize::new(
        "process-fixture",
        action,
        LocalTarget::new(&target),
        MaterializeMode::CreateNew,
    ));

    match result {
        Err(ProcessError::TimedOut { diagnostics, .. }) => {
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
    std::thread::sleep(Duration::from_millis(600));
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
    let root = common::temp_dir();
    let marker = root.path().join("descendant-marker");
    let target = root.path().join("published");

    let action = fixture_action(Fixture::SpawnsDescendant, "tree", Duration::from_secs(30))
        .with_environment(marker_environment(&marker));
    let token = CancellationToken::new();

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

    let result = ProcessAcquire::<Cooperative>::new().acquire_with_cancel(
        Materialize::new(
            "process-fixture",
            action,
            LocalTarget::new(&target),
            MaterializeMode::CreateNew,
        ),
        &token,
    );

    canceler.join().unwrap();

    match result {
        Err(ProcessError::Cancelled { diagnostics, .. }) => {
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
    std::thread::sleep(Duration::from_millis(600));
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
    let root = common::temp_dir();
    let target = root.path().join("published");
    let token = CancellationToken::new();
    token.cancel();

    let result = ProcessAcquire::<Cooperative>::new().acquire_with_cancel(
        Materialize::new(
            "process-fixture",
            fixture_action(Fixture::Success, "tree", Duration::from_secs(30))
                .with_capture_cap(4096),
            LocalTarget::new(&target),
            MaterializeMode::CreateNew,
        ),
        &token,
    );

    match result {
        Err(ProcessError::Cancelled { diagnostics, .. }) => {
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
    let root = common::temp_dir();
    let source = root.path().join("source-input.txt");
    std::fs::write(&source, b"closure-bytes-42").unwrap();
    let target = root.path().join("published");

    let action = fixture_action(Fixture::CopiesInputEnv, "tree", Duration::from_secs(30))
        .with_inputs([InputSpec::new(&source, "input.txt")]);

    let acquired = acquire(root.path(), action).unwrap();
    LocalApply.apply(acquired).unwrap();
    assert_eq!(
        std::fs::read(target.join("file.txt")).unwrap(),
        b"closure-bytes-42"
    );
}

#[test]
fn workspace_relative_argument_can_reference_a_staged_input() {
    let root = common::temp_dir();
    let source = root.path().join("source-input.txt");
    std::fs::write(&source, b"arg-referenced-bytes").unwrap();
    let target = root.path().join("published");

    let action = fixture_action(Fixture::CopiesInputArg, "tree", Duration::from_secs(30))
        .with_inputs([InputSpec::new(&source, "input.txt")]);

    let acquired = acquire(root.path(), action).unwrap();
    LocalApply.apply(acquired).unwrap();
    assert_eq!(
        std::fs::read(target.join("arg-file.txt")).unwrap(),
        b"arg-referenced-bytes"
    );
}

#[test]
fn missing_declared_input_fails_pre_spawn_and_keeps_target_missing() {
    let root = common::temp_dir();
    let target = root.path().join("published");
    let missing = root.path().join("nope.txt");

    let action = fixture_action(Fixture::Success, "tree", Duration::from_secs(30))
        .with_inputs([InputSpec::new(&missing, "nope.txt")]);

    let result = ProcessAcquire::<Cooperative>::new().acquire(Materialize::new(
        "process-fixture",
        action,
        LocalTarget::new(&target),
        MaterializeMode::CreateNew,
    ));

    match result {
        Err(ProcessError::InputMissing { path }) => assert_eq!(path, missing),
        other => panic!("expected InputMissing, got: {other:?}"),
    }
    assert!(!target.exists());
}

#[test]
fn colliding_staged_names_fail_pre_spawn() {
    let root = common::temp_dir();
    let first = root.path().join("first.txt");
    let second = root.path().join("second.txt");
    std::fs::write(&first, b"a").unwrap();
    std::fs::write(&second, b"b").unwrap();
    let target = root.path().join("published");

    let action = fixture_action(Fixture::Success, "tree", Duration::from_secs(30)).with_inputs([
        InputSpec::new(&first, "same"),
        InputSpec::new(&second, "same"),
    ]);

    let result = ProcessAcquire::<Cooperative>::new().acquire(Materialize::new(
        "process-fixture",
        action,
        LocalTarget::new(&target),
        MaterializeMode::CreateNew,
    ));

    match result {
        Err(ProcessError::InputCollision { name }) => assert_eq!(name, "same"),
        other => panic!("expected InputCollision, got: {other:?}"),
    }
    assert!(!target.exists());
}

#[test]
fn invalid_staged_name_fails_pre_spawn() {
    let root = common::temp_dir();
    let source = root.path().join("source.txt");
    std::fs::write(&source, b"x").unwrap();
    let target = root.path().join("published");

    for invalid in ["a/b", "..", ""] {
        let action = fixture_action(Fixture::Success, "tree", Duration::from_secs(30))
            .with_inputs([InputSpec::new(&source, invalid)]);

        let result = ProcessAcquire::<Cooperative>::new().acquire(Materialize::new(
            "process-fixture",
            action,
            LocalTarget::new(&target),
            MaterializeMode::CreateNew,
        ));

        match result {
            Err(ProcessError::InputCollision { name }) => assert_eq!(name, invalid),
            other => panic!("expected InputCollision for {invalid:?}, got: {other:?}"),
        }
    }
    assert!(!target.exists());
}

#[test]
fn input_root_environment_key_is_reserved() {
    let error =
        ExplicitEnvironment::new([(OsString::from("PULITH_INPUT_ROOT"), OsString::from("x"))])
            .unwrap_err();
    assert!(matches!(
        error,
        ProcessConfigError::ReservedEnvironmentKey(_)
    ));
}
