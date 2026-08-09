//! Private PM-T2 controlled-trial runner assembly point.
//!
//! This first slice contains credential custody only. It deliberately has no
//! CLI, network transport, mutation dispatch, or real-credential execution.

#![forbid(unsafe_code)]
#![allow(
    dead_code,
    reason = "the unregistered custody slice is wired by the later private runner assembly"
)]

mod credentials;

fn main() {}
