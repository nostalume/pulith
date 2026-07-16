//! Composable typed behaviors for acquiring, verifying, preparing, and applying artifacts.
//!
//! Pulith models behavior as an inductive tree of semantic nodes:
//!
//! ```text
//! materialize: Intent -> WithSource -> Chosen -> Acquired -> Verified -> Prepared -> Applied -> Remembered
//! forget:      Intent<Forget> -------------------------------------------------> Applied -> Remembered
//! ```
//!
//! Each behavior explicitly declares the associated contracts it uses: policy need where required,
//! plus its evidence, error, and output. Concrete mechanisms are attached through feature-gated
//! typed nodes rather than a global context, registry, or hidden workflow policy.
//!
//! # Capability boundaries
//!
//! - `local` owns local materialization and staged publication.
//! - `blake3` and `sha2` provide typed hash verification.
//! - `zip`, `tar`, `gzip`, `xz`, and `zstd` use mature format/codec crates while Pulith owns path,
//!   resource, evidence, scratch, and composition policy.
//! - `ureq` and `reqwest` provide sync and Tokio-backed async HTTP acquisition.
//!
//! Archive preparation writes only to an exclusive disposable extraction root; final destination
//! publication remains a separate local apply behavior. Network request admission and decoded-body
//! pacing are separate shared resources. Pacing controls materialization, not socket, TLS, or HTTP
//! flow-control timing.
//!
//! # Current maturity
//!
//! The state types are composition vocabulary, not an implicit package-manager implementation.
//! Pulith currently supplies concrete local/HTTP acquisition, identity, digest, or exact
//! digest-plus-size descriptor verification, identity or archive preparation, staged local
//! publication, direct local forgetting, and in-memory remembering. A descriptor proves byte
//! identity with a supplied expectation; it does not authenticate that expectation. Source
//! discovery, trust authorization, durable lifecycle storage, dependency solving, multi-target
//! transactions, and reconciliation are not provided. Async execution is concrete only for HTTP
//! acquisition; the other transitions currently expose synchronous behavior laws only.

pub mod application;
#[cfg(any(feature = "zip", feature = "tar"))]
pub mod archive;
pub mod behavior;
pub mod error;
pub mod evidence;
#[cfg(feature = "hash")]
pub mod hash;
#[cfg(feature = "local")]
pub mod local;
#[cfg(feature = "net")]
pub mod net;

pub use application::{
    Create, CreateOrReplace, Forget, Intent, Item, LocalPath, LocalTarget, Replace, WithSource,
};
#[cfg(feature = "gzip")]
pub use archive::Gzip;
#[cfg(feature = "xz")]
pub use archive::Xz;
#[cfg(feature = "zip")]
pub use archive::Zip;
#[cfg(feature = "zstd")]
pub use archive::Zstd;
#[cfg(any(feature = "zip", feature = "tar"))]
pub use archive::{
    ArchiveEvidence, ArchiveNeed, ArchivePolicy, ArchivePrepare, ArchiveTree, ExtractWorkspace,
};
#[cfg(feature = "tar")]
pub use archive::{Plain, Tar};
pub use behavior::{
    AcquireNode, Acquired, Applied, ApplyNode, AsyncAcquireNode, Chosen, EvidenceChain, NoEvidence,
    PrepareNode, Prepared, RememberNode, Remembered, SelectNode, Verified, VerifyNode,
};
pub use error::PulithError;
pub use evidence::{
    AcquireEvidence, ApplyEvidence, LocalApplyStats, LocalPlacement, PrepareEvidence, Receipt,
    RememberEvidence,
};
#[cfg(all(feature = "hash", feature = "blake3"))]
pub use hash::Blake3;
#[cfg(all(feature = "hash", feature = "sha2"))]
pub use hash::Sha256;
#[cfg(feature = "hash")]
pub use hash::{
    ArtifactDescriptor, DescriptorEvidence, DescriptorVerify, DigestAlgorithm, DigestEvidence,
    DigestNeed, DigestValue, HashVerify, NoHashResource,
};
#[cfg(feature = "local")]
pub use local::{
    Identity, IdentityPrepare, IdentityVerify, LocalAcquire, LocalApply, LocalMaterial,
    LocalPrepared, MaterialKind, MemoryRemember, SelectFirst,
};
#[cfg(feature = "net")]
pub use net::{
    AcquireError, AcquirePolicy, AdmissionError, AdmissionMode, AdmissionPermit, AttemptEvidence,
    AttemptOutcome, AttemptRate, BytePacingMode, BytePacingPermit, ByteRate, ByteRatePacer,
    PacingError, ProtocolError, RateAdmission, RemoteSource, RemoteUrl, ResumeEvidence, ResumeMode,
    ResumeOutcome, ResumePolicy, RetryPolicy, TransportPhase, UnsafeDestination, Validator,
};
#[cfg(feature = "reqwest")]
pub use net::{AsyncAdmission, AsyncBytePacer, ReqwestAcquire, ReqwestResource};
#[cfg(feature = "ureq")]
pub use net::{SyncAdmission, SyncBytePacer, UreqAcquire, UreqResource};
