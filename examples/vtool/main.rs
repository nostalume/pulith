mod manifest;
mod realize;

use std::ffi::OsString;
use std::path::PathBuf;
use std::process::ExitCode;

type BoxError = Box<dyn std::error::Error + Send + Sync + 'static>;

fn main() -> ExitCode {
    match Command::parse(std::env::args_os().skip(1)).and_then(Command::execute) {
        Ok(()) => ExitCode::SUCCESS,
        Err(CliError::Usage(message)) => {
            eprintln!("vtool: {message}");
            ExitCode::from(2)
        }
        Err(CliError::Operation(error)) => {
            eprintln!("vtool: {error}");
            ExitCode::FAILURE
        }
        Err(CliError::RepairFailed) => ExitCode::FAILURE,
    }
}

enum Command {
    Plan(PathBuf, PathBuf),
    Install(PathBuf, PathBuf),
    Activate(PathBuf, PathBuf),
    Deactivate(PathBuf, PathBuf),
    Repair(PathBuf, PathBuf, usize),
}

enum CliError {
    Usage(String),
    Operation(BoxError),
    RepairFailed,
}

impl Command {
    fn parse(args: impl IntoIterator<Item = OsString>) -> Result<Self, CliError> {
        let mut args = args.into_iter();
        let verb = args
            .next()
            .ok_or_else(|| CliError::Usage("missing verb".into()))?;
        let verb = verb
            .to_str()
            .ok_or_else(|| CliError::Usage("verb is not Unicode".into()))?;
        if args.next().as_deref() != Some(std::ffi::OsStr::new("--root")) {
            return Err(CliError::Usage(
                "expected --root <absolute-root> after verb".into(),
            ));
        }
        let root = PathBuf::from(
            args.next()
                .ok_or_else(|| CliError::Usage("--root requires a path".into()))?,
        );
        if !root.is_absolute() {
            return Err(CliError::Usage("--root must be absolute".into()));
        }
        let manifest = PathBuf::from(
            args.next()
                .ok_or_else(|| CliError::Usage(format!("{verb} requires a manifest")))?,
        );
        match verb {
            "plan" => return Ok(Self::Plan(root, manifest)),
            "install" => return Ok(Self::Install(root, manifest)),
            "activate" => return Ok(Self::Activate(root, manifest)),
            "deactivate" => return Ok(Self::Deactivate(root, manifest)),
            "repair" => {}
            _ => return Err(CliError::Usage(format!("unknown verb: {verb}"))),
        }
        let attempts = match (args.next(), args.next()) {
            (None, None) => 3,
            (Some(flag), Some(value)) if flag == "--attempts" => value
                .to_str()
                .and_then(|value| value.parse().ok())
                .filter(|value| *value > 0)
                .ok_or_else(|| CliError::Usage("--attempts needs a positive number".into()))?,
            _ => return Err(CliError::Usage("repair accepts only --attempts N".into())),
        };
        Ok(Self::Repair(root, manifest, attempts))
    }

    fn execute(self) -> Result<(), CliError> {
        let (root, path) = match &self {
            Self::Plan(root, manifest)
            | Self::Install(root, manifest)
            | Self::Activate(root, manifest)
            | Self::Deactivate(root, manifest)
            | Self::Repair(root, manifest, _) => (root, manifest),
        };
        let resolved = manifest::Manifest::load(path)
            .map_err(CliError::Operation)?
            .resolve(root)
            .map_err(|error| CliError::Operation(error.into()))?;
        match self {
            Self::Plan(..) => {
                println!(
                    "plan: {}@{}",
                    resolved.manifest.name.as_str(),
                    resolved.manifest.version.as_str()
                );
                println!("source={}", describe_source(&resolved.source));
                println!("target={}", resolved.target.display());
            }
            Self::Install(..) => {
                resolved.install(root).map_err(CliError::Operation)?;
                println!("installed");
            }
            Self::Activate(..) => {
                let change = resolved.activate(root).map_err(CliError::Operation)?;
                println!("outcome={change:?}");
            }
            Self::Deactivate(..) => {
                resolved.deactivate(root).map_err(CliError::Operation)?;
                println!("deactivated");
            }
            Self::Repair(_, _, attempts) => {
                let report = realize::repair(
                    &resolved,
                    root,
                    attempts,
                    std::time::Duration::from_millis(100),
                )
                .map_err(|error| CliError::Operation(error.into()))?;
                let name = resolved.manifest.name.as_str();
                let version = resolved.manifest.version.as_str();
                for diagnostic in report.diagnostics {
                    eprintln!("vtool: {name}@{version} {diagnostic}");
                }
                use realize::RepairOutcome;
                match report.outcome {
                    RepairOutcome::NotInstalled => println!("{name}@{version}: not installed"),
                    RepairOutcome::Deactivated => println!("{name}@{version}: deactivated"),
                    RepairOutcome::Unchanged(evidence) => {
                        println!("{name}@{version}: unchanged; {evidence}");
                    }
                    RepairOutcome::Repaired(evidence) => {
                        println!("{name}@{version}: repaired; {evidence}");
                    }
                    RepairOutcome::Failed { published } => {
                        if let Some(evidence) = published {
                            println!("{name}@{version}: failed after publication; {evidence}");
                        } else {
                            println!("{name}@{version}: failed");
                        }
                        return Err(CliError::RepairFailed);
                    }
                }
            }
        }
        Ok(())
    }
}

fn describe_source(source: &manifest::Source) -> String {
    match source {
        manifest::Source::Url { url, .. } => url.as_str().to_string(),
        manifest::Source::Local { path, .. } => path.display().to_string(),
    }
}

#[cfg(test)]
#[path = "../../tests/examples/vtool/main.rs"]
mod tests;
