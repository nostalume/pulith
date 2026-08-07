//! Realization wiring for the vtool vertical: install a manifest over landed behaviors.
//!
//! Owns the install flow by composing node methods — acquire (HTTP sync or local), then
//! `.materialize()` (verify blake3/sha2 over byte streams, magic-detect zip/tar/tar.gz else
//! copy, publish), then `.link()`/`.link_root()` (the core link law D6/D7: expose is a
//! directory in the tree, an occupied view is auto-switched, the view parent is created by the
//! law). A local directory source is copied without byte verification (digests attest byte
//! streams; a tree has none). No typestate is ever named by the caller. All error paths are
//! named and actionable; private workspaces are RAII-cleaned.
use std::error::Error;
use std::path::{Path, PathBuf};

use pulith::archive::ArchivePolicy;
use pulith::local::{LinkOutcome, LocalAcquire, OccupiedViewPolicy};
use pulith::net::{RemoteSource, SyncHttpAcquire};
use pulith::{Materialize, MaterializeMode};

use crate::manifest::Resolved;
use crate::manifest::Source;

impl Resolved {
    /// Acquire, materialize, and link this resolved manifest under `root`; `None` means the
    /// manifest declares no view.
    pub fn install(self, root: &Path) -> Result<Option<LinkOutcome>, Box<dyn Error + Send + Sync>> {
        let digest = &self.hash;

        // The publication law (create-new) never creates the target parent; the vertical owns
        // the artifacts/<name> structure. The private RAII workspace is cleaned on every exit.
        ensure_parent(&self.target)?;
        let workspace_root = root.join(".pulith-work").join(format!(
            "{}-{}",
            self.manifest.name.as_str(),
            self.manifest.version.as_str()
        ));
        let _work = WorkDir::create(&workspace_root)?;
        let workspace = &workspace_root;

        // Acquire by source kind, then compose the landed behaviors as node methods: materialize
        // always runs (the tree is published), then the core link law applies when a view is
        // declared. The two acquire evidence types differ, so each arm is one linear chain — no
        // type is named.
        // Acquire by source kind, then compose the landed behaviors as node methods: materialize
        // always runs (the tree is published); the core link law applies when a view is declared
        // (an inert `expose` is not checked). Each arm is one linear chain — no type is named.
        match &self.source {
            Source::Local { path } => {
                let materialized = LocalAcquire
                    .acquire(Materialize::new(
                        self.manifest.name.clone().into_string(),
                        (path).into(),
                        self.target.clone(),
                        MaterializeMode::CreateNew,
                    ))?
                    .materialize(digest.clone(), workspace, ArchivePolicy::new())?;
                let Some(view) = self.view.as_deref() else {
                    return Ok(None);
                };
                Ok(Some(match self.manifest.expose.as_deref() {
                    Some(expose) => {
                        materialized.link(view, expose, OccupiedViewPolicy::AutoSwitch)?
                    }
                    None => materialized.link_root(view, OccupiedViewPolicy::AutoSwitch)?,
                }))
            }
            Source::Url { url } => {
                let materialized = SyncHttpAcquire::default()
                    .acquire(Materialize::new(
                        self.manifest.name.clone().into_string(),
                        RemoteSource::new(url.clone()),
                        self.target.clone(),
                        MaterializeMode::CreateNew,
                    ))?
                    .materialize(digest.clone(), workspace, ArchivePolicy::new())?;
                let Some(view) = self.view.as_deref() else {
                    return Ok(None);
                };
                Ok(Some(match self.manifest.expose.as_deref() {
                    Some(expose) => {
                        materialized.link(view, expose, OccupiedViewPolicy::AutoSwitch)?
                    }
                    None => materialized.link_root(view, OccupiedViewPolicy::AutoSwitch)?,
                }))
            }
        }
    }
}

/// Create the parent directory of a layout path (the landed laws never create parents).
fn ensure_parent(path: &Path) -> std::io::Result<()> {
    let parent = match path.parent() {
        Some(parent) => parent,
        None => return Ok(()),
    };
    std::fs::create_dir_all(parent)
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
    use std::path::PathBuf;

    const WIN_ZIP_HEX: &str = "2edc44c413a0a47ab1977297f524ee8a87aae99d5db2f3e5d7ee668c33c22076";
    const LINUX_TAR_GZ_HEX: &str =
        "a92ceb63bee7ce63befa1cbd5454a84143cef36325e17f8da6b00b926bf7c0a1";
    const TAR_HEX: &str = "0c472076cda1af8189518b102682fa80058902d8454885ca1ccca758992fc07e";
    const PLAIN_BIN_HEX: &str = "83cb532b46738b2a268f95dac473a5642b6eea7497fc3b149be7fe7d9f5190fe";
    #[cfg(unix)]
    const SYMLINK_TAR_HEX: &str =
        "7e23e73d324c41c54134791ad98c6075d1331c3f83dcd16cf71ed7657f26c693";
    const WRAPPED_ZIP_HEX: &str =
        "b3422d3baa6aa950529e9b11e71d9d98f0a2ce17bff2a22ebc2b3f012d668138";

    fn fixture(name: &str) -> String {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("examples/vtool/fixtures")
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

    fn install_manifest(
        text: &str,
        layout_root: &Path,
    ) -> Result<Option<LinkOutcome>, Box<dyn Error + Send + Sync>> {
        let manifest = crate::manifest::Manifest::parse(text).expect("parse manifest");
        let resolved = manifest.resolve(layout_root).expect("resolve");
        resolved.install(layout_root)
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

        let outcome = install_manifest(&recipe, layout_root.path()).unwrap();

        let target = layout_root.path().join("artifacts/demo-tool/1.2.0");
        assert_eq!(outcome, Some(LinkOutcome::Activated));
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

        let outcome = install_manifest(&recipe, layout_root.path()).unwrap();
        let target = layout_root.path().join("artifacts/raw-tar-tool/1.0.0");
        assert_eq!(
            fs::read(target.join("bin/tool")).unwrap(),
            b"linux-tool-bytes\n"
        );
        assert_eq!(outcome, None);
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

        let outcome = install_manifest(&recipe, layout_root.path()).unwrap();
        assert_eq!(outcome, None);
        let target = layout_root.path().join("artifacts/plain-tool/1.0.0");
        assert_eq!(fs::read(&target).unwrap(), b"plain-file-bytes\n");
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

        let outcome = install_manifest(&recipe, layout_root.path()).unwrap();
        assert_eq!(outcome, None);
        let target = layout_root.path().join("artifacts/dir-tool/1.0.0");
        assert_eq!(
            fs::read(target.join("bin/tool")).unwrap(),
            b"dir-tool-bytes\n"
        );
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
        assert!(matches!(
            error
                .downcast_ref::<pulith::local::MaterializeError>()
                .expect("expected a materialize error"),
            pulith::local::MaterializeError::Prepare(_)
        ));
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
        assert!(matches!(
            error
                .downcast_ref::<pulith::local::MaterializeError>()
                .expect("expected a materialize error"),
            pulith::local::MaterializeError::Verify(_)
        ));
        // The versioned tree was never published and the private workspace was cleaned; the
        // artifacts/<name> layout parent is legitimately pre-created by the vertical.
        assert!(
            !layout_root
                .path()
                .join("artifacts/bad-hash-tool/1.0.0")
                .exists()
        );
        assert!(
            !layout_root
                .path()
                .join(".pulith-work/bad-hash-tool-1.0.0")
                .exists()
        );
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
        let outcome = install_manifest(&second, layout_root.path()).unwrap();

        assert_eq!(outcome, Some(LinkOutcome::Switched));
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
        assert!(
            error.to_string().contains("is not a directory"),
            "unexpected error: {error}"
        );
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

        let outcome = install_manifest(&recipe, layout_root.path()).unwrap();
        let target = layout_root.path().join("artifacts/wrapped-tool/1.2.0");
        assert_eq!(
            fs::read(target.join("tool-1.2.0/bin/tool")).unwrap(),
            b"wrapped-tool-bytes\n"
        );
        assert_eq!(outcome, None);

        // An inert expose (declared but no view linked) is not validated: the D7 law applies
        // only when a view is linked. Use a fresh version to avoid colliding on the target.
        let inert_recipe = recipe_text(
            "wrapped-tool",
            "1.2.1",
            &fixture("tool-wrapped.zip"),
            WRAPPED_ZIP_HEX,
            Some("bin"),
            None,
        );
        let outcome = install_manifest(&inert_recipe, layout_root.path()).unwrap();
        assert_eq!(outcome, None);
    }
}
