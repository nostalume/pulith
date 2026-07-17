use std::path::PathBuf;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LocalAcquireEvidence {
    pub path: PathBuf,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ApplyEvidence {
    pub files: usize,
    pub directories: usize,
    pub bytes: u64,
    pub strategy: LocalPlacement,
}

impl ApplyEvidence {
    pub(crate) fn new(stats: LocalApplyStats) -> Self {
        Self {
            files: stats.files,
            directories: stats.directories,
            bytes: stats.bytes,
            strategy: stats.strategy,
        }
    }

    pub(crate) fn removed() -> Self {
        Self {
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
pub(crate) struct LocalApplyStats {
    pub files: usize,
    pub directories: usize,
    pub bytes: u64,
    pub strategy: LocalPlacement,
}

impl LocalApplyStats {
    pub(crate) fn copied_file(bytes: u64) -> Self {
        Self {
            files: 1,
            directories: 0,
            bytes,
            strategy: LocalPlacement::Copied,
        }
    }

    pub(crate) fn copied_tree(files: usize, directories: usize, bytes: u64) -> Self {
        Self {
            files,
            directories,
            bytes,
            strategy: LocalPlacement::Copied,
        }
    }
}
