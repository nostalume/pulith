use std::path::PathBuf;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AcquireEvidence {
    pub path: PathBuf,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PrepareEvidence {
    pub path: PathBuf,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ApplyEvidence {
    pub target: PathBuf,
    pub files: usize,
    pub directories: usize,
    pub bytes: u64,
    pub strategy: LocalPlacement,
}

impl ApplyEvidence {
    pub fn new(target: PathBuf, stats: LocalApplyStats) -> Self {
        Self {
            target,
            files: stats.files,
            directories: stats.directories,
            bytes: stats.bytes,
            strategy: stats.strategy,
        }
    }

    pub fn removed(target: PathBuf) -> Self {
        Self {
            target,
            files: 0,
            directories: 0,
            bytes: 0,
            strategy: LocalPlacement::Removed,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LocalPlacement {
    Copied,
    Removed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LocalApplyStats {
    pub files: usize,
    pub directories: usize,
    pub bytes: u64,
    pub strategy: LocalPlacement,
}

impl LocalApplyStats {
    pub fn copied_file(bytes: u64) -> Self {
        Self {
            files: 1,
            directories: 0,
            bytes,
            strategy: LocalPlacement::Copied,
        }
    }

    pub fn copied_tree(files: usize, directories: usize, bytes: u64) -> Self {
        Self {
            files,
            directories,
            bytes,
            strategy: LocalPlacement::Copied,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RememberEvidence {
    pub item: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Receipt<O> {
    pub item: String,
    pub target: PathBuf,
    pub op: O,
}
