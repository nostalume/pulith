//! Realization wiring for the vtool vertical: install / deactivate / repair a manifest over
//! landed behaviors (zero core admission — everything here is caller-owned composition).
//!
//! One private `realize(mode)` chain is shared by install (create-new) and repair (replace):
//! acquire by source kind, `.materialize()` (verify blake3/sha2 over byte streams, magic-detect
//! zip/tar/tar.gz else copy, publish), then `.link()`/`.link_root()` (the core link law D6/D7).
//! Each arm is one linear chain — the two acquire evidence types differ, no type is ever named.
//! Deactivate re-runs the same materialize (`ReplaceOrCreate`) to obtain the real `Applied`
//! node, then `LocalDeactivate` removes the view (s3-1 law), then commits `Deactivated`.
//! Repair (s2-13 law) folds the journal — the caller's own committed intent, never derived from
//! observation — reconciles the address against the manifest it was given, and on mismatch runs
//! a bounded fresh cycle with a fixed backoff, reported per attempt, report-and-stop. All error
//! paths are named and actionable; private workspaces are RAII-cleaned.
use std::error::Error;
use std::path::{Path, PathBuf};
use std::time::Duration;

use pulith::archive::ArchivePolicy;
use pulith::local::{
    LinkOutcome, LocalAcquire, LocalDeactivate, LocalInspect, LocalObservation, OccupiedViewPolicy,
};
use pulith::net::{RemoteSource, SyncHttpAcquire};
use pulith::{Materialize, MaterializeMode};

use crate::manifest::{Journal, Phase, Record, Resolved, Source, StateError};

/// Link the declared view of a freshly materialized tree (per-format arm of the heterogeneous
/// acquire chain; a macro keeps one linear chain per arm without naming the type).
macro_rules! link_view {
    ($materialized:expr, $view:expr, $expose:expr) => {{
        let materialized = $materialized;
        match $view.as_deref() {
            None => None,
            Some(view) => Some(match $expose.as_deref() {
                Some(expose) => materialized.link(view, expose, OccupiedViewPolicy::AutoSwitch)?,
                None => materialized.link_root(view, OccupiedViewPolicy::AutoSwitch)?,
            }),
        }
    }};
}

impl Resolved {
    /// The shared realization chain: acquire (create-new or replace), materialize, and link the
    /// declared view (`None` when the manifest declares no view). The private RAII workspace is
    /// cleaned on every exit; the publication law never creates the target parent, so the
    /// vertical creates it (it owns the `artifacts/<name>` structure).
    fn realize(
        &self,
        mode: MaterializeMode,
        root: &Path,
    ) -> Result<Option<LinkOutcome>, Box<dyn Error + Send + Sync>> {
        ensure_parent(&self.target)?;
        let workspace_root = root.join(".pulith-work").join(format!(
            "{}-{}",
            self.manifest.name.as_str(),
            self.manifest.version.as_str()
        ));
        let _work = WorkDir::create(&workspace_root)?;
        let digest = &self.hash;

        let outcome = match &self.source {
            Source::Local { path } => {
                let materialized = LocalAcquire
                    .acquire(Materialize::new(
                        self.manifest.name.clone().into_string(),
                        (path).into(),
                        self.target.clone(),
                        mode,
                    ))?
                    .materialize(digest.clone(), &workspace_root, ArchivePolicy::new())?;
                link_view!(materialized, &self.view, &self.manifest.expose)
            }
            Source::Url { url } => {
                let materialized = SyncHttpAcquire::default()
                    .acquire(Materialize::new(
                        self.manifest.name.clone().into_string(),
                        RemoteSource::new(url.clone()),
                        self.target.clone(),
                        mode,
                    ))?
                    .materialize(digest.clone(), &workspace_root, ArchivePolicy::new())?;
                link_view!(materialized, &self.view, &self.manifest.expose)
            }
        };
        Ok(outcome)
    }

    /// Acquire, materialize, and link this resolved manifest under `root`; journal-before-
    /// acknowledge: the effect ran, the `Installed` intent is committed (next generation,
    /// fsync'd) before the caller sees success.
    pub fn install(self, root: &Path) -> Result<Option<LinkOutcome>, Box<dyn Error + Send + Sync>> {
        let outcome = self.realize(MaterializeMode::CreateNew, root)?;
        commit(
            root,
            self.manifest.name.as_str(),
            self.manifest.version.as_str(),
            Phase::Installed,
        )?;
        Ok(outcome)
    }

    /// Remove the active view without touching foreign objects (the s3-1 `LocalDeactivate`
    /// law). The cross-process caller has no stored receipt, so this is a fresh authorized
    /// cycle (s2-13 D3): re-materialize `ReplaceOrCreate` to obtain the real `Applied` node,
    /// then deactivate, then commit the `Deactivated` intent.
    pub fn deactivate(self, root: &Path) -> Result<(), Box<dyn Error + Send + Sync>> {
        let Some(view) = self.view else {
            return Ok(());
        };
        // Move the per-cycle inputs out once (the arms are mutually exclusive, so each is
        // consumed exactly once); only the journal-facing name is cloned (two consumers).
        let name = self.manifest.name.into_string();
        let version = self.manifest.version.into_string();
        let target = self.target;
        let digest = self.hash;
        let source = self.source;

        ensure_parent(&target)?;
        let workspace_root = root.join(".pulith-work").join(format!("{name}-{version}"));
        let _work = WorkDir::create(&workspace_root)?;

        match source {
            Source::Local { path } => {
                let materialized = LocalAcquire
                    .acquire(Materialize::new(
                        name.clone(),
                        path,
                        target,
                        MaterializeMode::ReplaceOrCreate,
                    ))?
                    .materialize(digest, &workspace_root, ArchivePolicy::new())?;
                LocalDeactivate.activate(materialized, view)?;
            }
            Source::Url { url } => {
                let materialized = SyncHttpAcquire::default()
                    .acquire(Materialize::new(
                        name.clone(),
                        RemoteSource::new(url),
                        target,
                        MaterializeMode::ReplaceOrCreate,
                    ))?
                    .materialize(digest, &workspace_root, ArchivePolicy::new())?;
                LocalDeactivate.activate(materialized, view)?;
            }
        }

        commit(root, &name, &version, Phase::Deactivated)?;
        Ok(())
    }
}

/// Per-address outcome of a repair pass: satisfied, repaired (fresh cycle committed), or failed
/// (budget exhausted — report-and-stop, no convergence claim).
#[derive(Debug, Default)]
pub struct RepairReport {
    pub satisfied: Vec<String>,
    pub repaired: Vec<String>,
    pub failed: Vec<String>,
    pub attempts: Vec<String>,
}

impl std::fmt::Display for RepairReport {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(formatter, "satisfied: {}", self.satisfied.join(", "))?;
        writeln!(formatter, "repaired: {}", self.repaired.join(", "))?;
        writeln!(formatter, "failed: {}", self.failed.join(", "))?;
        for attempt in &self.attempts {
            writeln!(formatter, "  attempt: {attempt}")?;
        }
        Ok(())
    }
}

/// Run one repair pass for the given manifest address: the expectation is the caller's own
/// committed intent — the journal's latest phase for `name@version` (never derived from
/// observation) — plus the manifest's content. A never-installed or explicitly deactivated
/// address is left alone (repair does not resurrect). On a mismatch with observation, run a
/// fresh authorized cycle, bounded by `attempts` with a fixed `backoff`, report-and-stop.
/// Ctrl-C stops the process by the OS default (the loop is not caught).
pub fn repair(
    resolved: &Resolved,
    root: &Path,
    attempts: usize,
    backoff: Duration,
) -> Result<RepairReport, StateError> {
    let journal = Journal::open(root)?;
    let mut report = RepairReport::default();
    let address = || {
        format!(
            "{}@{}",
            resolved.manifest.name.as_str(),
            resolved.manifest.version.as_str()
        )
    };

    let records = journal.read()?;
    let Some(latest) = records
        .iter()
        .filter(|record| {
            record.name == resolved.manifest.name.as_str()
                && record.version == resolved.manifest.version.as_str()
        })
        .max_by_key(|record| record.generation)
    else {
        return Ok(report); // never installed: nothing to repair
    };
    if latest.phase != Phase::Installed {
        return Ok(report); // explicitly deactivated: repair does not resurrect
    }
    if is_satisfied(resolved) {
        report.satisfied.push(address());
        return Ok(report);
    }

    let mut success = false;
    for attempt in 1..=attempts {
        match resolved.realize(MaterializeMode::ReplaceOrCreate, root) {
            Ok(_) => {
                commit(
                    root,
                    resolved.manifest.name.as_str(),
                    resolved.manifest.version.as_str(),
                    Phase::Installed,
                )?;
                report.repaired.push(address());
                success = true;
                break;
            }
            Err(error) => {
                report
                    .attempts
                    .push(format!("{} attempt {attempt}: {error}", address()));
                std::thread::sleep(backoff);
            }
        }
    }
    if !success {
        report.failed.push(address());
    }
    Ok(report)
}

/// The committed intent is satisfied when the published tree is a directory and, if a view was
/// declared, the view is a directory symlink (the observation is read-only).
fn is_satisfied(resolved: &Resolved) -> bool {
    let target_ok = matches!(
        LocalInspect.observe(&resolved.target),
        Ok(LocalObservation::Directory)
    );
    if !target_ok {
        return false;
    }
    match &resolved.view {
        None => true,
        Some(view) => matches!(
            LocalInspect.observe(view),
            Ok(LocalObservation::SymlinkToDirectory)
        ),
    }
}

/// Journal the committed intent (journal-before-acknowledge): the next generation for this
/// address, fsync'd before the caller sees success. The record is the caller's own expectation —
/// a repair reconciles against it, never against observation.
fn commit(root: &Path, name: &str, version: &str, phase: Phase) -> Result<(), StateError> {
    let mut journal = Journal::open(root)?;
    let generation = journal
        .read()?
        .iter()
        .filter(|record| record.name == name && record.version == version)
        .map(|record| record.generation)
        .max()
        .unwrap_or(0)
        + 1;
    journal.append(&Record {
        name: name.to_string(),
        version: version.to_string(),
        phase,
        generation,
    })?;
    Ok(())
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

    fn make_directory_symlink(target: &Path, view: &Path) {
        #[cfg(windows)]
        std::os::windows::fs::symlink_dir(target, view).unwrap();
        #[cfg(not(windows))]
        std::os::unix::fs::symlink(target, view).unwrap();
    }

    /// A journal seed for a repair address (the committed intent repair reconciles against).
    fn seed_intent(root: &Path, name: &str, version: &str, phase: Phase) {
        let mut journal = Journal::open(root).unwrap();
        journal
            .append(&Record {
                name: name.to_string(),
                version: version.to_string(),
                phase,
                generation: 1,
            })
            .unwrap();
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

    // --- repair controller tests ---

    fn resolve_manifest(text: &str, layout_root: &Path) -> Resolved {
        crate::manifest::Manifest::parse(text)
            .expect("parse manifest")
            .resolve(layout_root)
            .expect("resolve")
    }

    /// A local-source recipe whose source directory exists (real bytes for the fresh cycle).
    fn local_recipe(name: &str, source: &Path, link_at: Option<&Path>) -> String {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(std::fs::read(source.join("bin/tool")).unwrap());
        let hex: String = hasher
            .finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        recipe_text(
            name,
            "1.0.0",
            &source.to_string_lossy(),
            &hex,
            Some("bin"),
            link_at,
        )
    }

    #[test]
    fn repair_reports_satisfied_when_tree_and_view_match_the_intent() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("source");
        fs::create_dir_all(source.join("bin")).unwrap();
        fs::write(source.join("bin/tool"), b"ok").unwrap();
        let target = root.path().join("artifacts/demo/1.0.0");
        fs::create_dir_all(&target).unwrap();
        let view = root.path().join("views/demo");
        fs::create_dir_all(view.parent().unwrap()).unwrap();
        make_directory_symlink(&target, &view);

        let recipe = local_recipe("demo", &source, Some(&view));
        let resolved = resolve_manifest(&recipe, root.path());
        seed_intent(root.path(), "demo", "1.0.0", Phase::Installed);
        let report = repair(&resolved, root.path(), 3, Duration::from_millis(1)).unwrap();
        assert!(report.satisfied.iter().any(|a| a == "demo@1.0.0"));
        assert!(report.repaired.is_empty());
    }

    #[test]
    fn repair_restores_a_missing_view_and_commits_the_next_generation() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("source");
        fs::create_dir_all(source.join("bin")).unwrap();
        fs::write(source.join("bin/tool"), b"repair me").unwrap();
        let target = root.path().join("artifacts/demo/1.0.0");
        fs::create_dir_all(&target).unwrap();
        let view = root.path().join("views/demo");
        fs::create_dir_all(view.parent().unwrap()).unwrap();

        let recipe = local_recipe("demo", &source, Some(&view));
        let resolved = resolve_manifest(&recipe, root.path());
        seed_intent(root.path(), "demo", "1.0.0", Phase::Installed);
        let report = repair(&resolved, root.path(), 3, Duration::from_millis(1)).unwrap();
        assert!(
            report.repaired.iter().any(|a| a == "demo@1.0.0"),
            "attempts: {:?}",
            report.attempts
        );
        assert!(view.is_symlink() || view.join("tool").exists());
        // The repair committed the next generation (supersede: latest record wins).
        let records = Journal::open(root.path()).unwrap().read().unwrap();
        let latest = records.iter().max_by_key(|r| r.generation).unwrap();
        assert_eq!(latest.generation, 2);
    }

    #[test]
    fn repair_does_not_resurrect_a_deactivated_address() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("source");
        fs::create_dir_all(source.join("bin")).unwrap();
        fs::write(source.join("bin/tool"), b"quiet").unwrap();

        let recipe = local_recipe("demo", &source, None);
        let resolved = resolve_manifest(&recipe, root.path());
        seed_intent(root.path(), "demo", "1.0.0", Phase::Deactivated);
        let report = repair(&resolved, root.path(), 3, Duration::from_millis(1)).unwrap();
        assert!(report.satisfied.is_empty());
        assert!(report.repaired.is_empty());
        assert!(report.failed.is_empty());
    }

    #[test]
    fn repair_budget_exhaustion_reports_and_stops() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("source");
        fs::create_dir_all(source.join("bin")).unwrap();
        fs::write(source.join("bin/tool"), b"budget").unwrap();

        // The resolved source points at a missing path → the fresh cycle must fail every attempt.
        let recipe = local_recipe("demo", &source, None);
        let mut resolved = resolve_manifest(&recipe, root.path());
        resolved.source = Source::Local {
            path: root.path().join("missing-source"),
        };
        seed_intent(root.path(), "demo", "1.0.0", Phase::Installed);
        let report = repair(&resolved, root.path(), 2, Duration::from_millis(1)).unwrap();
        assert!(report.failed.iter().any(|a| a == "demo@1.0.0"));
        assert_eq!(report.attempts.len(), 2, "each attempt is reported");
    }
}
