//! Runner-private live evidence owners for PM-T2 controlled trials.
//!
//! These modules deliberately remain below the binary's sealed composition
//! boundary. They expose move-only evidence and leases to this parent only;
//! none of them can construct a production mutation request or transport.

mod current_runtime;
mod linux_egress_local_facts;
mod online_preflight;
mod private_reads;
mod public_book;
mod user_stream;
