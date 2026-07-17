#![cfg_attr(
    not(feature = "reqwest"),
    doc = r#"
`AsyncInspect` is admitted only with the concrete `reqwest` adapter:

```compile_fail
use pulith::behavior::AsyncInspect;
```
"#
)]

use std::future::Future;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EvidenceChain<A, B> {
    pub previous: A,
    pub current: B,
}

impl<A, B> EvidenceChain<A, B> {
    pub fn new(previous: A, current: B) -> Self {
        Self { previous, current }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Acquired<I, M, E> {
    pub(crate) input: I,
    pub(crate) material: M,
    pub(crate) evidence: E,
}

impl<I, M, E> Acquired<I, M, E> {
    #[allow(dead_code)]
    pub(crate) fn from_acquire(input: I, material: M, evidence: E) -> Self {
        Self {
            input,
            material,
            evidence,
        }
    }

    pub fn input(&self) -> &I {
        &self.input
    }

    pub fn material(&self) -> &M {
        &self.material
    }

    pub fn evidence(&self) -> &E {
        &self.evidence
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Verified<I, M, E> {
    pub(crate) input: I,
    pub(crate) material: M,
    pub(crate) evidence: E,
}

impl<I, M, E> Verified<I, M, E> {
    #[allow(dead_code)]
    pub(crate) fn from_verify(input: I, material: M, evidence: E) -> Self {
        Self {
            input,
            material,
            evidence,
        }
    }

    pub fn input(&self) -> &I {
        &self.input
    }

    pub fn material(&self) -> &M {
        &self.material
    }

    pub fn evidence(&self) -> &E {
        &self.evidence
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Prepared<I, P, E> {
    pub(crate) input: I,
    pub(crate) prepared: P,
    pub(crate) evidence: E,
}

impl<I, P, E> Prepared<I, P, E> {
    #[allow(dead_code)]
    pub(crate) fn from_prepare(input: I, prepared: P, evidence: E) -> Self {
        Self {
            input,
            prepared,
            evidence,
        }
    }

    pub fn input(&self) -> &I {
        &self.input
    }

    pub fn prepared(&self) -> &P {
        &self.prepared
    }

    pub fn evidence(&self) -> &E {
        &self.evidence
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Applied<I, E> {
    pub(crate) input: I,
    pub(crate) evidence: E,
}

impl<I, E> Applied<I, E> {
    #[allow(dead_code)]
    pub(crate) fn from_apply(input: I, evidence: E) -> Self {
        Self { input, evidence }
    }

    pub fn input(&self) -> &I {
        &self.input
    }

    pub fn evidence(&self) -> &E {
        &self.evidence
    }
}

/// Read-only observation of an external resource.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Inspected<I, O, E> {
    pub(crate) input: I,
    pub(crate) observation: O,
    pub(crate) evidence: E,
}

impl<I, O, E> Inspected<I, O, E> {
    #[allow(dead_code)]
    pub(crate) fn from_inspect(input: I, observation: O, evidence: E) -> Self {
        Self {
            input,
            observation,
            evidence,
        }
    }

    pub fn input(&self) -> &I {
        &self.input
    }

    pub fn observation(&self) -> &O {
        &self.observation
    }

    pub fn evidence(&self) -> &E {
        &self.evidence
    }
}

/// Pure comparison result between caller-supplied expectation and an observation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Reconciled<I, R, E> {
    pub(crate) input: I,
    pub(crate) reconciliation: R,
    pub(crate) evidence: E,
}

impl<I, R, E> Reconciled<I, R, E> {
    #[allow(dead_code)]
    pub(crate) fn from_reconcile(input: I, reconciliation: R, evidence: E) -> Self {
        Self {
            input,
            reconciliation,
            evidence,
        }
    }

    pub fn input(&self) -> &I {
        &self.input
    }

    pub fn reconciliation(&self) -> &R {
        &self.reconciliation
    }

    pub fn evidence(&self) -> &E {
        &self.evidence
    }
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
