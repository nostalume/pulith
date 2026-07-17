#![cfg_attr(
    not(feature = "reqwest"),
    doc = r#"
`AsyncInspect` is admitted only with the concrete `reqwest` adapter:

```compile_fail
use pulith::behavior::AsyncInspect;
```
"#
)]

//! Open behavior contracts and canonical adapter-attested state records.
//!
//! Callers establish evidence trust by selecting an adapter. Public state construction enables
//! third-party adapters to enter and continue canonical chains; it is not a provenance,
//! authorization, or authenticity boundary. Resource-specific outputs may enforce their own
//! construction invariants independently.

use std::future::Future;

/// Evidence preserved from an upstream transition and produced by the current transition.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EvidenceChain<A, B> {
    /// Evidence preserved from the preceding transition.
    pub previous: A,
    /// Evidence produced by the current transition.
    pub current: B,
}

/// Adapter-attested result of acquisition.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Acquired<I, M, E> {
    /// Request or state consumed by acquisition.
    pub input: I,
    /// Acquired material, including any custody carried by its type.
    pub material: M,
    /// Acquisition adapter's attestation.
    pub evidence: E,
}

/// Adapter-attested result of verification.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Verified<I, M, E> {
    /// Original request or upstream state identity.
    pub input: I,
    /// Material whose custody continues through verification.
    pub material: M,
    /// Preserved and current verification evidence.
    pub evidence: E,
}

/// Adapter-attested result of preparation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Prepared<I, P, E> {
    /// Original request or upstream state identity.
    pub input: I,
    /// Prepared resource-specific output.
    pub prepared: P,
    /// Preserved and current preparation evidence.
    pub evidence: E,
}

/// Adapter-attested result of application.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Applied<I, E> {
    /// Request or state whose target effect was applied.
    pub input: I,
    /// Preserved and current application evidence.
    pub evidence: E,
}

/// Adapter-attested read-only observation of an external resource.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Inspected<I, O, E> {
    /// Resource inspected without mutation.
    pub input: I,
    /// Resource-specific observed facts.
    pub observation: O,
    /// Inspection adapter's attestation.
    pub evidence: E,
}

/// Pure comparison result between caller-supplied expectation and an observation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Reconciled<I, R, E> {
    /// Inspected resource identity.
    pub input: I,
    /// Pure expected-versus-observed classification.
    pub reconciliation: R,
    /// Preserved and current reconciliation evidence.
    pub evidence: E,
}

/// Acquires resource-specific material for a caller-owned input `N`.
///
/// The associated `Output` encodes material and evidence; `Error` preserves the adapter's failure
/// law. Acquisition may perform source I/O but has no authority to publish the final target. Source
/// semantics belong to the concrete adapter, currently local paths, `ureq`, or `reqwest`.
pub trait Acquire<N> {
    type Error;
    type Output;

    fn acquire(&self, node: N) -> Result<Self::Output, Self::Error>;
}

/// Asynchronous form of [`Acquire`] with the same authority, output-evidence, and failure laws.
///
/// Only concrete async adapters implement this behavior; Pulith does not require a global runtime.
pub trait AsyncAcquire<N> {
    type Error;
    type Output;

    fn acquire_async<'a>(
        &'a self,
        node: N,
    ) -> impl Future<Output = Result<Self::Output, Self::Error>> + 'a
    where
        N: 'a;
}

/// Asynchronously observes resource-specific facts without mutation or desired-state decisions.
///
/// `Output` carries the observation and evidence, while `Error` reports failures that produced no
/// observation. The current concrete adapter is reqwest HTTP HEAD inspection.
#[cfg(feature = "reqwest")]
pub trait AsyncInspect<N> {
    type Error;
    type Output;

    fn inspect_async<'a>(
        &'a self,
        node: N,
    ) -> impl Future<Output = Result<Self::Output, Self::Error>> + 'a
    where
        N: 'a;
}

/// Establishes one factual proof about material in `N` against caller-supplied `Need`.
///
/// Verification does not authorize provenance or mutate a final target. `Output` carries verified
/// material and evidence; `Error` reports an unmet proof. Concrete digest semantics live in `hash`.
pub trait Verify<N, Need> {
    type Error;
    type Output;

    fn verify(&self, node: N, need: Need) -> Result<Self::Output, Self::Error>;
}

/// Transforms material in `N` according to caller-supplied `Need` without final publication.
///
/// `Output` carries prepared material and evidence; `Error` preserves transformation and cleanup
/// failures. The current archive adapters operate only in caller-owned destructive scratch space.
pub trait Prepare<N, Need> {
    type Error;
    type Output;

    fn prepare(&self, node: N, need: Need) -> Result<Self::Output, Self::Error>;
}

/// Applies a caller-authorized target effect represented by `N`.
///
/// `Output` carries the applied request and evidence; `Error` means the requested effect did not
/// complete. Resource-specific commit and failure laws belong to the adapter. Local apply is the
/// current concrete implementation, including direct target-only [`crate::Forget`].
pub trait Apply<N> {
    type Error;
    type Output;

    fn apply(&self, node: N) -> Result<Self::Output, Self::Error>;
}

/// Observes an external resource without mutating it or deciding desired state.
///
/// `Output` carries resource-specific facts and evidence; `Error` means no valid observation was
/// produced. Local no-follow metadata and HTTP HEAD are the current concrete semantics.
pub trait Inspect<N> {
    type Error;
    type Output;

    fn inspect(&self, node: N) -> Result<Self::Output, Self::Error>;
}

/// Compares a typed observation with caller-owned `Need` without mutation.
///
/// `Output` carries the classification and preserved evidence. `Error` describes comparison
/// failure; it grants no authority to repair, adopt, delete, or persist the observed resource.
/// Local reconciliation is the current concrete implementation.
pub trait Reconcile<N, Need> {
    type Error;
    type Output;

    fn reconcile(&self, node: N, need: Need) -> Result<Self::Output, Self::Error>;
}
