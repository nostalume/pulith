mod recipe;
#[allow(dead_code)]
mod service;

use pulith::local::LocalTarget;
use pulith::{Link, Unlink};
use std::ffi::OsString;
use std::path::PathBuf;
use std::process::ExitCode;

type BoxError = Box<dyn std::error::Error + Send + Sync + 'static>;

fn main() -> ExitCode {
    let result = Command::parse(std::env::args_os().skip(1))
        .and_then(|command| command.execute().map_err(CliError::Operation));
    match result {
        Ok(code) => ExitCode::from(code),
        Err(CliError::Usage(message)) => {
            eprintln!("toolhost: {message}");
            ExitCode::from(2)
        }
        Err(CliError::Operation(error)) => {
            eprintln!("toolhost: {error}");
            ExitCode::FAILURE
        }
    }
}

enum Command {
    Install(PathBuf, PathBuf),
    Activate(PathBuf, String, String),
    Deactivate(PathBuf),
    Env(PathBuf),
    Run(PathBuf, OsString, Vec<OsString>),
    Service(ServiceVerb, PathBuf, PathBuf),
}

#[derive(Clone, Copy)]
enum ServiceVerb {
    Install,
    Rebind,
    Enable,
    Start,
    Restart,
    Status,
    Stop,
    Disable,
    Remove,
}

#[derive(Debug)]
enum CliError {
    Usage(String),
    Operation(BoxError),
}

impl Command {
    fn parse(args: impl IntoIterator<Item = OsString>) -> Result<Self, CliError> {
        let mut args = args.into_iter();
        let verb = required(&mut args, "missing verb")?;
        let verb = verb
            .to_str()
            .ok_or_else(|| CliError::Usage("verb is not Unicode".into()))?;
        if verb == "service" {
            let action = ServiceVerb::parse(required(&mut args, "service requires a verb")?)?;
            if args.next().as_deref() != Some(std::ffi::OsStr::new("--root")) {
                return Err(CliError::Usage("expected --root after service verb".into()));
            }
            let root = PathBuf::from(required(&mut args, "--root requires a path")?);
            if !root.is_absolute() {
                return Err(CliError::Usage("--root must be absolute".into()));
            }
            let declaration = PathBuf::from(required(&mut args, "service requires a declaration")?);
            if args.next().is_some() {
                return Err(CliError::Usage("unexpected service argument".into()));
            }
            return Ok(Self::Service(action, root, declaration));
        }
        if args.next().as_deref() != Some(std::ffi::OsStr::new("--root")) {
            return Err(CliError::Usage(
                "expected --root <absolute-root> after verb".into(),
            ));
        }
        let root = PathBuf::from(required(&mut args, "--root requires a path")?);
        if !root.is_absolute() {
            return Err(CliError::Usage("--root must be absolute".into()));
        }
        match verb {
            "install" => Ok(Self::Install(
                root,
                PathBuf::from(required(&mut args, "install requires a recipe")?),
            )),
            "activate" => Ok(Self::Activate(
                root,
                unicode(required(&mut args, "activate requires name")?)?,
                unicode(required(&mut args, "activate requires version")?)?,
            )),
            "deactivate" => Ok(Self::Deactivate(root)),
            "env" => Ok(Self::Env(root)),
            "run" => {
                if args.next().as_deref() != Some(std::ffi::OsStr::new("--")) {
                    return Err(CliError::Usage("run requires -- before the command".into()));
                }
                Ok(Self::Run(
                    root,
                    required(&mut args, "run requires a command")?,
                    args.collect(),
                ))
            }
            _ => Err(CliError::Usage(format!("unknown verb: {verb}"))),
        }
    }

    fn execute(self) -> Result<u8, BoxError> {
        match self {
            Self::Install(root, path) => {
                match recipe::Recipe::load(&path)?
                    .resolve(&path)?
                    .install(&root)?
                {
                    recipe::InstallOutcome::NoBuild => println!("no-build"),
                    recipe::InstallOutcome::Published { release, process } => println!(
                        "published={} program={} elapsed_ms={}",
                        release.display(),
                        process.evidence.program.display(),
                        process.evidence.elapsed.as_millis()
                    ),
                }
                Ok(0)
            }
            Self::Activate(root, name, version) => {
                recipe::component(&name, "name")?;
                recipe::component(&version, "version")?;
                let release = root.join("installs").join(name).join(version);
                LocalTarget::new(release)?.link_root(&root.join("current"))?;
                Ok(0)
            }
            Self::Deactivate(root) => {
                LocalTarget::new(root.join("current"))?.unlink()?;
                Ok(0)
            }
            Self::Env(root) => {
                let plan = EnvironmentPlan::new(root);
                println!("TOOLHOST_HOME={}", plan.home.display());
                println!("PATH_PREPEND={}", plan.path_prepend.display());
                Ok(0)
            }
            Self::Run(root, program, args) => {
                let status = EnvironmentPlan::new(root).run(program, args)?;
                Ok(status
                    .code()
                    .and_then(|code| u8::try_from(code).ok())
                    .unwrap_or(1))
            }
            Self::Service(verb, root, path) => {
                let declaration = service::ServiceDecl::load(&path)?.normalize()?;
                let service =
                    service::Service::new(service::ServiceRoot::admit(root)?, declaration);
                match verb {
                    ServiceVerb::Status => println!("{}", service.status()?),
                    ServiceVerb::Install => println!("{}", service.install()?),
                    ServiceVerb::Rebind => println!("{}", service.rebind()?),
                    ServiceVerb::Enable => println!("{}", service.enable()?),
                    ServiceVerb::Start => println!("{}", service.start()?),
                    ServiceVerb::Restart => println!("{}", service.restart()?),
                    ServiceVerb::Stop => println!("{}", service.stop()?),
                    ServiceVerb::Disable => println!("{}", service.disable()?),
                    ServiceVerb::Remove => println!("{}", service.remove()?),
                }
                Ok(0)
            }
        }
    }
}

impl ServiceVerb {
    fn parse(value: OsString) -> Result<Self, CliError> {
        match value.to_str() {
            Some("install") => Ok(Self::Install),
            Some("rebind") => Ok(Self::Rebind),
            Some("enable") => Ok(Self::Enable),
            Some("start") => Ok(Self::Start),
            Some("restart") => Ok(Self::Restart),
            Some("status") => Ok(Self::Status),
            Some("stop") => Ok(Self::Stop),
            Some("disable") => Ok(Self::Disable),
            Some("remove") => Ok(Self::Remove),
            Some(value) => Err(CliError::Usage(format!("unknown service verb: {value}"))),
            None => Err(CliError::Usage("service verb is not Unicode".into())),
        }
    }
}

fn required(
    args: &mut impl Iterator<Item = OsString>,
    message: &'static str,
) -> Result<OsString, CliError> {
    args.next().ok_or_else(|| CliError::Usage(message.into()))
}

fn unicode(value: OsString) -> Result<String, CliError> {
    value
        .into_string()
        .map_err(|_| CliError::Usage("name and version must be Unicode".into()))
}

#[derive(Debug, Eq, PartialEq)]
struct EnvironmentPlan {
    home: PathBuf,
    path_prepend: PathBuf,
}

impl EnvironmentPlan {
    fn new(home: PathBuf) -> Self {
        let path_prepend = home.join("current/shims");
        Self { home, path_prepend }
    }

    fn run(
        self,
        command: OsString,
        args: Vec<OsString>,
    ) -> Result<std::process::ExitStatus, BoxError> {
        let current = std::env::var_os("PATH").unwrap_or_default();
        let path = std::env::join_paths(
            std::iter::once(self.path_prepend).chain(std::env::split_paths(&current)),
        )?;
        Ok(std::process::Command::new(command)
            .args(args)
            .env("TOOLHOST_HOME", self.home)
            .env("PATH", path)
            .status()?)
    }
}

#[cfg(test)]
#[path = "../../tests/examples/toolhost/main.rs"]
mod tests;
