//! Caller-owned vtool realization: capture, verify, stage, publish, link, and repair.
use std::path::Path;
use std::time::Duration;

use pulith::archive::{ArchivePolicy, extract_tar, extract_zip};
use pulith::local::{
    LinkChange, LinkError, LocalError, LocalObservation, LocalPath, TreeAdmission, ViewStatus,
};
use pulith::net::{RemoteSource, UreqResources};

use crate::manifest::{ArchiveFormat, Phase, Resolved, Source, State, StateError};

type BoxResult<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

fn prepare_archive(
    file: pulith::local::FileSnapshot,
    format: ArchiveFormat,
    stage: pulith::local::StagedTree,
) -> BoxResult<pulith::local::StagedTree> {
    match format {
        ArchiveFormat::Plain => return Ok(file.stage_into(stage)?),
        ArchiveFormat::Zip => extract_zip(file, stage.root(), ArchivePolicy::new())?,
        ArchiveFormat::Tar => extract_tar(file, stage.root(), ArchivePolicy::new())?,
    };
    Ok(stage)
}

impl Resolved {
    fn realize(
        source: Source,
        hash: pulith::hash::DigestValue,
        target: &Path,
    ) -> BoxResult<RepairEvidence> {
        let target = LocalPath::new(target)?;
        let format = source.format();
        let stage = match source {
            Source::Local { path, .. } => {
                let file = match LocalPath::new(&path)?.open_file() {
                    Ok(Some(file)) => file,
                    Ok(None) => return Err(LocalError::MissingSource(path).into()),
                    Err(error) => return Err(error.into()),
                };
                if format == ArchiveFormat::Plain {
                    target.publish_verified_file(file, &hash)?;
                    return Ok(RepairEvidence::FileDigest);
                }
                let (snapshot, _) = file.snapshot_verified(&hash)?;
                prepare_archive(snapshot, format, target.stage()?)?
            }
            Source::Url { url, .. } => {
                let (snapshot, _) = UreqResources::default().download(RemoteSource::new(*url))?;
                let (snapshot, _) = snapshot.verify_digest(&hash)?;
                prepare_archive(snapshot, format, target.stage()?)?
            }
        };
        stage.publish(target, TreeAdmission::Absent)?;
        Ok(match format {
            ArchiveFormat::Plain => RepairEvidence::FileDigest,
            ArchiveFormat::Zip | ArchiveFormat::Tar => RepairEvidence::ArchiveInput,
        })
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
        let target = LocalPath::new(&self.target)?;
        let outcome = target
            .link_view(view, self.manifest.expose.as_deref())?
            .change;
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
        LocalPath::new(view)?.unlink_view()?;
        commit(
            root,
            self.manifest.name.as_str(),
            self.manifest.version.as_str(),
            Phase::Deactivated,
        )?;
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RepairEvidence {
    FileDigest,
    ArchiveDirectory,
    ArchiveInput,
}

impl std::fmt::Display for RepairEvidence {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::FileDigest => "file digest verified",
            Self::ArchiveDirectory => "archive directory present (contents unchecked)",
            Self::ArchiveInput => "archive input verified",
        })
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum RepairOutcome {
    NotInstalled,
    Deactivated,
    Unchanged(RepairEvidence),
    Repaired(RepairEvidence),
    Failed { published: Option<RepairEvidence> },
}

#[derive(Debug)]
pub struct RepairReport {
    pub outcome: RepairOutcome,
    pub diagnostics: Vec<String>,
}

pub fn repair(
    resolved: &Resolved,
    root: &Path,
    attempts: usize,
    backoff: Duration,
) -> Result<RepairReport, StateError> {
    let state = State::open(root)?;
    let mut failures = Vec::new();
    let mut published = None;

    let records = state.read()?;
    let Some(latest) = records
        .iter()
        .filter(|record| {
            record.name == resolved.manifest.name.as_str()
                && record.version == resolved.manifest.version.as_str()
        })
        .max_by_key(|record| record.generation)
    else {
        return Ok(RepairReport {
            outcome: RepairOutcome::NotInstalled,
            diagnostics: failures,
        });
    };
    if latest.phase != Phase::Installed {
        return Ok(RepairReport {
            outcome: RepairOutcome::Deactivated,
            diagnostics: failures,
        });
    }

    let mut observed = inspect_target(resolved);
    if let Ok(TargetState::Ready(evidence)) = &observed
        && view_matches(resolved)
    {
        return Ok(RepairReport {
            outcome: RepairOutcome::Unchanged(*evidence),
            diagnostics: failures,
        });
    }
    for attempt in 1..=attempts {
        let evidence = match observed {
            Ok(TargetState::Conflict(error)) => {
                failures.push(error);
                break;
            }
            Ok(TargetState::Ready(evidence)) => Ok(evidence),
            Ok(TargetState::Missing) => Resolved::realize(
                resolved.source.clone(),
                resolved.hash.clone(),
                &resolved.target,
            )
            .inspect(|evidence| published = Some(*evidence)),
            Err(error) => Err(error),
        };
        let repaired = evidence.and_then(|evidence| {
            if resolved.view.is_some() {
                resolved.activate(root)?;
            } else {
                commit(
                    root,
                    resolved.manifest.name.as_str(),
                    resolved.manifest.version.as_str(),
                    Phase::Installed,
                )?;
            }
            Ok(evidence)
        });
        match repaired {
            Ok(evidence) => {
                return Ok(RepairReport {
                    outcome: RepairOutcome::Repaired(evidence),
                    diagnostics: failures,
                });
            }
            Err(error) => {
                let view_conflict = matches!(
                    error.downcast_ref::<LinkError>(),
                    Some(LinkError::ViewConflict { .. })
                );
                failures.push(format!("attempt {attempt}: {error}"));
                if view_conflict || attempt == attempts {
                    break;
                }
                std::thread::sleep(backoff);
                observed = inspect_target(resolved);
            }
        }
    }
    Ok(RepairReport {
        outcome: RepairOutcome::Failed { published },
        diagnostics: failures,
    })
}

fn view_matches(resolved: &Resolved) -> bool {
    resolved.view.as_deref().is_none_or(|view| {
        LocalPath::new(&resolved.target).is_ok_and(|target| {
            matches!(
                target.inspect_view(view, resolved.manifest.expose.as_deref()),
                Ok(ViewStatus::Matches)
            )
        })
    })
}

enum TargetState {
    Missing,
    Ready(RepairEvidence),
    Conflict(String),
}

fn inspect_target(resolved: &Resolved) -> BoxResult<TargetState> {
    let target = LocalPath::new(&resolved.target)?;
    if resolved.source.format() != ArchiveFormat::Plain {
        return match target.inspect_entry()? {
            LocalObservation::Missing => Ok(TargetState::Missing),
            LocalObservation::Directory => Ok(TargetState::Ready(RepairEvidence::ArchiveDirectory)),
            entry => Ok(TargetState::Conflict(
                LocalError::OccupiedTarget {
                    path: resolved.target.clone(),
                    kind: entry.kind(),
                }
                .to_string(),
            )),
        };
    }
    let file = match target.open_file() {
        Ok(Some(file)) => file,
        Ok(None) => return Ok(TargetState::Missing),
        Err(error @ LocalError::NotRegularFile { .. }) => {
            return Ok(TargetState::Conflict(error.to_string()));
        }
        Err(error) => return Err(error.into()),
    };
    let descriptor = file.digest(resolved.hash.algorithm())?;
    if descriptor.digest != resolved.hash {
        return Ok(TargetState::Conflict(format!(
            "installed file digest differs at {}; review the file before reinstalling",
            resolved.target.display()
        )));
    }
    Ok(TargetState::Ready(RepairEvidence::FileDigest))
}

fn commit(root: &Path, name: &str, version: &str, phase: Phase) -> Result<(), StateError> {
    State::open(root)?.commit(name, version, phase)
}

#[cfg(test)]
#[path = "../../tests/examples/vtool/realize.rs"]
mod tests;
