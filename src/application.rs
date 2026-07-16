use std::marker::PhantomData;
use std::path::PathBuf;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Intent<I, T, O = CreateOrReplace> {
    pub item: I,
    pub target: T,
    pub op: PhantomData<O>,
}

impl<I, T> Intent<I, T, CreateOrReplace> {
    pub fn new(item: I, target: T) -> Self {
        Self {
            item,
            target,
            op: PhantomData,
        }
    }
}

impl<I, T, O> Intent<I, T, O> {
    pub fn op<N>(self) -> Intent<I, T, N> {
        Intent {
            item: self.item,
            target: self.target,
            op: PhantomData,
        }
    }

    pub fn with_source<S>(self, source: S) -> WithSource<Self, S> {
        WithSource {
            input: self,
            source,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WithSource<I, S> {
    pub input: I,
    pub source: S,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Item {
    pub name: String,
    pub version: Option<String>,
}

impl Item {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            version: None,
        }
    }

    pub fn version(mut self, version: impl Into<String>) -> Self {
        self.version = Some(version.into());
        self
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LocalPath {
    pub path: PathBuf,
}

impl LocalPath {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LocalTarget {
    pub path: PathBuf,
}

impl LocalTarget {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Create;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Replace;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct CreateOrReplace;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Forget;
