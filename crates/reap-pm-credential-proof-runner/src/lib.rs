//! Denied-by-default proof-transport prototype for the L1 credential-
//! derivation seam.
//!
//! This crate has no production constructor, command, binary, or reachable
//! transport API. Its private implementation exists only to exercise a
//! synthetic loopback contract around the consume-only
//! `L1CredentialDerivationRequestSink` boundary. A production API remains
//! denied until a reviewed transport policy and a durable attempt/source-time
//! owner are designed together.
//!
//! Even the loopback result is caller/test evidence only. It does not prove a
//! server or TLS peer, response currentness or uniqueness, credential-tuple
//! equality, provider delivery, proxy mapping or control, or mutation authority.
//! The crate contains no credential create/list/delete operation, order route,
//! generic request surface, fallback, or reporting command. Its normal surface
//! contains no L2 route; the private loopback-only attempt below has exactly
//! one fixed authenticated closed-only read and no mutation route.
//!
//! Reap-owned response buffers are zeroized. Buffers retained or copied inside
//! reqwest, hyper, rustls, the allocator, or the operating system are outside that guarantee.
//!
//! A second private seam, also limited to tests or the explicit
//! `loopback-evidence` feature, exercises a crash-durable, no-resume sequence
//! around two synthetic `/time` reads, the derivation equality join, and the
//! same-holder closed-only continuation. Its journal and burn claim are always
//! `DENIED`, expose no production constructor, and never grant a mutation or
//! network permit.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

#[cfg(any(test, feature = "loopback-evidence"))]
#[allow(
    dead_code,
    reason = "the private loopback attempt has no externally reachable constructor"
)]
mod attempt;
mod transport;

/// Normal builds deliberately provide no credential-proof transport entry
/// point. This marker grants no capability or authority.
pub const PRODUCTION_CREDENTIAL_PROOF_TRANSPORT_DENIED: &str =
    "DENIED: reviewed policy and durable attempt/source-time owner required";
