//! Realization wiring for the versioned-tool vertical: install a manifest over landed behaviors.
//!
//! Owns the install flow: acquire (HTTP sync or local), verify (blake3/sha2 over byte streams),
//! materialize by magic detection (zip / tar / tar.gz, else copy), publish, and activate or
//! auto-switch an exposed view. Only landed behaviors are used, including the expose-aware
//! activation admission (`LocalActivate::activate_at` / `LocalSwitch::activate_at`). A local
//! directory source is copied without byte verification (digests attest byte streams; a tree has
//! none). All error paths are named and actionable; private workspaces are RAII-cleaned.
use std::fmt;
use std::io::Read;
use std::path::{Path, PathBuf};

use pulith::archive::{ArchivePolicy, ArchivePrepare, ExtractWorkspace, Gzip, Tar, Zip};
use pulith::hash::{Blake3, DigestValue, HashVerify, Sha256};
use pulith::local::{
    LocalAcquire, LocalActivate, LocalApply, LocalInspect, LocalMaterial, LocalObservation,
    LocalPath, LocalSwitch, LocalTarget,
};
use pulith::net::{RemoteSource, RemoteUrl, SyncHttpAcquire, SyncHttpResources};
use pulith::{
    Acquire, Acquired, Activate, Applied, Apply, Inspect, Materialize, MaterializeMode, Prepare,
    Verified, Verify,
};

use crate::manifest::{HashExpectation, Source};
use crate::resolve::{Layout, Resolved};

/// Outcome of the activation step of an install.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ActivateOutcome {
    /// No `link_at` declared; nothing was exposed.
    None,
    /// A new directory-symlink view was created.
    Activated,
    /// An existing directory-symlink view was natively replaced.
    Switched,
}

/// What an install produced.
#[derive(Debug)]
pub struct InstallReport {
    pub target: PathBuf,
    pub view: Option<PathBuf>,
    pub outcome: ActivateOutcome,
}

/// Named, actionable failure of the install flow.
#[derive(Debug)]
pub enum InstallError {
    Acquire {
        message: String,
    },
    RemoteUrl {
        url: String,
    },
    VerifyMismatch {
        expected: String,
    },
    Prepare {
        message: String,
    },
    Apply {
        message: String,
    },
    Io {
        action: &'static str,
        path: PathBuf,
        message: String,
    },
    ExposeNotDirectory {
        path: PathBuf,
        observed: LocalObservation,
    },
    ViewConflict {
        view: PathBuf,
        observed: LocalObservation,
    },
}

impl fmt::Display for InstallError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Acquire { message } => write!(f, "acquire failed: {message}"),
            Self::RemoteUrl { url } => write!(f, "invalid remote url {url:?}"),
            Self::VerifyMismatch { expected } => write!(
                f,
                "digest mismatch: the fetched bytes do not match the manifest expectation {expected:?}"
            ),
            Self::Prepare { message } => write!(f, "archive preparation failed: {message}"),
            Self::Apply { message } => write!(f, "publication failed: {message}"),
            Self::Io {
                action,
                path,
                message,
            } => {
                write!(f, "{action} failed at {}: {message}", path.display())
            }
            Self::ExposeNotDirectory { path, observed } => write!(
                f,
                "expose path {} is not a directory (observed {observed:?}); no view created",
                path.display()
            ),
            Self::ViewConflict { view, observed } => write!(
                f,
                "link_at {} holds {observed:?}, which is not a directory-symlink view; nothing replaced",
                view.display()
            ),
        }
    }
}

impl std::error::Error for InstallError {}

/// Acquire, verify, materialize, publish, and activate one manifest.
pub fn install(resolved: Resolved, layout: &Layout) -> Result<InstallReport, InstallError> {
    let name = resolved.manifest.name.clone();
    let target = resolved.target.clone();
    match &resolved.source {
        Source::Local { path } => {
            let request = Materialize::new(
                name,
                LocalPath::new(path),
                target,
                MaterializeMode::CreateNew,
            );
            let acquired =
                LocalAcquire
                    .acquire(request)
                    .map_err(|error| InstallError::Acquire {
                        message: error.to_string(),
                    })?;
            realize(acquired, &resolved, layout)
        }
        Source::Url { url } => {
            let remote =
                RemoteUrl::parse(url).map_err(|_| InstallError::RemoteUrl { url: url.clone() })?;
            let request = Materialize::new(
                name,
                RemoteSource::new(remote),
                target,
                MaterializeMode::CreateNew,
            );
            let acquired = SyncHttpAcquire::new(SyncHttpResources::default())
                .acquire(request)
                .map_err(|error| InstallError::Acquire {
                    message: error.to_string(),
                })?;
            realize(acquired, &resolved, layout)
        }
    }
}

/// Verify with the manifest's algorithm (concrete types; `DigestAlgorithm` is private), then
/// materialize and activate.
fn realize<I, S, E>(
    acquired: Acquired<Materialize<I, S, LocalTarget>, LocalMaterial, E>,
    resolved: &Resolved,
    layout: &Layout,
) -> Result<InstallReport, InstallError> {
    if material_path(&acquired.material).is_none() {
        ensure_parent(&resolved.target.path)?;
        let applied = LocalApply
            .apply(acquired)
            .map_err(|error| InstallError::Apply {
                message: error.to_string(),
            })?;
        return report_and_activate(applied, resolved);
    }
    match &resolved.hash {
        HashExpectation::Blake3 { hex } => {
            let digest = DigestValue::<Blake3>::new(hex.clone());
            let verified = HashVerify::<Blake3>::new()
                .verify(acquired, digest)
                .map_err(|_| InstallError::VerifyMismatch {
                    expected: format!("blake3 {hex}"),
                })?;
            realize_verified(verified, resolved, layout)
        }
        HashExpectation::Sha2 { hex } => {
            let digest = DigestValue::<Sha256>::new(hex.clone());
            let verified = HashVerify::<Sha256>::new()
                .verify(acquired, digest)
                .map_err(|_| InstallError::VerifyMismatch {
                    expected: format!("sha256 {hex}"),
                })?;
            realize_verified(verified, resolved, layout)
        }
    }
}

/// Materialize the verified bytes (detect zip/tar/gz, else copy), publish, and activate.
fn realize_verified<I, S, E>(
    verified: Verified<Materialize<I, S, LocalTarget>, LocalMaterial, E>,
    resolved: &Resolved,
    layout: &Layout,
) -> Result<InstallReport, InstallError> {
    let materialization = match material_path(&verified.material) {
        Some(path) => detect_materialization(path).map_err(|error| InstallError::Io {
            action: "detect material kind",
            path: path.to_path_buf(),
            message: error.to_string(),
        })?,
        None => Materialization::Copy,
    };

    let workspace_root = layout.root.join(".pulith-work").join(format!(
        "{}-{}",
        resolved.manifest.name, resolved.manifest.version
    ));

    // The landed publication law requires the target's parent directory to exist; the vertical
    // owns the layout structure (artifacts/<name>/).
    ensure_parent(&resolved.target.path)?;

    match materialization {
        Materialization::Copy => {
            let applied = LocalApply
                .apply(verified)
                .map_err(|error| InstallError::Apply {
                    message: error.to_string(),
                })?;
            report_and_activate(applied, resolved)
        }
        Materialization::Zip => {
            let _work = WorkDir::create(&workspace_root).map_err(|error| InstallError::Io {
                action: "create private workspace",
                path: workspace_root.clone(),
                message: error.to_string(),
            })?;
            let prepared = ArchivePrepare::<Zip>::new(ExtractWorkspace::new(&workspace_root))
                .prepare(verified, ArchivePolicy::new())
                .map_err(|error| InstallError::Prepare {
                    message: error.to_string(),
                })?;
            let applied = LocalApply
                .apply(prepared)
                .map_err(|error| InstallError::Apply {
                    message: error.to_string(),
                })?;
            report_and_activate(applied, resolved)
        }
        Materialization::Tar => {
            let _work = WorkDir::create(&workspace_root).map_err(|error| InstallError::Io {
                action: "create private workspace",
                path: workspace_root.clone(),
                message: error.to_string(),
            })?;
            let prepared = ArchivePrepare::<Tar>::new(ExtractWorkspace::new(&workspace_root))
                .prepare(verified, ArchivePolicy::new())
                .map_err(|error| InstallError::Prepare {
                    message: error.to_string(),
                })?;
            let applied = LocalApply
                .apply(prepared)
                .map_err(|error| InstallError::Apply {
                    message: error.to_string(),
                })?;
            report_and_activate(applied, resolved)
        }
        Materialization::TarGz => {
            let _work = WorkDir::create(&workspace_root).map_err(|error| InstallError::Io {
                action: "create private workspace",
                path: workspace_root.clone(),
                message: error.to_string(),
            })?;
            let prepared = ArchivePrepare::<Tar<Gzip>>::new(ExtractWorkspace::new(&workspace_root))
                .prepare(verified, ArchivePolicy::new())
                .map_err(|error| InstallError::Prepare {
                    message: error.to_string(),
                })?;
            let applied = LocalApply
                .apply(prepared)
                .map_err(|error| InstallError::Apply {
                    message: error.to_string(),
                })?;
            report_and_activate(applied, resolved)
        }
    }
}

/// Activate (or auto-switch) the view and build the report.
fn report_and_activate<I, S, E>(
    applied: Applied<Materialize<I, S, LocalTarget>, E>,
    resolved: &Resolved,
) -> Result<InstallReport, InstallError> {
    let outcome = activate(applied, resolved)?;
    Ok(InstallReport {
        target: resolved.target.path.clone(),
        view: resolved.view.clone(),
        outcome,
    })
}

/// Expose-aware activation: D7 (expose must be a directory in the tree) and D6 (an occupied
/// `link_at` is auto-switched only when it is a directory-symlink view).
fn activate<I, S, E>(
    applied: Applied<Materialize<I, S, LocalTarget>, E>,
    resolved: &Resolved,
) -> Result<ActivateOutcome, InstallError> {
    let expose = resolved.manifest.expose.as_deref().unwrap_or("");

    let source_dir = if expose.is_empty() {
        applied.input.target.path.clone()
    } else {
        applied.input.target.path.join(expose)
    };
    let observed = LocalInspect
        .inspect(LocalTarget::new(&source_dir))
        .map_err(|error| InstallError::Io {
            action: "observe expose path",
            path: source_dir.clone(),
            message: error.to_string(),
        })?
        .observation;
    // D7 holds whenever a view is wanted (a view is declared, or expose is set): the exposed
    // path must be a directory in the materialized tree. A file-only tool with no view and no
    // expose declares nothing to validate (D3).
    let wants_view = !expose.is_empty() || resolved.view.is_some();
    if wants_view && observed != LocalObservation::Directory {
        return Err(InstallError::ExposeNotDirectory {
            path: source_dir,
            observed,
        });
    }

    let Some(link_at) = resolved.view.clone() else {
        return Ok(ActivateOutcome::None);
    };

    let view_observed = LocalInspect
        .inspect(LocalTarget::new(&link_at))
        .map_err(|error| InstallError::Io {
            action: "observe link_at",
            path: link_at.clone(),
            message: error.to_string(),
        })?
        .observation;
    match view_observed {
        LocalObservation::Missing => {
            // The activation law never creates a missing view parent; the vertical owns the
            // layout structure (views/) and pre-creates it.
            if let Some(parent) = link_at.parent() {
                std::fs::create_dir_all(parent).map_err(|error| InstallError::Io {
                    action: "create view parent",
                    path: parent.to_path_buf(),
                    message: error.to_string(),
                })?;
            }
            if expose.is_empty() {
                LocalActivate
                    .activate(applied, link_at)
                    .map_err(|error| InstallError::Io {
                        action: "create active view",
                        path: source_dir,
                        message: error.to_string(),
                    })?;
            } else {
                LocalActivate
                    .activate_at(applied, link_at, Path::new(expose))
                    .map_err(|error| InstallError::Io {
                        action: "create exposed active view",
                        path: source_dir,
                        message: error.to_string(),
                    })?;
            }
            Ok(ActivateOutcome::Activated)
        }
        LocalObservation::Symlink => {
            let linked_target = std::fs::read_link(&link_at).map_err(|error| InstallError::Io {
                action: "read existing view",
                path: link_at.clone(),
                message: error.to_string(),
            })?;
            let target_observed = LocalInspect
                .inspect(LocalTarget::new(&linked_target))
                .map_err(|error| InstallError::Io {
                    action: "observe existing view target",
                    path: linked_target,
                    message: error.to_string(),
                })?
                .observation;
            if target_observed == LocalObservation::Directory {
                if expose.is_empty() {
                    LocalSwitch
                        .activate(applied, link_at)
                        .map_err(|error| InstallError::Io {
                            action: "switch active view",
                            path: source_dir,
                            message: error.to_string(),
                        })?;
                } else {
                    LocalSwitch
                        .activate_at(applied, link_at, Path::new(expose))
                        .map_err(|error| InstallError::Io {
                            action: "switch exposed active view",
                            path: source_dir,
                            message: error.to_string(),
                        })?;
                }
                Ok(ActivateOutcome::Switched)
            } else {
                Err(InstallError::ViewConflict {
                    view: link_at,
                    observed: view_observed,
                })
            }
        }
        other => Err(InstallError::ViewConflict {
            view: link_at,
            observed: other,
        }),
    }
}

/// Create the parent directory of a layout path (the landed laws never create parents).
fn ensure_parent(path: &Path) -> Result<(), InstallError> {
    let parent = match path.parent() {
        Some(parent) => parent,
        None => return Ok(()),
    };
    std::fs::create_dir_all(parent).map_err(|error| InstallError::Io {
        action: "create layout parent",
        path: parent.to_path_buf(),
        message: error.to_string(),
    })
}
fn material_path(material: &LocalMaterial) -> Option<&Path> {
    match material {
        LocalMaterial::File { path } => Some(path.as_path()),
        LocalMaterial::StagedFile { path } => Some(path.as_ref()),
        LocalMaterial::Directory { .. } => None,
    }
}

/// Materialization kind detected by magic bytes (S3.3-D2; never decompresses a non-archive).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Materialization {
    Zip,
    Tar,
    TarGz,
    Copy,
}

fn detect_materialization(path: &Path) -> std::io::Result<Materialization> {
    let mut file = std::fs::File::open(path)?;
    let mut head = [0u8; 512];
    let n = file.read(&mut head)?;
    let head = &head[..n];
    if head.starts_with(b"PK\x03\x04") {
        return Ok(Materialization::Zip);
    }
    if head.starts_with(&[0x1f, 0x8b]) {
        return Ok(Materialization::TarGz);
    }
    if n >= 262 && &head[257..262] == b"ustar" {
        return Ok(Materialization::Tar);
    }
    Ok(Materialization::Copy)
}

/// RAII private workspace: created on demand, removed on drop (D5 staged cleanup).
struct WorkDir(PathBuf);

impl WorkDir {
    fn create(root: &Path) -> std::io::Result<Self> {
        std::fs::create_dir_all(root)?;
        Ok(Self(root.to_path_buf()))
    }
}

impl Drop for WorkDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    const WIN_ZIP_HEX: &str = "2edc44c413a0a47ab1977297f524ee8a87aae99d5db2f3e5d7ee668c33c22076";
    const LINUX_TAR_GZ_HEX: &str =
        "a92ceb63bee7ce63befa1cbd5454a84143cef36325e17f8da6b00b926bf7c0a1";
    const TAR_HEX: &str = "0c472076cda1af8189518b102682fa80058902d8454885ca1ccca758992fc07e";
    const PLAIN_BIN_HEX: &str = "83cb532b46738b2a268f95dac473a5642b6eea7497fc3b149be7fe7d9f5190fe";
    const SYMLINK_TAR_HEX: &str =
        "7e23e73d324c41c54134791ad98c6075d1331c3f83dcd16cf71ed7657f26c693";
    const WRAPPED_ZIP_HEX: &str =
        "b3422d3baa6aa950529e9b11e71d9d98f0a2ce17bff2a22ebc2b3f012d668138";

    fn fixture(name: &str) -> String {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("examples/versioned_tool/fixtures")
            .join(name)
            .to_string_lossy()
            .into_owned()
    }

    /// A manifest whose selected platform points at a local fixture.
    /// Escape a path for embedding in a TOML string (backslash and quote).
    fn toml_string(value: &str) -> String {
        value.replace('\\', "\\\\").replace('"', "\\\"")
    }

    fn recipe_text(
        name: &str,
        version: &str,
        fixture: &str,
        hex: &str,
        expose: Option<&str>,
        link_at: Option<&Path>,
    ) -> String {
        let mut text = format!("name = \"{name}\"\nversion = \"{version}\"\n");
        if let Some(expose) = expose {
            text.push_str(&format!("expose = \"{expose}\"\n"));
        }
        if let Some(link_at) = link_at {
            text.push_str(&format!(
                "link_at = \"{}\"\n",
                toml_string(&link_at.display().to_string())
            ));
        }
        if cfg!(windows) {
            text.push_str(&format!(
                "[windows.source]\nkind = \"local\"\npath = \"{}\"\n\n\
                 [windows.hash]\nkind = \"sha2\"\nhex = \"{hex}\"\n\n",
                toml_string(fixture)
            ));
        } else {
            text.push_str(&format!(
                "[linux.source]\nkind = \"local\"\npath = \"{}\"\n\n\
                 [linux.hash]\nkind = \"sha2\"\nhex = \"{hex}\"\n\n",
                toml_string(fixture)
            ));
        }
        text
    }

    fn install_manifest(text: &str, layout_root: &Path) -> Result<InstallReport, InstallError> {
        let manifest = crate::manifest::Manifest::parse(text).expect("parse manifest");
        let layout = Layout {
            root: layout_root.to_path_buf(),
        };
        let resolved = crate::resolve::resolve(manifest, &layout).expect("resolve");
        install(resolved, &layout)
    }

    #[test]
    fn install_materializes_and_activates_the_exposed_view() {
        let layout_root = tempfile::tempdir().unwrap();
        let link_at = layout_root.path().join("views/demo-tool");
        let (fixture_name, hex, tool_bytes) = if cfg!(windows) {
            ("tool-win.zip", WIN_ZIP_HEX, b"win-tool-bytes\n".as_slice())
        } else {
            (
                "tool-linux.tar.gz",
                LINUX_TAR_GZ_HEX,
                b"linux-tool-bytes\n".as_slice(),
            )
        };
        let recipe = recipe_text(
            "demo-tool",
            "1.2.0",
            &fixture(fixture_name),
            hex,
            Some("bin"),
            Some(&link_at),
        );

        let report = install_manifest(&recipe, layout_root.path()).unwrap();

        let target = layout_root.path().join("artifacts/demo-tool/1.2.0");
        assert_eq!(report.target, target);
        assert_eq!(report.outcome, ActivateOutcome::Activated);
        assert_eq!(fs::read(target.join("bin/tool")).unwrap(), tool_bytes);
        // The view is a directory symlink pointing at the EXPOSED subpath.
        assert!(
            fs::symlink_metadata(&link_at)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(fs::read_link(&link_at).unwrap(), target.join("bin"));
        assert_eq!(fs::read(link_at.join("tool")).unwrap(), tool_bytes);
    }

    #[test]
    fn install_detects_a_raw_tar() {
        let layout_root = tempfile::tempdir().unwrap();
        let recipe = recipe_text(
            "raw-tar-tool",
            "1.0.0",
            &fixture("tool.tar"),
            TAR_HEX,
            Some("bin"),
            None,
        );

        let report = install_manifest(&recipe, layout_root.path()).unwrap();
        let target = layout_root.path().join("artifacts/raw-tar-tool/1.0.0");
        assert_eq!(
            fs::read(target.join("bin/tool")).unwrap(),
            b"linux-tool-bytes\n"
        );
        assert_eq!(report.outcome, ActivateOutcome::None);
    }

    #[test]
    fn install_copies_a_plain_file_source() {
        let layout_root = tempfile::tempdir().unwrap();
        let recipe = recipe_text(
            "plain-tool",
            "1.0.0",
            &fixture("plain.bin"),
            PLAIN_BIN_HEX,
            None,
            None,
        );

        let report = install_manifest(&recipe, layout_root.path()).unwrap();
        let target = layout_root.path().join("artifacts/plain-tool/1.0.0");
        assert_eq!(fs::read(&target).unwrap(), b"plain-file-bytes\n");
        assert_eq!(report.target, target);
    }

    #[test]
    fn install_copies_a_local_directory_source_without_byte_verification() {
        let layout_root = tempfile::tempdir().unwrap();
        let recipe = recipe_text(
            "dir-tool",
            "1.0.0",
            &fixture("plain-dir"),
            "0000000000000000000000000000000000000000000000000000000000000000",
            None,
            None,
        );

        let report = install_manifest(&recipe, layout_root.path()).unwrap();
        let target = layout_root.path().join("artifacts/dir-tool/1.0.0");
        assert_eq!(
            fs::read(target.join("bin/tool")).unwrap(),
            b"dir-tool-bytes\n"
        );
        assert_eq!(report.target, target);
    }

    #[cfg(unix)]
    #[test]
    fn install_rejects_an_archive_with_symlink_entries() {
        let layout_root = tempfile::tempdir().unwrap();
        let recipe = recipe_text(
            "symlink-tool",
            "1.0.0",
            &fixture("tool-symlink.tar"),
            SYMLINK_TAR_HEX,
            None,
            None,
        );

        let error = install_manifest(&recipe, layout_root.path()).unwrap_err();
        assert!(matches!(error, InstallError::Prepare { .. }));
        // The versioned tree was never published and the private workspace was cleaned.
        assert!(
            !layout_root
                .path()
                .join("artifacts/symlink-tool/1.0.0")
                .exists()
        );
        assert!(
            !layout_root
                .path()
                .join(".pulith-work/symlink-tool-1.0.0")
                .exists()
        );
    }

    #[test]
    fn install_aborts_on_hash_mismatch_and_leaves_no_target_or_workspace() {
        let layout_root = tempfile::tempdir().unwrap();
        let recipe = recipe_text(
            "bad-hash-tool",
            "1.0.0",
            &fixture("tool-win.zip"),
            "0000000000000000000000000000000000000000000000000000000000000000",
            None,
            None,
        );

        let error = install_manifest(&recipe, layout_root.path()).unwrap_err();
        assert!(matches!(error, InstallError::VerifyMismatch { .. }));
        assert!(!layout_root.path().join("artifacts").exists());
        assert!(!layout_root.path().join(".pulith-work").exists());
    }

    #[test]
    fn install_auto_switches_an_occupied_directory_symlink_view() {
        let layout_root = tempfile::tempdir().unwrap();
        let link_at = layout_root.path().join("views/demo-tool");
        let (fixture_name, hex) = if cfg!(windows) {
            ("tool-win.zip", WIN_ZIP_HEX)
        } else {
            ("tool-linux.tar.gz", LINUX_TAR_GZ_HEX)
        };

        let first = recipe_text(
            "demo-tool",
            "1.0.0",
            &fixture(fixture_name),
            hex,
            Some("bin"),
            Some(&link_at),
        );
        install_manifest(&first, layout_root.path()).unwrap();

        let second = recipe_text(
            "demo-tool",
            "2.0.0",
            &fixture(fixture_name),
            hex,
            Some("bin"),
            Some(&link_at),
        );
        let report = install_manifest(&second, layout_root.path()).unwrap();

        assert_eq!(report.outcome, ActivateOutcome::Switched);
        let second_target = layout_root.path().join("artifacts/demo-tool/2.0.0");
        assert_eq!(fs::read_link(&link_at).unwrap(), second_target.join("bin"));
        // The first version's tree is untouched (retention is caller-owned).
        assert!(
            layout_root
                .path()
                .join("artifacts/demo-tool/1.0.0")
                .is_dir()
        );
    }

    #[test]
    fn install_fails_when_expose_is_not_a_directory_and_creates_no_view() {
        let layout_root = tempfile::tempdir().unwrap();
        let link_at = layout_root.path().join("views/demo-tool");
        let recipe = recipe_text(
            "demo-tool",
            "1.0.0",
            &fixture("tool-win.zip"),
            WIN_ZIP_HEX,
            Some("missing"),
            Some(&link_at),
        );

        let error = install_manifest(&recipe, layout_root.path()).unwrap_err();
        match error {
            InstallError::ExposeNotDirectory { observed, .. } => {
                assert_ne!(observed, LocalObservation::Directory);
            }
            other => panic!("expected ExposeNotDirectory, got {other:?}"),
        }
        assert!(!link_at.exists());
    }

    #[test]
    fn install_keeps_a_wrapped_tree_without_magic_unwrap() {
        let layout_root = tempfile::tempdir().unwrap();
        // The archive has entries under tool-1.2.0/; expose must name the real shape (D8).
        let recipe = recipe_text(
            "wrapped-tool",
            "1.2.0",
            &fixture("tool-wrapped.zip"),
            WRAPPED_ZIP_HEX,
            Some("tool-1.2.0/bin"),
            None,
        );

        let report = install_manifest(&recipe, layout_root.path()).unwrap();
        let target = layout_root.path().join("artifacts/wrapped-tool/1.2.0");
        assert_eq!(
            fs::read(target.join("tool-1.2.0/bin/tool")).unwrap(),
            b"wrapped-tool-bytes\n"
        );
        assert_eq!(report.outcome, ActivateOutcome::None);

        // expose "bin" (the unwrapped shape) must fail (D7, no fallback). Use a fresh version
        // so the run reaches the expose gate instead of colliding on the existing target.
        let bad_recipe = recipe_text(
            "wrapped-tool",
            "1.2.1",
            &fixture("tool-wrapped.zip"),
            WRAPPED_ZIP_HEX,
            Some("bin"),
            None,
        );
        let error = install_manifest(&bad_recipe, layout_root.path()).unwrap_err();
        assert!(matches!(error, InstallError::ExposeNotDirectory { .. }));
    }
}
