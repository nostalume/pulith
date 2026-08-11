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

#![cfg_attr(docsrs, feature(doc_cfg))]

use std::future::Future;

/// Acquires resource-specific material from a caller-supplied source. The source type is the
/// impl caller: `LocalSource::acquire()`, `RemoteSource::acquire()`. Acquisition may perform
/// source I/O but has no authority to publish a final target.
pub trait Acquire {
    /// Error returned when acquisition cannot complete.
    type Error;
    /// Material or evidence-bearing value produced by acquisition.
    type Output;

    /// Acquires material without publishing a final destination.
    fn acquire(self) -> Result<Self::Output, Self::Error>;
}

/// Asynchronously acquires resource-specific material without publishing a final destination.
pub trait AsyncAcquire {
    /// Error returned when acquisition cannot complete.
    type Error;
    /// Material or evidence-bearing value produced by acquisition.
    type Output;

    /// Acquires material asynchronously.
    fn acquire(self) -> impl Future<Output = Result<Self::Output, Self::Error>> + Send;
}

/// Asynchronously observes a resource without changing it.
pub trait AsyncInspect {
    /// Error returned when inspection cannot complete.
    type Error;
    /// Observation and its evidence.
    type Output;

    /// Inspects the resource asynchronously.
    fn inspect(self) -> impl Future<Output = Result<Self::Output, Self::Error>> + Send;
}

/// Establishes one factual proof about the consumed material against a caller-supplied
/// expectation `D` (e.g. `DigestValue`). The material type is the impl caller; verification
/// does not authorize provenance or mutate a final target.
pub trait Verify<D> {
    /// Error returned when proof cannot be established.
    type Error;
    /// Verified material and/or factual evidence.
    type Output;

    /// Verifies the consumed material against `expected`.
    fn verify(self, expected: D) -> Result<Self::Output, Self::Error>;
}

/// Removes one caller-selected resource target.
pub trait Remove {
    /// Error returned when removal cannot complete.
    type Error;
    /// Evidence describing the removal outcome.
    type Output;

    /// Removes the selected target.
    fn remove(self) -> Result<Self::Output, Self::Error>;
}

/// Links a published tree into one caller-selected consumer view.
pub trait Link {
    /// Error returned when the view cannot be linked.
    type Error;
    /// Evidence describing the link outcome.
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
    /// Error returned when the view cannot be unlinked.
    type Error;
    /// Evidence describing the unlink outcome.
    type Output;

    /// Removes the selected consumer view.
    fn unlink(self) -> Result<Self::Output, Self::Error>;
}

/// Observes a resource according to caller-selected need `N` without changing it.
pub trait Inspect<N> {
    /// Error returned when inspection cannot complete.
    type Error;
    /// Observation and its evidence.
    type Output;

    /// Inspects the consumed resource according to `need`.
    fn inspect(self, need: N) -> Result<Self::Output, Self::Error>;
}

/// Compares an observation with caller-owned expected state `E` without mutation.
pub trait Reconcile<E> {
    /// Error returned when comparison cannot complete.
    type Error;
    /// Difference classification and its evidence.
    type Output;

    /// Reconciles the consumed observation with `expected`.
    fn reconcile(self, expected: E) -> Result<Self::Output, Self::Error>;
}

#[cfg(any(feature = "zip", feature = "tar"))]
#[cfg_attr(docsrs, doc(cfg(any(feature = "zip", feature = "tar"))))]
pub mod archive;
#[cfg(feature = "hash")]
#[cfg_attr(docsrs, doc(cfg(feature = "hash")))]
pub mod hash;
#[cfg(feature = "local")]
#[cfg_attr(docsrs, doc(cfg(feature = "local")))]
pub mod local;
#[cfg(feature = "net")]
#[cfg_attr(docsrs, doc(cfg(feature = "net")))]
pub mod net;
#[cfg(feature = "process")]
#[cfg_attr(docsrs, doc(cfg(feature = "process")))]
pub mod process;
