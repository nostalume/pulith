//! Pure resolve for the versioned-tool vertical: identity and target/view admission.
//!
//! Owns the caller layout convention: `target = root/artifacts/<name>/<version>` and, when the
//! manifest declares `link_at`, a view at exactly that absolute path (the caller chooses where the
//! directory-symlink view is created). Resolve also selects the running OS's artifact pair
//! (flattened `[windows.source]`+`[windows.hash]` / `[linux.source]`+`[linux.hash]`); a platform
//! without a pair is `ResolveError::NoSourceForPlatform`. Resolve performs no I/O and cannot
//! observe or mutate the filesystem; it only produces the typed inputs later steps consume. It is
//! deliberately not dependency resolution — the manifest pins the source.
use std::fmt;
use std::path::PathBuf;

use pulith::local::LocalTarget;

use crate::manifest::{HashExpectation, Manifest, Source, is_single_component};

/// Caller-chosen root of the versioned-tool layout.
pub struct Layout {
    pub root: PathBuf,
}

/// The admitted plan: validated recipe plus the exact target, view, and selected OS artifact pair.
#[derive(Debug)]
pub struct Resolved {
    pub manifest: Manifest,
    pub target: LocalTarget,
    pub view: Option<PathBuf>,
    /// The running OS's artifact source (selected from the flattened platform pairs).
    pub source: Source,
    /// The running OS's digest expectation for that source.
    pub hash: HashExpectation,
}

/// Failure of pure resolve; all failures name the offending field.
#[derive(Debug)]
pub enum ResolveError {
    InvalidVersion { version: String },
    NoSourceForPlatform { platform: &'static str },
}

impl fmt::Display for ResolveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidVersion { version } => write!(
                f,
                "version {version:?} must be a single non-empty path component"
            ),
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

/// Admit identity and policy: compute the exact target and view paths and select the OS artifact.
pub fn resolve(manifest: Manifest, layout: &Layout) -> Result<Resolved, ResolveError> {
    if !is_single_component(&manifest.version) {
        return Err(ResolveError::InvalidVersion {
            version: manifest.version.clone(),
        });
    }
    let target = LocalTarget::new(
        layout
            .root
            .join("artifacts")
            .join(&manifest.name)
            .join(&manifest.version),
    );
    let view = manifest.link_at.clone();
    let (platform, spec) = if cfg!(windows) {
        ("windows", &manifest.windows)
    } else {
        ("linux", &manifest.linux)
    };
    let spec = spec
        .as_ref()
        .ok_or(ResolveError::NoSourceForPlatform { platform })?;
    Ok(Resolved {
        source: spec.source.clone(),
        hash: spec.hash.clone(),
        manifest,
        target,
        view,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest() -> Manifest {
        Manifest {
            name: "demo-tool".to_string(),
            version: "1.2.0".to_string(),
            expose: Some("bin".to_string()),
            link_at: Some(PathBuf::from("/opt/demo-tool")),
            windows: Some(crate::manifest::PlatformSpec {
                source: Source::Url {
                    url: "https://example.com/demo-tool-win.zip".to_string(),
                },
                hash: HashExpectation::Blake3 {
                    hex: "0".repeat(64),
                },
            }),
            linux: Some(crate::manifest::PlatformSpec {
                source: Source::Url {
                    url: "https://example.com/demo-tool-linux.tar.gz".to_string(),
                },
                hash: HashExpectation::Blake3 {
                    hex: "1".repeat(64),
                },
            }),
        }
    }

    #[test]
    fn resolves_the_versioned_target_path() {
        let layout = Layout {
            root: PathBuf::from("/opt/tools"),
        };
        let resolved = resolve(manifest(), &layout).unwrap();
        assert_eq!(
            resolved.target.path,
            PathBuf::from("/opt/tools/artifacts/demo-tool/1.2.0")
        );
    }

    #[test]
    fn resolves_the_view_path_only_when_link_at_is_declared() {
        let layout = Layout {
            root: PathBuf::from("/opt/tools"),
        };
        let with_view = resolve(manifest(), &layout).unwrap();
        assert_eq!(with_view.view, Some(PathBuf::from("/opt/demo-tool")));

        let mut no_view = manifest();
        no_view.link_at = None;
        let without_view = resolve(no_view, &layout).unwrap();
        assert_eq!(without_view.view, None);
    }

    #[test]
    fn selects_the_running_platforms_artifact_pair() {
        let layout = Layout {
            root: PathBuf::from("/opt/tools"),
        };
        let resolved = resolve(manifest(), &layout).unwrap();
        if cfg!(windows) {
            assert_eq!(
                resolved.hash,
                HashExpectation::Blake3 {
                    hex: "0".repeat(64)
                }
            );
        } else {
            assert_eq!(
                resolved.hash,
                HashExpectation::Blake3 {
                    hex: "1".repeat(64)
                }
            );
        }
    }

    #[test]
    fn rejects_a_platform_without_a_pair() {
        let layout = Layout {
            root: PathBuf::from("/opt/tools"),
        };
        let mut bad = manifest();
        if cfg!(windows) {
            bad.windows = None;
        } else {
            bad.linux = None;
        }
        let error = resolve(bad, &layout).unwrap_err();
        assert!(matches!(error, ResolveError::NoSourceForPlatform { .. }));
    }

    #[test]
    fn rejects_a_version_that_is_not_a_single_path_component() {
        let layout = Layout {
            root: PathBuf::from("/opt/tools"),
        };
        let mut bad = manifest();
        bad.version = "1.0/../2".to_string();
        let error = resolve(bad, &layout).unwrap_err();
        assert!(matches!(error, ResolveError::InvalidVersion { .. }));
    }
}
