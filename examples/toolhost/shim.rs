use std::path::Path;

fn main() {
    match dispatch() {
        Ok(code) => std::process::exit(code),
        Err(error) => {
            eprintln!("toolhost shim: {error}");
            std::process::exit(127);
        }
    }
}

type BoxError = Box<dyn std::error::Error + Send + Sync + 'static>;

fn dispatch() -> Result<i32, BoxError> {
    let own = std::env::current_exe()?;
    let target = selected_target(&own)?;
    if !target.is_file() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("selected release binary is missing: {}", target.display()),
        )
        .into());
    }
    let mut command = std::process::Command::new(target);
    command.args(std::env::args_os().skip(1));
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        Err(command.exec().into())
    }
    #[cfg(windows)]
    {
        let status = command.status()?;
        Ok(status.code().unwrap_or(1))
    }
}

fn selected_target(own: &Path) -> Result<std::path::PathBuf, BoxError> {
    let name = own
        .file_stem()
        .ok_or_else(|| std::io::Error::other("dispatcher has no filename"))?;
    let release = own
        .parent()
        .and_then(Path::parent)
        .ok_or_else(|| std::io::Error::other("dispatcher is outside a release"))?;
    let mut target = release.join("bin").join(name);
    if cfg!(windows) {
        target.set_extension("exe");
    }
    Ok(target)
}

#[cfg(test)]
#[path = "../../tests/examples/toolhost/shim.rs"]
mod tests;
