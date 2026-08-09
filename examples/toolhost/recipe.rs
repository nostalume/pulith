use pulith::Inspect;
use pulith::local::{LocalObservation, LocalSource, LocalTarget, StagedTree};
use pulith::process::{EnvVars, WorktreeProcess, WorktreeResult};
use serde::Deserialize;
use std::ffi::OsString;
use std::path::{Component, Path, PathBuf};
use std::time::Duration;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Recipe {
    pub name: String,
    pub version: String,
    pub build: Option<Build>,
    pub outputs: Outputs,
    #[serde(default, rename = "verify")]
    pub verifications: Vec<Verification>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Build {
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    pub working_dir: Option<PathBuf>,
    pub timeout_seconds: u64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Outputs {
    pub binary: PathBuf,
    pub runtime: Option<PathBuf>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum Verification {
    Stdout {
        #[serde(default)]
        args: Vec<String>,
        stdout: String,
    },
    LoadedRuntime {
        #[serde(default)]
        args: Vec<String>,
        loaded_runtime: LoadedRuntime,
    },
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LoadedRuntime {
    pub identity: String,
    pub path: PathBuf,
}

#[derive(Debug)]
pub enum Resolved {
    NoBuild,
    Build(Box<ResolvedBuild>),
}

#[derive(Debug)]
pub struct ResolvedBuild {
    identity: (String, String),
    process: WorktreeProcess,
    outputs: Outputs,
    verifications: Vec<Verification>,
}

struct Companions {
    dispatcher: LocalSource,
    service_host: LocalSource,
}

pub enum InstallOutcome {
    NoBuild,
    Published {
        release: PathBuf,
        process: WorktreeResult,
    },
}

impl Recipe {
    pub fn load(path: &Path) -> Result<Self, crate::BoxError> {
        Self::parse(&std::fs::read_to_string(path)?)
    }

    pub fn parse(text: &str) -> Result<Self, crate::BoxError> {
        Ok(toml::from_str(text)?)
    }

    pub fn resolve(self, recipe_path: &Path) -> Result<Resolved, crate::BoxError> {
        component(&self.name, "name")?;
        component(&self.version, "version")?;
        let Some(build) = self.build else {
            return Ok(Resolved::NoBuild);
        };
        let recipe_dir = recipe_path
            .parent()
            .ok_or_else(|| std::io::Error::other("recipe has no parent directory"))?
            .canonicalize()?;
        if build.timeout_seconds == 0 {
            return Err(std::io::Error::other("build.timeout_seconds must be positive").into());
        }
        let worktree = contained(
            &recipe_dir,
            build.working_dir.as_deref().unwrap_or(Path::new(".")),
        )?
        .canonicalize()?;
        let program = resolve_command(&recipe_dir, &build.command)?;
        let binary = contained(&worktree, &self.outputs.binary)?;
        let runtime = self
            .outputs
            .runtime
            .as_deref()
            .map(|path| contained(&worktree, path))
            .transpose()?;
        validate_runtime_assertions(runtime.is_some(), &self.verifications)?;
        let process = WorktreeProcess::new(
            program,
            worktree,
            Duration::from_secs(build.timeout_seconds),
        )?
        .with_arguments(build.args.into_iter().map(OsString::from));
        Ok(Resolved::Build(Box::new(ResolvedBuild {
            identity: (self.name, self.version),
            process,
            outputs: Outputs { binary, runtime },
            verifications: self.verifications,
        })))
    }
}

impl Resolved {
    pub fn install(self, root: &Path) -> Result<InstallOutcome, crate::BoxError> {
        match self {
            Self::NoBuild => Ok(InstallOutcome::NoBuild),
            Self::Build(build) => build.install(root, companions()?),
        }
    }
}

impl ResolvedBuild {
    fn install(
        self,
        root: &Path,
        companions: Companions,
    ) -> Result<InstallOutcome, crate::BoxError> {
        let Companions {
            dispatcher,
            service_host,
        } = companions;
        let process = self.process.execute()?;
        let worktree = &process.evidence.working_dir;
        let executable = executable_name(&self.identity.0);
        let target = root
            .join("installs")
            .join(self.identity.0)
            .join(self.identity.1);
        let admitted = LocalTarget::new(&target)?;
        let stage = admitted
            .stage()?
            .copy_file(
                admitted_source(self.outputs.binary, worktree)?,
                Path::new("bin").join(&executable),
            )?
            .copy_file(dispatcher, Path::new("shims").join(&executable))?
            .copy_file(service_host, Path::new("service").join(&executable))?;
        let has_runtime = self.outputs.runtime.is_some();
        let stage = match self.outputs.runtime {
            Some(path) => stage.copy_tree(admitted_source(path, worktree)?, "private-runtime")?,
            None => stage,
        };
        let mut paths = (
            stage.root().join("bin").join(&executable).canonicalize()?,
            stage.root().canonicalize()?,
        );
        for verification in self.verifications {
            paths = verification.verify(paths)?;
        }
        Self::validate(&stage, &executable, has_runtime)?;
        stage.publish(admitted)?;
        Ok(InstallOutcome::Published {
            release: target,
            process,
        })
    }

    fn validate(
        stage: &StagedTree,
        executable: &str,
        runtime: bool,
    ) -> Result<(), crate::BoxError> {
        for path in [
            stage.root().join("bin").join(executable),
            stage.root().join("shims").join(executable),
        ] {
            if !matches!(
                LocalTarget::new(path)?.inspect(())?.0,
                LocalObservation::File { .. }
            ) {
                return Err(std::io::Error::other("missing staged binary or dispatcher").into());
            }
        }
        if runtime
            && !matches!(
                LocalTarget::new(stage.root().join("private-runtime"))?
                    .inspect(())?
                    .0,
                LocalObservation::Directory
            )
        {
            return Err(std::io::Error::other("missing staged private-runtime").into());
        }
        Ok(())
    }
}

impl Verification {
    fn verify(self, paths: (PathBuf, PathBuf)) -> Result<(PathBuf, PathBuf), crate::BoxError> {
        let args = match &self {
            Self::Stdout { args, .. } | Self::LoadedRuntime { args, .. } => args,
        };
        let result = WorktreeProcess::new(paths.0, paths.1, Duration::from_secs(10))?
            .with_arguments(args.iter().map(OsString::from))
            .execute_in_environment(EnvVars::new([])?)?;
        let stdout = result.diagnostics.stdout.unwrap_or_default();
        match self {
            Self::Stdout {
                stdout: expected, ..
            } if stdout != expected.as_bytes() => {
                return Err(std::io::Error::other("verification stdout mismatch").into());
            }
            Self::LoadedRuntime { loaded_runtime, .. } => {
                let origin = contained(
                    &result.evidence.working_dir.join("private-runtime"),
                    &loaded_runtime.path,
                )?
                .canonicalize()?;
                let expected = format!(
                    "identity={}\norigin={}\n",
                    loaded_runtime.identity,
                    origin.display()
                );
                if stdout != expected.as_bytes() {
                    return Err(std::io::Error::other("loaded runtime witness mismatch").into());
                }
            }
            _ => {}
        }
        Ok((result.evidence.program, result.evidence.working_dir))
    }
}

fn companions() -> Result<Companions, crate::BoxError> {
    let own = std::env::current_exe()?;
    let parent = own
        .parent()
        .ok_or_else(|| std::io::Error::other("toolhost executable has no parent"))?;
    Ok(Companions {
        dispatcher: LocalSource::new(parent.join(executable_name("toolhost-shim")))?,
        service_host: LocalSource::new(parent.join(executable_name("toolhost-service")))?,
    })
}

fn admitted_source(path: PathBuf, boundary: &Path) -> Result<LocalSource, crate::BoxError> {
    if path.canonicalize()?.starts_with(boundary) {
        LocalSource::new(path).map_err(Into::into)
    } else {
        Err(std::io::Error::other(format!("artifact escapes worktree: {}", path.display())).into())
    }
}

pub fn executable_name(name: &str) -> String {
    format!("{name}{}", std::env::consts::EXE_SUFFIX)
}

pub(crate) fn component(value: &str, label: &str) -> Result<(), crate::BoxError> {
    let mut parts = Path::new(value).components();
    if matches!(parts.next(), Some(Component::Normal(_))) && parts.next().is_none() {
        Ok(())
    } else {
        Err(std::io::Error::other(format!("{label} must be one normal path component")).into())
    }
}

fn contained(base: &Path, relative: &Path) -> Result<PathBuf, crate::BoxError> {
    let valid = !relative.is_absolute()
        && relative
            .components()
            .all(|part| matches!(part, Component::Normal(_) | Component::CurDir));
    if !valid {
        return Err(std::io::Error::other(format!(
            "path must remain contained: {}",
            relative.display()
        ))
        .into());
    }
    Ok(base.join(relative))
}

fn resolve_command(recipe_dir: &Path, command: &str) -> Result<PathBuf, crate::BoxError> {
    if command.contains('/') || command.contains('\\') {
        let path = contained(recipe_dir, Path::new(command))?;
        if path.is_file() {
            return Ok(path);
        }
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("build command not found: {}", path.display()),
        )
        .into());
    }
    let path = std::env::var_os("PATH").ok_or_else(|| {
        std::io::Error::other("PATH is unavailable while resolving build command")
    })?;
    #[cfg(windows)]
    let suffixes = std::iter::once(String::new())
        .chain(
            std::env::var("PATHEXT")
                .unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".into())
                .split(';')
                .map(str::to_ascii_lowercase)
                .collect::<Vec<_>>(),
        )
        .collect::<Vec<_>>();
    #[cfg(not(windows))]
    let suffixes = vec![String::new()];
    let found = std::env::split_paths(&path)
        .flat_map(|directory| {
            suffixes
                .iter()
                .map(move |suffix| directory.join(format!("{command}{suffix}")))
        })
        .find(|candidate| candidate.is_file());
    if let Some(found) = found {
        return Ok(std::path::absolute(found)?);
    }
    Err(std::io::Error::other(format!(
        "build command `{command}` was not found in PATH; install it or select an explicit recipe-relative driver"
    )).into())
}

fn validate_runtime_assertions(
    runtime: bool,
    checks: &[Verification],
) -> Result<(), crate::BoxError> {
    let count = checks
        .iter()
        .filter(|check| matches!(check, Verification::LoadedRuntime { .. }))
        .count();
    if count == usize::from(runtime) {
        return Ok(());
    }
    let message = if runtime {
        "outputs.runtime requires exactly one loaded_runtime assertion"
    } else {
        "loaded_runtime requires outputs.runtime"
    };
    Err(std::io::Error::other(message).into())
}

#[cfg(test)]
#[path = "../../tests/examples/toolhost/recipe.rs"]
mod tests;
