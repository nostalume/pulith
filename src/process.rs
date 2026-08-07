//! Cooperative, caller-authorized process realization into local staged-tree custody.
//!
//! This module does not sandbox the selected program, resolve dependencies, or publish a final
//! target. It creates one private workspace, provides an explicit output path, and returns a
//! local [`StagedTree`] only after the direct child exits successfully and that output is an
//! admitted directory.
//!
//! On timeout the adapter stops the admitted process tree: the direct child plus everything it
//! spawned while inside the Unix process group or Windows Job Object. This is a best-effort stop,
//! not proof that zero descendants survive — a descendant that detaches into a new session or
//! group (Unix) or breaks away from the job (Windows) is outside the claim. No sandbox, namespace,
//! cgroup, resource-limit, or network-isolation guarantee is made.
//!
//! Standard output and error are captured to files inside the workspace and read back with a
//! caller-configurable byte cap. The cap bounds the retained memory only: during the run the child
//! may write unbounded bytes to those files, bounded only by workspace lifetime and disk.
//! Captured diagnostics are payload, not safe-facts attestation; they are never rendered in
//! [`fmt::Display`] error text and never copied into [`ProcessEvidence`].
//!
//! Declared inputs ([`InputSpec`]) are staged as copies under `inputs/<name>` inside the workspace
//! before the run, with `PULITH_INPUT_ROOT` pointing at the staged directory and
//! `PULITH_OUTPUT_ROOT` at the declared output. This is input closure, not isolation: the admitted
//! program's visible input world is exactly the declared copies, the explicit environment, and the
//! workspace, but ambient host reads are not guaranteed blocked.
//!
//! With the `process-async` feature, [`ProcessAcquire`] also implements [`AsyncAcquire`]: the same
//! realization law with a tokio-awaited wait loop. Dropping or aborting the acquire future stops
//! the admitted tree (the same tree-stop path as sync, plus a direct-child kill signal), so an
//! abandoned build does not leak a running process tree. The async adapter reuses the shared
//! platform helpers; only the orchestration is duplicated.
//!
//! Both adapters accept a caller-owned [`CancellationToken`]: once cancelled (sticky, `Send +
//! Sync`), the wait loop stops the admitted tree via the same path and returns
//! [`ProcessError::Cancelled`] — a caller stop request, never confused with a timeout. A token
//! already cancelled at entry fails fast before the program spawns.

use std::ffi::{OsStr, OsString};
use std::fmt;
use std::fs::File;
use std::io::{self, Read};
use std::path::{Component, Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

#[cfg(unix)]
use std::os::unix::process::CommandExt;
#[cfg(windows)]
use std::os::windows::process::CommandExt;

#[cfg(feature = "process-async")]
use crate::AsyncAcquire;
use crate::local::{LocalInspect, LocalObservation, StagedTree};
use crate::{Acquire, Acquired, EvidenceChain, Materialize};

const OUTPUT_ENV: &str = "PULITH_OUTPUT_ROOT";
const INPUT_ENV: &str = "PULITH_INPUT_ROOT";
const DEFAULT_CAPTURE_CAP: usize = 1024 * 1024;
#[cfg(windows)]
const CREATE_SUSPENDED: u32 = 0x0000_0004;

/// Caller-authorized execution without a host-containment or sandbox claim.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Cooperative;

/// A path contained below the process workspace output root.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkspaceRelativePath(PathBuf);

impl WorkspaceRelativePath {
    /// Admits one nonempty, normal-component-only relative output path.
    pub fn new(path: impl Into<PathBuf>) -> Result<Self, ProcessConfigError> {
        let path = path.into();
        if path.as_os_str().is_empty()
            || path
                .components()
                .any(|component| !matches!(component, Component::Normal(_)))
        {
            return Err(ProcessConfigError::InvalidWorkspaceOutput(path));
        }
        Ok(Self(path))
    }

    /// Returns the admitted relative path.
    pub fn as_path(&self) -> &Path {
        &self.0
    }
}

/// One argument passed to the selected program without shell interpolation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ActionArgument {
    Literal(OsString),
    WorkspaceRoot,
    OutputRoot,
    OutputPath(WorkspaceRelativePath),
}

/// Explicit process environment entries.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ExplicitEnvironment {
    entries: Vec<(OsString, OsString)>,
}

impl ExplicitEnvironment {
    /// Admits caller entries while reserving Pulith's output-root variable.
    pub fn new(
        entries: impl IntoIterator<Item = (OsString, OsString)>,
    ) -> Result<Self, ProcessConfigError> {
        let entries = entries.into_iter().collect::<Vec<_>>();
        for (index, (key, _)) in entries.iter().enumerate() {
            if key.is_empty() || key.to_string_lossy().contains('=') {
                return Err(ProcessConfigError::InvalidEnvironmentKey(key.clone()));
            }
            if environment_keys_equal(key, OsStr::new(OUTPUT_ENV))
                || environment_keys_equal(key, OsStr::new(INPUT_ENV))
            {
                return Err(ProcessConfigError::ReservedEnvironmentKey(key.clone()));
            }
            if entries[..index]
                .iter()
                .any(|(prior, _)| environment_keys_equal(prior, key))
            {
                return Err(ProcessConfigError::DuplicateEnvironmentKey(key.clone()));
            }
        }
        Ok(Self { entries })
    }
}

fn environment_keys_equal(left: &OsStr, right: &OsStr) -> bool {
    #[cfg(windows)]
    {
        left.to_string_lossy()
            .eq_ignore_ascii_case(&right.to_string_lossy())
    }
    #[cfg(not(windows))]
    {
        left == right
    }
}

/// Process configuration rejected before workspace creation or spawn.
#[non_exhaustive]
#[derive(Debug)]
pub enum ProcessConfigError {
    InvalidWorkspaceOutput(PathBuf),
    EmptyProgram,
    NonAbsoluteProgram(PathBuf),
    ZeroTimeout,
    InvalidEnvironmentKey(OsString),
    ReservedEnvironmentKey(OsString),
    DuplicateEnvironmentKey(OsString),
}

impl fmt::Display for ProcessConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidWorkspaceOutput(path) => write!(
                formatter,
                "workspace output must be a nonempty contained relative path: {}",
                path.display()
            ),
            Self::EmptyProgram => formatter.write_str("process program path must not be empty"),
            Self::NonAbsoluteProgram(path) => write!(
                formatter,
                "process program path must be absolute: {}",
                path.display()
            ),
            Self::ZeroTimeout => formatter.write_str("process timeout must be nonzero"),
            Self::InvalidEnvironmentKey(key) => write!(
                formatter,
                "invalid explicit environment key: {}",
                key.to_string_lossy()
            ),
            Self::ReservedEnvironmentKey(key) => write!(
                formatter,
                "environment key is reserved by Pulith: {}",
                key.to_string_lossy()
            ),
            Self::DuplicateEnvironmentKey(key) => write!(
                formatter,
                "explicit environment key appears more than once: {}",
                key.to_string_lossy()
            ),
        }
    }
}

impl std::error::Error for ProcessConfigError {}

/// One declared input file staged into the private workspace before the run.
///
/// `source` is the caller's host path; `name` is the deterministic staged name under
/// `inputs/<name>`, reachable as `$PULITH_INPUT_ROOT/<name>` or via a workspace-relative
/// argument. The file is copied, never linked, so the program's view is a snapshot: later host
/// edits do not reach the run, and the run cannot write back through the staged copy.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InputSpec {
    source: PathBuf,
    name: String,
}

impl InputSpec {
    /// Declares a source path staged under an explicit name.
    ///
    /// The name is validated at staging time: it must be a non-empty single path component.
    pub fn new(source: impl Into<PathBuf>, name: impl Into<String>) -> Self {
        Self {
            source: source.into(),
            name: name.into(),
        }
    }

    /// Declares a source path staged under its file name.
    pub fn from_path(source: impl Into<PathBuf>) -> Self {
        let source = source.into();
        let name = source
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        Self { source, name }
    }
}

/// One bounded process action that must create a directory below its private output root.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProcessAction<P> {
    program: PathBuf,
    arguments: Vec<ActionArgument>,
    environment: ExplicitEnvironment,
    inputs: Vec<InputSpec>,
    output: WorkspaceRelativePath,
    timeout: Duration,
    capture_cap: usize,
    policy: P,
}

impl ProcessAction<Cooperative> {
    /// Creates a cooperative action with explicit executable path and declared output directory.
    pub fn new(
        program: impl Into<PathBuf>,
        output: WorkspaceRelativePath,
        timeout: Duration,
    ) -> Result<Self, ProcessConfigError> {
        let program = program.into();
        if program.as_os_str().is_empty() {
            return Err(ProcessConfigError::EmptyProgram);
        }
        if !program.is_absolute() {
            return Err(ProcessConfigError::NonAbsoluteProgram(program));
        }
        if timeout.is_zero() {
            return Err(ProcessConfigError::ZeroTimeout);
        }
        Ok(Self {
            program,
            arguments: Vec::new(),
            environment: ExplicitEnvironment::default(),
            inputs: Vec::new(),
            output,
            timeout,
            capture_cap: DEFAULT_CAPTURE_CAP,
            policy: Cooperative,
        })
    }

    /// Replaces the structured program arguments.
    pub fn with_arguments(mut self, arguments: impl IntoIterator<Item = ActionArgument>) -> Self {
        self.arguments = arguments.into_iter().collect();
        self
    }

    /// Replaces the explicit environment after its reserved-key admission.
    pub fn with_environment(mut self, environment: ExplicitEnvironment) -> Self {
        self.environment = environment;
        self
    }

    /// Replaces the declared input files staged into the private workspace before the run.
    ///
    /// Each input is copied to `inputs/<name>` (never linked) with `PULITH_INPUT_ROOT` pointing
    /// at the staged directory; missing sources, collisions, and invalid names fail before the
    /// program spawns.
    pub fn with_inputs(mut self, inputs: impl IntoIterator<Item = InputSpec>) -> Self {
        self.inputs = inputs.into_iter().collect();
        self
    }

    /// Bounds each captured stream to `cap` bytes at read time.
    ///
    /// The cap bounds the retained memory only; during the run the child may write unbounded bytes
    /// to the workspace diagnostic files. `cap = 0` disables capture entirely (streams use
    /// [`Stdio::null`] and no files are created). Defaults to 1 MiB per stream.
    pub fn with_capture_cap(mut self, cap: usize) -> Self {
        self.capture_cap = cap;
        self
    }
}

/// Safe facts from a successful cooperative process realization.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProcessEvidence<P> {
    pub output: WorkspaceRelativePath,
    pub elapsed: Duration,
    _policy: std::marker::PhantomData<P>,
}

/// Capped standard-stream output captured from the admitted process action.
///
/// Diagnostics are payload, not safe-facts attestation: they are never rendered in [`fmt::Display`]
/// error text and never copied into [`ProcessEvidence`]. Each stream is `None` when capture was
/// disabled (`cap = 0`) or the workspace diagnostic file could not be read back.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProcessDiagnostics {
    /// First `cap` bytes of standard output, or `None` when unavailable or disabled.
    pub stdout: Option<Vec<u8>>,
    /// First `cap` bytes of standard error, or `None` when unavailable or disabled.
    pub stderr: Option<Vec<u8>>,
    /// `true` when standard output exceeded `cap` and was truncated.
    pub stdout_truncated: bool,
    /// `true` when standard error exceeded `cap` and was truncated.
    pub stderr_truncated: bool,
    /// The per-stream byte bound applied at read time.
    pub cap: usize,
}

impl ProcessDiagnostics {
    fn disabled() -> Self {
        Self {
            stdout: None,
            stderr: None,
            stdout_truncated: false,
            stderr_truncated: false,
            cap: 0,
        }
    }
}

/// Caller-owned cancellation signal for one process action.
///
/// The token is sticky (once cancelled it stays cancelled), `Send + Sync`, and carries no data
/// beyond the cancelled bit. [`ProcessAcquire::acquire_with_cancel`] polls it once per wait-loop
/// tick and stops the admitted tree via the frozen tree-stop path; a token already cancelled at
/// entry fails fast before the program spawns. Cancellation is the caller's explicit stop request
/// and is never confused with a timeout: it surfaces as [`ProcessError::Cancelled`].
#[derive(Clone, Default)]
pub struct CancellationToken {
    cancelled: Arc<AtomicBool>,
}

impl CancellationToken {
    /// Creates an uncancelled token.
    pub fn new() -> Self {
        Self::default()
    }

    /// Marks the token cancelled; sticky and safe to call from any thread.
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }

    /// Returns whether the token has been cancelled.
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }
}

/// Failure before a staged output tree could be returned.
#[non_exhaustive]
#[derive(Debug)]
pub enum ProcessError {
    Workspace {
        source: io::Error,
    },
    InputMissing {
        path: PathBuf,
    },
    InputCollision {
        name: String,
    },
    InputStaging {
        path: PathBuf,
        source: io::Error,
    },
    Spawn {
        program: PathBuf,
        source: io::Error,
        diagnostics: Box<ProcessDiagnostics>,
    },
    Wait {
        program: PathBuf,
        source: io::Error,
        diagnostics: Box<ProcessDiagnostics>,
    },
    TimedOut {
        program: PathBuf,
        timeout: Duration,
        diagnostics: Box<ProcessDiagnostics>,
    },
    Cancelled {
        program: PathBuf,
        diagnostics: Box<ProcessDiagnostics>,
    },
    ExitedNonZero {
        program: PathBuf,
        status: ExitStatus,
        diagnostics: Box<ProcessDiagnostics>,
    },
    OutputMissing {
        path: PathBuf,
        diagnostics: Box<ProcessDiagnostics>,
    },
    OutputWrongKind {
        path: PathBuf,
        observed: LocalObservation,
        diagnostics: Box<ProcessDiagnostics>,
    },
    OutputInspect {
        path: PathBuf,
        source: crate::local::LocalError,
        diagnostics: Box<ProcessDiagnostics>,
    },
    #[cfg(windows)]
    CapabilityUnavailable {
        program: PathBuf,
        source: io::Error,
        diagnostics: Box<ProcessDiagnostics>,
    },
    WorkspaceCleanup {
        primary: Box<ProcessError>,
        cleanup: io::Error,
    },
}

impl fmt::Display for ProcessError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Captured diagnostics are never rendered here; they may contain program output.
        match self {
            Self::Workspace { source } => {
                write!(formatter, "failed to create process workspace: {source}")
            }
            Self::InputMissing { path } => write!(
                formatter,
                "declared process input is missing or not a readable file: {}",
                path.display()
            ),
            Self::InputCollision { name } => {
                write!(
                    formatter,
                    "declared process inputs collide on staged name {name:?}"
                )
            }
            Self::InputStaging { path, source } => write!(
                formatter,
                "failed to stage process input {}: {source}",
                path.display()
            ),
            Self::Spawn {
                program, source, ..
            } => write!(
                formatter,
                "failed to spawn process {}: {source}",
                program.display()
            ),
            Self::Wait {
                program, source, ..
            } => write!(
                formatter,
                "failed while waiting for process {}: {source}",
                program.display()
            ),
            Self::TimedOut {
                program, timeout, ..
            } => write!(
                formatter,
                "process {} exceeded timeout {timeout:?}",
                program.display()
            ),
            Self::Cancelled { program, .. } => {
                write!(
                    formatter,
                    "process {} was cancelled by the caller",
                    program.display()
                )
            }
            Self::ExitedNonZero {
                program, status, ..
            } => write!(
                formatter,
                "process {} exited unsuccessfully: {status}",
                program.display()
            ),
            Self::OutputMissing { path, .. } => write!(
                formatter,
                "process did not create declared output directory: {}",
                path.display()
            ),
            Self::OutputWrongKind { path, observed, .. } => write!(
                formatter,
                "process output is not an admitted directory {}: {observed:?}",
                path.display()
            ),
            Self::OutputInspect { path, source, .. } => write!(
                formatter,
                "failed to inspect process output {}: {source}",
                path.display()
            ),
            #[cfg(windows)]
            Self::CapabilityUnavailable {
                program, source, ..
            } => write!(
                formatter,
                "process-tree capability unavailable for {}: {source}",
                program.display()
            ),
            Self::WorkspaceCleanup { primary, cleanup } => write!(
                formatter,
                "{primary}; workspace cleanup also failed: {cleanup}"
            ),
        }
    }
}

impl std::error::Error for ProcessError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Workspace { source }
            | Self::Spawn { source, .. }
            | Self::Wait { source, .. }
            | Self::InputStaging { source, .. } => Some(source),
            Self::OutputInspect { source, .. } => Some(source),
            #[cfg(windows)]
            Self::CapabilityUnavailable { source, .. } => Some(source),
            Self::WorkspaceCleanup { primary, .. } => Some(primary.as_ref()),
            _ => None,
        }
    }
}

type ProcessAcquired<I> = Acquired<
    Materialize<I, ProcessAction<Cooperative>, PathBuf>,
    StagedTree,
    EvidenceChain<ProcessEvidence<Cooperative>, ProcessDiagnostics>,
>;

/// Adapter that realizes one cooperative action into local staged-tree custody.
#[derive(Clone, Copy, Debug, Default)]
pub struct ProcessAcquire<P> {
    _policy: std::marker::PhantomData<P>,
}

impl ProcessAcquire<Cooperative> {
    /// Creates the cooperative process-realization adapter.
    pub fn new() -> Self {
        Self::default()
    }
}

impl<I> Acquire<Materialize<I, ProcessAction<Cooperative>, PathBuf>>
    for ProcessAcquire<Cooperative>
{
    type Error = ProcessError;
    type Output = Acquired<
        Materialize<I, ProcessAction<Cooperative>, PathBuf>,
        StagedTree,
        EvidenceChain<ProcessEvidence<Cooperative>, ProcessDiagnostics>,
    >;

    fn acquire(
        &self,
        input: Materialize<I, ProcessAction<Cooperative>, PathBuf>,
    ) -> Result<Self::Output, Self::Error> {
        acquire_process(input, None)
    }
}

impl ProcessAcquire<Cooperative> {
    /// Runs one cooperative action to staged-tree custody, stopping the admitted tree when the
    /// caller's token is set (sticky; polled once per wait-loop tick).
    ///
    /// Prefer the trait's [`Acquire::acquire`] for token-free calls; this inherent entry exists so
    /// the caller can stop a long realization without waiting for the timeout. Cancellation
    /// reuses the frozen tree-stop path and surfaces as [`ProcessError::Cancelled`], never as
    /// [`ProcessError::TimedOut`].
    pub fn acquire_with_cancel<I>(
        &self,
        input: Materialize<I, ProcessAction<Cooperative>, PathBuf>,
        cancel: &CancellationToken,
    ) -> Result<ProcessAcquired<I>, ProcessError> {
        acquire_process(input, Some(cancel))
    }
}

fn acquire_process<I>(
    input: Materialize<I, ProcessAction<Cooperative>, PathBuf>,
    cancel: Option<&CancellationToken>,
) -> Result<ProcessAcquired<I>, ProcessError> {
    if cancel.is_some_and(CancellationToken::is_cancelled) {
        return Err(ProcessError::Cancelled {
            program: input.source.program.clone(),
            diagnostics: Box::new(ProcessDiagnostics::disabled()),
        });
    }
    let workspace = tempfile::Builder::new()
        .prefix(".pulith-process-")
        .tempdir()
        .map_err(|source| ProcessError::Workspace { source })?;
    let output_base = workspace.path().join("output");
    if let Err(source) = std::fs::create_dir(&output_base) {
        return fail_cleanup(workspace, ProcessError::Workspace { source });
    }
    let selected_output = output_base.join(input.source.output.as_path());
    let input_root = workspace.path().join("inputs");
    let capture_cap = input.source.capture_cap;
    let started = Instant::now();

    if let Err(error) = stage_inputs(workspace.path(), &input.source.inputs) {
        return fail_cleanup(workspace, error);
    }

    let stdout_path = workspace.path().join("stdout.log");
    let stderr_path = workspace.path().join("stderr.log");

    let mut command = Command::new(&input.source.program);
    command
        .current_dir(workspace.path())
        .env_clear()
        .envs(
            input
                .source
                .environment
                .entries
                .iter()
                .map(|(key, value)| (key, value)),
        )
        .env(OUTPUT_ENV, &selected_output)
        .stdin(Stdio::null());
    if !input.source.inputs.is_empty() {
        command.env(INPUT_ENV, &input_root);
    }
    if capture_cap > 0 {
        match (File::create(&stdout_path), File::create(&stderr_path)) {
            (Ok(stdout), Ok(stderr)) => {
                command.stdout(stdout).stderr(stderr);
            }
            (stdout, stderr) => {
                let source = match (stdout, stderr) {
                    (Err(source), _) | (_, Err(source)) => source,
                    _ => unreachable!(),
                };
                let diagnostics = read_diagnostics(workspace.path(), capture_cap);
                return fail_cleanup(
                    workspace,
                    ProcessError::Spawn {
                        program: input.source.program.clone(),
                        source,
                        diagnostics: Box::new(diagnostics),
                    },
                );
            }
        }
    } else {
        command.stdout(Stdio::null()).stderr(Stdio::null());
    }
    for argument in &input.source.arguments {
        command.arg(resolve_argument(
            argument,
            workspace.path(),
            &selected_output,
        ));
    }
    #[cfg(unix)]
    command.process_group(0);
    #[cfg(windows)]
    command.creation_flags(CREATE_SUSPENDED);

    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(source) => {
            let diagnostics = read_diagnostics(workspace.path(), capture_cap);
            return fail_cleanup(
                workspace,
                ProcessError::Spawn {
                    program: input.source.program.clone(),
                    source,
                    diagnostics: Box::new(diagnostics),
                },
            );
        }
    };

    #[cfg(windows)]
    let job = match assign_to_job(child.id()) {
        Ok(job) => job,
        Err(source) => {
            let _ = child.kill();
            let _ = child.wait();
            let diagnostics = read_diagnostics(workspace.path(), capture_cap);
            return fail_cleanup(
                workspace,
                ProcessError::CapabilityUnavailable {
                    program: input.source.program.clone(),
                    source,
                    diagnostics: Box::new(diagnostics),
                },
            );
        }
    };

    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if started.elapsed() >= input.source.timeout => {
                #[cfg(unix)]
                stop_tree(child.id() as i32);
                #[cfg(windows)]
                stop_tree(&job);
                let _ = child.wait();
                let diagnostics = read_diagnostics(workspace.path(), capture_cap);
                return fail_cleanup(
                    workspace,
                    ProcessError::TimedOut {
                        program: input.source.program.clone(),
                        timeout: input.source.timeout,
                        diagnostics: Box::new(diagnostics),
                    },
                );
            }
            Ok(None) if cancel.is_some_and(CancellationToken::is_cancelled) => {
                #[cfg(unix)]
                stop_tree(child.id() as i32);
                #[cfg(windows)]
                stop_tree(&job);
                let _ = child.wait();
                let diagnostics = read_diagnostics(workspace.path(), capture_cap);
                return fail_cleanup(
                    workspace,
                    ProcessError::Cancelled {
                        program: input.source.program.clone(),
                        diagnostics: Box::new(diagnostics),
                    },
                );
            }
            Ok(None) => thread::sleep(Duration::from_millis(10)),
            Err(source) => {
                let diagnostics = read_diagnostics(workspace.path(), capture_cap);
                return fail_cleanup(
                    workspace,
                    ProcessError::Wait {
                        program: input.source.program.clone(),
                        source,
                        diagnostics: Box::new(diagnostics),
                    },
                );
            }
        }
    };
    if !status.success() {
        let diagnostics = read_diagnostics(workspace.path(), capture_cap);
        return fail_cleanup(
            workspace,
            ProcessError::ExitedNonZero {
                program: input.source.program.clone(),
                status,
                diagnostics: Box::new(diagnostics),
            },
        );
    }
    let diagnostics = read_diagnostics(workspace.path(), capture_cap);
    let observation = match LocalInspect.inspect((&selected_output).into()) {
        Ok(inspected) => inspected.observation,
        Err(source) => {
            return fail_cleanup(
                workspace,
                ProcessError::OutputInspect {
                    path: selected_output,
                    source,
                    diagnostics: Box::new(diagnostics),
                },
            );
        }
    };
    if observation == LocalObservation::Missing {
        return fail_cleanup(
            workspace,
            ProcessError::OutputMissing {
                path: selected_output,
                diagnostics: Box::new(diagnostics),
            },
        );
    }
    if observation != LocalObservation::Directory {
        return fail_cleanup(
            workspace,
            ProcessError::OutputWrongKind {
                path: selected_output,
                observed: observation,
                diagnostics: Box::new(diagnostics),
            },
        );
    }
    let evidence = ProcessEvidence {
        output: input.source.output.clone(),
        elapsed: started.elapsed(),
        _policy: std::marker::PhantomData,
    };
    Ok(Acquired {
        input,
        material: StagedTree::new(workspace, selected_output),
        evidence: EvidenceChain {
            previous: evidence,
            current: diagnostics,
        },
    })
}

fn stage_inputs(workspace: &Path, inputs: &[InputSpec]) -> Result<(), ProcessError> {
    if inputs.is_empty() {
        return Ok(());
    }
    let input_root = workspace.join("inputs");
    std::fs::create_dir(&input_root).map_err(|source| ProcessError::InputStaging {
        path: input_root.clone(),
        source,
    })?;
    let mut seen = std::collections::HashSet::new();
    for spec in inputs {
        // Component law: the staged name must be a single non-empty path component, so it can
        // never escape the workspace (rejects separators, `.`, `..`, and empty names).
        let mut components = Path::new(&spec.name).components();
        let valid = matches!(
            (components.next(), components.next()),
            (Some(Component::Normal(part)), None) if !part.is_empty()
        );
        if !valid || !seen.insert(spec.name.clone()) {
            return Err(ProcessError::InputCollision {
                name: spec.name.clone(),
            });
        }
        if !spec.source.is_file() {
            return Err(ProcessError::InputMissing {
                path: spec.source.clone(),
            });
        }
        let destination = input_root.join(&spec.name);
        if let Err(source) = std::fs::copy(&spec.source, &destination) {
            return Err(ProcessError::InputStaging {
                path: destination,
                source,
            });
        }
    }
    Ok(())
}

#[cfg(feature = "process-async")]
impl<I> AsyncAcquire<Materialize<I, ProcessAction<Cooperative>, PathBuf>>
    for ProcessAcquire<Cooperative>
{
    type Error = ProcessError;
    type Output = Acquired<
        Materialize<I, ProcessAction<Cooperative>, PathBuf>,
        StagedTree,
        EvidenceChain<ProcessEvidence<Cooperative>, ProcessDiagnostics>,
    >;

    #[allow(
        clippy::manual_async_fn,
        reason = "async fn cannot express the trait's explicit `'a` + `where N: 'a` bounds on the returned future"
    )]
    fn acquire<'a>(
        &'a self,
        input: Materialize<I, ProcessAction<Cooperative>, PathBuf>,
    ) -> impl std::future::Future<Output = Result<Self::Output, Self::Error>> + 'a
    where
        Materialize<I, ProcessAction<Cooperative>, PathBuf>: 'a,
    {
        async move { acquire_process_async(input, None).await }
    }
}

#[cfg(feature = "process-async")]
impl ProcessAcquire<Cooperative> {
    /// Awaitable token-cancellable entry, mirroring [`ProcessAcquire::acquire_with_cancel`] for
    /// the async path: the wait loop polls the same token, so the caller can cancel while keeping
    /// the future alive to await the outcome. Dropping the future still stops the tree.
    pub async fn acquire_with_token<I>(
        &self,
        input: Materialize<I, ProcessAction<Cooperative>, PathBuf>,
        cancel: &CancellationToken,
    ) -> Result<ProcessAcquired<I>, ProcessError> {
        acquire_process_async(input, Some(cancel)).await
    }
}

#[cfg(feature = "process-async")]
async fn acquire_process_async<I>(
    input: Materialize<I, ProcessAction<Cooperative>, PathBuf>,
    cancel: Option<&CancellationToken>,
) -> Result<ProcessAcquired<I>, ProcessError> {
    if cancel.is_some_and(CancellationToken::is_cancelled) {
        return Err(ProcessError::Cancelled {
            program: input.source.program.clone(),
            diagnostics: Box::new(ProcessDiagnostics::disabled()),
        });
    }
    let workspace = tempfile::Builder::new()
        .prefix(".pulith-process-")
        .tempdir()
        .map_err(|source| ProcessError::Workspace { source })?;
    let output_base = workspace.path().join("output");
    if let Err(source) = std::fs::create_dir(&output_base) {
        return fail_cleanup(workspace, ProcessError::Workspace { source });
    }
    let selected_output = output_base.join(input.source.output.as_path());
    let input_root = workspace.path().join("inputs");
    let capture_cap = input.source.capture_cap;
    let started = Instant::now();

    if let Err(error) = stage_inputs(workspace.path(), &input.source.inputs) {
        return fail_cleanup(workspace, error);
    }

    let stdout_path = workspace.path().join("stdout.log");
    let stderr_path = workspace.path().join("stderr.log");

    let mut command = tokio::process::Command::new(&input.source.program);
    command
        .current_dir(workspace.path())
        .env_clear()
        .envs(
            input
                .source
                .environment
                .entries
                .iter()
                .map(|(key, value)| (key, value)),
        )
        .env(OUTPUT_ENV, &selected_output)
        .stdin(Stdio::null());
    if !input.source.inputs.is_empty() {
        command.env(INPUT_ENV, &input_root);
    }
    if capture_cap > 0 {
        match (File::create(&stdout_path), File::create(&stderr_path)) {
            (Ok(stdout), Ok(stderr)) => {
                command.stdout(stdout).stderr(stderr);
            }
            (stdout, stderr) => {
                let source = match (stdout, stderr) {
                    (Err(source), _) | (_, Err(source)) => source,
                    _ => unreachable!(),
                };
                let diagnostics = read_diagnostics(workspace.path(), capture_cap);
                return fail_cleanup(
                    workspace,
                    ProcessError::Spawn {
                        program: input.source.program.clone(),
                        source,
                        diagnostics: Box::new(diagnostics),
                    },
                );
            }
        }
    } else {
        command.stdout(Stdio::null()).stderr(Stdio::null());
    }
    for argument in &input.source.arguments {
        command.arg(resolve_argument(
            argument,
            workspace.path(),
            &selected_output,
        ));
    }
    #[cfg(unix)]
    command.process_group(0);
    #[cfg(windows)]
    command.creation_flags(CREATE_SUSPENDED);

    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(source) => {
            let diagnostics = read_diagnostics(workspace.path(), capture_cap);
            return fail_cleanup(
                workspace,
                ProcessError::Spawn {
                    program: input.source.program.clone(),
                    source,
                    diagnostics: Box::new(diagnostics),
                },
            );
        }
    };

    #[cfg(windows)]
    let job = match assign_to_job(
        child
            .id()
            .expect("a freshly spawned child always has a process id"),
    ) {
        Ok(job) => job,
        Err(source) => {
            let _ = child.start_kill();
            let _ = child.wait().await;
            let diagnostics = read_diagnostics(workspace.path(), capture_cap);
            return fail_cleanup(
                workspace,
                ProcessError::CapabilityUnavailable {
                    program: input.source.program.clone(),
                    source,
                    diagnostics: Box::new(diagnostics),
                },
            );
        }
    };

    let mut guard = AsyncTreeGuard::new(
        child,
        #[cfg(windows)]
        job,
    );

    let status = loop {
        match guard.child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if started.elapsed() >= input.source.timeout => {
                guard.stop();
                let _ = guard.child.wait().await;
                let diagnostics = read_diagnostics(workspace.path(), capture_cap);
                return fail_cleanup(
                    workspace,
                    ProcessError::TimedOut {
                        program: input.source.program.clone(),
                        timeout: input.source.timeout,
                        diagnostics: Box::new(diagnostics),
                    },
                );
            }
            Ok(None) if cancel.is_some_and(CancellationToken::is_cancelled) => {
                guard.stop();
                let _ = guard.child.wait().await;
                let diagnostics = read_diagnostics(workspace.path(), capture_cap);
                return fail_cleanup(
                    workspace,
                    ProcessError::Cancelled {
                        program: input.source.program.clone(),
                        diagnostics: Box::new(diagnostics),
                    },
                );
            }
            Ok(None) => tokio::time::sleep(Duration::from_millis(10)).await,
            Err(source) => {
                let diagnostics = read_diagnostics(workspace.path(), capture_cap);
                return fail_cleanup(
                    workspace,
                    ProcessError::Wait {
                        program: input.source.program.clone(),
                        source,
                        diagnostics: Box::new(diagnostics),
                    },
                );
            }
        }
    };
    if !status.success() {
        guard.disarm();
        let diagnostics = read_diagnostics(workspace.path(), capture_cap);
        return fail_cleanup(
            workspace,
            ProcessError::ExitedNonZero {
                program: input.source.program.clone(),
                status,
                diagnostics: Box::new(diagnostics),
            },
        );
    }
    guard.disarm();
    let diagnostics = read_diagnostics(workspace.path(), capture_cap);
    let observation = match LocalInspect.inspect((&selected_output).into()) {
        Ok(inspected) => inspected.observation,
        Err(source) => {
            return fail_cleanup(
                workspace,
                ProcessError::OutputInspect {
                    path: selected_output,
                    source,
                    diagnostics: Box::new(diagnostics),
                },
            );
        }
    };
    if observation == LocalObservation::Missing {
        return fail_cleanup(
            workspace,
            ProcessError::OutputMissing {
                path: selected_output,
                diagnostics: Box::new(diagnostics),
            },
        );
    }
    if observation != LocalObservation::Directory {
        return fail_cleanup(
            workspace,
            ProcessError::OutputWrongKind {
                path: selected_output,
                observed: observation,
                diagnostics: Box::new(diagnostics),
            },
        );
    }
    let evidence = ProcessEvidence {
        output: input.source.output.clone(),
        elapsed: started.elapsed(),
        _policy: std::marker::PhantomData,
    };
    drop(guard);
    Ok(Acquired {
        input,
        material: StagedTree::new(workspace, selected_output),
        evidence: EvidenceChain {
            previous: evidence,
            current: diagnostics,
        },
    })
}

/// Stops the admitted tree when the async acquire future is dropped or aborted.
///
/// The guard is armed only while the wait loop is running. A future dropped or aborted then runs
/// the frozen S2.7 tree-stop path (group kill / `TerminateJobObject` plus a direct-child kill
/// signal) with tokio reaping the direct child; best-effort per the S2.7-D4 boundary, no
/// zero-survivor claim. Once the child has produced an exit status the guard is disarmed, so
/// normal completion leaves surviving descendants running exactly like the sync adapter.
#[cfg(feature = "process-async")]
struct AsyncTreeGuard {
    child: tokio::process::Child,
    #[cfg(unix)]
    pid: u32,
    armed: bool,
    #[cfg(windows)]
    job: JobHandle,
}

#[cfg(feature = "process-async")]
impl AsyncTreeGuard {
    fn new(child: tokio::process::Child, #[cfg(windows)] job: JobHandle) -> Self {
        #[cfg(unix)]
        let pid = child
            .id()
            .expect("a freshly spawned child always has a process id");
        Self {
            child,
            #[cfg(unix)]
            pid,
            armed: true,
            #[cfg(windows)]
            job,
        }
    }

    fn stop(&mut self) {
        #[cfg(unix)]
        stop_tree(self.pid as i32);
        #[cfg(windows)]
        stop_tree(&self.job);
        let _ = self.child.start_kill();
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

#[cfg(feature = "process-async")]
impl Drop for AsyncTreeGuard {
    fn drop(&mut self) {
        if self.armed {
            self.stop();
        }
    }
}

fn resolve_argument(argument: &ActionArgument, workspace: &Path, output: &Path) -> OsString {
    match argument {
        ActionArgument::Literal(argument) => argument.clone(),
        ActionArgument::WorkspaceRoot => workspace.as_os_str().to_os_string(),
        ActionArgument::OutputRoot => output.as_os_str().to_os_string(),
        ActionArgument::OutputPath(relative) => output.join(relative.as_path()).into_os_string(),
    }
}

/// Stops the admitted process tree: the direct child and everything spawned inside its group.
#[cfg(unix)]
fn stop_tree(pgid: i32) {
    if let Some(group) = rustix::process::Pid::from_raw(pgid) {
        let _ = rustix::process::kill_process_group(group, rustix::process::Signal::KILL);
    }
}

#[cfg(windows)]
struct JobHandle(windows_sys::Win32::Foundation::HANDLE);

// SAFETY: a Windows HANDLE is a kernel object reference usable and closable from any thread;
// only Drop ordering matters, and the guard drops wherever the owning future is dropped.
#[cfg(windows)]
unsafe impl Send for JobHandle {}

#[cfg(windows)]
impl Drop for JobHandle {
    fn drop(&mut self) {
        unsafe {
            windows_sys::Win32::Foundation::CloseHandle(self.0);
        }
    }
}

/// Stops the admitted process tree by terminating the Job Object.
#[cfg(windows)]
fn stop_tree(job: &JobHandle) {
    unsafe {
        windows_sys::Win32::System::JobObjects::TerminateJobObject(job.0, 1);
    }
}

/// Spawns the child suspended, assigns it to a fresh Job Object, then resumes its main thread.
///
/// Assignment failure (for example the caller already runs inside a non-breakaway job) is surfaced
/// as a capability error; the adapter never silently falls back to direct-child-only termination.
#[cfg(windows)]
fn assign_to_job(pid: u32) -> io::Result<JobHandle> {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, TH32CS_SNAPTHREAD, THREADENTRY32, Thread32First, Thread32Next,
    };
    use windows_sys::Win32::System::JobObjects::{AssignProcessToJobObject, CreateJobObjectW};
    use windows_sys::Win32::System::Threading::{
        OpenProcess, OpenThread, PROCESS_SET_QUOTA, PROCESS_TERMINATE, ResumeThread,
        THREAD_SUSPEND_RESUME,
    };

    unsafe {
        let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
        if job.is_null() {
            return Err(io::Error::last_os_error());
        }
        let job = JobHandle(job);
        let process = OpenProcess(PROCESS_SET_QUOTA | PROCESS_TERMINATE, 0, pid);
        if process.is_null() {
            return Err(io::Error::last_os_error());
        }
        let assigned = AssignProcessToJobObject(job.0, process);
        CloseHandle(process);
        if assigned == 0 {
            return Err(io::Error::last_os_error());
        }
        // The child was spawned suspended; resume its main thread so user code starts only after
        // the job assignment above.
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0);
        if snapshot.is_null() {
            return Err(io::Error::last_os_error());
        }
        let mut entry = THREADENTRY32 {
            dwSize: std::mem::size_of::<THREADENTRY32>() as u32,
            cntUsage: 0,
            th32ThreadID: 0,
            th32OwnerProcessID: 0,
            tpBasePri: 0,
            tpDeltaPri: 0,
            dwFlags: 0,
        };
        let mut resumed = false;
        if Thread32First(snapshot, &mut entry) != 0 {
            loop {
                if entry.th32OwnerProcessID == pid {
                    let thread = OpenThread(THREAD_SUSPEND_RESUME, 0, entry.th32ThreadID);
                    if !thread.is_null() {
                        ResumeThread(thread);
                        CloseHandle(thread);
                        resumed = true;
                        break;
                    }
                }
                if Thread32Next(snapshot, &mut entry) == 0 {
                    break;
                }
            }
        }
        CloseHandle(snapshot);
        if !resumed {
            return Err(io::Error::other(
                "suspended process has no resumable main thread",
            ));
        }
        Ok(job)
    }
}

/// Reads the workspace diagnostic files under the per-stream cap.
fn read_diagnostics(workspace: &Path, cap: usize) -> ProcessDiagnostics {
    if cap == 0 {
        return ProcessDiagnostics::disabled();
    }
    let (stdout, stdout_truncated) = read_capped(&workspace.join("stdout.log"), cap);
    let (stderr, stderr_truncated) = read_capped(&workspace.join("stderr.log"), cap);
    ProcessDiagnostics {
        stdout,
        stderr,
        stdout_truncated,
        stderr_truncated,
        cap,
    }
}

/// Reads at most `cap` bytes from one diagnostic file.
///
/// A read failure yields `(None, false)`: capture is best-effort after the outcome decision and
/// must never turn a successful acquisition into a failure.
fn read_capped(path: &Path, cap: usize) -> (Option<Vec<u8>>, bool) {
    let mut file = match File::open(path) {
        Ok(file) => file,
        Err(_) => return (None, false),
    };
    let length = file.metadata().map(|meta| meta.len() as usize).unwrap_or(0);
    let truncated = length > cap;
    let mut buffer = vec![0u8; length.min(cap)];
    let read = file.read(&mut buffer).unwrap_or(0);
    buffer.truncate(read);
    (Some(buffer), truncated)
}

fn fail_cleanup<T>(workspace: tempfile::TempDir, primary: ProcessError) -> Result<T, ProcessError> {
    match workspace.close() {
        Ok(()) => Err(primary),
        Err(error) => Err(ProcessError::WorkspaceCleanup {
            primary: Box::new(primary),
            cleanup: error,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;

    fn absolute_program() -> PathBuf {
        std::env::current_exe().expect("the test binary has a path")
    }

    #[test]
    fn workspace_relative_path_rejects_parent_traversal() {
        assert!(WorkspaceRelativePath::new("../outside").is_err());
        assert!(WorkspaceRelativePath::new("tree").is_ok());
    }

    #[test]
    fn explicit_environment_reserves_pulith_keys_and_rejects_duplicates() {
        assert!(
            ExplicitEnvironment::new([(
                OsString::from("PULITH_OUTPUT_ROOT"),
                OsString::from("outside"),
            )])
            .is_err()
        );
        assert!(
            ExplicitEnvironment::new([(
                OsString::from("PULITH_INPUT_ROOT"),
                OsString::from("outside"),
            )])
            .is_err()
        );
        assert!(
            ExplicitEnvironment::new([
                (OsString::from("PULITH_TEST"), OsString::from("one")),
                (OsString::from("PULITH_TEST"), OsString::from("two")),
            ])
            .is_err()
        );
    }

    #[cfg(windows)]
    #[test]
    fn explicit_environment_reserves_keys_case_insensitively() {
        assert!(
            ExplicitEnvironment::new([(
                OsString::from("pulith_output_root"),
                OsString::from("outside"),
            )])
            .is_err()
        );
    }

    #[test]
    fn cooperative_action_rejects_relative_program_and_zero_timeout() {
        let output = WorkspaceRelativePath::new("tree").unwrap();
        assert!(
            ProcessAction::new("relative-program", output.clone(), Duration::from_secs(1)).is_err()
        );
        assert!(ProcessAction::new(absolute_program(), output, Duration::ZERO).is_err());
    }
}
