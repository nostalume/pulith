//! Typed TOML recipe and durable intent journal for the vtool vertical.
use std::fmt;
use std::path::{Path, PathBuf};

use pulith::hash::DigestValue;
use pulith::net::RemoteUrl;
use serde::Deserialize;

macro_rules! law_newtype {
    ($name:ident, $inner:ty, $err:ty, $parse:expr) => {
        #[derive(Clone, Debug, PartialEq, Deserialize)]
        #[serde(try_from = "String")]
        pub struct $name($inner);

        impl std::str::FromStr for $name {
            type Err = $err;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                Ok(Self($parse(value)?))
            }
        }

        impl TryFrom<String> for $name {
            type Error = $err;

            fn try_from(value: String) -> Result<Self, Self::Error> {
                value.parse()
            }
        }
    };
}

law_newtype!(Component, String, String, |value: &str| {
    if is_single_component(value) {
        Ok(value.to_string())
    } else {
        Err(format!(
            "value {value:?} must be a single non-empty path component"
        ))
    }
});

impl Component {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

law_newtype!(ViewPath, PathBuf, String, |value: &str| {
    let path = PathBuf::from(value);
    if path.is_absolute() {
        Ok(path)
    } else {
        Err(format!("value {value:?} must be an absolute path"))
    }
});

impl ViewPath {
    pub fn as_path(&self) -> &Path {
        &self.0
    }
}

fn is_single_component(value: &str) -> bool {
    let mut components = Path::new(value).components();
    matches!(
        (components.next(), components.next()),
        (Some(std::path::Component::Normal(part)), None) if !part.is_empty()
    )
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    /// Artifact identity; a single non-empty path component.
    pub name: Component,
    /// Resolved version; a single non-empty path component.
    pub version: Component,
    /// Subpath the active view links; the core link law validates it when a view is linked.
    #[serde(default)]
    pub expose: Option<PathBuf>,
    /// Absolute view path; absent means no view.
    #[serde(default)]
    pub link_at: Option<ViewPath>,
    /// Atomic source+hash pair per platform; absent means no artifact for that OS.
    pub windows: Option<PlatformSpec>,
    pub linux: Option<PlatformSpec>,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct PlatformSpec {
    pub source: Source,
    pub hash: DigestValue,
}

impl Manifest {
    pub fn load(path: &Path) -> Result<Self, crate::BoxError> {
        let text = std::fs::read_to_string(path)?;
        Ok(Self::parse(&text)?)
    }

    pub fn parse(text: &str) -> Result<Self, ManifestError> {
        toml::from_str(text).map_err(|error| ManifestError::Toml {
            message: error.to_string(),
        })
    }
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Source {
    Url { url: Box<RemoteUrl> },
    Local { path: PathBuf },
}

#[derive(Debug)]
pub struct Resolved {
    pub manifest: Manifest,
    pub target: PathBuf,
    pub view: Option<PathBuf>,
    pub source: Source,
    pub hash: DigestValue,
}

#[derive(Debug)]
pub enum ResolveError {
    NoSourceForPlatform { platform: &'static str },
}

impl fmt::Display for ResolveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoSourceForPlatform { platform } => {
                write!(
                    f,
                    "manifest declares no source+hash pair for platform {platform:?}"
                )
            }
        }
    }
}

impl std::error::Error for ResolveError {}

impl Manifest {
    pub fn resolve(mut self, root: &Path) -> Result<Resolved, ResolveError> {
        let target = root
            .join("artifacts")
            .join(self.name.as_str())
            .join(self.version.as_str());
        let view = self
            .link_at
            .as_ref()
            .map(|view| view.as_path().to_path_buf());
        let (platform, spec) = if cfg!(windows) {
            ("windows", self.windows.take())
        } else {
            ("linux", self.linux.take())
        };
        let spec = spec.ok_or(ResolveError::NoSourceForPlatform { platform })?;
        Ok(Resolved {
            source: spec.source,
            hash: spec.hash,
            manifest: self,
            target,
            view,
        })
    }
}

#[derive(Debug)]
pub enum ManifestError {
    Toml { message: String },
}

impl fmt::Display for ManifestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let Self::Toml { message } = self;
        write!(f, "invalid manifest: {message}")
    }
}

impl std::error::Error for ManifestError {}

use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Write};

use serde::Serialize;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Record {
    pub name: String,
    pub version: String,
    pub phase: Phase,
    pub generation: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Phase {
    Installed,
    Deactivated,
}

pub struct Journal {
    path: PathBuf,
}

impl Journal {
    pub fn open(root: &Path) -> Result<Self, StateError> {
        let dir = root.join(".pulith-state");
        fs::create_dir_all(&dir)
            .map_err(|source| StateError::io("create state directory", &dir, source))?;
        Ok(Self {
            path: dir.join("journal.jsonl"),
        })
    }

    pub fn append(&mut self, record: &Record) -> Result<(), StateError> {
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .map_err(|source| StateError::io("open journal for append", &self.path, source))?;
        serde_json::to_writer(&mut file, record)
            .map_err(|error| StateError::encode(&self.path, error))?;
        file.write_all(b"\n")
            .map_err(|source| StateError::io("write journal line", &self.path, source))?;
        if std::env::var("PULITH_VT_CRASH_AFTER").as_deref() == Ok("journal-append") {
            std::process::abort();
        }
        file.sync_all()
            .map_err(|source| StateError::io("fsync journal", &self.path, source))?;
        if std::env::var("PULITH_VT_CRASH_AFTER").as_deref() == Ok("journal-fsync") {
            std::process::abort();
        }
        Ok(())
    }

    pub fn read(&self) -> Result<Vec<Record>, StateError> {
        let file = match File::open(&self.path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(source) => return Err(StateError::io("open journal for read", &self.path, source)),
        };
        let mut records = Vec::new();
        for (index, line) in BufReader::new(file).lines().enumerate() {
            let line =
                line.map_err(|source| StateError::io("read journal line", &self.path, source))?;
            if line.trim().is_empty() {
                continue;
            }
            records.push(
                serde_json::from_str(&line).map_err(|error| StateError::Decode {
                    path: self.path.clone(),
                    line: index + 1,
                    message: error.to_string(),
                })?,
            );
        }
        Ok(records)
    }
}

#[derive(Debug)]
pub enum StateError {
    Io {
        action: &'static str,
        path: PathBuf,
        source: std::io::Error,
    },
    Decode {
        path: PathBuf,
        line: usize,
        message: String,
    },
    Encode {
        path: PathBuf,
        message: String,
    },
}

impl StateError {
    fn io(action: &'static str, path: &Path, source: std::io::Error) -> Self {
        Self::Io {
            action,
            path: path.to_path_buf(),
            source,
        }
    }

    fn encode(path: &Path, error: serde_json::Error) -> Self {
        Self::Encode {
            path: path.to_path_buf(),
            message: error.to_string(),
        }
    }
}

impl std::fmt::Display for StateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io {
                action,
                path,
                source,
            } => write!(f, "{action} `{}`: {source}", path.display()),
            Self::Decode {
                path,
                line,
                message,
            } => {
                write!(
                    f,
                    "corrupt journal line {line} in `{}`: {message}",
                    path.display()
                )
            }
            Self::Encode { path, message } => {
                write!(f, "encode journal record `{}`: {message}", path.display())
            }
        }
    }
}

impl std::error::Error for StateError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}

#[cfg(test)]
#[path = "../../tests/examples/vtool/manifest.rs"]
mod tests;
