/// Caller-selected publication semantics for one materialization request.
///
/// The mode authorizes only the final target effect. It does not select a source, establish
/// ownership, or imply rollback across multiple targets.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MaterializeMode {
    /// Publish only when the target predecessor is missing.
    CreateNew,
    /// Permit either creation or replacement without a predecessor condition.
    ReplaceOrCreate,
}

/// A caller-owned request to materialize one selected source at one target.
///
/// `Materialize` is input vocabulary, not evidence that acquisition, verification, preparation, or
/// application has occurred. Callers compose only the concrete behaviors required for the resource.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Materialize<I, S, T> {
    pub item: I,
    pub source: S,
    pub target: T,
    pub mode: MaterializeMode,
}

impl<I, S, T> Materialize<I, S, T> {
    pub fn new(item: I, source: S, target: T, mode: MaterializeMode) -> Self {
        Self {
            item,
            source,
            target,
            mode,
        }
    }
}

/// A caller-authorized request to remove one exact target directly.
///
/// `Forget` does not claim package ownership and deliberately has no source, acquisition,
/// verification, or preparation branch.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Forget<I, T> {
    pub item: I,
    pub target: T,
}

impl<I, T> Forget<I, T> {
    pub fn new(item: I, target: T) -> Self {
        Self { item, target }
    }
}
