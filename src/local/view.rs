//! Local activation and active-view switch: `LocalActivate`, `LocalSwitch`, and `LocalDeactivate`.
//!
//! Owns the exposure law: one directory symlink per activation, native replacement of exactly one
//! existing directory-symlink view for the switch, removal of exactly one directory-symlink view
//! for the deactivation, and read-only post-observation of the exposed view. It never copies the
//! tree, publishes a target, retains a prior generation, or persists active state.
//! Platform-specific link mechanics are cfg-split here. Feature-gated on `local`.
#![allow(clippy::result_large_err)] // receipt-preserving errors are the family law
#![allow(clippy::type_complexity)] // the node methods echo the family's full typestate
use std::fmt;
use std::fs;
#[cfg(windows)]
use std::fs::File;
use std::io;
use std::path::{Path, PathBuf};

use crate::{Activate, Activated, Applied, EvidenceChain, Materialize};

use super::materialize::{MaterializeEvidence, Materialized};
use super::{LocalError, LocalInspect, LocalObservation};
/// Link strategy used by a local active-view activation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LocalActivationStrategy {
    DirectorySymlink,
}

/// Evidence that a published local directory was exposed at one active view.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LocalActivationEvidence {
    pub source: PathBuf,
    pub strategy: LocalActivationStrategy,
    pub view_observation: Option<LocalObservation>,
}

/// Backend that committed one active-view replacement.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LocalSwitchBackend {
    UnixRename,
    WindowsFileRenameInfoExPosix,
}

/// Evidence from deliberately replacing one existing active directory-symlink view.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LocalSwitchEvidence {
    pub previous_source: PathBuf,
    pub current_source: PathBuf,
    pub strategy: LocalActivationStrategy,
    pub backend: LocalSwitchBackend,
    pub view_observation: Option<LocalObservation>,
}

/// Local active-view replacement adapter. It replaces only an existing directory symbolic-link name.
#[derive(Clone, Copy, Debug, Default)]
pub struct LocalSwitch;

type LocalSwitched<E> = Activated<PathBuf, EvidenceChain<E, LocalSwitchEvidence>>;

/// Failure of a deliberate local active-view replacement.
#[derive(Debug)]
pub enum LocalSwitchError<N, E> {
    SourceNotDirectory {
        applied: Applied<N, E>,
        view: PathBuf,
        observed: LocalObservation,
    },
    ViewNotSymlink {
        applied: Applied<N, E>,
        view: PathBuf,
        observed: LocalObservation,
    },
    BeforeSwitch {
        applied: Applied<N, E>,
        view: PathBuf,
        cause: LocalError,
    },
    ViewBusy {
        applied: Applied<N, E>,
        view: PathBuf,
        cause: LocalError,
    },
    CapabilityUnavailable {
        applied: Applied<N, E>,
        view: PathBuf,
        cause: LocalError,
    },
    Cleanup {
        applied: Applied<N, E>,
        view: PathBuf,
        cause: LocalError,
        cleanup: LocalError,
    },
    AfterSwitch {
        activated: LocalSwitched<E>,
        cause: LocalError,
    },
}

impl<N, E> fmt::Display for LocalSwitchError<N, E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SourceNotDirectory { .. } => f.write_str("switch source is not a directory"),
            Self::ViewNotSymlink { .. } => f.write_str("active view is not a directory symlink"),
            Self::BeforeSwitch { cause, .. } => write!(f, "active-view switch failed: {cause}"),
            Self::ViewBusy { cause, .. } => write!(f, "active-view switch is busy: {cause}"),
            Self::CapabilityUnavailable { cause, .. } => {
                write!(f, "active-view switch is unavailable: {cause}")
            }
            Self::Cleanup { cause, cleanup, .. } => write!(
                f,
                "active-view switch failed: {cause}; staged-view cleanup also failed: {cleanup}"
            ),
            Self::AfterSwitch { cause, .. } => write!(
                f,
                "active-view switch completed but post-observation failed: {cause}"
            ),
        }
    }
}
impl<N: fmt::Debug, E: fmt::Debug> std::error::Error for LocalSwitchError<N, E> {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::BeforeSwitch { cause, .. }
            | Self::ViewBusy { cause, .. }
            | Self::CapabilityUnavailable { cause, .. }
            | Self::Cleanup { cause, .. }
            | Self::AfterSwitch { cause, .. } => Some(cause),
            _ => None,
        }
    }
}
/// Local directory-symlink activation adapter.
#[derive(Clone, Copy, Debug, Default)]
pub struct LocalActivate;

type LocalActivated<E> = Activated<PathBuf, EvidenceChain<E, LocalActivationEvidence>>;

/// An activation failure that distinguishes an unperformed effect from an unavailable
/// post-activation observation.
#[derive(Debug)]
pub enum LocalActivateError<N, E> {
    SourceNotDirectory {
        applied: Applied<N, E>,
        view: PathBuf,
        observed: LocalObservation,
    },
    ViewAlreadyExists {
        applied: Applied<N, E>,
        view: PathBuf,
        observed: LocalObservation,
    },
    BeforeActivation {
        applied: Applied<N, E>,
        view: PathBuf,
        cause: LocalError,
    },
    CapabilityUnavailable {
        applied: Applied<N, E>,
        view: PathBuf,
        cause: LocalError,
    },
    AfterActivation {
        activated: LocalActivated<E>,
        cause: LocalError,
    },
}

impl<N, E> fmt::Display for LocalActivateError<N, E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SourceNotDirectory { .. } => f.write_str("activation source is not a directory"),
            Self::ViewAlreadyExists { .. } => f.write_str("active view already exists"),
            Self::BeforeActivation { cause, .. } => write!(f, "local activation failed: {cause}"),
            Self::CapabilityUnavailable { cause, .. } => {
                write!(f, "directory symlink activation is unavailable: {cause}")
            }
            Self::AfterActivation { cause, .. } => {
                write!(
                    f,
                    "activation completed but post-observation failed: {cause}"
                )
            }
        }
    }
}

impl<N: fmt::Debug, E: fmt::Debug> std::error::Error for LocalActivateError<N, E> {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::BeforeActivation { cause, .. }
            | Self::CapabilityUnavailable { cause, .. }
            | Self::AfterActivation { cause, .. } => Some(cause),
            Self::SourceNotDirectory { .. } | Self::ViewAlreadyExists { .. } => None,
        }
    }
}
impl LocalActivate {
    /// Inherent mirror of [`Activate::activate`] — callable without importing the trait.
    pub fn activate<N, V>(
        &self,
        applied: N,
        view: V,
    ) -> Result<<Self as Activate<N, V>>::Output, <Self as Activate<N, V>>::Error>
    where
        Self: Activate<N, V>,
    {
        Activate::activate(self, applied, view)
    }
}

impl<I, S, E> Activate<crate::local::LocalApplied<I, S, E>, PathBuf> for LocalActivate {
    type Error = LocalActivateError<Materialize<I, S, PathBuf>, E>;
    type Output = LocalActivated<E>;

    fn activate(
        &self,
        applied: crate::local::LocalApplied<I, S, E>,
        view: PathBuf,
    ) -> Result<Self::Output, Self::Error> {
        let source = applied.input.target.clone();
        perform_activation(applied, view, source)
    }
}

/// The activate-family error over a materialization request (receipt-preserving by law).
type ActivationError<I, S, E> = LocalActivateError<Materialize<I, S, PathBuf>, E>;
/// The switch-family error over a materialization request (receipt-preserving by law).
type SwitchError<I, S, E> = LocalSwitchError<Materialize<I, S, PathBuf>, E>;

/// Expose-aware activation: link a caller-selected subpath of the published tree.
///
/// Same law as the `Activate` trait entry, with `source = target.join(expose)`: the source must
/// be a directory, the view must be missing, and the post-observation must see a symlink. The
/// evidence records the exposed source path. This is the vertical's justified gap admission
/// (S3.3-A1); the trait input stays `PathBuf`.
impl LocalActivate {
    pub fn activate_at<I, S, E>(
        &self,
        applied: crate::local::LocalApplied<I, S, E>,
        view: PathBuf,
        expose: &Path,
    ) -> Result<LocalActivated<E>, ActivationError<I, S, E>> {
        let source = applied.input.target.join(expose);
        perform_activation(applied, view, source)
    }
}

fn perform_activation<I, S, E>(
    applied: crate::local::LocalApplied<I, S, E>,
    view: PathBuf,
    source: PathBuf,
) -> Result<LocalActivated<E>, ActivationError<I, S, E>> {
    let source_observation = match observe_activation_path(&source) {
        Ok(observation) => observation,
        Err(cause) => {
            return Err(LocalActivateError::BeforeActivation {
                applied,
                view,
                cause,
            });
        }
    };
    if source_observation != LocalObservation::Directory {
        return Err(LocalActivateError::SourceNotDirectory {
            applied,
            view,
            observed: source_observation,
        });
    }

    let parent = match view.parent() {
        Some(parent) => parent.to_path_buf(),
        None => {
            return Err(LocalActivateError::BeforeActivation {
                applied,
                view,
                cause: LocalError::io(
                    "inspect active view parent",
                    Path::new(""),
                    io::Error::new(io::ErrorKind::InvalidInput, "active view has no parent"),
                ),
            });
        }
    };
    let parent_observation = match observe_activation_path(&parent) {
        Ok(observation) => observation,
        Err(cause) => {
            return Err(LocalActivateError::BeforeActivation {
                applied,
                view,
                cause,
            });
        }
    };
    if parent_observation != LocalObservation::Directory {
        return Err(LocalActivateError::BeforeActivation {
            applied,
            view,
            cause: LocalError::io(
                "inspect active view parent",
                parent,
                io::Error::new(
                    io::ErrorKind::NotFound,
                    "active view parent is not a directory",
                ),
            ),
        });
    }

    let view_observation = match observe_activation_path(&view) {
        Ok(observation) => observation,
        Err(cause) => {
            return Err(LocalActivateError::BeforeActivation {
                applied,
                view,
                cause,
            });
        }
    };
    if view_observation != LocalObservation::Missing {
        return Err(LocalActivateError::ViewAlreadyExists {
            applied,
            view,
            observed: view_observation,
        });
    }

    if let Err(error) = create_directory_symlink(&source, &view) {
        let capability_unavailable = directory_symlink_capability_unavailable(&error);
        let cause = LocalError::io("create active directory symlink", &view, error);
        return if capability_unavailable {
            Err(LocalActivateError::CapabilityUnavailable {
                applied,
                view,
                cause,
            })
        } else {
            Err(LocalActivateError::BeforeActivation {
                applied,
                view,
                cause,
            })
        };
    }

    match observe_activation_path(&view) {
        Ok(observed) => {
            let activated =
                activation_receipt(view, applied.evidence, source, Some(observed.clone()));
            if observed == LocalObservation::SymlinkToDirectory {
                Ok(activated)
            } else {
                Err(LocalActivateError::AfterActivation {
                    cause: LocalError::io(
                        "verify active directory symlink",
                        &activated.input,
                        io::Error::other(format!("expected symlink, observed {observed:?}")),
                    ),
                    activated,
                })
            }
        }
        Err(cause) => Err(LocalActivateError::AfterActivation {
            activated: activation_receipt(view, applied.evidence, source, None),
            cause,
        }),
    }
}

impl LocalSwitch {
    /// Inherent mirror of [`Activate::activate`] — callable without importing the trait.
    pub fn activate<N, V>(
        &self,
        applied: N,
        view: V,
    ) -> Result<<Self as Activate<N, V>>::Output, <Self as Activate<N, V>>::Error>
    where
        Self: Activate<N, V>,
    {
        Activate::activate(self, applied, view)
    }
}

impl<I, S, E> Activate<crate::local::LocalApplied<I, S, E>, PathBuf> for LocalSwitch {
    type Error = LocalSwitchError<Materialize<I, S, PathBuf>, E>;
    type Output = LocalSwitched<E>;

    fn activate(
        &self,
        applied: crate::local::LocalApplied<I, S, E>,
        view: PathBuf,
    ) -> Result<Self::Output, Self::Error> {
        let source = applied.input.target.clone();
        perform_switch(applied, view, source)
    }
}

/// Expose-aware switch: natively replace an existing view with a caller-selected subpath.
///
/// Same law as the `Activate` trait entry, with `source = target.join(expose)`. This is the
/// switch twin of `LocalActivate::activate_at` (S3.3-A1).
impl LocalSwitch {
    pub fn activate_at<I, S, E>(
        &self,
        applied: crate::local::LocalApplied<I, S, E>,
        view: PathBuf,
        expose: &Path,
    ) -> Result<LocalSwitched<E>, SwitchError<I, S, E>> {
        let source = applied.input.target.join(expose);
        perform_switch(applied, view, source)
    }
}

fn perform_switch<I, S, E>(
    applied: crate::local::LocalApplied<I, S, E>,
    view: PathBuf,
    source: PathBuf,
) -> Result<LocalSwitched<E>, SwitchError<I, S, E>> {
    let source_observation = match observe_activation_path(&source) {
        Ok(observation) => observation,
        Err(cause) => {
            return Err(LocalSwitchError::BeforeSwitch {
                applied,
                view,
                cause,
            });
        }
    };
    if source_observation != LocalObservation::Directory {
        return Err(LocalSwitchError::SourceNotDirectory {
            applied,
            view,
            observed: source_observation,
        });
    }

    let parent = match view
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        Some(parent) => parent.to_path_buf(),
        None => {
            return Err(LocalSwitchError::BeforeSwitch {
                applied,
                view,
                cause: LocalError::io(
                    "inspect active view parent",
                    Path::new(""),
                    io::Error::new(io::ErrorKind::InvalidInput, "active view has no parent"),
                ),
            });
        }
    };
    let parent_observation = match observe_activation_path(&parent) {
        Ok(observation) => observation,
        Err(cause) => {
            return Err(LocalSwitchError::BeforeSwitch {
                applied,
                view,
                cause,
            });
        }
    };
    if parent_observation != LocalObservation::Directory {
        return Err(LocalSwitchError::BeforeSwitch {
            applied,
            view,
            cause: LocalError::io(
                "inspect active view parent",
                parent,
                io::Error::new(
                    io::ErrorKind::NotFound,
                    "active view parent is not a directory",
                ),
            ),
        });
    }

    let observed = match observe_activation_path(&view) {
        Ok(observation) => observation,
        Err(cause) => {
            return Err(LocalSwitchError::BeforeSwitch {
                applied,
                view,
                cause,
            });
        }
    };
    if observed != LocalObservation::SymlinkToDirectory {
        return Err(LocalSwitchError::ViewNotSymlink {
            applied,
            view,
            observed,
        });
    }
    let previous_source = match fs::read_link(&view) {
        Ok(source) => source,
        Err(error) => {
            return Err(LocalSwitchError::BeforeSwitch {
                applied,
                view: view.clone(),
                cause: LocalError::io("read active directory symlink", &view, error),
            });
        }
    };
    let stage = match unique_switch_stage(&parent, &view, &source) {
        Ok(stage) => stage,
        Err(cause) => {
            return Err(LocalSwitchError::BeforeSwitch {
                applied,
                view: view.clone(),
                cause,
            });
        }
    };
    let backend = match replace_active_view(&stage, &view) {
        Ok(backend) => backend,
        Err(error) => {
            let error_code = error.raw_os_error();
            let cause = LocalError::io("replace active directory symlink", &view, error);
            match remove_staged_active_view(&stage) {
                Ok(()) => {}
                Err(ref cleanup) if cleanup.kind() == io::ErrorKind::NotFound => {}
                Err(cleanup) => {
                    return Err(LocalSwitchError::Cleanup {
                        applied,
                        view,
                        cause,
                        cleanup: LocalError::io(
                            "remove staged active directory symlink",
                            stage,
                            cleanup,
                        ),
                    });
                }
            }
            return if active_view_is_busy(error_code) {
                Err(LocalSwitchError::ViewBusy {
                    applied,
                    view,
                    cause,
                })
            } else if active_view_capability_unavailable(error_code) {
                Err(LocalSwitchError::CapabilityUnavailable {
                    applied,
                    view,
                    cause,
                })
            } else {
                Err(LocalSwitchError::BeforeSwitch {
                    applied,
                    view,
                    cause,
                })
            };
        }
    };

    match observe_activation_path(&view) {
        Ok(LocalObservation::SymlinkToDirectory) => Ok(switch_receipt(
            view,
            applied.evidence,
            previous_source,
            source,
            backend,
            Some(LocalObservation::SymlinkToDirectory),
        )),
        Ok(observed) => {
            let activated = switch_receipt(
                view,
                applied.evidence,
                previous_source,
                source,
                backend,
                Some(observed.clone()),
            );
            Err(LocalSwitchError::AfterSwitch {
                cause: LocalError::io(
                    "verify active directory symlink",
                    &activated.input,
                    io::Error::other(format!("expected symlink, observed {observed:?}")),
                ),
                activated,
            })
        }
        Err(cause) => Err(LocalSwitchError::AfterSwitch {
            activated: switch_receipt(
                view,
                applied.evidence,
                previous_source,
                source,
                backend,
                None,
            ),
            cause,
        }),
    }
}
/// The link law's policy: what to do when the view path is already occupied.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum OccupiedViewPolicy {
    /// An existing directory-symlink view is natively replaced (the vertical's law).
    #[default]
    AutoSwitch,
    /// An occupied view is refused; nothing is replaced.
    Refuse,
}

/// Outcome of the expose-aware link law (D6/D7): a view is always linked.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LinkOutcome {
    /// A new directory-symlink view was created.
    Activated,
    /// An existing directory-symlink view was natively replaced.
    Switched,
}

/// The link law error; receipt-preserving like the activation family.
#[derive(Debug)]
pub enum LinkError<N, E> {
    /// A pre-dispatch observation or the view-parent creation failed.
    BeforeLink {
        applied: Applied<N, E>,
        view: PathBuf,
        cause: LocalError,
    },
    /// The expose subpath is not a relative, non-escaping path (D7 shape law).
    InvalidExpose {
        applied: Applied<N, E>,
        expose: PathBuf,
    },
    /// The exposed path is not a directory in the materialized tree (D7).
    ExposeNotDirectory {
        applied: Applied<N, E>,
        path: PathBuf,
        observed: LocalObservation,
    },
    /// The view path holds an entry that is not a directory-symlink view (D6).
    ViewConflict {
        applied: Applied<N, E>,
        view: PathBuf,
        observed: LocalObservation,
    },
    /// The view was created but the activation failed.
    Activation { cause: LocalActivateError<N, E> },
    /// The existing view was replaced but the switch failed.
    Switch { cause: LocalSwitchError<N, E> },
}

impl<N, E> fmt::Display for LinkError<N, E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BeforeLink { cause, .. } => write!(f, "link failed: {cause}"),
            Self::InvalidExpose { expose, .. } => write!(
                f,
                "expose path {} must be a relative, non-escaping subpath",
                expose.display()
            ),
            Self::ExposeNotDirectory { path, .. } => {
                write!(f, "expose path {} is not a directory", path.display())
            }
            Self::ViewConflict { view, observed, .. } => write!(
                f,
                "view {} holds {observed:?}, which is not a directory-symlink view; nothing replaced",
                view.display()
            ),
            Self::Activation { cause } => write!(f, "view activation failed: {cause}"),
            Self::Switch { cause } => write!(f, "view switch failed: {cause}"),
        }
    }
}

impl<N: fmt::Debug + 'static, E: fmt::Debug + 'static> std::error::Error for LinkError<N, E> {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::BeforeLink { cause, .. } => Some(cause),
            Self::Activation { cause } => Some(cause),
            Self::Switch { cause } => Some(cause),
            Self::InvalidExpose { .. }
            | Self::ExposeNotDirectory { .. }
            | Self::ViewConflict { .. } => None,
        }
    }
}

/// D7 shape law: an expose subpath is relative, non-empty, and cannot escape the tree.
fn is_safe_expose(expose: &Path) -> bool {
    if expose.as_os_str().is_empty() || expose.is_absolute() {
        return false;
    }
    expose
        .components()
        .all(|component| matches!(component, std::path::Component::Normal(_)))
}

impl LocalActivate {
    /// The expose-aware link law (D6/D7): link a view to the `expose` subpath of the
    /// materialized tree, creating the view parent and switching an occupied view per `policy`.
    pub fn link<I, S, E>(
        &self,
        applied: crate::local::LocalApplied<I, S, E>,
        view: &Path,
        expose: &Path,
        policy: OccupiedViewPolicy,
    ) -> Result<LinkOutcome, LinkError<Materialize<I, S, PathBuf>, E>> {
        if !is_safe_expose(expose) {
            return Err(LinkError::InvalidExpose {
                applied,
                expose: expose.to_path_buf(),
            });
        }
        let source = applied.input.target.join(expose);
        link_view(applied, view, source, policy)
    }

    /// The expose-aware link law (D6/D7): link a view to the materialized tree root.
    pub fn link_root<I, S, E>(
        &self,
        applied: crate::local::LocalApplied<I, S, E>,
        view: &Path,
        policy: OccupiedViewPolicy,
    ) -> Result<LinkOutcome, LinkError<Materialize<I, S, PathBuf>, E>> {
        let source = applied.input.target.clone();
        link_view(applied, view, source, policy)
    }
}

impl<I, S, E> Materialized<I, S, E> {
    /// The link law as a node method: link a view to the `expose` subpath of this materialized
    /// tree, creating the view parent and switching an occupied view per `policy`.
    pub fn link(
        self,
        view: &Path,
        expose: &Path,
        policy: OccupiedViewPolicy,
    ) -> Result<
        LinkOutcome,
        LinkError<Materialize<I, S, PathBuf>, EvidenceChain<E, MaterializeEvidence>>,
    > {
        LocalActivate.link(self, view, expose, policy)
    }

    /// The link law as a node method: link a view to this materialized tree root.
    pub fn link_root(
        self,
        view: &Path,
        policy: OccupiedViewPolicy,
    ) -> Result<
        LinkOutcome,
        LinkError<Materialize<I, S, PathBuf>, EvidenceChain<E, MaterializeEvidence>>,
    > {
        LocalActivate.link_root(self, view, policy)
    }
}

/// The shared link dispatch: D7 source observation, view-parent creation, then D6
/// (create when missing, switch an occupied view per policy, refuse any other entry).
fn link_view<I, S, E>(
    applied: crate::local::LocalApplied<I, S, E>,
    view: &Path,
    source: PathBuf,
    policy: OccupiedViewPolicy,
) -> Result<LinkOutcome, LinkError<Materialize<I, S, PathBuf>, E>> {
    // D7: the exposed path must be a directory in the materialized tree.
    let source_observed = match LocalInspect.observe(&source) {
        Ok(observation) => observation,
        Err(cause) => {
            return Err(LinkError::BeforeLink {
                applied,
                view: view.to_path_buf(),
                cause,
            });
        }
    };
    if source_observed != LocalObservation::Directory {
        return Err(LinkError::ExposeNotDirectory {
            applied,
            path: source,
            observed: source_observed,
        });
    }
    // The link law owns the view structure: create the parent before any dispatch.
    if let Some(parent) = view.parent()
        && let Err(cause) = std::fs::create_dir_all(parent)
    {
        return Err(LinkError::BeforeLink {
            applied,
            view: view.to_path_buf(),
            cause: LocalError::io("create view parent", parent, cause),
        });
    }
    let observed = match LocalInspect.observe(view) {
        Ok(observation) => observation,
        Err(cause) => {
            return Err(LinkError::BeforeLink {
                applied,
                view: view.to_path_buf(),
                cause,
            });
        }
    };
    match observed {
        LocalObservation::Missing => perform_activation(applied, view.to_path_buf(), source)
            .map(|_| LinkOutcome::Activated)
            .map_err(|cause| LinkError::Activation { cause }),
        LocalObservation::SymlinkToDirectory => match policy {
            OccupiedViewPolicy::AutoSwitch => perform_switch(applied, view.to_path_buf(), source)
                .map(|_| LinkOutcome::Switched)
                .map_err(|cause| LinkError::Switch { cause }),
            OccupiedViewPolicy::Refuse => Err(LinkError::ViewConflict {
                applied,
                view: view.to_path_buf(),
                observed,
            }),
        },
        observed => Err(LinkError::ViewConflict {
            applied,
            view: view.to_path_buf(),
            observed,
        }),
    }
}

/// Prior view state recorded by a deactivation: what was actually removed or absent.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LocalDeactivatePrior {
    /// A directory-symlink active view was removed.
    DirectorySymlink,
    /// No view existed at the path; the deactivation was a no-op.
    Missing,
}

/// Evidence that one active directory-symlink view is no longer exposed.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LocalDeactivateEvidence {
    /// The view path that was deactivated.
    pub view: PathBuf,
    /// What the adapter actually removed (or that nothing existed).
    pub prior: LocalDeactivatePrior,
}

/// Local active-view deactivation adapter: removes exactly one directory-symlink view.
///
/// The versioned tree the view pointed at is never touched. A missing view is an idempotent
/// no-op (recorded as `LocalDeactivatePrior::Missing`); an entry that is not a directory-symlink
/// view is refused with `LocalDeactivateError::NotActiveView` and left intact.
#[derive(Clone, Copy, Debug, Default)]
pub struct LocalDeactivate;

type LocalDeactivated<E> = Activated<PathBuf, EvidenceChain<E, LocalDeactivateEvidence>>;

/// Failure of a deliberate local active-view deactivation.
#[derive(Debug)]
pub enum LocalDeactivateError<N, E> {
    /// The entry at the view path is not an active directory-symlink view; nothing was removed.
    /// `observed` is the view observation, or for a symlink the observation of its resolved
    /// target (a file symlink or dangling link is refused, not removed).
    NotActiveView {
        applied: Applied<N, E>,
        view: PathBuf,
        observed: LocalObservation,
    },
    BeforeDeactivate {
        applied: Applied<N, E>,
        view: PathBuf,
        cause: LocalError,
    },
    AfterDeactivate {
        deactivated: LocalDeactivated<E>,
        cause: LocalError,
    },
}

impl<N, E> fmt::Display for LocalDeactivateError<N, E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotActiveView { .. } => f.write_str("active view is not a directory symlink"),
            Self::BeforeDeactivate { cause, .. } => write!(f, "local deactivation failed: {cause}"),
            Self::AfterDeactivate { cause, .. } => write!(
                f,
                "deactivation completed but post-observation failed: {cause}"
            ),
        }
    }
}

impl<N: fmt::Debug, E: fmt::Debug> std::error::Error for LocalDeactivateError<N, E> {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::BeforeDeactivate { cause, .. } | Self::AfterDeactivate { cause, .. } => {
                Some(cause)
            }
            Self::NotActiveView { .. } => None,
        }
    }
}

impl LocalDeactivate {
    /// Inherent mirror of [`Activate::activate`] — callable without importing the trait.
    pub fn activate<N, V>(
        &self,
        applied: N,
        view: V,
    ) -> Result<<Self as Activate<N, V>>::Output, <Self as Activate<N, V>>::Error>
    where
        Self: Activate<N, V>,
    {
        Activate::activate(self, applied, view)
    }
}

impl<I, S, E> Activate<crate::local::LocalApplied<I, S, E>, PathBuf> for LocalDeactivate {
    type Error = LocalDeactivateError<Materialize<I, S, PathBuf>, E>;
    type Output = LocalDeactivated<E>;

    fn activate(
        &self,
        applied: crate::local::LocalApplied<I, S, E>,
        view: PathBuf,
    ) -> Result<Self::Output, Self::Error> {
        let observed = match observe_activation_path(&view) {
            Ok(observation) => observation,
            Err(cause) => {
                return Err(LocalDeactivateError::BeforeDeactivate {
                    applied,
                    view,
                    cause,
                });
            }
        };
        match observed {
            // Idempotent law: nothing to remove; record the truthful prior.
            LocalObservation::Missing => {
                return Ok(deactivate_receipt(
                    view,
                    applied.evidence,
                    LocalDeactivatePrior::Missing,
                ));
            }
            // Directory gate: only a directory-symlink is a view. The classification is the
            // single-home `LocalInspect` observation — no link read here.
            LocalObservation::SymlinkToDirectory => {}
            // Any other entry (file, directory, file-symlink, other) is refused without removal.
            other => {
                return Err(LocalDeactivateError::NotActiveView {
                    applied,
                    view,
                    observed: other,
                });
            }
        }

        if let Err(error) = remove_active_view(&view) {
            return Err(LocalDeactivateError::BeforeDeactivate {
                applied,
                view: view.clone(),
                cause: LocalError::io("remove active directory symlink", &view, error),
            });
        }

        match observe_activation_path(&view) {
            Ok(LocalObservation::Missing) => Ok(deactivate_receipt(
                view,
                applied.evidence,
                LocalDeactivatePrior::DirectorySymlink,
            )),
            Ok(observed) => {
                let deactivated = deactivate_receipt(
                    view,
                    applied.evidence,
                    LocalDeactivatePrior::DirectorySymlink,
                );
                Err(LocalDeactivateError::AfterDeactivate {
                    cause: LocalError::io(
                        "verify active directory symlink removal",
                        &deactivated.input,
                        io::Error::other(format!("expected missing, observed {observed:?}")),
                    ),
                    deactivated,
                })
            }
            Err(cause) => Err(LocalDeactivateError::AfterDeactivate {
                deactivated: deactivate_receipt(
                    view,
                    applied.evidence,
                    LocalDeactivatePrior::DirectorySymlink,
                ),
                cause,
            }),
        }
    }
}

fn deactivate_receipt<E>(
    view: PathBuf,
    previous: E,
    prior: LocalDeactivatePrior,
) -> LocalDeactivated<E> {
    Activated {
        input: view.clone(),
        evidence: EvidenceChain {
            previous,
            current: LocalDeactivateEvidence { view, prior },
        },
    }
}

#[cfg(unix)]
fn remove_active_view(view: &Path) -> io::Result<()> {
    fs::remove_file(view)
}

#[cfg(windows)]
fn remove_active_view(view: &Path) -> io::Result<()> {
    fs::remove_dir(view)
}

fn unique_switch_stage(parent: &Path, view: &Path, source: &Path) -> Result<PathBuf, LocalError> {
    for attempt in 0..128u32 {
        let stage = parent.join(format!(".pulith-switch-{}-{attempt}", std::process::id()));
        match create_directory_symlink(source, &stage) {
            Ok(()) => return Ok(stage),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(LocalError::io(
                    "create staged active directory symlink",
                    &stage,
                    error,
                ));
            }
        }
    }
    Err(LocalError::io(
        "create staged active directory symlink",
        view,
        io::Error::new(
            io::ErrorKind::AlreadyExists,
            "no unique staged active view name",
        ),
    ))
}

#[cfg(unix)]
fn replace_active_view(stage: &Path, view: &Path) -> io::Result<LocalSwitchBackend> {
    fs::rename(stage, view).map(|()| LocalSwitchBackend::UnixRename)
}
#[cfg(unix)]
fn remove_staged_active_view(stage: &Path) -> io::Result<()> {
    fs::remove_file(stage)
}

#[cfg(windows)]
fn remove_staged_active_view(stage: &Path) -> io::Result<()> {
    fs::remove_dir(stage)
}

#[cfg(windows)]
fn replace_active_view(stage: &Path, view: &Path) -> io::Result<LocalSwitchBackend> {
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::fs::OpenOptionsExt;
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_SHARE_DELETE,
        FILE_SHARE_READ, FILE_SHARE_WRITE, FileRenameInfoEx, SetFileInformationByHandle,
    };
    let file = File::options()
        .access_mode(0x0001_0000)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS)
        .open(stage)?;
    let name: Vec<u16> = view.as_os_str().encode_wide().collect();
    let name_bytes = name.len() * 2;
    let payload_len = 20 + name_bytes;
    // The first UTF-16 unit starts at byte 20. Keep a trailing zero unit in aligned backing
    // storage for the native path, while reporting only the counted FILE_RENAME_INFO payload.
    let mut info = vec![0u64; (payload_len + 2).div_ceil(std::mem::size_of::<u64>())];
    let info_bytes = unsafe {
        std::slice::from_raw_parts_mut(
            info.as_mut_ptr().cast::<u8>(),
            info.len() * std::mem::size_of::<u64>(),
        )
    };
    info_bytes[0..4].copy_from_slice(&3u32.to_ne_bytes());
    info_bytes[16..20].copy_from_slice(&(name_bytes as u32).to_ne_bytes());
    for (index, unit) in name.iter().enumerate() {
        info_bytes[20 + index * 2..22 + index * 2].copy_from_slice(&unit.to_ne_bytes());
    }
    if unsafe {
        SetFileInformationByHandle(
            file.as_raw_handle(),
            FileRenameInfoEx,
            info_bytes.as_ptr().cast(),
            payload_len as u32,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(LocalSwitchBackend::WindowsFileRenameInfoExPosix)
}

fn active_view_is_busy(error_code: Option<i32>) -> bool {
    #[cfg(windows)]
    {
        matches!(error_code, Some(32 | 33))
    }
    #[cfg(not(windows))]
    {
        let _ = error_code;
        false
    }
}

fn active_view_capability_unavailable(error_code: Option<i32>) -> bool {
    #[cfg(windows)]
    {
        matches!(error_code, Some(1 | 50 | 87))
    }
    #[cfg(not(windows))]
    {
        let _ = error_code;
        false
    }
}

fn switch_receipt<E>(
    view: PathBuf,
    previous: E,
    previous_source: PathBuf,
    current_source: PathBuf,
    backend: LocalSwitchBackend,
    view_observation: Option<LocalObservation>,
) -> LocalSwitched<E> {
    Activated {
        input: view,
        evidence: EvidenceChain {
            previous,
            current: LocalSwitchEvidence {
                previous_source,
                current_source,
                strategy: LocalActivationStrategy::DirectorySymlink,
                backend,
                view_observation,
            },
        },
    }
}

fn activation_receipt<E>(
    view: PathBuf,
    previous: E,
    source: PathBuf,
    view_observation: Option<LocalObservation>,
) -> LocalActivated<E> {
    Activated {
        input: view,
        evidence: EvidenceChain {
            previous,
            current: LocalActivationEvidence {
                source,
                strategy: LocalActivationStrategy::DirectorySymlink,
                view_observation,
            },
        },
    }
}

fn observe_activation_path(path: &Path) -> Result<LocalObservation, LocalError> {
    LocalInspect.observe(path)
}

#[cfg(unix)]
fn create_directory_symlink(source: &Path, view: &Path) -> io::Result<()> {
    std::os::unix::fs::symlink(source, view)
}

#[cfg(windows)]
fn create_directory_symlink(source: &Path, view: &Path) -> io::Result<()> {
    std::os::windows::fs::symlink_dir(source, view)
}

fn directory_symlink_capability_unavailable(error: &io::Error) -> bool {
    #[cfg(windows)]
    {
        error.raw_os_error() == Some(1314)
    }
    #[cfg(not(windows))]
    {
        let _ = error;
        false
    }
}
