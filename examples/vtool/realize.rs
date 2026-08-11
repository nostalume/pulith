//! Caller-owned vtool realization: acquire, verify, prepare, apply, link, and state repair.
use std::path::Path;
use std::time::Duration;

use pulith::archive::ArchivePolicy;
use pulith::local::{
    LinkChange, LocalExpectation, LocalMaterial, LocalReconciliation, LocalSource, LocalTarget,
};
use pulith::net::RemoteSource;
use pulith::{Acquire, Inspect, Link, Reconcile, Unlink, Verify};

use crate::manifest::{Phase, Resolved, Source, State, StateError};

type BoxResult<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

impl Resolved {
    fn realize(source: Source, hash: pulith::hash::DigestValue, target: &Path) -> BoxResult<()> {
        let target = LocalTarget::new(target)?;
        let stage = target.stage()?;
        let material = match source {
            Source::Local { path } => LocalSource::new(path)?.acquire()?,
            Source::Url { url } => RemoteSource::new(*url).prepare()?.acquire()?.0,
        };
        let material = match material {
            LocalMaterial::Directory { .. } => material,
            _ => material.verify(hash)?.0,
        };
        let (tree, _) = material.prepare(stage, ArchivePolicy::new())?;
        tree.publish(target)?;
        Ok(())
    }

    pub fn install(self, root: &Path) -> BoxResult<()> {
        Self::realize(self.source, self.hash, &self.target)?;
        commit(
            root,
            self.manifest.name.as_str(),
            self.manifest.version.as_str(),
            Phase::Installed,
        )?;
        Ok(())
    }

    pub fn activate(&self, root: &Path) -> BoxResult<LinkChange> {
        let view = self
            .view
            .as_deref()
            .ok_or_else(|| std::io::Error::other("manifest does not declare an activation view"))?;
        let target = LocalTarget::new(&self.target)?;
        let outcome = match self.manifest.expose.as_deref() {
            Some(expose) => target.link(view, expose)?.change,
            None => target.link_root(view)?.change,
        };
        commit(
            root,
            self.manifest.name.as_str(),
            self.manifest.version.as_str(),
            Phase::Installed,
        )?;
        Ok(outcome)
    }

    pub fn deactivate(self, root: &Path) -> BoxResult<()> {
        let Some(view) = self.view else {
            return Ok(());
        };
        LocalTarget::new(view)?.unlink()?;
        commit(
            root,
            self.manifest.name.as_str(),
            self.manifest.version.as_str(),
            Phase::Deactivated,
        )?;
        Ok(())
    }
}

#[derive(Debug, Default)]
pub struct RepairReport {
    pub satisfied: Vec<String>,
    pub repaired: Vec<String>,
    pub failed: Vec<String>,
    pub attempts: Vec<String>,
}

pub fn repair(
    resolved: &Resolved,
    root: &Path,
    attempts: usize,
    backoff: Duration,
) -> Result<RepairReport, StateError> {
    let state = State::open(root)?;
    let mut report = RepairReport::default();
    let address = || {
        format!(
            "{}@{}",
            resolved.manifest.name.as_str(),
            resolved.manifest.version.as_str()
        )
    };

    let records = state.read()?;
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

    for attempt in 1..=attempts {
        let target_ready = matches!(
            LocalTarget::new(&resolved.target)
                .and_then(|target| target.inspect(()))
                .map(|(observation, _)| observation),
            Ok(pulith::local::LocalObservation::Directory)
        );
        let realized = if target_ready {
            Ok(())
        } else {
            Resolved::realize(
                resolved.source.clone(),
                resolved.hash.clone(),
                &resolved.target,
            )
        };
        let repaired = realized.and_then(|()| match &resolved.view {
            Some(_) => resolved.activate(root).map(|_| ()),
            None => Ok(()),
        });
        match repaired {
            Ok(()) => {
                if resolved.view.is_none() {
                    commit(
                        root,
                        resolved.manifest.name.as_str(),
                        resolved.manifest.version.as_str(),
                        Phase::Installed,
                    )?;
                }
                report.repaired.push(address());
                return Ok(report);
            }
            Err(error) => {
                report
                    .attempts
                    .push(format!("{} attempt {attempt}: {error}", address()));
                std::thread::sleep(backoff);
            }
        }
    }
    report.failed.push(address());
    Ok(report)
}

fn is_satisfied(resolved: &Resolved) -> bool {
    reconciles(&resolved.target, LocalExpectation::Directory)
        && resolved
            .view
            .as_deref()
            .is_none_or(|view| reconciles(view, LocalExpectation::Symlink))
}

fn reconciles(path: &Path, expectation: LocalExpectation) -> bool {
    matches!(
        LocalTarget::new(path)
            .and_then(|target| target.inspect(()))
            .map(|(observed, _)| observed.reconcile(expectation)),
        Ok(Ok((LocalReconciliation::Matches, _)))
    )
}

fn commit(root: &Path, name: &str, version: &str, phase: Phase) -> Result<(), StateError> {
    State::open(root)?.commit(name, version, phase)
}

#[cfg(test)]
#[path = "../../tests/examples/vtool/realize.rs"]
mod tests;
