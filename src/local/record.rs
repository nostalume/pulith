use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read};
use std::path::{Path, PathBuf};

mod native;
use native::{
    Entry, classify, commit, copy_bounded, ensure_stage, inspect, lock_exclusive, reclaim,
    record_path, sync_parent,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RecordLimit(u64);

impl RecordLimit {
    pub fn new(bytes: u64) -> Result<Self, RecordError> {
        if bytes == 0 {
            Err(RecordError::InvalidLimit)
        } else {
            Ok(Self(bytes))
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RecordObservation {
    Missing,
    Present(Vec<u8>),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecordEvidence {
    pub path: PathBuf,
    pub bytes: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RecordChange {
    Created,
    Replaced,
    Removed,
    Unchanged,
}

#[derive(Debug)]
pub enum RecordError {
    InvalidStore(PathBuf),
    InvalidName(PathBuf),
    InvalidLimit,
    Busy {
        path: PathBuf,
    },
    TooLarge {
        path: PathBuf,
        limit: u64,
    },
    Conflict {
        path: PathBuf,
    },
    BeforeCommit {
        action: &'static str,
        path: PathBuf,
        source: io::Error,
    },
    Committed {
        change: RecordChange,
        path: PathBuf,
        source: io::Error,
    },
}

impl RecordError {
    fn before(action: &'static str, path: &Path, source: io::Error) -> Self {
        Self::BeforeCommit {
            action,
            path: path.into(),
            source,
        }
    }
}

impl fmt::Display for RecordError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidStore(path) => write!(f, "invalid record store: {}", path.display()),
            Self::InvalidName(path) => write!(f, "invalid record name: {}", path.display()),
            Self::InvalidLimit => f.write_str("record limit must be positive"),
            Self::Busy { path } => write!(f, "record store is busy: {}", path.display()),
            Self::TooLarge { path, limit } => {
                write!(f, "record {} exceeds {limit} bytes", path.display())
            }
            Self::Conflict { path } => write!(f, "record state conflicts at {}", path.display()),
            Self::BeforeCommit {
                action,
                path,
                source,
            } => write!(
                f,
                "failed to {action} {} before commit: {source}",
                path.display()
            ),
            Self::Committed {
                change,
                path,
                source,
            } => write!(
                f,
                "{change:?} {} but durability proof failed: {source}",
                path.display()
            ),
        }
    }
}

impl std::error::Error for RecordError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::BeforeCommit { source, .. } | Self::Committed { source, .. } => Some(source),
            _ => None,
        }
    }
}

pub struct RecordStore {
    root: PathBuf,
}

impl RecordStore {
    pub fn new(root: impl Into<PathBuf>) -> Result<Self, RecordError> {
        let root = root.into();
        let valid = root.is_absolute()
            && fs::symlink_metadata(&root)
                .map(|metadata| metadata.is_dir() && !metadata.file_type().is_symlink())
                .unwrap_or(false);
        if !valid {
            return Err(RecordError::InvalidStore(root));
        }
        Ok(Self { root })
    }

    pub fn inspect(
        &self,
        name: impl AsRef<Path>,
        limit: RecordLimit,
    ) -> Result<(RecordObservation, RecordEvidence), RecordError> {
        inspect(&self.path(name)?, limit)
    }

    pub fn edit(self) -> Result<RecordEdit, RecordError> {
        let path = self.root.join("lock");
        if matches!(classify(&path)?, Entry::Other) {
            return Err(RecordError::Conflict { path });
        }
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .map_err(|error| RecordError::before("open record-store lock", &path, error))?;
        lock_exclusive(&lock, &path)?;
        Ok(RecordEdit {
            root: self.root,
            _lock: lock,
        })
    }

    fn path(&self, name: impl AsRef<Path>) -> Result<PathBuf, RecordError> {
        record_path(&self.root, name.as_ref())
    }
}

#[derive(Debug)]
pub struct RecordEdit {
    root: PathBuf,
    _lock: File,
}

impl RecordEdit {
    pub fn inspect(
        &self,
        name: impl AsRef<Path>,
        limit: RecordLimit,
    ) -> Result<(RecordObservation, RecordEvidence), RecordError> {
        inspect(&record_path(&self.root, name.as_ref())?, limit)
    }

    pub fn create_from(
        &mut self,
        name: impl AsRef<Path>,
        limit: RecordLimit,
        source: impl Read,
    ) -> Result<RecordEvidence, RecordError> {
        self.write(name.as_ref(), limit, source, false)
    }

    pub fn replace_from(
        &mut self,
        name: impl AsRef<Path>,
        limit: RecordLimit,
        source: impl Read,
    ) -> Result<RecordEvidence, RecordError> {
        self.write(name.as_ref(), limit, source, true)
    }

    pub fn remove(&mut self, name: impl AsRef<Path>) -> Result<RecordChange, RecordError> {
        let path = record_path(&self.root, name.as_ref())?;
        match classify(&path)? {
            Entry::Missing => Ok(RecordChange::Unchanged),
            Entry::File => {
                fs::remove_file(&path)
                    .map_err(|error| RecordError::before("remove record", &path, error))?;
                sync_parent(&self.root).map_err(|source| RecordError::Committed {
                    change: RecordChange::Removed,
                    path: path.clone(),
                    source,
                })?;
                if !matches!(classify(&path)?, Entry::Missing) {
                    return Err(RecordError::Conflict { path });
                }
                Ok(RecordChange::Removed)
            }
            Entry::Other => Err(RecordError::Conflict { path }),
        }
    }

    fn write(
        &mut self,
        name: &Path,
        limit: RecordLimit,
        mut source: impl Read,
        replace: bool,
    ) -> Result<RecordEvidence, RecordError> {
        let target = record_path(&self.root, name)?;
        match (replace, classify(&target)?) {
            (false, Entry::Missing) | (true, Entry::File) => {}
            _ => return Err(RecordError::Conflict { path: target }),
        }
        let stage_dir = self.root.join("stage");
        ensure_stage(&stage_dir)?;
        let stage = stage_dir.join(name);
        reclaim(&stage)?;
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&stage)
            .map_err(|error| RecordError::before("create staged record", &stage, error))?;
        let bytes = copy_bounded(&mut source, &mut file, &stage, limit)?;
        file.sync_all()
            .map_err(|error| RecordError::before("sync staged record", &stage, error))?;
        drop(file);
        commit(&stage, &target, replace)
            .map_err(|error| RecordError::before("commit record", &target, error))?;
        let change = if replace {
            RecordChange::Replaced
        } else {
            RecordChange::Created
        };
        sync_parent(&self.root).map_err(|source| RecordError::Committed {
            change,
            path: target.clone(),
            source,
        })?;
        let evidence = inspect(&target, limit)?.1;
        if evidence.bytes != bytes {
            return Err(RecordError::Conflict { path: target });
        }
        Ok(evidence)
    }
}
