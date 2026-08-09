//! Private PM-T2 controlled-trial runner assembly point.
//!
//! This assembly currently contains staged credential custody and one
//! cancellation-safe, bin-private authentication authority. It deliberately
//! has no CLI, network transport, mutation dispatch, or real-credential
//! execution.

#![forbid(unsafe_code)]
#![allow(
    dead_code,
    reason = "the private authority is wired by later runner session and transport slices"
)]

mod controlled_trial;
mod credentials;

fn main() {}
