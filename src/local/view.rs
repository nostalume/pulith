//! Local activation and active-view switch: `LocalActivate` and `LocalSwitch`.
//!
//! Owns the exposure law: one directory symlink per activation, native replacement of exactly one
//! existing directory-symlink view for the switch, and read-only post-observation of the exposed
//! view. It never copies the tree, publishes a target, retains a prior generation, or persists
//! active state. Platform-specific link mechanics are cfg-split here. Feature-gated on `local`.
use std::fmt;
use std::fs;
#[cfg(windows)]
use std::fs::File;
use std::io;
use std::path::{Path, PathBuf};

use crate::{Activate, Activated, Applied, EvidenceChain, Inspect, Materialize};

use super::{LocalError, LocalInspect, LocalObservation, LocalTarget};
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
impl<I, S, E> Activate<Applied<Materialize<I, S, LocalTarget>, E>, PathBuf> for LocalActivate {
    type Error = LocalActivateError<Materialize<I, S, LocalTarget>, E>;
    type Output = LocalActivated<E>;

    fn activate(
        &self,
        applied: Applied<Materialize<I, S, LocalTarget>, E>,
        view: PathBuf,
    ) -> Result<Self::Output, Self::Error> {
        let source = applied.input.target.path.clone();
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
                if observed == LocalObservation::Symlink {
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
}

impl<I, S, E> Activate<Applied<Materialize<I, S, LocalTarget>, E>, PathBuf> for LocalSwitch {
    type Error = LocalSwitchError<Materialize<I, S, LocalTarget>, E>;
    type Output = LocalSwitched<E>;

    fn activate(
        &self,
        applied: Applied<Materialize<I, S, LocalTarget>, E>,
        view: PathBuf,
    ) -> Result<Self::Output, Self::Error> {
        let source = applied.input.target.path.clone();
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
        if observed != LocalObservation::Symlink {
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
            Ok(LocalObservation::Symlink) => Ok(switch_receipt(
                view,
                applied.evidence,
                previous_source,
                source,
                backend,
                Some(LocalObservation::Symlink),
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
    LocalInspect
        .inspect(LocalTarget::new(path))
        .map(|inspected| inspected.observation)
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
