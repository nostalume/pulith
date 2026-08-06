//! A single-root crate ecosystem for composing typed external-resource management behaviors.
//!
//! Pulith models behavior as an inductive tree of semantic nodes:
//!
//! ```text
//! Materialize -> Acquired -> Applied
//!                         -> Verified -> Applied
//!                         -> Prepared -> Applied
//!                         -> Verified -> Prepared -> Applied
//! Forget -----------------------------> Applied
//! Applied<..., LocalTarget> + PathBuf -> Activated
//! Applied<..., LocalTarget> ----------> Inspected -> Reconciled
//! LocalTarget -> Inspected -> Reconciled
//! RemoteUrl  --> Inspected
//! ```
//!
//! Each behavior passes policy need as a trait type parameter where required and declares associated
//! error and output contracts. Canonical outputs are open adapter-attested records: callers select
//! the adapter and establish whether its evidence is trusted. Concrete mechanisms are
//! attached through feature-gated typed nodes rather than a global context, registry, or hidden
//! workflow policy.
//!
//! # Domain boundaries
//!
//! Pulith keeps behavior law, resource-specific semantics, and concrete adapters orthogonal. The
//! behavior kernel defines typed transitions. Filesystem, HTTP, archive, artifact-identity,
//! trust/provenance, and durable-state contexts retain their own vocabulary and invariants. An
//! adapter implements one demonstrated behavior/resource intersection; it does not define the
//! universal workflow. Callers own desired state, application identity, trust/admission policy,
//! durable aggregates, orchestration, and rollback or retention policy.
//!
//! Canonical state construction is not a security boundary. Public records let external adapters
//! enter and continue the same typed chains as built-in adapters; typed composition prevents shape
//! and order mismatches, not false attestations. Invariant-bearing resource outputs may restrict
//! construction and direct field mutation separately while retaining read-only observation. Open
//! canonical records can still replace whole resource values. Evidence remains adapter-attested
//! rather than becoming authentic through field privacy. It is not
//! automatically a domain event. State nodes are
//! composition products, not entities or aggregates. Pulith introduces no universal installation,
//! repository, transaction manager, or package-manager model.
//!
//! # Capability boundaries
//!
//! - `local` owns local materialization, staged publication, create-only directory-symlink
//!   activation, read-only no-follow inspection (including optional post-effect observation that
//!   preserves a completed receipt), and pure expected/observed reconciliation.
//! - `blake3` and `sha2` provide typed verification and, with `local`, opt-in full-read exact
//!   artifact inspection/reconciliation. The existing local inspector remains metadata-only.
//! - `zip`, `tar`, `gzip`, `xz`, and `zstd` use mature format/codec crates while Pulith owns path,
//!   resource, evidence, scratch, and composition policy.
//! - `http-sync` and `http-async` provide sync and Tokio-backed async HTTP acquisition and HEAD
//!   inspection. Their HTTP-client crates are private implementation dependencies.
//!
//! Archive preparation writes only to an exclusive disposable extraction root; final destination
//! publication remains a separate local apply behavior. Network request admission and decoded-body
//! pacing are separate shared resources. Pacing controls materialization, not socket, TLS, or HTTP
//! flow-control timing.
//!
//! HTTP acquisition returns adapter-owned staged material and never publishes `Materialize.target`
//! or creates its parent. Staged custody continues through verification, is transformed into
//! prepared custody when preparation succeeds, or continues to apply directly. Dropping the owning
//! state removes disposable custody. Caller-owned local sources and resume partials are not cleaned
//! implicitly; only apply owns final-target publication.
//!
//! For local regular files, `MaterializeMode::CreateNew` means the expected target predecessor is
//! missing. The staged file's no-clobber persist is the execution-time commit check; a target that
//! already exists is a typed conflict and is not changed. This law does not extend to directory
//! publication, replacement modes, `Forget`, or digest-based compare-and-swap.
//!
//! # Current maturity
//!
//! The state types are composition vocabulary, not an implicit package-manager implementation.
//! Pulith currently supplies concrete local/HTTP/process acquisition, digest or exact digest-plus-size
//! descriptor verification, archive preparation, staged local publication, direct local forgetting,
//! local/HTTP inspection, optional local post-apply metadata inspection, exact local artifact
//! inspection, and local reconciliation. Post-apply inspection preserves a completed local receipt
//! on unavailable observation; all inspection is read-only, and exact artifact descriptors count bytes in the digest read loop but are not atomic
//! snapshots under concurrent writes. HTTP inspection reports status and declared response
//! length without GET fallback or body materialization. Reconciliation consumes caller-owned expected state
//! and produces only a classification plus evidence. A descriptor proves byte identity with a
//! supplied expectation; it does not authenticate that expectation. Source discovery, trust
//! selection, authorization, durable lifecycle storage, dependency solving, multi-target transactions, and
//! automatic repair are not provided. Async execution is concrete only for HTTP acquisition and
//! HTTP inspection; the other transitions currently expose synchronous behavior laws only.
//! A caller may compose a prebuilt archive into an unlinked artifact tree through acquisition,
//! optional verification, archive preparation, and local application. That composition neither
//! activates a view nor creates durable installation state. For directory `CreateNew`, callers own
//! the existing target parent and target serialization; a quiescent existing target is a preflight
//! conflict, not an atomic directory-store commit guarantee.
//! A caller may separately activate one published local directory through a missing
//! `PathBuf` using [`local::LocalActivate`]. It creates one directory symlink and
//! returns an activation receipt; it does not copy, replace, switch, link a shared prefix, or persist
//! an active record. The caller owns the view parent and target serialization. Windows reports
//! directory-symlink capability unavailability explicitly and never falls back to a junction, copy,
//! or elevation.
//! [`local::LocalSwitch`] is the separate, explicitly selected replacement adapter: it requires an
//! existing directory-symlink view and switches only that name to a completed published directory.
//! Unix uses a same-parent native rename; Windows uses `FileRenameInfoEx` with POSIX replacement
//! semantics. It does not publish a target, retain a prior generation, persist active state, or
//! fall back to deletion/recreation, a junction, or a copy.//!

#![cfg_attr(
    not(feature = "http-async"),
    doc = r#"
`AsyncInspect` is admitted only with the concrete Tokio async HTTP adapter:

```compile_fail
use pulith::AsyncInspect;
```
"#
)]

//! # Behavior contracts
//!
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

/// Adapter-attested result of activation/exposure.
///
/// Activation changes how consumers resolve a resource; it is distinct from publication and carries
/// its own resource-specific receipt.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Activated<I, E> {
    /// Resource identity made visible to consumers.
    pub input: I,
    /// Preserved upstream and current activation evidence.
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
/// semantics belong to the concrete adapter, currently local paths or the built-in sync/async HTTP
/// adapters.
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

    fn acquire<'a>(
        &'a self,
        node: N,
    ) -> impl Future<Output = Result<Self::Output, Self::Error>> + 'a
    where
        N: 'a;
}

/// Asynchronously observes resource-specific facts without mutation or desired-state decisions.
///
/// `Output` carries the observation and evidence, while `Error` reports failures that produced no
/// observation. The current concrete adapter is asynchronous HTTP HEAD inspection.
#[cfg(feature = "http-async")]
pub trait AsyncInspect<N> {
    type Error;
    type Output;

    fn inspect<'a>(
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
/// current concrete implementation, including direct target-only [`crate::Forget`]. For local
/// regular files, [`crate::MaterializeMode::CreateNew`] uses an execution-time no-clobber commit;
/// an existing predecessor is a typed conflict rather than a completed application.
pub trait Apply<N> {
    type Error;
    type Output;

    fn apply(&self, node: N) -> Result<Self::Output, Self::Error>;
}

/// Activates or exposes a resource to a caller-selected consumer view.
///
/// Activation is a named effect separate from publication: `Need` identifies the caller-owned view
/// or exposure policy, and `Output` carries a distinct activation receipt. Concrete adapters define
/// their commit, conflict, capability, and recovery laws; they must not infer package ownership,
/// durable state, replacement, or rollback.
pub trait Activate<N, Need> {
    type Error;
    type Output;

    fn activate(&self, node: N, need: Need) -> Result<Self::Output, Self::Error>;
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
