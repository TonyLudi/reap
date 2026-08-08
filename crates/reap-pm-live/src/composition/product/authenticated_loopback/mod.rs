//! Static authenticated-loopback product composition.
//!
//! This module is compiled only for tests or the explicit non-default
//! `loopback-evidence` feature. Its root is deliberately distinct from the
//! fixture [`super::PmProductRun`]: mutation connectivity is move-only and no
//! runtime backend selector can turn a fixture run into an authenticated one.

#[allow(
    dead_code,
    reason = "non-default authenticated-loopback library builds compile the private time-worker contract; crate-local feature tests enact it"
)]
mod mutation_time;
#[allow(
    dead_code,
    reason = "non-default authenticated-loopback library builds compile the private read-ingress contract; crate-local feature tests enact it"
)]
mod read_ingress;
#[allow(
    dead_code,
    reason = "the authenticated-loopback root is deliberately crate-private and constructed only by non-default feature evidence"
)]
mod root;
#[allow(
    dead_code,
    reason = "the authenticated-loopback run is deliberately crate-private and driven only by non-default feature evidence"
)]
mod run;
#[allow(
    dead_code,
    reason = "the authenticated-loopback startup typestate is deliberately crate-private and driven only by non-default feature evidence"
)]
mod startup;
#[allow(
    dead_code,
    reason = "private task guards are part of the non-default authenticated-loopback ownership contract"
)]
mod supervision;

#[cfg(test)]
mod tests;
