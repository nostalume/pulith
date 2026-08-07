//! Pure resolve for the versioned-tool vertical: identity and target/view admission.
//!
//! Owns the caller layout convention: `target = root/artifacts/<name>/<version>` and, when the
//! manifest declares `link_at`, a view at exactly that absolute path (the caller chooses where the
//! directory-symlink view is created). Resolve performs no I/O and cannot observe or mutate the
//! filesystem; it only produces the typed inputs later steps consume. It is deliberately not
//! dependency resolution — the manifest pins the source.
use std::fmt;
use std::path::PathBuf;

use pulith::local::LocalTarget;

use crate::manifest::{Manifest, is_single_component};

/// Caller-chosen root of the versioned-tool layout.
pub struct Layout {
    pub root: PathBuf,
}

/// The admitted plan: validated recipe plus the exact target and optional view paths.
#[derive(Debug)]
pub struct Resolved {
    pub manifest: Manifest,
    pub target: LocalTarget,
    pub view: Option<PathBuf>,
}

/// Failure of pure resolve; all failures name the offending field.
#[derive(Debug)]
pub enum ResolveError {
    InvalidVersion { version: String },
}

impl fmt::Display for ResolveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidVersion { version } => write!(
                f,
                "version {version:?} must be a single non-empty path component"
            ),
        }
    }
}

impl std::error::Error for ResolveError {}

/// Admit identity and policy: compute the exact target and view paths from the caller layout.
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
    Ok(Resolved {
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
            source: crate::manifest::Source::Url {
                url: "https://example.com/demo-tool.zip".to_string(),
            },
            hash: crate::manifest::HashExpectation::Blake3 {
                hex: "0".repeat(64),
            },
            expose: Some("bin".to_string()),
            link_at: Some(PathBuf::from("/opt/demo-tool")),
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
