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
//! - `local` owns local materialization, staged publication, read-only no-follow inspection, and
//!   pure expected/observed reconciliation.
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
//! Pulith currently supplies concrete local/HTTP acquisition, digest or exact digest-plus-size
//! descriptor verification, archive preparation, staged local publication, direct local forgetting,
//! local/HTTP inspection, exact local artifact inspection, and local reconciliation. Inspection is
//! read-only; exact artifact descriptors count bytes in the digest read loop but are not atomic
//! snapshots under concurrent writes. HTTP inspection reports status and declared response
//! length without GET fallback or body materialization. Reconciliation consumes caller-owned expected state
//! and produces only a classification plus evidence. A descriptor proves byte identity with a
//! supplied expectation; it does not authenticate that expectation. Source discovery, trust
//! selection, authorization, durable lifecycle storage, dependency solving, multi-target transactions, and
//! automatic repair are not provided. Async execution is concrete only for HTTP acquisition and
//! HTTP inspection; the other transitions currently expose synchronous behavior laws only.

pub mod application;
#[cfg(any(feature = "zip", feature = "tar"))]
pub mod archive;
pub mod behavior;
pub mod error;
#[cfg(feature = "local")]
mod evidence;
#[cfg(feature = "hash")]
pub mod hash;
#[cfg(feature = "local")]
pub mod local;
#[cfg(feature = "net")]
pub mod net;

pub use application::{Forget, Materialize, MaterializeMode};
#[cfg(feature = "http-async")]
pub use behavior::AsyncInspect;
pub use behavior::{
    Acquire, Acquired, Applied, Apply, AsyncAcquire, EvidenceChain, Inspect, Inspected, Prepare,
    Prepared, Reconcile, Reconciled, Verified, Verify,
};
pub use error::PulithError;
