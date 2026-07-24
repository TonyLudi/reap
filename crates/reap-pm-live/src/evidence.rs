//! Fixed Phase-6 local architecture evidence.
//!
//! These entrypoints deliberately expose reports rather than product
//! authority. The benchmark runner accepts no inputs at all. The replay
//! runner accepts only the destination of its real filesystem artifact.

mod contract;
mod fixture;
mod parser;
mod report;
mod runner;
mod workload;

#[cfg(test)]
mod overload_tests;

use std::path::PathBuf;

use thiserror::Error;

/// Runs the one frozen PM action-path workload and returns one JSON report.
///
/// Callers cannot supply records, sequences, scopes, acknowledgements,
/// prepared effects, schedule keys, or a persistence backend.
pub fn run_pm_action_path_evidence() -> Result<String, PmEvidenceError> {
    let report = runner::run_action_path()?;
    serde_json::to_string(&report).map_err(PmEvidenceError::serialize)
}

/// Writes the full nominal mutation artifact through the real durable writer,
/// closes it, and recovers that same artifact twice.
///
/// The path is storage placement only; it conveys no mutation authority and
/// cannot select the sealed benchmark acknowledgement backend.
pub async fn run_pm_combined_replay_evidence(
    journal_path: PathBuf,
) -> Result<String, PmEvidenceError> {
    let report = runner::run_combined_replay(journal_path).await?;
    serde_json::to_string(&report).map_err(PmEvidenceError::serialize)
}

#[cfg(test)]
pub(crate) use fixture::{
    ReachedOverloadProfile, account_scope, allowance_row, complete_reached_overload_reconciliation,
    completion, connectivity_config, instrument, market_metadata, prepare_reached_overload_product,
    query_occurrence, reconcile_reached_overload_fills_without_watermark_advance, risk_limits,
    start_reached_overload_product, start_reached_overload_product_for,
};

#[derive(Debug, Error)]
pub enum PmEvidenceError {
    #[error("PM Phase-6 evidence invariant failed: {0}")]
    Invariant(String),
    #[error("PM Phase-6 evidence serialization failed: {0}")]
    Serialization(String),
}

impl PmEvidenceError {
    pub(crate) fn invariant(message: impl Into<String>) -> Self {
        Self::Invariant(message.into())
    }

    fn serialize(error: serde_json::Error) -> Self {
        Self::Serialization(error.to_string())
    }
}
