use serde::Deserialize;
use std::collections::BTreeMap;
use std::fmt::{self, Display};
use std::path::{Component, Path, PathBuf};

#[path = "service/lifecycle.rs"]
mod lifecycle;
#[cfg(unix)]
#[path = "service/systemd.rs"]
mod platform;
pub(super) use lifecycle::ManagerObservation;
#[allow(unused_imports)]
pub use lifecycle::{Boot, Change, Definition, Observation, Registration, Runtime, Service};
#[cfg(test)]
use lifecycle::{RemovalPlan, removal_plan};
#[cfg(windows)]
#[path = "service/windows.rs"]
mod platform;

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub(crate) enum AcceptedEffect {
    BindingChangeRequested,
    DeletionRequested,
}

#[derive(Debug)]
pub(crate) enum Failure {
    Invalid,
    Conflict(Observation),
    Authority,
    Operation,
    Partial(AcceptedEffect, Observation),
}

#[derive(Debug)]
pub struct ServiceError(Failure, std::io::Error);

impl ServiceError {
    fn invalid(message: impl Into<String>) -> Self {
        Self(Failure::Invalid, std::io::Error::other(message.into()))
    }

    fn effect<T, E: Display>(result: Result<T, E>) -> Result<T, Self> {
        result.map_err(|error| Self::operation("", error))
    }

    fn conflict(subject: &'static str, observation: Observation) -> Self {
        Self(
            Failure::Conflict(observation),
            Self::cause(subject, observation),
        )
    }

    fn operation(action: &'static str, source: impl Display) -> Self {
        Self(Failure::Operation, Self::cause(action, source))
    }

    fn os(action: &'static str, source: std::io::Error) -> Self {
        let denied = source.kind() == std::io::ErrorKind::PermissionDenied
            || source.raw_os_error() == Some(5);
        let source = Self::cause(action, source);
        if denied {
            Self(Failure::Authority, source)
        } else {
            Self(Failure::Operation, source)
        }
    }

    fn partial(
        accepted: AcceptedEffect,
        observation: Observation,
        action: &'static str,
        source: impl Display,
    ) -> Self {
        Self(
            Failure::Partial(accepted, observation),
            Self::cause(
                action,
                format_args!("{source}; accepted={accepted} {observation}"),
            ),
        )
    }

    fn cause(action: &'static str, source: impl Display) -> std::io::Error {
        let message = if action.is_empty() {
            source.to_string()
        } else {
            format!("{action}: {source}")
        };
        std::io::Error::other(message)
    }
}

impl Display for ServiceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        Display::fmt(&self.1, formatter)
    }
}

impl Display for AcceptedEffect {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::BindingChangeRequested => "binding-change-requested",
            Self::DeletionRequested => "deletion-requested",
        })
    }
}

impl std::error::Error for ServiceError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.1)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServiceDecl {
    schema: u8,
    id: String,
    payload: String,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default)]
    environment: BTreeMap<String, String>,
}

pub struct NormalizedDecl {
    id: ServiceId,
    payload: String,
    args: Vec<String>,
    environment: Vec<(String, String)>,
    bytes: Vec<u8>,
}

pub struct ServiceId(String);

pub struct ServiceRoot(PathBuf);

pub struct Binding {
    pub release: PathBuf,
}

impl ServiceDecl {
    pub fn load(path: &Path) -> Result<Self, ServiceError> {
        let text = std::fs::read_to_string(path)
            .map_err(|error| ServiceError::invalid(format!("read service declaration: {error}")))?;
        Self::parse(&text)
    }

    pub fn parse(text: &str) -> Result<Self, ServiceError> {
        toml::from_str(text)
            .map_err(|error| ServiceError::invalid(format!("parse service declaration: {error}")))
    }

    pub fn normalize(self) -> Result<NormalizedDecl, ServiceError> {
        if self.schema != 1 {
            return Err(ServiceError::invalid("service schema must be 1"));
        }
        let id = ServiceId::new(self.id)?;
        one_component(&self.payload, "payload")?;
        if self.args.iter().any(|argument| argument.contains('\0')) {
            return Err(ServiceError::invalid("service argument contains NUL"));
        }
        let environment = normalize_environment(self.environment)?;
        let bytes = render(&id, &self.payload, &self.args, &environment).into_bytes();
        Ok(NormalizedDecl {
            id,
            payload: self.payload,
            args: self.args,
            environment,
            bytes,
        })
    }
}

impl NormalizedDecl {
    pub fn payload(&self) -> &str {
        &self.payload
    }

    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub fn into_launch(self) -> (String, Vec<String>, Vec<(String, String)>) {
        (self.payload, self.args, self.environment)
    }
}

impl ServiceId {
    fn new(value: String) -> Result<Self, ServiceError> {
        let bytes = value.as_bytes();
        let valid = (1..=63).contains(&bytes.len())
            && bytes[0].is_ascii_lowercase()
            && bytes[1..]
                .iter()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || *byte == b'-');
        valid
            .then_some(Self(value))
            .ok_or_else(|| ServiceError::invalid("service id must match [a-z][a-z0-9-]{0,62}"))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl ServiceRoot {
    pub fn admit(path: PathBuf) -> Result<Self, ServiceError> {
        if !path.is_absolute() {
            return Err(ServiceError::invalid("service root must be absolute"));
        }
        let root = Self(
            path.canonicalize()
                .map_err(|error| ServiceError::invalid(format!("admit service root: {error}")))?,
        );
        if !root.0.is_dir() {
            return Err(ServiceError::invalid("service root must be a directory"));
        }
        Ok(root)
    }

    pub fn recheck(&self) -> Result<(), ServiceError> {
        platform::secure_leaf(&self.0)?;
        self.0
            .ancestors()
            .skip(1)
            .try_for_each(platform::secure_ancestor)
    }

    fn admit_exposure(
        &self,
        binding: &Binding,
        declaration: &NormalizedDecl,
    ) -> Result<(), ServiceError> {
        self.recheck()?;
        for entry in walkdir::WalkDir::new(&binding.release) {
            let entry = entry.map_err(|error| {
                ServiceError::invalid(format!("inspect service release: {error}"))
            })?;
            platform::secure_input(entry.path())?;
        }
        let declaration = self.declaration(declaration);
        if declaration.exists() {
            platform::secure_input(&declaration)?;
        }
        Ok(())
    }

    fn active_binding(&self, declaration: &NormalizedDecl) -> Result<Binding, ServiceError> {
        let release =
            self.0.join("current").canonicalize().map_err(|error| {
                ServiceError::invalid(format!("resolve active release: {error}"))
            })?;
        Binding::admit(self, release, declaration)
    }

    fn directory(&self, declaration: &NormalizedDecl) -> PathBuf {
        self.0.join("services").join(declaration.id.as_str())
    }

    fn declaration(&self, declaration: &NormalizedDecl) -> PathBuf {
        self.directory(declaration).join("service.toml")
    }

    fn observe(&self, declaration: &NormalizedDecl) -> Result<Definition, ServiceError> {
        let directory = self.directory(declaration);
        let metadata = match std::fs::symlink_metadata(&directory) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Definition::Missing);
            }
            Err(error) => {
                return Err(ServiceError::invalid(format!(
                    "observe definition: {error}"
                )));
            }
        };
        if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
            return Ok(Definition::Broken);
        }
        let path = self.declaration(declaration);
        match std::fs::read(path) {
            Ok(bytes) if bytes == declaration.bytes => Ok(Definition::Exact),
            Ok(_) => Ok(Definition::Conflict),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Definition::Broken),
            Err(error) => Err(ServiceError::invalid(format!("read definition: {error}"))),
        }
    }

    fn publish(&self, declaration: &NormalizedDecl) -> Result<bool, ServiceError> {
        match self.observe(declaration)? {
            Definition::Exact => return Ok(false),
            Definition::Missing => {}
            state => {
                return Err(ServiceError::invalid(format!(
                    "definition is {}",
                    lifecycle::word(state)
                )));
            }
        }
        let target =
            ServiceError::effect(pulith::local::LocalTarget::new(self.directory(declaration)))?;
        let stage = ServiceError::effect(target.stage())?;
        let stage = ServiceError::effect(stage.write_file(declaration.bytes(), "service.toml"))?;
        let stage = ServiceError::effect(stage.write_file([], "state/lock"))?;
        ServiceError::effect(stage.publish(target))?;
        Ok(true)
    }
}

impl Binding {
    fn admit(
        root: &ServiceRoot,
        release: PathBuf,
        declaration: &NormalizedDecl,
    ) -> Result<Self, ServiceError> {
        let release = release
            .canonicalize()
            .map_err(|error| ServiceError::invalid(format!("admit service release: {error}")))?;
        let relative = release
            .strip_prefix(root.0.join("installs"))
            .map_err(|_| ServiceError::invalid("service release is outside installs"))?;
        let mut components = relative.components();
        if !matches!(components.next(), Some(Component::Normal(_)))
            || !matches!(components.next(), Some(Component::Normal(_)))
            || components.next().is_some()
        {
            return Err(ServiceError::invalid(
                "service release must be installs/<name>/<version>",
            ));
        }
        if relative
            .to_str()
            .is_none_or(|relative| relative.chars().any(char::is_control))
        {
            return Err(ServiceError::invalid(
                "service release identity must be Unicode without controls",
            ));
        }
        let executable = format!("{}{}", declaration.payload, std::env::consts::EXE_SUFFIX);
        if !release.join("service").join(&executable).is_file()
            || !release.join("bin").join(executable).is_file()
        {
            return Err(ServiceError::invalid(
                "service release is missing host or payload",
            ));
        }
        Ok(Self { release })
    }

    fn host(&self, declaration: &NormalizedDecl) -> PathBuf {
        self.release.join("service").join(format!(
            "{}{}",
            declaration.payload,
            std::env::consts::EXE_SUFFIX
        ))
    }

    fn from_host(
        root: &ServiceRoot,
        host: &Path,
        declaration: &NormalizedDecl,
    ) -> Result<Self, ServiceError> {
        let release = host
            .parent()
            .and_then(Path::parent)
            .ok_or_else(|| ServiceError::invalid("service host has no release parent"))?;
        Self::admit(root, release.to_path_buf(), declaration)
    }

    fn relative<'a>(&'a self, root: &'a ServiceRoot) -> &'a Path {
        self.release
            .strip_prefix(root.0.join("installs"))
            .expect("admitted binding remains beneath installs")
    }
}

fn one_component(value: &str, label: &str) -> Result<(), ServiceError> {
    let mut components = Path::new(value).components();
    if matches!(components.next(), Some(Component::Normal(_))) && components.next().is_none() {
        Ok(())
    } else {
        Err(ServiceError::invalid(format!(
            "service {label} must be one normal path component"
        )))
    }
}

fn normalize_environment(
    environment: BTreeMap<String, String>,
) -> Result<Vec<(String, String)>, ServiceError> {
    let mut entries = BTreeMap::new();
    for (name, value) in environment {
        let valid = name.bytes().enumerate().all(|(index, byte)| {
            byte == b'_' || byte.is_ascii_alphabetic() || (index > 0 && byte.is_ascii_digit())
        });
        if !valid || name.is_empty() || value.contains('\0') {
            return Err(ServiceError::invalid("invalid service environment entry"));
        }
        let folded = name.to_ascii_uppercase();
        if entries.contains_key(&folded) {
            return Err(ServiceError::invalid("duplicate service environment key"));
        }
        if matches!(
            folded.as_str(),
            "TOOLHOST_HOME"
                | "PATH"
                | "LD_LIBRARY_PATH"
                | "DYLD_LIBRARY_PATH"
                | "DYLD_FALLBACK_LIBRARY_PATH"
        ) {
            return Err(ServiceError::invalid("reserved service environment key"));
        }
        entries.insert(folded, (name, value));
    }
    Ok(entries.into_values().collect())
}

fn render(
    id: &ServiceId,
    payload: &str,
    args: &[String],
    environment: &[(String, String)],
) -> String {
    let arguments = args
        .iter()
        .map(|argument| quote(argument))
        .collect::<Vec<_>>()
        .join(", ");
    use std::fmt::Write as _;
    let mut output = format!(
        "schema = 1\nid = {}\npayload = {}\nargs = [{arguments}]\n\n[environment]\n",
        quote(id.as_str()),
        quote(payload)
    );
    for (name, value) in environment {
        let _ = writeln!(output, "{name} = {}", quote(value));
    }
    output
}

fn quote(value: &str) -> String {
    toml::Value::String(value.into()).to_string()
}

#[cfg(test)]
#[path = "../../tests/examples/toolhost/service.rs"]
mod tests;
