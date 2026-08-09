//! # Behavior contracts
//!
//! Open behavior contracts: each behavior is a trait implemented by its step's primary input
//! type (crate-defined restricted types — never std types). A behavior consumes its input and
//! produces its own output plus its own evidence; there are no state-record structs, no request
//! structs, and no chained previous evidence — history is the caller's own accumulation.
//!
//! Every trait method consumes its restricted semantic input: calling is `value.method(args)`.
//! There is no adapter instance, request record, or chained receipt. Evidence data structs and
//! law-enforcing newtypes are open records; public construction is not a provenance or
//! authenticity boundary.

use std::future::Future;

/// Acquires resource-specific material from a caller-supplied source. The source type is the
/// impl caller: `LocalSource::acquire()`, `RemoteSource::acquire()`. Acquisition may perform
/// source I/O but has no authority to publish a final target.
pub trait Acquire {
    type Error;
    type Output;

    fn acquire(self) -> Result<Self::Output, Self::Error>;
}

pub trait AsyncAcquire {
    type Error;
    type Output;

    fn acquire(self) -> impl Future<Output = Result<Self::Output, Self::Error>>;
}

#[cfg(feature = "http-async")]
pub trait AsyncInspect {
    type Error;
    type Output;

    fn inspect(self) -> impl Future<Output = Result<Self::Output, Self::Error>>;
}

/// Establishes one factual proof about the consumed material against a caller-supplied
/// expectation `D` (e.g. `DigestValue`). The material type is the impl caller; verification
/// does not authorize provenance or mutate a final target.
pub trait Verify<D> {
    type Error;
    type Output;

    fn verify(self, expected: D) -> Result<Self::Output, Self::Error>;
}

pub trait Remove {
    type Error;
    type Output;

    fn remove(self) -> Result<Self::Output, Self::Error>;
}

/// Links a published tree into one caller-selected consumer view.
pub trait Link {
    type Error;
    type Output;

    /// Link a view to the `expose` subpath of the published tree (the expose law: a directory in
    /// the tree; the view parent is created; an occupied view is switched per `policy`).
    fn link(
        self,
        view: &std::path::Path,
        expose: &std::path::Path,
    ) -> Result<Self::Output, Self::Error>;

    /// Link a view to the tree root.
    fn link_root(self, view: &std::path::Path) -> Result<Self::Output, Self::Error>;
}

/// Removes one active consumer view without touching its published source.
pub trait Unlink {
    type Error;
    type Output;

    fn unlink(self) -> Result<Self::Output, Self::Error>;
}

pub trait Inspect<N> {
    type Error;
    type Output;

    fn inspect(self, need: N) -> Result<Self::Output, Self::Error>;
}

pub trait Reconcile<E> {
    type Error;
    type Output;

    fn reconcile(self, expected: E) -> Result<Self::Output, Self::Error>;
}

#[cfg(any(feature = "zip", feature = "tar"))]
pub mod archive;
#[cfg(feature = "hash")]
pub mod hash;
#[cfg(feature = "local")]
pub mod local;
#[cfg(feature = "net")]
pub mod net;
#[cfg(feature = "process")]
pub mod process;
