use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

#[cfg(feature = "net")]
use crate::net::AcquireError;

#[derive(Debug)]
pub enum PulithError {
    #[cfg(feature = "net")]
    NetAcquire(Box<AcquireError>),
    EmptySourceOffer,
    MissingSource(PathBuf),
    DigestMismatch {
        expected: String,
        observed: String,
    },
    UnsupportedDigestMaterial(PathBuf),
    ArchiveRequiresFile(PathBuf),
    ArchiveInvalidPath(String),
    ArchiveLimitExceeded {
        limit: &'static str,
        actual: u64,
        max: u64,
    },
    UnsupportedArchiveEntry(PathBuf),
    InvalidPreparation(String),
    ApplyWouldOverwrite(PathBuf),
    ApplyMissingTarget(PathBuf),
    ApplySameFile(PathBuf),
    ApplyPathConflict {
        source: PathBuf,
        target: PathBuf,
    },
    UnsupportedLocalEntry(PathBuf),
    Io {
        action: &'static str,
        path: PathBuf,
        source: io::Error,
    },
}

impl PulithError {
    pub fn io(action: &'static str, path: impl AsRef<Path>, source: io::Error) -> Self {
        Self::Io {
            action,
            path: path.as_ref().to_path_buf(),
            source,
        }
    }
}

#[cfg(feature = "net")]
impl From<AcquireError> for PulithError {
    fn from(error: AcquireError) -> Self {
        Self::NetAcquire(Box::new(error))
    }
}

impl fmt::Display for PulithError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            #[cfg(feature = "net")]
            Self::NetAcquire(error) => write!(f, "{error}"),
            Self::EmptySourceOffer => write!(f, "no source was offered"),
            Self::MissingSource(path) => write!(f, "source does not exist: {}", path.display()),
            Self::DigestMismatch { expected, observed } => {
                write!(
                    f,
                    "digest mismatch: expected {expected}, observed {observed}"
                )
            }
            Self::UnsupportedDigestMaterial(path) => {
                write!(f, "digest verification requires a file: {}", path.display())
            }
            Self::ArchiveRequiresFile(path) => {
                write!(f, "archive preparation requires a file: {}", path.display())
            }
            Self::ArchiveInvalidPath(path) => write!(f, "archive entry path is invalid: {path}"),
            Self::ArchiveLimitExceeded { limit, actual, max } => {
                write!(f, "archive {limit} limit exceeded: {actual} > {max}")
            }
            Self::UnsupportedArchiveEntry(path) => {
                write!(f, "archive entry is unsupported: {}", path.display())
            }
            Self::InvalidPreparation(message) => write!(f, "invalid preparation: {message}"),
            Self::ApplyWouldOverwrite(path) => {
                write!(
                    f,
                    "apply would overwrite existing target: {}",
                    path.display()
                )
            }
            Self::ApplyMissingTarget(path) => {
                write!(f, "target does not exist: {}", path.display())
            }
            Self::ApplySameFile(path) => {
                write!(
                    f,
                    "source and target refer to the same file: {}",
                    path.display()
                )
            }
            Self::ApplyPathConflict { source, target } => {
                write!(
                    f,
                    "source and target paths conflict: {} -> {}",
                    source.display(),
                    target.display()
                )
            }
            Self::UnsupportedLocalEntry(path) => {
                write!(
                    f,
                    "local filesystem entry is unsupported: {}",
                    path.display()
                )
            }
            Self::Io {
                action,
                path,
                source,
            } => {
                write!(f, "failed to {action} {}: {source}", path.display())
            }
        }
    }
}

impl std::error::Error for PulithError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            #[cfg(feature = "net")]
            Self::NetAcquire(error) => Some(error.as_ref()),
            Self::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}
