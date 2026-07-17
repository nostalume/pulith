use std::fmt;
use std::io;
#[cfg(feature = "local")]
use std::path::Path;
use std::path::PathBuf;

#[cfg(feature = "net")]
use crate::net::AcquireError;

/// Errors produced by concrete Pulith behaviors.
///
/// This enum is non-exhaustive because new typed behaviors and policy failures may add variants.
#[non_exhaustive]
#[derive(Debug)]
pub enum PulithError {
    #[cfg(feature = "net")]
    NetAcquire(Box<AcquireError>),
    MissingSource(PathBuf),
    DigestMismatch {
        expected: String,
        observed: String,
    },
    ArtifactSizeMismatch {
        expected: u64,
        observed: u64,
    },
    UnsupportedDigestMaterial(PathBuf),
    ArchiveRequiresFile(PathBuf),
    ArchiveInvalidPath(String),
    ArchiveLimitExceeded {
        limit: &'static str,
        actual: u64,
        max: u64,
    },
    ArchiveSizeMismatch {
        path: PathBuf,
        declared: u64,
        observed: u64,
    },
    ArchivePathConflict(PathBuf),
    /// Extraction failed and resetting the exclusive workspace also failed.
    ///
    /// `extraction` is the primary [`std::error::Error::source`]. The secondary cleanup failure
    /// remains available through `cleanup` so callers can detect a contaminated workspace.
    ArchiveCleanupFailed {
        workspace: PathBuf,
        extraction: Box<PulithError>,
        cleanup: Box<PulithError>,
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

#[cfg(feature = "local")]
impl PulithError {
    pub(crate) fn io(action: &'static str, path: impl AsRef<Path>, source: io::Error) -> Self {
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
            Self::MissingSource(path) => write!(f, "source does not exist: {}", path.display()),
            Self::DigestMismatch { expected, observed } => {
                write!(
                    f,
                    "digest mismatch: expected {expected}, observed {observed}"
                )
            }
            Self::ArtifactSizeMismatch { expected, observed } => {
                write!(
                    f,
                    "artifact size mismatch: expected {expected}, observed {observed}"
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
            Self::ArchiveSizeMismatch {
                path,
                declared,
                observed,
            } => write!(
                f,
                "archive entry size mismatch for {}: declared {declared}, observed {observed}",
                path.display()
            ),
            Self::ArchivePathConflict(path) => {
                write!(f, "archive entries conflict at path: {}", path.display())
            }
            Self::ArchiveCleanupFailed { workspace, .. } => write!(
                f,
                "archive extraction and cleanup both failed for {}",
                workspace.display()
            ),
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
            Self::ArchiveCleanupFailed { extraction, .. } => Some(extraction.as_ref()),
            Self::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}
