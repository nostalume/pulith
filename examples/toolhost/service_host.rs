#[allow(dead_code)]
#[path = "service.rs"]
mod service;

use pulith::process::{EnvVars, ManagedProcess, ProcessObservation};
use service::{NormalizedDecl, ServiceDecl};
use std::ffi::OsString;
use std::fmt::Display;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

#[cfg(target_os = "linux")]
#[path = "service_host/systemd.rs"]
mod platform_host;
#[cfg(windows)]
#[path = "service_host/windows.rs"]
mod platform_host;
#[cfg(not(any(target_os = "linux", windows)))]
mod platform_host {
    pub(super) struct Control;

    impl Control {
        pub(super) fn arm() -> Result<Self, String> {
            Err("the platform has no supported service manager".into())
        }

        pub(super) fn ready(&self) -> Result<(), String> {
            Err("the platform has no supported service manager".into())
        }

        pub(super) fn stop_requested(&self) -> bool {
            false
        }
    }
}
use platform_host::Control;

#[cfg(not(windows))]
fn main() -> ExitCode {
    run_from_args()
}

#[cfg(windows)]
fn main() -> ExitCode {
    platform_host::dispatch()
}

fn run_from_args() -> ExitCode {
    let result = std::env::args_os()
        .nth(1)
        .ok_or_else(|| std::io::Error::other("missing service declaration"))
        .and_then(|path| Host::load(path.into()).map_err(std::io::Error::other))
        .and_then(|host| host.run().map_err(std::io::Error::other));
    match result {
        Ok(code) => code,
        Err(error) => {
            eprintln!("toolhost-service: {error}");
            ExitCode::FAILURE
        }
    }
}

struct Host {
    declaration: NormalizedDecl,
    release: PathBuf,
    home: PathBuf,
}

impl Host {
    fn load(path: PathBuf) -> Result<Self, String> {
        let executable = text(std::env::current_exe())?;
        Self::load_from(executable, path)
    }

    fn load_from(executable: PathBuf, path: PathBuf) -> Result<Self, String> {
        let executable = text(executable.canonicalize())?;
        let release = executable
            .parent()
            .and_then(Path::parent)
            .ok_or("service host is not release-local")?
            .to_path_buf();
        let home = release
            .parent()
            .and_then(Path::parent)
            .and_then(Path::parent)
            .ok_or("service release is outside an installs root")?
            .to_path_buf();
        let declaration = text(ServiceDecl::load(&path).and_then(ServiceDecl::normalize))?;
        let expected = format!("{}{}", declaration.payload(), std::env::consts::EXE_SUFFIX);
        if executable.file_name() != Some(expected.as_ref()) {
            return Err("service host name does not match payload".into());
        }
        Ok(Self {
            declaration,
            release,
            home,
        })
    }

    fn run(self) -> Result<ExitCode, String> {
        let (payload, args, environment) = self.declaration.into_launch();
        let runtime = self.release.join("private-runtime");
        let mut environment = environment
            .into_iter()
            .map(|(key, value)| (OsString::from(key), OsString::from(value)))
            .collect::<Vec<_>>();
        environment.push(("TOOLHOST_HOME".into(), self.home.into_os_string()));
        #[cfg(windows)]
        environment.push(("PATH".into(), runtime.into_os_string()));
        #[cfg(target_os = "linux")]
        environment.push(("LD_LIBRARY_PATH".into(), runtime.into_os_string()));
        let control = Control::arm()?;
        let process = text(ManagedProcess::new(
            self.release
                .join("bin")
                .join(format!("{payload}{}", std::env::consts::EXE_SUFFIX)),
            self.release,
        ))?
        .with_arguments(args.into_iter().map(OsString::from));
        let mut session = text(process.start_in_environment(text(EnvVars::new(environment))?))?;
        match text(session.observe())? {
            ProcessObservation::Exited(status) => return Ok(exit_code(status.code())),
            ProcessObservation::Running => control.ready()?,
        }
        loop {
            if control.stop_requested() {
                return session
                    .stop_within(Duration::from_secs(30))
                    .map(|_| ExitCode::SUCCESS)
                    .map_err(|error| error.to_string());
            }
            match text(session.observe())? {
                ProcessObservation::Running => std::thread::sleep(Duration::from_millis(50)),
                ProcessObservation::Exited(status) => return Ok(exit_code(status.code())),
            }
        }
    }
}

fn exit_code(code: Option<i32>) -> ExitCode {
    ExitCode::from(code.and_then(|value| u8::try_from(value).ok()).unwrap_or(1))
}

fn text<T, E: Display>(result: Result<T, E>) -> Result<T, String> {
    result.map_err(|error| error.to_string())
}

#[cfg(test)]
#[path = "../../tests/examples/toolhost/service_host.rs"]
mod tests;
