//! Local activation, active-view switching, and independent view removal.
//!
//! Owns the exposure law: one directory symlink per activation, native replacement of exactly one
//! existing directory-symlink view for the switch, removal of exactly one directory-symlink view
//! for the deactivation, and read-only post-observation of the exposed view. It never copies the
//! tree, publishes a target, retains a prior generation, or persists active state.
//! Platform-specific link mechanics are cfg-split here. Feature-gated on `local`.
#![allow(clippy::result_large_err)] // receipt-preserving errors are the family law
#![allow(clippy::type_complexity)] // the node methods echo the family's full typestate
use std::fs;
#[cfg(windows)]
use std::fs::File;
use std::io;
use std::path::{Path, PathBuf};

use crate::{Link, Unlink};

use super::{
    LinkChange, LinkError, LinkEvidence, LocalError, LocalObservation, UnlinkChange, UnlinkError,
    UnlinkEvidence, observe_path,
};

/// D7 shape law: an expose subpath is relative, non-empty, and cannot escape the tree.
fn is_safe_expose(expose: &Path) -> bool {
    // An empty expose is the root (link_root); any other expose must be a relative,
    // non-escaping sequence of normal components.
    if expose.as_os_str().is_empty() {
        return true;
    }
    if expose.is_absolute() {
        return false;
    }
    expose
        .components()
        .all(|component| matches!(component, std::path::Component::Normal(_)))
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
fn replace_active_view(stage: &Path, view: &Path) -> io::Result<()> {
    fs::rename(stage, view)
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
fn replace_active_view(stage: &Path, view: &Path) -> io::Result<()> {
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
    Ok(())
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

#[cfg(unix)]
fn create_directory_symlink(source: &Path, view: &Path) -> io::Result<()> {
    std::os::unix::fs::symlink(source, view)
}

#[cfg(windows)]
fn create_directory_symlink(source: &Path, view: &Path) -> io::Result<()> {
    std::os::windows::fs::symlink_dir(source, view)
}

impl Link for super::LocalTarget {
    type Error = LinkError;
    type Output = LinkEvidence;

    fn link(self, view: &Path, expose: &Path) -> Result<Self::Output, Self::Error> {
        let target = self.0;
        if !is_safe_expose(expose) {
            return Err(LinkError::InvalidExpose {
                expose: expose.to_path_buf(),
            });
        }
        let source = if expose.as_os_str().is_empty() {
            target
        } else {
            target.join(expose)
        };
        link_view(view, source)
    }

    fn link_root(self, view: &Path) -> Result<Self::Output, Self::Error> {
        self.link(view, Path::new(""))
    }
}

impl Unlink for super::LocalTarget {
    type Error = UnlinkError;
    type Output = UnlinkEvidence;

    fn unlink(self) -> Result<Self::Output, Self::Error> {
        let view = self.0;
        let change = match observe_path(&view).map_err(|cause| UnlinkError::Observe {
            view: view.clone(),
            cause,
        })? {
            LocalObservation::Missing => UnlinkChange::Unchanged,
            LocalObservation::SymlinkToDirectory => {
                remove_active_view(&view).map_err(|error| UnlinkError::Remove {
                    view: view.clone(),
                    cause: LocalError::io("remove active directory symlink", &view, error),
                })?;
                UnlinkChange::Removed
            }
            observed => return Err(UnlinkError::NotActiveView { view, observed }),
        };
        Ok(UnlinkEvidence { view, change })
    }
}

fn link_view(view: &Path, source: PathBuf) -> Result<LinkEvidence, LinkError> {
    let source_observed = observe_path(&source).map_err(|cause| LinkError::BeforeLink {
        view: view.to_path_buf(),
        cause,
    })?;
    if source_observed != LocalObservation::Directory {
        return Err(LinkError::ExposeNotDirectory {
            path: source,
            observed: source_observed,
        });
    }
    if let Some(parent) = view.parent()
        && let Err(cause) = std::fs::create_dir_all(parent)
    {
        return Err(LinkError::BeforeLink {
            view: view.to_path_buf(),
            cause: LocalError::io("create view parent", parent, cause),
        });
    }
    let observed = observe_path(view).map_err(|cause| LinkError::BeforeLink {
        view: view.to_path_buf(),
        cause,
    })?;
    match observed {
        LocalObservation::Missing => {
            match create_directory_symlink(&source, view) {
                Ok(()) => {}
                Err(error) => {
                    let error_code = error.raw_os_error();
                    let cause = LocalError::io("create active directory symlink", view, error);
                    return if active_view_capability_unavailable(error_code) {
                        Err(LinkError::CapabilityUnavailable {
                            view: view.to_path_buf(),
                            cause,
                        })
                    } else {
                        Err(LinkError::BeforeLink {
                            view: view.to_path_buf(),
                            cause,
                        })
                    };
                }
            }
            Ok(LinkEvidence {
                source,
                view: view.to_path_buf(),
                change: LinkChange::Created,
            })
        }
        LocalObservation::SymlinkToDirectory => switch_view(view, &source),
        observed => Err(LinkError::ViewConflict {
            view: view.to_path_buf(),
            observed,
        }),
    }
}

/// Natively replace one existing directory-symlink view (the switch law: the previous source is
/// read first, the replacement is staged, and the resulting view is observed).
fn switch_view(view: &Path, source: &Path) -> Result<LinkEvidence, LinkError> {
    let _previous_source = fs::read_link(view).map_err(|error| LinkError::BeforeLink {
        view: view.to_path_buf(),
        cause: LocalError::io("read active directory symlink", view, error),
    })?;
    let parent = match view
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        Some(parent) => parent.to_path_buf(),
        None => {
            return Err(LinkError::BeforeLink {
                view: view.to_path_buf(),
                cause: LocalError::io(
                    "inspect active view parent",
                    Path::new(""),
                    io::Error::new(io::ErrorKind::InvalidInput, "active view has no parent"),
                ),
            });
        }
    };
    let stage =
        unique_switch_stage(&parent, view, source).map_err(|cause| LinkError::BeforeLink {
            view: view.to_path_buf(),
            cause,
        })?;
    match replace_active_view(&stage, view) {
        Ok(_backend) => {}
        Err(error) => {
            let error_code = error.raw_os_error();
            let cause = LocalError::io("replace active directory symlink", view, error);
            match remove_staged_active_view(&stage) {
                Ok(()) => {}
                Err(ref cleanup) if cleanup.kind() == io::ErrorKind::NotFound => {}
                Err(_cleanup) => {}
            }
            return if active_view_is_busy(error_code) {
                Err(LinkError::ViewConflict {
                    view: view.to_path_buf(),
                    observed: LocalObservation::SymlinkToDirectory,
                })
            } else if active_view_capability_unavailable(error_code) {
                Err(LinkError::CapabilityUnavailable {
                    view: view.to_path_buf(),
                    cause,
                })
            } else {
                Err(LinkError::BeforeLink {
                    view: view.to_path_buf(),
                    cause,
                })
            };
        }
    }
    match observe_path(view) {
        Ok(LocalObservation::SymlinkToDirectory) => Ok(LinkEvidence {
            source: source.to_path_buf(),
            view: view.to_path_buf(),
            change: LinkChange::Replaced,
        }),
        Ok(observed) => Err(LinkError::ViewConflict {
            view: view.to_path_buf(),
            observed,
        }),
        Err(cause) => Err(LinkError::BeforeLink {
            view: view.to_path_buf(),
            cause,
        }),
    }
}
