//! Caller-authorized process realization into local staged-tree custody.
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
//! Standard output and error are captured with a caller-configurable byte cap. The synchronous
//! adapter drains workspace files after exit; the Tokio adapter concurrently drains pipes while
//! retaining only the cap, so a noisy child cannot fill a pipe or grow retained memory without
//! bound.
//! Captured diagnostics are payload, not safe-facts attestation; they are never rendered in
//! [`fmt::Display`] error text and never copied into [`OutputEvidence`].
//!
//! Declared inputs ([`StagedInput`]) are staged as copies under `inputs/<name>` inside the workspace
//! before the run, with `PULITH_INPUT_ROOT` pointing at the staged directory and
//! `PULITH_OUTPUT_ROOT` at the declared output. This is input closure, not isolation: the admitted
//! program's visible input world is exactly the declared copies, the explicit environment, and the
//! workspace, but ambient host reads are not guaranteed blocked.
//!
//! With the `process-tokio` feature, [`PreparedProcess`] also implements [`AsyncAcquire`]: the same
//! realization law with a tokio-awaited wait loop. Dropping or aborting the acquire future stops
//! the admitted tree (the same tree-stop path as sync, plus a direct-child kill signal), so an
//! abandoned build does not leak a running process tree. The async entry reuses the shared
//! platform helpers; only the orchestration is duplicated.
//!
//! Both the sync and async entries accept a caller-owned [`CancelToken`]: once cancelled (sticky, `Send +
//! Sync`), the wait loop stops the admitted tree via the same path and returns
//! [`RunError::Cancelled`] — a caller stop request, never confused with a timeout. A token
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

#[cfg(feature = "process-tokio")]
use crate::AsyncAcquire;
use crate::local::{LocalObservation, LocalTarget, StagedTree};
use crate::{Acquire, Inspect};

const OUTPUT_ENV: &str = "PULITH_OUTPUT_ROOT";
const INPUT_ENV: &str = "PULITH_INPUT_ROOT";
const DEFAULT_CAPTURE_CAP: usize = 1024 * 1024;
#[cfg(windows)]
const CREATE_SUSPENDED: u32 = 0x0000_0004;

/// One long-lived child process whose complete admitted tree remains under session custody.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ManagedProcess {
    program: PathBuf,
    arguments: Vec<OsString>,
    working_dir: PathBuf,
}

/// Exclusive custody of one running managed process tree.
pub struct ProcessSession {
    child: std::process::Child,
    #[cfg(windows)]
    job: JobHandle,
    program: PathBuf,
    started: Instant,
    resolved: bool,
}

/// One non-consuming observation of a managed process.
#[derive(Debug)]
pub enum ProcessObservation {
    /// The running outcome.
    Running,
    /// The exited outcome.
    Exited(ExitStatus),
}

/// The terminal fact produced by waiting for or explicitly stopping a managed process.
#[derive(Debug)]
pub enum ProcessEnd {
    /// The exited outcome.
    Exited {
        /// The status value.
        status: ExitStatus,
        /// The elapsed value.
        elapsed: Duration,
    },
    /// The stopped outcome.
    Stopped {
        /// The elapsed value.
        elapsed: Duration,
    },
}

/// A managed process could not be started, observed, or stopped within its caller deadline.
#[derive(Debug)]
pub enum SessionError {
    /// The spawn outcome.
    Spawn {
        /// The program value.
        program: PathBuf,
        /// The source value.
        source: io::Error,
    },
    /// The wait outcome.
    Wait {
        /// The program value.
        program: PathBuf,
        /// The source value.
        source: io::Error,
    },
    /// The stop timed out outcome.
    StopTimedOut {
        /// The program value.
        program: PathBuf,
        /// The deadline value.
        deadline: Duration,
    },
    #[cfg(windows)]
    /// The capability unavailable outcome.
    CapabilityUnavailable {
        /// The program value.
        program: PathBuf,
        /// The source value.
        source: io::Error,
    },
}

impl fmt::Display for SessionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Spawn { program, source } => {
                write!(
                    formatter,
                    "failed to spawn managed process {}: {source}",
                    program.display()
                )
            }
            Self::Wait { program, source } => {
                write!(
                    formatter,
                    "failed to observe managed process {}: {source}",
                    program.display()
                )
            }
            Self::StopTimedOut { program, deadline } => write!(
                formatter,
                "managed process {} did not stop within {deadline:?}",
                program.display()
            ),
            #[cfg(windows)]
            Self::CapabilityUnavailable { program, source } => write!(
                formatter,
                "managed process-tree capability unavailable for {}: {source}",
                program.display()
            ),
        }
    }
}

impl std::error::Error for SessionError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Spawn { source, .. } | Self::Wait { source, .. } => Some(source),
            #[cfg(windows)]
            Self::CapabilityUnavailable { source, .. } => Some(source),
            Self::StopTimedOut { .. } => None,
        }
    }
}

impl ManagedProcess {
    /// Admits an absolute executable and absolute working directory.
    pub fn new(
        program: impl Into<PathBuf>,
        working_dir: impl Into<PathBuf>,
    ) -> Result<Self, ConfigError> {
        let program = program.into();
        let working_dir = working_dir.into();
        admit_program(&program)?;
        if !working_dir.is_absolute() {
            return Err(ConfigError::NonAbsoluteWorktree(working_dir));
        }
        Ok(Self {
            program,
            arguments: Vec::new(),
            working_dir,
        })
    }

    /// Replaces literal arguments without shell interpolation.
    pub fn with_arguments(mut self, arguments: impl IntoIterator<Item = OsString>) -> Self {
        self.arguments = arguments.into_iter().collect();
        self
    }

    /// Starts with the caller environment inherited unchanged.
    pub fn start(self) -> Result<ProcessSession, SessionError> {
        let mut command = self.command();
        self.spawn(&mut command)
    }

    /// Starts with only the admitted environment entries.
    pub fn start_in_environment(
        self,
        environment: EnvVars,
    ) -> Result<ProcessSession, SessionError> {
        let mut command = self.command();
        command.env_clear().envs(environment.entries);
        self.spawn(&mut command)
    }

    fn command(&self) -> Command {
        let mut command = Command::new(&self.program);
        command
            .current_dir(&self.working_dir)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .args(&self.arguments);
        command
    }

    fn spawn(self, command: &mut Command) -> Result<ProcessSession, SessionError> {
        #[cfg(unix)]
        command.process_group(0);
        #[cfg(windows)]
        command.creation_flags(CREATE_SUSPENDED);
        let started = Instant::now();
        let child = command.spawn().map_err(|source| SessionError::Spawn {
            program: self.program.clone(),
            source,
        })?;
        #[cfg(windows)]
        let (child, job) = managed_job(child, &self.program)?;
        Ok(ProcessSession {
            child,
            #[cfg(windows)]
            job,
            program: self.program,
            started,
            resolved: false,
        })
    }
}

#[cfg(windows)]
fn managed_job(
    mut child: std::process::Child,
    program: &Path,
) -> Result<(std::process::Child, JobHandle), SessionError> {
    let job = assign_to_job(child.id()).map_err(|source| {
        let _ = child.kill();
        let _ = child.wait();
        SessionError::CapabilityUnavailable {
            program: program.to_path_buf(),
            source,
        }
    })?;
    Ok((child, job))
}

impl ProcessSession {
    /// Observes the child without consuming a running session.
    pub fn observe(&mut self) -> Result<ProcessObservation, SessionError> {
        match self
            .child
            .try_wait()
            .map_err(|source| self.wait_error(source))?
        {
            Some(status) => {
                self.resolved = true;
                Ok(ProcessObservation::Exited(status))
            }
            None => Ok(ProcessObservation::Running),
        }
    }

    /// Waits for natural child termination; every exit status is factual output.
    pub fn wait(mut self) -> Result<ProcessEnd, SessionError> {
        let status = self
            .child
            .wait()
            .map_err(|source| self.wait_error(source))?;
        self.resolved = true;
        Ok(ProcessEnd::Exited {
            status,
            elapsed: self.started.elapsed(),
        })
    }

    /// Stops the admitted tree and waits no longer than the caller's deadline for reaping.
    pub fn stop_within(mut self, deadline: Duration) -> Result<ProcessEnd, SessionError> {
        self.stop_tree();
        let stop_started = Instant::now();
        loop {
            if self
                .child
                .try_wait()
                .map_err(|source| self.wait_error(source))?
                .is_some()
            {
                self.resolved = true;
                return Ok(ProcessEnd::Stopped {
                    elapsed: self.started.elapsed(),
                });
            }
            if stop_started.elapsed() >= deadline {
                return Err(SessionError::StopTimedOut {
                    program: self.program.clone(),
                    deadline,
                });
            }
            thread::sleep(Duration::from_millis(10));
        }
    }

    fn wait_error(&self, source: io::Error) -> SessionError {
        SessionError::Wait {
            program: self.program.clone(),
            source,
        }
    }

    fn stop_tree(&mut self) {
        #[cfg(unix)]
        stop_tree(self.child.id() as i32);
        #[cfg(windows)]
        stop_tree(&self.job);
    }
}

impl Drop for ProcessSession {
    fn drop(&mut self) {
        if !self.resolved {
            self.stop_tree();
            let _ = self.child.wait();
            self.resolved = true;
        }
    }
}

fn admit_program(program: &Path) -> Result<(), ConfigError> {
    if program.as_os_str().is_empty() {
        return Err(ConfigError::EmptyProgram);
    }
    if !program.is_absolute() {
        return Err(ConfigError::NonAbsoluteProgram(program.to_path_buf()));
    }
    Ok(())
}

/// A path contained below the process workspace output root.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OutputPath(PathBuf);

impl OutputPath {
    /// Admits one nonempty, normal-component-only relative output path.
    pub fn new(path: impl Into<PathBuf>) -> Result<Self, ConfigError> {
        let path = path.into();
        if path.as_os_str().is_empty()
            || path
                .components()
                .any(|component| !matches!(component, Component::Normal(_)))
        {
            return Err(ConfigError::InvalidOutputPath(path));
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
pub enum Arg {
    /// The literal outcome.
    Literal(OsString),
    /// The workspace root outcome.
    WorkspaceRoot,
    /// The output root outcome.
    OutputRoot,
    /// The output path outcome.
    OutputPath(OutputPath),
}

/// Explicit process environment entries.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct EnvVars {
    entries: Vec<(OsString, OsString)>,
}

impl EnvVars {
    /// Admits caller entries while reserving Pulith's input/output-root variables.
    pub fn new(
        entries: impl IntoIterator<Item = (OsString, OsString)>,
    ) -> Result<Self, ConfigError> {
        let entries = entries.into_iter().collect::<Vec<_>>();
        for (index, (key, _)) in entries.iter().enumerate() {
            if key.is_empty() || key.to_string_lossy().contains('=') {
                return Err(ConfigError::InvalidEnvironmentKey(key.clone()));
            }
            if environment_keys_equal(key, OsStr::new(OUTPUT_ENV))
                || environment_keys_equal(key, OsStr::new(INPUT_ENV))
            {
                return Err(ConfigError::ReservedEnvironmentKey(key.clone()));
            }
            if entries[..index]
                .iter()
                .any(|(prior, _)| environment_keys_equal(prior, key))
            {
                return Err(ConfigError::DuplicateEnvironmentKey(key.clone()));
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
pub enum ConfigError {
    /// The invalid output path outcome.
    InvalidOutputPath(PathBuf),
    /// The invalid input name outcome.
    InvalidInputName(OsString),
    /// The non absolute input outcome.
    NonAbsoluteInput(PathBuf),
    /// The empty program outcome.
    EmptyProgram,
    /// The non absolute program outcome.
    NonAbsoluteProgram(PathBuf),
    /// The non absolute worktree outcome.
    NonAbsoluteWorktree(PathBuf),
    /// The zero timeout outcome.
    ZeroTimeout,
    /// The invalid environment key outcome.
    InvalidEnvironmentKey(OsString),
    /// The reserved environment key outcome.
    ReservedEnvironmentKey(OsString),
    /// The duplicate environment key outcome.
    DuplicateEnvironmentKey(OsString),
}

impl fmt::Display for ConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidOutputPath(path) => write!(
                formatter,
                "workspace output must be a nonempty contained relative path: {}",
                path.display()
            ),
            Self::InvalidInputName(name) => write!(
                formatter,
                "staged input name must be one normal path component: {:?}",
                name
            ),
            Self::NonAbsoluteInput(path) => write!(
                formatter,
                "staged input source path must be absolute: {}",
                path.display()
            ),
            Self::EmptyProgram => formatter.write_str("process program path must not be empty"),
            Self::NonAbsoluteProgram(path) => write!(
                formatter,
                "process program path must be absolute: {}",
                path.display()
            ),
            Self::NonAbsoluteWorktree(path) => write!(
                formatter,
                "process worktree path must be absolute: {}",
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

/// One bounded process that executes inside an existing caller-owned worktree.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorktreeProcess {
    program: PathBuf,
    arguments: Vec<OsString>,
    working_dir: PathBuf,
    timeout: Duration,
    capture_cap: usize,
}

impl WorktreeProcess {
    /// Admits an absolute program and absolute caller-owned worktree.
    pub fn new(
        program: impl Into<PathBuf>,
        working_dir: impl Into<PathBuf>,
        timeout: Duration,
    ) -> Result<Self, ConfigError> {
        let program = program.into();
        let working_dir = working_dir.into();
        if program.as_os_str().is_empty() {
            return Err(ConfigError::EmptyProgram);
        }
        if !program.is_absolute() {
            return Err(ConfigError::NonAbsoluteProgram(program));
        }
        if !working_dir.is_absolute() {
            return Err(ConfigError::NonAbsoluteWorktree(working_dir));
        }
        if timeout.is_zero() {
            return Err(ConfigError::ZeroTimeout);
        }
        Ok(Self {
            program,
            arguments: Vec::new(),
            working_dir,
            timeout,
            capture_cap: DEFAULT_CAPTURE_CAP,
        })
    }

    /// Replaces literal process arguments without shell interpolation.
    pub fn with_arguments(mut self, arguments: impl IntoIterator<Item = OsString>) -> Self {
        self.arguments = arguments.into_iter().collect();
        self
    }

    /// Bounds each retained diagnostic stream at read time.
    pub fn with_capture_cap(mut self, cap: usize) -> Self {
        self.capture_cap = cap;
        self
    }
}

/// Safe facts from a successful caller-worktree execution.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorktreeEvidence {
    /// The program value.
    pub program: PathBuf,
    /// The working dir value.
    pub working_dir: PathBuf,
    /// The elapsed value.
    pub elapsed: Duration,
}

/// Named result of a successful bounded caller-worktree execution.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorktreeResult {
    /// The evidence value.
    pub evidence: WorktreeEvidence,
    /// The diagnostics value.
    pub diagnostics: Diagnostics,
}

impl std::error::Error for ConfigError {}

/// One declared input file staged into the private workspace before the run.
///
/// `source` is the caller's host path; `name` is the deterministic staged name under
/// `inputs/<name>`, reachable as `$PULITH_INPUT_ROOT/<name>` or via a workspace-relative
/// argument. The file is copied, never linked, so the program's view is a snapshot: later host
/// edits do not reach the run, and the run cannot write back through the staged copy.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StagedInput {
    source: PathBuf,
    name: OsString,
}

impl StagedInput {
    /// Admits an absolute source path and one normal staged name.
    pub fn new(source: impl Into<PathBuf>, name: impl Into<OsString>) -> Result<Self, ConfigError> {
        let source = source.into();
        if !source.is_absolute() {
            return Err(ConfigError::NonAbsoluteInput(source));
        }
        let name = name.into();
        let mut components = Path::new(&name).components();
        if !matches!(
            (components.next(), components.next()),
            (Some(Component::Normal(part)), None) if !part.is_empty()
        ) {
            return Err(ConfigError::InvalidInputName(name));
        }
        Ok(Self { source, name })
    }
}

/// One bounded process that must create a directory below its private output root.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OutputProcess {
    program: PathBuf,
    arguments: Vec<Arg>,
    environment: EnvVars,
    inputs: Vec<StagedInput>,
    output: OutputPath,
    timeout: Duration,
    capture_cap: usize,
}

/// Exclusive private workspace, staged inputs, and admitted command awaiting one process run.
pub struct PreparedProcess {
    command: Command,
    workspace: tempfile::TempDir,
    selected_output: PathBuf,
    output: OutputPath,
    program: PathBuf,
    timeout: Duration,
    capture_cap: usize,
}

impl OutputProcess {
    /// Creates a cooperative process with explicit executable path and declared output directory.
    pub fn new(
        program: impl Into<PathBuf>,
        output: OutputPath,
        timeout: Duration,
    ) -> Result<Self, ConfigError> {
        let program = program.into();
        if program.as_os_str().is_empty() {
            return Err(ConfigError::EmptyProgram);
        }
        if !program.is_absolute() {
            return Err(ConfigError::NonAbsoluteProgram(program));
        }
        if timeout.is_zero() {
            return Err(ConfigError::ZeroTimeout);
        }
        Ok(Self {
            program,
            arguments: Vec::new(),
            environment: EnvVars::default(),
            inputs: Vec::new(),
            output,
            timeout,
            capture_cap: DEFAULT_CAPTURE_CAP,
        })
    }

    /// Replaces the structured program arguments.
    pub fn with_arguments(mut self, arguments: impl IntoIterator<Item = Arg>) -> Self {
        self.arguments = arguments.into_iter().collect();
        self
    }

    /// Replaces the explicit environment after its reserved-key admission.
    pub fn with_environment(mut self, environment: EnvVars) -> Self {
        self.environment = environment;
        self
    }

    /// Replaces the declared input files staged into the private workspace before the run.
    ///
    /// Each input is copied to `inputs/<name>` (never linked) with `PULITH_INPUT_ROOT` pointing
    /// at the staged directory; missing sources, collisions, and invalid names fail before the
    /// program spawns.
    pub fn with_inputs(mut self, inputs: impl IntoIterator<Item = StagedInput>) -> Self {
        self.inputs = inputs.into_iter().collect();
        self
    }

    /// Bounds each captured stream to `cap` bytes at read time.
    ///
    /// The cap bounds retained memory; both adapters continue draining bytes beyond it to avoid
    /// blocking the child. `cap = 0` disables capture entirely. Defaults to 1 MiB per stream.
    pub fn with_capture_cap(mut self, cap: usize) -> Self {
        self.capture_cap = cap;
        self
    }

    /// Creates exclusive workspace custody and snapshots every declared input before process spawn.
    pub fn prepare(self) -> Result<PreparedProcess, RunError> {
        let workspace = tempfile::Builder::new()
            .prefix(".pulith-process-")
            .tempdir()
            .map_err(|source| RunError::Workspace { source })?;
        let output_base = workspace.path().join("output");
        if let Err(source) = std::fs::create_dir(&output_base) {
            return fail_cleanup(workspace, RunError::Workspace { source });
        }
        let selected_output = output_base.join(self.output.as_path());
        let input_root = workspace.path().join("inputs");
        if let Err(error) = stage_inputs(workspace.path(), &self.inputs) {
            return fail_cleanup(workspace, error);
        }

        let mut command = Command::new(&self.program);
        command
            .current_dir(workspace.path())
            .env_clear()
            .envs(
                self.environment
                    .entries
                    .iter()
                    .map(|(key, value)| (key, value)),
            )
            .env(OUTPUT_ENV, &selected_output)
            .stdin(Stdio::null());
        if !self.inputs.is_empty() {
            command.env(INPUT_ENV, &input_root);
        }
        for argument in &self.arguments {
            command.arg(resolve_argument(
                argument,
                workspace.path(),
                &selected_output,
            ));
        }
        Ok(PreparedProcess {
            command,
            workspace,
            selected_output,
            output: self.output,
            program: self.program,
            timeout: self.timeout,
            capture_cap: self.capture_cap,
        })
    }
}

/// Safe facts from a successful cooperative process realization.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OutputEvidence {
    /// The output value.
    pub output: OutputPath,
    /// The elapsed value.
    pub elapsed: Duration,
}

/// Named material and evidence returned by private-output realization.
#[derive(Debug)]
pub struct OutputResult {
    /// The tree value.
    pub tree: StagedTree,
    /// The evidence value.
    pub evidence: OutputEvidence,
    /// The diagnostics value.
    pub diagnostics: Diagnostics,
}

/// Capped standard-stream output captured from the admitted process.
///
/// Diagnostics are payload, not safe-facts attestation: they are never rendered in [`fmt::Display`]
/// error text and never copied into [`OutputEvidence`]. Each stream is `None` when capture was
/// disabled (`cap = 0`) or the workspace diagnostic file could not be read back.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Diagnostics {
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

impl Diagnostics {
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

/// Caller-owned cancellation signal for one process.
///
/// The token is sticky (once cancelled it stays cancelled), `Send + Sync`, and carries no data
/// beyond the cancelled bit. [`PreparedProcess::acquire_cancellable`] polls it once per wait-loop
/// tick and stops the admitted tree via the frozen tree-stop path; a token already cancelled at
/// entry fails fast before the program spawns. Cancellation is the caller's explicit stop request
/// and is never confused with a timeout: it surfaces as [`RunError::Cancelled`].
#[derive(Clone, Default)]
pub struct CancelToken {
    cancelled: Arc<AtomicBool>,
}

impl CancelToken {
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

    #[cfg(feature = "process-tokio")]
    async fn cancelled(&self) {
        while !self.is_cancelled() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }
}

/// Failure before a staged output tree could be returned.
#[non_exhaustive]
#[derive(Debug)]
pub enum RunError {
    /// The workspace outcome.
    Workspace {
        /// The source value.
        source: io::Error,
    },
    /// The worktree missing outcome.
    WorktreeMissing {
        /// The path value.
        path: PathBuf,
    },
    /// The worktree wrong kind outcome.
    WorktreeWrongKind {
        /// The path value.
        path: PathBuf,
    },
    /// The worktree inspect outcome.
    WorktreeInspect {
        /// The path value.
        path: PathBuf,
        /// The source value.
        source: io::Error,
    },
    /// The input missing outcome.
    InputMissing {
        /// The path value.
        path: PathBuf,
    },
    /// The input collision outcome.
    InputCollision {
        /// The name value.
        name: OsString,
    },
    /// The input wrong kind outcome.
    InputWrongKind {
        /// The path value.
        path: PathBuf,
    },
    /// The input staging outcome.
    InputStaging {
        /// The path value.
        path: PathBuf,
        /// The source value.
        source: io::Error,
    },
    /// The spawn outcome.
    Spawn {
        /// The program value.
        program: PathBuf,
        /// The source value.
        source: io::Error,
        /// The diagnostics value.
        diagnostics: Box<Diagnostics>,
    },
    /// The wait outcome.
    Wait {
        /// The program value.
        program: PathBuf,
        /// The source value.
        source: io::Error,
        /// The diagnostics value.
        diagnostics: Box<Diagnostics>,
    },
    /// The timed out outcome.
    TimedOut {
        /// The program value.
        program: PathBuf,
        /// The timeout value.
        timeout: Duration,
        /// The diagnostics value.
        diagnostics: Box<Diagnostics>,
    },
    /// The cancelled outcome.
    Cancelled {
        /// The program value.
        program: PathBuf,
        /// The diagnostics value.
        diagnostics: Box<Diagnostics>,
    },
    /// The exited non zero outcome.
    ExitedNonZero {
        /// The program value.
        program: PathBuf,
        /// The status value.
        status: ExitStatus,
        /// The diagnostics value.
        diagnostics: Box<Diagnostics>,
    },
    /// The output missing outcome.
    OutputMissing {
        /// The path value.
        path: PathBuf,
        /// The diagnostics value.
        diagnostics: Box<Diagnostics>,
    },
    /// The output wrong kind outcome.
    OutputWrongKind {
        /// The path value.
        path: PathBuf,
        /// The observed value.
        observed: LocalObservation,
        /// The diagnostics value.
        diagnostics: Box<Diagnostics>,
    },
    /// The output inspect outcome.
    OutputInspect {
        /// The path value.
        path: PathBuf,
        /// The source value.
        source: crate::local::LocalError,
        /// The diagnostics value.
        diagnostics: Box<Diagnostics>,
    },
    #[cfg(windows)]
    /// The capability unavailable outcome.
    CapabilityUnavailable {
        /// The program value.
        program: PathBuf,
        /// The source value.
        source: io::Error,
        /// The diagnostics value.
        diagnostics: Box<Diagnostics>,
    },
    /// The workspace cleanup outcome.
    WorkspaceCleanup {
        /// The primary value.
        primary: Box<RunError>,
        /// The cleanup value.
        cleanup: io::Error,
    },
}

impl fmt::Display for RunError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Captured diagnostics are never rendered here; they may contain program output.
        match self {
            Self::Workspace { source } => {
                write!(formatter, "failed to create process workspace: {source}")
            }
            Self::WorktreeMissing { path } => {
                write!(formatter, "caller worktree is missing: {}", path.display())
            }
            Self::WorktreeWrongKind { path } => {
                write!(
                    formatter,
                    "caller worktree is not a directory: {}",
                    path.display()
                )
            }
            Self::WorktreeInspect { path, source } => write!(
                formatter,
                "failed to inspect caller worktree {}: {source}",
                path.display()
            ),
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
            Self::InputWrongKind { path } => write!(
                formatter,
                "declared process input is not a regular non-link file: {}",
                path.display()
            ),
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

impl std::error::Error for RunError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Workspace { source }
            | Self::WorktreeInspect { source, .. }
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

/// The material and evidence returned by a successful process realization: the adapter-owned
/// [`StagedTree`] custody, the safe-facts [`OutputEvidence`], and the capped captured
/// diagnostics (payload, never part of the safe facts).
type ProcessAcquiredOutput = OutputResult;

type WorktreeExecutedOutput = WorktreeResult;

struct WorktreeExecution {
    session: ChildSession,
    program: PathBuf,
    working_dir: PathBuf,
    _capture: tempfile::TempDir,
}

struct PrivateExecution {
    session: ChildSession,
    workspace: tempfile::TempDir,
    selected_output: PathBuf,
    output: OutputPath,
    program: PathBuf,
}

impl PrivateExecution {
    fn wait(self) -> Result<ProcessAcquiredOutput, RunError> {
        let PrivateExecution {
            session,
            workspace,
            selected_output,
            output,
            program,
        } = self;
        match session.wait() {
            Ok(completion) => Self::finish(workspace, selected_output, output, program, completion),
            Err(error) => fail_cleanup(workspace, error),
        }
    }

    fn wait_cancellable(self, cancel: &CancelToken) -> Result<ProcessAcquiredOutput, RunError> {
        let PrivateExecution {
            session,
            workspace,
            selected_output,
            output,
            program,
        } = self;
        match session.wait_cancellable(cancel) {
            Ok(completion) => Self::finish(workspace, selected_output, output, program, completion),
            Err(error) => fail_cleanup(workspace, error),
        }
    }

    fn finish(
        workspace: tempfile::TempDir,
        selected_output: PathBuf,
        output: OutputPath,
        program: PathBuf,
        completion: ChildCompletion,
    ) -> Result<ProcessAcquiredOutput, RunError> {
        if !completion.status.success() {
            return fail_cleanup(
                workspace,
                RunError::ExitedNonZero {
                    program,
                    status: completion.status,
                    diagnostics: Box::new(completion.diagnostics),
                },
            );
        }
        let diagnostics = completion.diagnostics;
        let observation = match Inspect::inspect(
            LocalTarget::new(selected_output.clone())
                .expect("a selected output path is always a nonempty contained workspace path"),
            (),
        ) {
            Ok((observation, _)) => observation,
            Err(source) => {
                return fail_cleanup(
                    workspace,
                    RunError::OutputInspect {
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
                RunError::OutputMissing {
                    path: selected_output,
                    diagnostics: Box::new(diagnostics),
                },
            );
        }
        if observation != LocalObservation::Directory {
            return fail_cleanup(
                workspace,
                RunError::OutputWrongKind {
                    path: selected_output,
                    observed: observation,
                    diagnostics: Box::new(diagnostics),
                },
            );
        }
        let evidence = OutputEvidence {
            output,
            elapsed: completion.elapsed,
        };
        Ok(OutputResult {
            tree: StagedTree::new(workspace, selected_output),
            evidence,
            diagnostics,
        })
    }
}

impl WorktreeExecution {
    fn wait(self) -> Result<WorktreeExecutedOutput, RunError> {
        let WorktreeExecution {
            session,
            program,
            working_dir,
            _capture,
        } = self;
        Self::finish(program, working_dir, session.wait()?)
    }

    fn wait_cancellable(self, cancel: &CancelToken) -> Result<WorktreeExecutedOutput, RunError> {
        let WorktreeExecution {
            session,
            program,
            working_dir,
            _capture,
        } = self;
        Self::finish(program, working_dir, session.wait_cancellable(cancel)?)
    }

    fn finish(
        program: PathBuf,
        working_dir: PathBuf,
        completion: ChildCompletion,
    ) -> Result<WorktreeExecutedOutput, RunError> {
        if !completion.status.success() {
            return Err(RunError::ExitedNonZero {
                program,
                status: completion.status,
                diagnostics: Box::new(completion.diagnostics),
            });
        }
        Ok(WorktreeResult {
            evidence: WorktreeEvidence {
                program,
                working_dir,
                elapsed: completion.elapsed,
            },
            diagnostics: completion.diagnostics,
        })
    }
}

struct ChildSession {
    child: std::process::Child,
    #[cfg(windows)]
    job: JobHandle,
    program: PathBuf,
    timeout: Duration,
    started: Instant,
    capture_root: PathBuf,
    capture_cap: usize,
}

struct ChildCompletion {
    status: ExitStatus,
    diagnostics: Diagnostics,
    elapsed: Duration,
}

enum ChildPoll {
    Running,
    Completed(ChildCompletion),
}

impl ChildSession {
    fn spawn(
        command: &mut Command,
        program: &Path,
        timeout: Duration,
        capture_root: &Path,
        capture_cap: usize,
    ) -> Result<Self, RunError> {
        if capture_cap > 0 {
            let stdout = File::create(capture_root.join("stdout.log"));
            let stderr = File::create(capture_root.join("stderr.log"));
            match (stdout, stderr) {
                (Ok(stdout), Ok(stderr)) => {
                    command.stdout(stdout).stderr(stderr);
                }
                (stdout, stderr) => {
                    let source = match (stdout, stderr) {
                        (Err(source), _) | (_, Err(source)) => source,
                        _ => unreachable!(),
                    };
                    return Err(RunError::Spawn {
                        program: program.to_path_buf(),
                        source,
                        diagnostics: Box::new(read_diagnostics(capture_root, capture_cap)),
                    });
                }
            }
        } else {
            command.stdout(Stdio::null()).stderr(Stdio::null());
        }
        #[cfg(unix)]
        command.process_group(0);
        #[cfg(windows)]
        command.creation_flags(CREATE_SUSPENDED);
        let started = Instant::now();
        let child = command.spawn().map_err(|source| RunError::Spawn {
            program: program.to_path_buf(),
            source,
            diagnostics: Box::new(read_diagnostics(capture_root, capture_cap)),
        })?;
        #[cfg(windows)]
        let (child, job) = {
            let mut child = child;
            let job = assign_to_job(child.id()).map_err(|source| {
                let _ = child.kill();
                let _ = child.wait();
                RunError::CapabilityUnavailable {
                    program: program.to_path_buf(),
                    source,
                    diagnostics: Box::new(read_diagnostics(capture_root, capture_cap)),
                }
            })?;
            (child, job)
        };
        Ok(Self {
            child,
            #[cfg(windows)]
            job,
            program: program.to_path_buf(),
            timeout,
            started,
            capture_root: capture_root.to_path_buf(),
            capture_cap,
        })
    }

    fn wait(mut self) -> Result<ChildCompletion, RunError> {
        loop {
            match self.poll()? {
                ChildPoll::Running => thread::sleep(Duration::from_millis(10)),
                ChildPoll::Completed(completion) => return Ok(completion),
            }
        }
    }

    fn wait_cancellable(mut self, cancel: &CancelToken) -> Result<ChildCompletion, RunError> {
        loop {
            if cancel.is_cancelled() {
                self.stop();
                return Err(self.cancelled());
            }
            match self.poll()? {
                ChildPoll::Running => thread::sleep(Duration::from_millis(10)),
                ChildPoll::Completed(completion) => return Ok(completion),
            }
        }
    }

    fn poll(&mut self) -> Result<ChildPoll, RunError> {
        match self.child.try_wait() {
            Ok(Some(status)) => Ok(ChildPoll::Completed(ChildCompletion {
                status,
                diagnostics: self.diagnostics(),
                elapsed: self.started.elapsed(),
            })),
            Ok(None) if self.started.elapsed() >= self.timeout => {
                self.stop();
                Err(RunError::TimedOut {
                    program: self.program.clone(),
                    timeout: self.timeout,
                    diagnostics: Box::new(self.diagnostics()),
                })
            }
            Ok(None) => Ok(ChildPoll::Running),
            Err(source) => Err(RunError::Wait {
                program: self.program.clone(),
                source,
                diagnostics: Box::new(self.diagnostics()),
            }),
        }
    }

    fn stop(&mut self) {
        #[cfg(unix)]
        stop_tree(self.child.id() as i32);
        #[cfg(windows)]
        stop_tree(&self.job);
        let _ = self.child.wait();
    }

    fn cancelled(&self) -> RunError {
        RunError::Cancelled {
            program: self.program.clone(),
            diagnostics: Box::new(self.diagnostics()),
        }
    }

    fn diagnostics(&self) -> Diagnostics {
        read_diagnostics(&self.capture_root, self.capture_cap)
    }
}

impl WorktreeProcess {
    /// Executes the admitted program in its existing caller-owned worktree.
    pub fn execute(self) -> Result<WorktreeExecutedOutput, RunError> {
        start_worktree(self)?.wait()
    }

    /// Executes with only the admitted environment entries instead of inheriting the caller.
    pub fn execute_in_environment(
        self,
        environment: EnvVars,
    ) -> Result<WorktreeExecutedOutput, RunError> {
        start_worktree_in_environment(self, environment)?.wait()
    }

    /// Runs in the admitted caller worktree and stops the admitted tree when cancelled.
    pub fn execute_cancellable(
        self,
        cancel: &CancelToken,
    ) -> Result<WorktreeExecutedOutput, RunError> {
        if cancel.is_cancelled() {
            return Err(RunError::Cancelled {
                program: self.program,
                diagnostics: Box::new(Diagnostics::disabled()),
            });
        }
        start_worktree(self)?.wait_cancellable(cancel)
    }

    /// Executes with an explicit environment and stops the admitted tree when cancelled.
    pub fn execute_cancellable_in_environment(
        self,
        environment: EnvVars,
        cancel: &CancelToken,
    ) -> Result<WorktreeExecutedOutput, RunError> {
        if cancel.is_cancelled() {
            return Err(RunError::Cancelled {
                program: self.program,
                diagnostics: Box::new(Diagnostics::disabled()),
            });
        }
        start_worktree_in_environment(self, environment)?.wait_cancellable(cancel)
    }
}

fn start_worktree(input: WorktreeProcess) -> Result<WorktreeExecution, RunError> {
    let mut command = worktree_command(&input);
    start_worktree_command(input, &mut command)
}

fn start_worktree_in_environment(
    input: WorktreeProcess,
    environment: EnvVars,
) -> Result<WorktreeExecution, RunError> {
    let mut command = worktree_command(&input);
    command.env_clear().envs(environment.entries);
    start_worktree_command(input, &mut command)
}

fn worktree_command(input: &WorktreeProcess) -> Command {
    let mut command = Command::new(&input.program);
    command
        .current_dir(&input.working_dir)
        .stdin(Stdio::null())
        .args(&input.arguments);
    command
}

fn start_worktree_command(
    input: WorktreeProcess,
    command: &mut Command,
) -> Result<WorktreeExecution, RunError> {
    match std::fs::metadata(&input.working_dir) {
        Ok(metadata) if metadata.is_dir() => {}
        Ok(_) => {
            return Err(RunError::WorktreeWrongKind {
                path: input.working_dir,
            });
        }
        Err(source) if source.kind() == io::ErrorKind::NotFound => {
            return Err(RunError::WorktreeMissing {
                path: input.working_dir,
            });
        }
        Err(source) => {
            return Err(RunError::WorktreeInspect {
                path: input.working_dir,
                source,
            });
        }
    }
    let capture = tempfile::Builder::new()
        .prefix(".pulith-process-capture-")
        .tempdir()
        .map_err(|source| RunError::Workspace { source })?;
    let session = ChildSession::spawn(
        command,
        &input.program,
        input.timeout,
        capture.path(),
        input.capture_cap,
    )?;
    Ok(WorktreeExecution {
        session,
        program: input.program,
        working_dir: input.working_dir,
        _capture: capture,
    })
}

impl Acquire for PreparedProcess {
    type Error = RunError;
    type Output = ProcessAcquiredOutput;

    fn acquire(self) -> Result<Self::Output, Self::Error> {
        self.start()?.wait()
    }
}

impl PreparedProcess {
    /// Runs one cooperative process to staged-tree custody, stopping the admitted tree when the
    /// caller's token is set (sticky; polled once per wait-loop tick).
    ///
    /// Prefer the trait's [`Acquire::acquire`] for token-free calls; this inherent entry exists so
    /// the caller can stop a long realization without waiting for the timeout. Cancellation
    /// reuses the frozen tree-stop path and surfaces as [`RunError::Cancelled`], never as
    /// [`RunError::TimedOut`].
    pub fn acquire_cancellable(
        self,
        cancel: &CancelToken,
    ) -> Result<ProcessAcquiredOutput, RunError> {
        if cancel.is_cancelled() {
            return Err(RunError::Cancelled {
                program: self.program,
                diagnostics: Box::new(Diagnostics::disabled()),
            });
        }
        self.start()?.wait_cancellable(cancel)
    }

    fn start(mut self) -> Result<PrivateExecution, RunError> {
        let session = match ChildSession::spawn(
            &mut self.command,
            &self.program,
            self.timeout,
            self.workspace.path(),
            self.capture_cap,
        ) {
            Ok(session) => session,
            Err(error) => return fail_cleanup(self.workspace, error),
        };
        Ok(PrivateExecution {
            session,
            workspace: self.workspace,
            selected_output: self.selected_output,
            output: self.output,
            program: self.program,
        })
    }
}

fn stage_inputs(workspace: &Path, inputs: &[StagedInput]) -> Result<(), RunError> {
    if inputs.is_empty() {
        return Ok(());
    }
    let input_root = workspace.join("inputs");
    std::fs::create_dir(&input_root).map_err(|source| RunError::InputStaging {
        path: input_root.clone(),
        source,
    })?;
    let mut seen = std::collections::HashSet::new();
    for spec in inputs {
        if !seen.insert(spec.name.clone()) {
            return Err(RunError::InputCollision {
                name: spec.name.clone(),
            });
        }
        admit_staged_source(&spec.source)?;
        let destination = input_root.join(&spec.name);
        if let Err(source) = std::fs::copy(&spec.source, &destination) {
            return Err(RunError::InputStaging {
                path: destination,
                source,
            });
        }
    }
    Ok(())
}

fn admit_staged_source(path: &Path) -> Result<(), RunError> {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(source) if source.kind() == io::ErrorKind::NotFound => {
            return Err(RunError::InputMissing {
                path: path.to_path_buf(),
            });
        }
        Err(source) => {
            return Err(RunError::InputStaging {
                path: path.to_path_buf(),
                source,
            });
        }
    };
    #[cfg(windows)]
    let reparse = {
        use std::os::windows::fs::MetadataExt;
        metadata.file_attributes()
            & windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT
            != 0
    };
    #[cfg(not(windows))]
    let reparse = false;
    if !metadata.file_type().is_file() || reparse {
        return Err(RunError::InputWrongKind {
            path: path.to_path_buf(),
        });
    }
    Ok(())
}

#[cfg(feature = "process-tokio")]
impl AsyncAcquire for PreparedProcess {
    type Error = RunError;
    type Output = ProcessAcquiredOutput;

    async fn acquire(self) -> Result<Self::Output, Self::Error> {
        self.acquire_async().await
    }
}

#[cfg(feature = "process-tokio")]
enum Awaited {
    Exit(ExitStatus),
    Wait(io::Error),
    Timeout,
    Cancelled,
}

#[cfg(feature = "process-tokio")]
impl Awaited {
    fn finish(
        self,
        program: &Path,
        timeout: Duration,
        diagnostics: Diagnostics,
    ) -> Result<(ExitStatus, Diagnostics), RunError> {
        let error = match self {
            Self::Exit(status) => return Ok((status, diagnostics)),
            Self::Wait(source) => RunError::Wait {
                program: program.to_path_buf(),
                source,
                diagnostics: Box::new(diagnostics),
            },
            Self::Timeout => RunError::TimedOut {
                program: program.to_path_buf(),
                timeout,
                diagnostics: Box::new(diagnostics),
            },
            Self::Cancelled => RunError::Cancelled {
                program: program.to_path_buf(),
                diagnostics: Box::new(diagnostics),
            },
        };
        Err(error)
    }
}

#[cfg(feature = "process-tokio")]
impl PreparedProcess {
    async fn acquire_async(self) -> Result<ProcessAcquiredOutput, RunError> {
        let cancel = CancelToken::new();
        self.acquire_async_cancellable_inner(&cancel).await
    }

    /// Awaitable token-cancellable entry, mirroring [`PreparedProcess::acquire_cancellable`] for
    /// the async path: the wait loop polls the same token, so the caller can cancel while keeping
    /// the future alive to await the outcome. Dropping the future still stops the tree.
    pub async fn acquire_async_cancellable(
        self,
        cancel: &CancelToken,
    ) -> Result<ProcessAcquiredOutput, RunError> {
        self.acquire_async_cancellable_inner(cancel).await
    }

    async fn acquire_async_cancellable_inner(
        self,
        cancel: &CancelToken,
    ) -> Result<ProcessAcquiredOutput, RunError> {
        let PreparedProcess {
            command,
            workspace,
            selected_output,
            output,
            program,
            timeout,
            capture_cap,
        } = self;
        if cancel.is_cancelled() {
            return Err(RunError::Cancelled {
                program,
                diagnostics: Box::new(Diagnostics::disabled()),
            });
        }
        let started = Instant::now();

        let mut command = tokio::process::Command::from(command);
        if capture_cap > 0 {
            command.stdout(Stdio::piped()).stderr(Stdio::piped());
        } else {
            command.stdout(Stdio::null()).stderr(Stdio::null());
        }
        #[cfg(unix)]
        command.process_group(0);
        #[cfg(windows)]
        command.creation_flags(CREATE_SUSPENDED);

        let child = match command.spawn() {
            Ok(child) => child,
            Err(source) => {
                return fail_cleanup(
                    workspace,
                    RunError::Spawn {
                        program: program.clone(),
                        source,
                        diagnostics: Box::new(Diagnostics::disabled()),
                    },
                );
            }
        };

        #[cfg(windows)]
        let mut child = child;

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
                return fail_cleanup(
                    workspace,
                    RunError::CapabilityUnavailable {
                        program: program.clone(),
                        source,
                        diagnostics: Box::new(Diagnostics::disabled()),
                    },
                );
            }
        };

        let mut guard = AsyncTreeGuard::new(
            child,
            #[cfg(windows)]
            job,
        );
        let stdout = guard.child.stdout.take();
        let stderr = guard.child.stderr.take();

        let wait = async {
            let awaited = tokio::select! {
                status = guard.child.wait() => status.map_or_else(Awaited::Wait, Awaited::Exit),
                () = tokio::time::sleep(timeout) => Awaited::Timeout,
                () = cancel.cancelled() => Awaited::Cancelled,
            };
            if !matches!(awaited, Awaited::Exit(_)) {
                guard.stop();
                let _ = guard.child.wait().await;
            }
            awaited
        };
        let (awaited, stdout, stderr) = tokio::join!(
            wait,
            capture_optional_pipe(stdout, capture_cap),
            capture_optional_pipe(stderr, capture_cap),
        );
        let diagnostics = pipe_diagnostics(stdout, stderr, capture_cap);
        let (status, diagnostics) = match awaited.finish(&program, timeout, diagnostics) {
            Ok(completion) => completion,
            Err(error) => return fail_cleanup(workspace, error),
        };
        guard.disarm();
        drop(guard);
        PrivateExecution::finish(
            workspace,
            selected_output,
            output,
            program,
            ChildCompletion {
                status,
                diagnostics,
                elapsed: started.elapsed(),
            },
        )
    }
}

#[cfg(feature = "process-tokio")]
async fn capture_pipe<R>(mut stream: R, cap: usize) -> (Option<Vec<u8>>, bool)
where
    R: tokio::io::AsyncRead + Unpin,
{
    use tokio::io::AsyncReadExt;

    let mut retained = Vec::with_capacity(cap.min(8192));
    let mut truncated = false;
    let mut chunk = [0_u8; 8192];
    loop {
        let read = match stream.read(&mut chunk).await {
            Ok(0) => break,
            Ok(read) => read,
            Err(_) => return (None, false),
        };
        let available = cap.saturating_sub(retained.len());
        let keep = read.min(available);
        retained.extend_from_slice(&chunk[..keep]);
        truncated |= keep < read;
    }
    (Some(retained), truncated)
}

#[cfg(feature = "process-tokio")]
async fn capture_optional_pipe<R>(stream: Option<R>, cap: usize) -> (Option<Vec<u8>>, bool)
where
    R: tokio::io::AsyncRead + Unpin,
{
    match stream {
        Some(stream) => capture_pipe(stream, cap).await,
        None => (None, false),
    }
}

#[cfg(feature = "process-tokio")]
fn pipe_diagnostics(
    stdout: (Option<Vec<u8>>, bool),
    stderr: (Option<Vec<u8>>, bool),
    cap: usize,
) -> Diagnostics {
    if cap == 0 {
        return Diagnostics::disabled();
    }
    Diagnostics {
        stdout: stdout.0,
        stderr: stderr.0,
        stdout_truncated: stdout.1,
        stderr_truncated: stderr.1,
        cap,
    }
}

/// Stops the admitted tree when the async acquire future is dropped or aborted.
///
/// The guard is armed only while the wait loop is running. A future dropped or aborted then runs
/// the frozen S2.7 tree-stop path (group kill / `TerminateJobObject` plus a direct-child kill
/// signal) with tokio reaping the direct child; best-effort per the S2.7-D4 boundary, no
/// zero-survivor claim. Once the child has produced an exit status the guard is disarmed, so
/// normal completion leaves surviving descendants running exactly like the sync adapter.
#[cfg(feature = "process-tokio")]
struct AsyncTreeGuard {
    child: tokio::process::Child,
    #[cfg(unix)]
    pid: u32,
    armed: bool,
    #[cfg(windows)]
    job: JobHandle,
}

#[cfg(feature = "process-tokio")]
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

#[cfg(feature = "process-tokio")]
impl Drop for AsyncTreeGuard {
    fn drop(&mut self) {
        if self.armed {
            self.stop();
        }
    }
}

fn resolve_argument(argument: &Arg, workspace: &Path, output: &Path) -> OsString {
    match argument {
        Arg::Literal(argument) => argument.clone(),
        Arg::WorkspaceRoot => workspace.as_os_str().to_os_string(),
        Arg::OutputRoot => output.as_os_str().to_os_string(),
        Arg::OutputPath(relative) => output.join(relative.as_path()).into_os_string(),
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
fn read_diagnostics(workspace: &Path, cap: usize) -> Diagnostics {
    if cap == 0 {
        return Diagnostics::disabled();
    }
    let (stdout, stdout_truncated) = read_capped(&workspace.join("stdout.log"), cap);
    let (stderr, stderr_truncated) = read_capped(&workspace.join("stderr.log"), cap);
    Diagnostics {
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

fn fail_cleanup<T>(workspace: tempfile::TempDir, primary: RunError) -> Result<T, RunError> {
    match workspace.close() {
        Ok(()) => Err(primary),
        Err(error) => Err(RunError::WorkspaceCleanup {
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
    fn output_path_rejects_parent_traversal() {
        assert!(OutputPath::new("../outside").is_err());
        assert!(OutputPath::new("tree").is_ok());
    }

    #[test]
    fn env_vars_reserve_pulith_keys_and_reject_duplicates() {
        assert!(
            EnvVars::new([(
                OsString::from("PULITH_OUTPUT_ROOT"),
                OsString::from("outside"),
            )])
            .is_err()
        );
        assert!(
            EnvVars::new([(
                OsString::from("PULITH_INPUT_ROOT"),
                OsString::from("outside"),
            )])
            .is_err()
        );
        assert!(
            EnvVars::new([
                (OsString::from("PULITH_TEST"), OsString::from("one")),
                (OsString::from("PULITH_TEST"), OsString::from("two")),
            ])
            .is_err()
        );
    }

    #[cfg(windows)]
    #[test]
    fn env_vars_reserve_keys_case_insensitively() {
        assert!(
            EnvVars::new([(
                OsString::from("pulith_output_root"),
                OsString::from("outside"),
            )])
            .is_err()
        );
    }

    #[test]
    fn output_process_rejects_relative_program_and_zero_timeout() {
        let output = OutputPath::new("tree").unwrap();
        assert!(
            OutputProcess::new("relative-program", output.clone(), Duration::from_secs(1)).is_err()
        );
        assert!(OutputProcess::new(absolute_program(), output, Duration::ZERO).is_err());
    }

    #[test]
    fn staged_input_preserves_a_non_unicode_platform_name() {
        #[cfg(unix)]
        let name = {
            use std::os::unix::ffi::OsStringExt;
            OsString::from_vec(vec![0xff])
        };
        #[cfg(windows)]
        let name = {
            use std::os::windows::ffi::OsStringExt;
            OsString::from_wide(&[0xd800])
        };
        let input = StagedInput::new(absolute_program(), name.clone()).unwrap();
        assert_eq!(input.name, name);
    }
}
