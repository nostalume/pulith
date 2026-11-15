//! # Behavior contracts
//!
//! Resource behaviors compose through their inputs, effects, and output contracts. The
//! application chooses their order and owns its history. A consumed path value carries a
//! reference. A resource with custody carries the lifetime guaranteed by its adapter.
//!
//! Methods name the operation provided by each resource. A read returns observed facts,
//! and a publication returns the effect that actually committed.
//!
#![cfg_attr(docsrs, feature(doc_cfg))]

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
