#![cfg(feature = "process-async")]

use std::time::Duration;

mod common;
use common::{Fixture, captured_contains, fixture_process, marker_environment};
use pulith::AsyncAcquire;
use pulith::process::{CancelToken, OutputResult, RunError, StagedInput};

fn block_on<F: std::future::Future>(future: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(future)
}

type AcquireOutput = OutputResult;

#[test]
fn async_success_stages_tree_before_local_apply_and_preserves_evidence_order() {
    let root = tempfile::tempdir().unwrap();
    let target = root.path().join("published");
    let acquired: AcquireOutput = block_on(AsyncAcquire::acquire(fixture_process(
        Fixture::Success,
        "tree",
        Duration::from_secs(10),
    )))
    .unwrap();

    assert!(!target.exists());
    assert_eq!(
        std::fs::read(acquired.tree.root().join("bin/tool")).unwrap(),
        b"pulith"
    );
    let previous = &acquired.evidence;
    let current = &acquired.diagnostics;
    assert_eq!(
        previous.output,
        pulith::process::OutputPath::new("tree").unwrap()
    );
    let stdout = current.stdout.as_deref().expect("captured stdout");
    let stderr = current.stderr.as_deref().expect("captured stderr");
    assert!(
        stdout.windows(8).any(|window| window == b"out-line"),
        "stdout missing out-line: {:?}",
        String::from_utf8_lossy(stdout)
    );
    assert!(
        stderr.windows(8).any(|window| window == b"err-line"),
        "stderr missing err-line: {:?}",
        String::from_utf8_lossy(stderr)
    );
    assert!(!current.stdout_truncated && !current.stderr_truncated);

    acquired
        .tree
        .publish(pulith::local::LocalTarget::new(target.clone()).unwrap())
        .unwrap();
    assert_eq!(std::fs::read(target.join("bin/tool")).unwrap(), b"pulith");
}

#[test]
fn async_staged_input_is_reachable_with_exact_bytes_via_input_root() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source-input.txt");
    std::fs::write(&source, b"async-closure-bytes").unwrap();
    let target = root.path().join("published");

    let action = fixture_process(Fixture::CopiesInputEnv, "tree", Duration::from_secs(30))
        .with_inputs([StagedInput::new(&source, "input.txt").unwrap()]);

    let acquired: AcquireOutput = block_on(AsyncAcquire::acquire(action)).unwrap();
    acquired
        .tree
        .publish(pulith::local::LocalTarget::new(target.clone()).unwrap())
        .unwrap();
    assert_eq!(
        std::fs::read(target.join("file.txt")).unwrap(),
        b"async-closure-bytes"
    );
}

#[test]
fn async_timeout_stops_descendant_and_carries_captured_diagnostics() {
    let root = tempfile::tempdir().unwrap();
    let marker = root.path().join("descendant-marker");
    let target = root.path().join("published");

    let environment = marker_environment(&marker);

    let action = fixture_process(Fixture::SpawnsDescendant, "tree", Duration::from_secs(10))
        .with_environment(environment);

    let result = block_on(AsyncAcquire::acquire(action));

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
fn async_capture_truncates_streams_at_the_cap() {
    let root = tempfile::tempdir().unwrap();
    let _target = root.path().join("published");
    let action =
        fixture_process(Fixture::Success, "tree", Duration::from_secs(10)).with_capture_cap(16);
    let acquired: AcquireOutput = block_on(AsyncAcquire::acquire(action)).unwrap();

    let current = &acquired.diagnostics;
    assert_eq!(current.cap, 16);
    let stdout = current.stdout.as_deref().expect("captured stdout");
    assert!(stdout.len() <= 16, "stdout exceeded cap: {}", stdout.len());
    assert!(current.stdout_truncated, "expected stdout truncation");
}

#[test]
fn async_capture_cap_zero_disables_capture() {
    let root = tempfile::tempdir().unwrap();
    let _target = root.path().join("published");
    let action =
        fixture_process(Fixture::Success, "tree", Duration::from_secs(10)).with_capture_cap(0);
    let acquired: AcquireOutput = block_on(AsyncAcquire::acquire(action)).unwrap();

    let current = &acquired.diagnostics;
    assert_eq!(current.cap, 0);
    assert!(current.stdout.is_none() && current.stderr.is_none());
    assert!(!current.stdout_truncated && !current.stderr_truncated);
}

#[test]
fn async_nonzero_exit_carries_captured_diagnostics() {
    assert_failure(Fixture::Nonzero, Duration::from_secs(10), |error| {
        matches!(
            error,
            RunError::ExitedNonZero { diagnostics, .. }
                if captured_contains(diagnostics, b"dying-message")
        )
    });
}

#[test]
fn async_wrong_kind_output_carries_captured_diagnostics() {
    assert_failure(Fixture::FileOutput, Duration::from_secs(10), |error| {
        matches!(
            error,
            RunError::OutputWrongKind { diagnostics, .. }
                if captured_contains(diagnostics, b"warn")
        )
    });
}

fn assert_failure(fixture: Fixture, timeout: Duration, check: impl Fn(&RunError) -> bool) {
    let root = tempfile::tempdir().unwrap();
    let target = root.path().join("published");
    let result = block_on(AsyncAcquire::acquire(fixture_process(
        fixture, "tree", timeout,
    )));
    let error = match result {
        Err(error) => error,
        Ok(output) => panic!(
            "expected failure, got success with {:?}",
            output.tree.root()
        ),
    };
    assert!(check(&error), "unexpected error: {error:?}");
    assert!(!target.exists());
}

#[test]
fn async_drop_cancellation_stops_the_admitted_tree() {
    let root = tempfile::tempdir().unwrap();
    let marker = root.path().join("descendant-marker");
    let target = root.path().join("published");

    let environment = marker_environment(&marker);

    let action = fixture_process(Fixture::SpawnsDescendant, "tree", Duration::from_secs(30))
        .with_environment(environment);

    block_on(async {
        {
            let mut future = std::pin::pin!(AsyncAcquire::acquire(action));
            let waker = noop_waker();
            let mut context = std::task::Context::from_waker(&waker);
            // First poll starts the spawn and reaches the awaited wait loop.
            let _ = future.as_mut().poll(&mut context);
            // Let the admitted tree start writing; the pinned future value then drops at the
            // end of this scope, running the Drop guard that stops the tree.
            tokio::time::sleep(Duration::from_millis(1500)).await;
        }
    });

    // The Drop guard stopped the admitted tree: the descendant marker stops growing.
    let before = std::fs::metadata(&marker)
        .map(|meta| meta.len())
        .unwrap_or(0);
    assert!(
        before > 0,
        "descendant should have written before cancellation"
    );
    std::thread::sleep(Duration::from_millis(600));
    let after = std::fs::metadata(&marker)
        .map(|meta| meta.len())
        .unwrap_or(0);
    assert_eq!(
        before, after,
        "descendant marker kept growing after the future was dropped"
    );
    assert!(!target.exists());
}

#[test]
fn async_token_cancel_stops_tree_while_future_stays_alive() {
    let root = tempfile::tempdir().unwrap();
    let marker = root.path().join("descendant-marker");
    let target = root.path().join("published");

    let environment = marker_environment(&marker);

    let action = fixture_process(Fixture::SpawnsDescendant, "tree", Duration::from_secs(30))
        .with_environment(environment);
    let token = CancelToken::new();

    let result = block_on(async {
        let mut future = std::pin::pin!(action.acquire_async_cancellable(&token));
        let waker = noop_waker();
        let mut context = std::task::Context::from_waker(&waker);
        // First poll starts the spawn and reaches the awaited wait loop.
        let _ = future.as_mut().poll(&mut context);
        // Wait until the admitted tree is demonstrably running, then cancel without dropping.
        for _ in 0..200 {
            if std::fs::metadata(&marker)
                .map(|meta| meta.len())
                .unwrap_or(0)
                > 0
            {
                token.cancel();
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        assert!(
            token.is_cancelled(),
            "descendant marker never appeared before cancellation"
        );
        future.await
    });

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

    // The token cancellation stopped the admitted tree: the descendant marker stops growing.
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
        "descendant marker kept growing after token cancellation"
    );
    assert!(!target.exists());
}

fn noop_waker() -> std::task::Waker {
    fn raw_clone(_: *const ()) -> std::task::RawWaker {
        std::task::RawWaker::new(std::ptr::null(), &VTABLE)
    }
    unsafe fn raw_noop(_: *const ()) {}
    const VTABLE: std::task::RawWakerVTable =
        std::task::RawWakerVTable::new(raw_clone, raw_noop, raw_noop, raw_noop);
    // SAFETY: the vtable functions never touch the null placeholder data pointer.
    unsafe { std::task::Waker::from_raw(std::task::RawWaker::new(std::ptr::null(), &VTABLE)) }
}
