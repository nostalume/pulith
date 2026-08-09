use super::{
    Binding, Boot, ManagerObservation, NormalizedDecl, Registration, Runtime, ServiceError,
    ServiceRoot,
};
use pulith::Remove;
use std::os::unix::fs::MetadataExt;
use std::path::Path;

pub fn secure_leaf(path: &Path) -> Result<(), ServiceError> {
    secure(path, "root", |metadata| {
        metadata.file_type().is_dir() && metadata.uid() == 0 && metadata.mode() & 0o022 == 0
    })
}

pub fn secure_ancestor(path: &Path) -> Result<(), ServiceError> {
    secure(path, "ancestor", |metadata| {
        metadata.file_type().is_dir()
            && metadata.uid() == 0
            && (metadata.mode() & 0o022 == 0 || metadata.mode() & 0o1000 != 0)
    })
}

pub fn secure_input(path: &Path) -> Result<(), ServiceError> {
    secure(path, "input", |metadata| {
        (metadata.file_type().is_dir() || metadata.file_type().is_file())
            && metadata.uid() == 0
            && metadata.mode() & 0o022 == 0
    })
}

fn secure(
    path: &Path,
    role: &str,
    admitted: impl FnOnce(&std::fs::Metadata) -> bool,
) -> Result<(), ServiceError> {
    let metadata = std::fs::symlink_metadata(path)
        .map_err(|error| ServiceError::invalid(format!("inspect service {role}: {error}")))?;
    if !metadata.file_type().is_symlink() && admitted(&metadata) {
        Ok(())
    } else {
        Err(ServiceError::invalid(format!(
            "service {role} is not root-owned and protected: {}",
            path.display()
        )))
    }
}

pub fn observe(
    root: &ServiceRoot,
    declaration: &NormalizedDecl,
) -> Result<ManagerObservation, ServiceError> {
    let unit = unit_path(root, declaration);
    let registration = match std::fs::read_to_string(&unit) {
        Ok(text) => binding_from_unit(root, declaration, &text)
            .map(|binding| text == render_definition(root, declaration, &binding))
            .is_ok_and(|exact| exact)
            .then_some(Registration::Exact)
            .unwrap_or(Registration::Conflict),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Registration::Missing,
        Err(error) => return Err(ServiceError::invalid(format!("read systemd unit: {error}"))),
    };
    if registration != Registration::Exact {
        return Ok(ManagerObservation {
            registration,
            boot: Boot::Disabled,
            runtime: Runtime::Stopped,
        });
    }
    let output = systemctl(
        root,
        [
            "show",
            declaration.id.as_str(),
            "--property=LoadState",
            "--property=UnitFileState",
            "--property=ActiveState",
        ],
    )?;
    let text = String::from_utf8(output)
        .map_err(|_| ServiceError::invalid("systemctl output is not UTF-8"))?;
    Ok(parse_observation(&text))
}

pub fn binding(root: &ServiceRoot, declaration: &NormalizedDecl) -> Result<Binding, ServiceError> {
    let text = std::fs::read_to_string(unit_path(root, declaration))
        .map_err(|error| ServiceError::invalid(format!("read systemd unit: {error}")))?;
    let binding = binding_from_unit(root, declaration, &text)?;
    (text == render_definition(root, declaration, &binding))
        .then_some(binding)
        .ok_or_else(|| ServiceError::invalid("systemd unit conflicts"))
}

pub(super) fn parse_observation(text: &str) -> ManagerObservation {
    let value = |key| {
        text.lines()
            .filter_map(|line| line.split_once('='))
            .find_map(|(name, value)| (name == key).then_some(value))
            .unwrap_or("")
    };
    ManagerObservation {
        registration: (value("LoadState") == "loaded")
            .then_some(Registration::Exact)
            .unwrap_or(Registration::Broken),
        boot: match value("UnitFileState") {
            "enabled" => Boot::Enabled,
            "linked" | "disabled" => Boot::Disabled,
            _ => Boot::Conflict,
        },
        runtime: match value("ActiveState") {
            "inactive" => Runtime::Stopped,
            "activating" => Runtime::Starting,
            "active" => Runtime::Running,
            "deactivating" => Runtime::Stopping,
            _ => Runtime::Failed,
        },
    }
}

pub fn install(
    root: &ServiceRoot,
    declaration: &NormalizedDecl,
    binding: &Binding,
) -> Result<(), ServiceError> {
    let path = unit_path(root, declaration);
    if path.exists() {
        return Err(ServiceError::invalid("systemd unit already exists"));
    }
    root.write_unit(declaration, binding)?;
    link(root, declaration)?;
    Ok(())
}

pub fn repair(root: &ServiceRoot, declaration: &NormalizedDecl) -> Result<(), ServiceError> {
    link(root, declaration)?;
    systemctl(root, ["daemon-reload"]).map(drop)
}

pub fn enable(root: &ServiceRoot, declaration: &NormalizedDecl) -> Result<(), ServiceError> {
    systemctl(root, ["enable", declaration.id.as_str()]).map(drop)
}
pub fn disable(root: &ServiceRoot, declaration: &NormalizedDecl) -> Result<(), ServiceError> {
    systemctl(root, ["disable", declaration.id.as_str()])?;
    link(root, declaration)?;
    Ok(())
}

pub fn rebind(
    root: &ServiceRoot,
    declaration: &NormalizedDecl,
    binding: &Binding,
) -> Result<(), ServiceError> {
    let path = unit_path(root, declaration);
    let target = ServiceError::effect(pulith::local::LocalTarget::new(path.parent().unwrap()))?;
    ServiceError::effect(target.remove())?;
    root.write_unit(declaration, binding)?;
    systemctl(root, ["daemon-reload"]).map(drop)
}
pub fn start(root: &ServiceRoot, declaration: &NormalizedDecl) -> Result<(), ServiceError> {
    systemctl(root, ["start", declaration.id.as_str()]).map(drop)
}
pub fn stop(root: &ServiceRoot, declaration: &NormalizedDecl) -> Result<(), ServiceError> {
    systemctl(root, ["stop", declaration.id.as_str()]).map(drop)
}

pub fn remove(root: &ServiceRoot, declaration: &NormalizedDecl) -> Result<(), ServiceError> {
    systemctl(root, ["disable", declaration.id.as_str()])?;
    let path = unit_path(root, declaration);
    let target = ServiceError::effect(pulith::local::LocalTarget::new(path.parent().unwrap()))?;
    ServiceError::effect(target.remove())?;
    Ok(())
}

fn link(root: &ServiceRoot, declaration: &NormalizedDecl) -> Result<(), ServiceError> {
    let path = unit_path(root, declaration);
    let path = path
        .to_str()
        .ok_or_else(|| ServiceError::invalid("unit path is not UTF-8"))?;
    systemctl(root, ["link", path]).map(drop)
}

fn systemctl<'a>(
    root: &ServiceRoot,
    args: impl IntoIterator<Item = &'a str>,
) -> Result<Vec<u8>, ServiceError> {
    use pulith::process::WorktreeProcess;
    let program = ["/usr/bin/systemctl", "/bin/systemctl"]
        .into_iter()
        .map(std::path::PathBuf::from)
        .find(|path| path.is_file())
        .ok_or_else(|| ServiceError::invalid("systemctl is unavailable"))?;
    let process = ServiceError::effect(WorktreeProcess::new(
        program,
        &root.0,
        std::time::Duration::from_secs(35),
    ))?
    .with_arguments(
        std::iter::once("--system")
            .chain(args)
            .map(std::ffi::OsString::from),
    );
    let result = ServiceError::effect(process.execute())?;
    Ok(result.diagnostics.stdout.unwrap_or_default())
}

fn unit_path(root: &ServiceRoot, declaration: &NormalizedDecl) -> std::path::PathBuf {
    root.directory(declaration)
        .join("systemd")
        .join(format!("{}.service", declaration.id.as_str()))
}

impl ServiceRoot {
    fn write_unit(
        &self,
        declaration: &NormalizedDecl,
        binding: &Binding,
    ) -> Result<(), ServiceError> {
        let path = unit_path(self, declaration);
        let target = ServiceError::effect(pulith::local::LocalTarget::new(path.parent().unwrap()))?;
        let stage = ServiceError::effect(target.stage())?;
        let stage = ServiceError::effect(stage.write_file(
            render_definition(self, declaration, binding).as_bytes(),
            path.file_name().unwrap(),
        ))?;
        ServiceError::effect(stage.publish(target))?;
        Ok(())
    }
}

pub(super) fn render_definition(
    root: &ServiceRoot,
    declaration: &NormalizedDecl,
    binding: &Binding,
) -> String {
    let host = binding.host(declaration);
    let config = root.declaration(declaration);
    format!(
        "# toolhost-release={}\n[Unit]\nDescription=toolhost {}\n\n[Service]\nType=notify\nDynamicUser=yes\nNoNewPrivileges=yes\nCapabilityBoundingSet=\nAmbientCapabilities=\nProtectSystem=strict\nProtectHome=yes\nPrivateTmp=yes\nExecStart=\"{}\" \"{}\"\n\n[Install]\nWantedBy=multi-user.target\n",
        binding.relative(root).display(),
        declaration.id.as_str(),
        host.display(),
        config.display()
    )
}

fn binding_from_unit(
    root: &ServiceRoot,
    declaration: &NormalizedDecl,
    text: &str,
) -> Result<Binding, ServiceError> {
    let relative = text
        .lines()
        .next()
        .and_then(|line| line.strip_prefix("# toolhost-release="))
        .ok_or_else(|| ServiceError::invalid("systemd unit has no release binding"))?;
    Binding::admit(root, root.0.join("installs").join(relative), declaration)
}
