use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Component, Path, PathBuf};

use crate::local::{LocalEntryKind, LocalError, LocalPath, entry_kind};

use super::{RecordError, RecordLimit};

pub(super) enum Entry {
    Missing,
    File,
    Other,
}

pub(super) fn record_path(root: &Path, name: &Path) -> Result<PathBuf, RecordError> {
    let mut parts = name.components();
    let valid = parts.next().is_some_and(|component| {
        if let Component::Normal(leaf) = component {
            let bytes = leaf.as_encoded_bytes();
            super::super::descendant_component(component)
                && !bytes.eq_ignore_ascii_case(b"lock")
                && !bytes.eq_ignore_ascii_case(b"stage")
        } else {
            false
        }
    }) && parts.next().is_none();
    if valid {
        Ok(root.join(name))
    } else {
        Err(RecordError::InvalidName(name.into()))
    }
}

pub(super) fn classify(path: &Path) -> Result<Entry, RecordError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            match entry_kind(path, &metadata).map_err(|error| record_open_error(path, error))? {
                LocalEntryKind::File => Ok(Entry::File),
                _ => Ok(Entry::Other),
            }
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Entry::Missing),
        Err(error) => Err(RecordError::before("inspect record", path, error)),
    }
}

pub(super) fn read(path: &Path, limit: RecordLimit) -> Result<Option<Vec<u8>>, RecordError> {
    let file = LocalPath::new(path).and_then(|target| target.open_file());
    let Some(mut file) = file.map_err(|error| record_open_error(path, error))? else {
        return Ok(None);
    };
    let mut bytes = Vec::new();
    Read::by_ref(&mut file)
        .take(limit.0.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|error| RecordError::before("read record", path, error))?;
    if bytes.len() as u64 > limit.0 {
        return Err(RecordError::TooLarge {
            path: path.into(),
            limit: limit.0,
        });
    }
    Ok(Some(bytes))
}

fn record_open_error(path: &Path, error: LocalError) -> RecordError {
    match error {
        LocalError::NotRegularFile { path, .. } => RecordError::Conflict { path },
        LocalError::Io {
            action,
            path,
            source,
        } => RecordError::before(action, &path, source),
        LocalError::InvalidPath(path) => RecordError::InvalidName(path),
        other => RecordError::before("open record", path, io::Error::other(other)),
    }
}

pub(super) fn ensure_stage(path: &Path) -> Result<(), RecordError> {
    match fs::symlink_metadata(path) {
        Ok(metadata)
            if entry_kind(path, &metadata).map_err(|error| record_open_error(path, error))?
                == LocalEntryKind::Directory =>
        {
            Ok(())
        }
        Ok(_) => Err(RecordError::Conflict { path: path.into() }),
        Err(error) if error.kind() == io::ErrorKind::NotFound => fs::create_dir(path)
            .map_err(|error| RecordError::before("create record stage", path, error)),
        Err(error) => Err(RecordError::before("inspect record stage", path, error)),
    }
}

pub(super) fn reclaim(path: &Path) -> Result<(), RecordError> {
    match classify(path)? {
        Entry::Missing => Ok(()),
        Entry::File => fs::remove_file(path)
            .map_err(|error| RecordError::before("reclaim staged record", path, error)),
        Entry::Other => Err(RecordError::Conflict { path: path.into() }),
    }
}

pub(super) fn copy_bounded(
    source: &mut impl Read,
    target: &mut File,
    path: &Path,
    limit: RecordLimit,
) -> Result<u64, RecordError> {
    let bytes = io::copy(&mut source.take(limit.0), target)
        .map_err(|error| RecordError::before("write staged record", path, error))?;
    let mut extra = [0];
    let exceeds = source
        .read(&mut extra)
        .map_err(|error| RecordError::before("read record source", path, error))?
        != 0;
    if exceeds {
        Err(RecordError::TooLarge {
            path: path.into(),
            limit: limit.0,
        })
    } else {
        target
            .flush()
            .map_err(|error| RecordError::before("flush staged record", path, error))?;
        Ok(bytes)
    }
}

#[cfg(unix)]
pub(super) fn lock_exclusive(file: &File, path: &Path) -> Result<(), RecordError> {
    use rustix::fs::{FlockOperation, flock};
    match flock(file, FlockOperation::NonBlockingLockExclusive) {
        Ok(()) => Ok(()),
        Err(error) if error == rustix::io::Errno::WOULDBLOCK => {
            Err(RecordError::Busy { path: path.into() })
        }
        Err(error) => Err(RecordError::before(
            "lock record store",
            path,
            io::Error::from(error),
        )),
    }
}

#[cfg(windows)]
pub(super) fn lock_exclusive(file: &File, path: &Path) -> Result<(), RecordError> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Foundation::ERROR_LOCK_VIOLATION;
    use windows_sys::Win32::Storage::FileSystem::{
        LOCKFILE_EXCLUSIVE_LOCK, LOCKFILE_FAIL_IMMEDIATELY, LockFileEx,
    };
    use windows_sys::Win32::System::IO::OVERLAPPED;
    let mut overlapped: OVERLAPPED = unsafe { std::mem::zeroed() };
    let ok = unsafe {
        LockFileEx(
            file.as_raw_handle(),
            LOCKFILE_EXCLUSIVE_LOCK | LOCKFILE_FAIL_IMMEDIATELY,
            0,
            u32::MAX,
            u32::MAX,
            &mut overlapped,
        )
    };
    if ok != 0 {
        return Ok(());
    }
    let error = io::Error::last_os_error();
    if error.raw_os_error() == Some(ERROR_LOCK_VIOLATION as i32) {
        Err(RecordError::Busy { path: path.into() })
    } else {
        Err(RecordError::before("lock record store", path, error))
    }
}

#[cfg(unix)]
pub(super) fn commit(stage: &Path, target: &Path, replace: bool) -> io::Result<()> {
    if replace {
        fs::rename(stage, target)
    } else {
        rustix::fs::renameat_with(
            rustix::fs::CWD,
            stage,
            rustix::fs::CWD,
            target,
            rustix::fs::RenameFlags::NOREPLACE,
        )
        .map_err(io::Error::from)
    }
}

#[cfg(windows)]
pub(super) fn commit(stage: &Path, target: &Path, replace: bool) -> io::Result<()> {
    use windows_sys::Win32::Storage::FileSystem::{
        MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
    };
    let source = super::super::wide_path(stage)?;
    let destination = super::super::wide_path(target)?;
    let flags = MOVEFILE_WRITE_THROUGH
        | if replace {
            MOVEFILE_REPLACE_EXISTING
        } else {
            0
        };
    if unsafe { MoveFileExW(source.as_ptr(), destination.as_ptr(), flags) } != 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

#[cfg(unix)]
pub(super) fn sync_parent(root: &Path) -> io::Result<()> {
    File::open(root)?.sync_all()
}

#[cfg(windows)]
pub(super) fn sync_parent(_: &Path) -> io::Result<()> {
    Ok(())
}
