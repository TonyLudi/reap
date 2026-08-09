//! Strict, secret-free schema for one credentialed read-only PM smoke.
//!
//! Authenticated response bodies and WebSocket frames are deliberately absent
//! from this schema.  Private evidence is reduced to bounded, canonical rows
//! only after the live edge has checked credential-owner and market scope.

use serde::{Deserialize, Serialize};

use crate::PmReadOnlyConfigEvidence;

pub const PM_READ_ONLY_ARTIFACT_SCHEMA_VERSION: u32 = 1;

pub const PM_READ_ONLY_PRODUCT: &str = "polymarket";
pub const PM_READ_ONLY_COVERAGE: &str = "credentialed_read_only_smoke";
pub const PM_READ_ONLY_CONTRACT_VERSION: &str = "pm-read-only-smoke-v1";

/// Bounded failure classification for an evidence-producing attempt.
///
/// Both fields use verifier-owned closed vocabularies.  There is deliberately
/// no free-form message, path, response excerpt, header, or credential value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PmReadOnlyCollectionFailureEvidence {
    pub stage: String,
    pub kind: String,
    pub occurred_unix_ms: u64,
}

/// Raw public metadata plus its authoritative, non-secret projection.
///
/// Both raw fields are public endpoint responses.  No authenticated payload is
/// representable here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PmReadOnlyMetadataEvidence {
    pub market_body_base64: String,
    pub market_body_bytes: u64,
    pub market_body_sha256: String,
    pub clob_body_base64: String,
    pub clob_body_bytes: u64,
    pub clob_body_sha256: String,
    pub condition_id: String,
    pub market_id: String,
    pub token_id: String,
    pub outcome: String,
    pub tick: String,
    pub minimum_order_size: String,
    pub negative_risk: bool,
    pub active: bool,
    pub closed: bool,
    pub archived: bool,
    pub accepting_orders: bool,
    pub order_book_enabled: bool,
    pub token_count: u64,
    pub lifecycle_fingerprint_sha256: String,
    pub trading_fingerprint_sha256: String,
    pub joined_fingerprint_sha256: String,
}

/// One exact required allowance observation.  Zero is retained as evidence;
/// absence is represented separately and never rewritten to zero.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PmReadOnlyAllowanceEvidence {
    /// Exactly `collateral` or `outcome`.
    pub asset_kind: String,
    pub asset_contract: String,
    pub token_id: Option<String>,
    pub spender_address: String,
    pub amount: String,
    pub present: bool,
    pub unscoped_scalar_present: bool,
}

/// Exact account balances and the complete fixed Goal-F allowance set.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PmReadOnlyAccountEvidence {
    pub authenticated_response_count: u64,
    pub collateral_balance: String,
    pub outcome_balance: String,
    pub position_token_id: String,
    pub position_balance: String,
    pub position_available: bool,
    pub allowances: Vec<PmReadOnlyAllowanceEvidence>,
    pub allowance_count: u64,
    pub canonical_sha256: String,
}

/// Canonical redacted projection of one authenticated open-order row.
///
/// The credential owner is checked before construction and is intentionally
/// not retained (nor hashed) in the artifact.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PmReadOnlyOrderEvidence {
    pub order_id: String,
    pub condition_id: String,
    pub token_id: String,
    pub side: String,
    pub original_size: String,
    pub size_matched: String,
    pub price: String,
    pub status: String,
    pub maker_address: String,
    pub created_at: u64,
    pub expiration: u64,
    pub outcome: Option<String>,
    pub order_type: Option<String>,
}

/// Canonical redacted maker-leg projection for one authenticated trade.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PmReadOnlyTradeMakerEvidence {
    pub order_id: String,
    pub token_id: String,
    pub side: String,
    pub price: String,
    pub matched_amount: String,
    pub fee_rate_bps: Option<String>,
    pub maker_address: String,
}

/// Canonical redacted projection of one authenticated trade row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PmReadOnlyTradeEvidence {
    pub trade_id: String,
    pub condition_id: String,
    pub token_id: String,
    pub side: String,
    pub size: String,
    pub price: String,
    pub status: String,
    pub order_id: Option<String>,
    pub taker_order_id: Option<String>,
    pub trader_side: Option<String>,
    pub transaction_hash: Option<String>,
    pub fee_rate_bps: Option<String>,
    pub maker_orders: Vec<PmReadOnlyTradeMakerEvidence>,
    pub maker_address: Option<String>,
    pub timestamp: Option<u64>,
    pub match_time: Option<u64>,
    pub last_update: Option<u64>,
}

/// Complete terminal account-wide REST reconciliation with redacted owner
/// attestations.
///
/// `open_order_count` and `trade_count` are the complete account-wide totals.
/// Only exact configured-scope rows are retained in the vectors.  Therefore
/// each `*_scope_bound_count` equals its vector length and each
/// `*_scope_mismatch_count` accounts for omitted foreign-market rows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PmReadOnlyReconciliationEvidence {
    pub open_order_page_count: u64,
    pub open_order_terminal_cursor_seen: bool,
    pub open_order_count: u64,
    pub open_order_owner_bound_count: u64,
    pub open_order_scope_bound_count: u64,
    pub open_order_owner_mismatch_count: u64,
    pub open_order_scope_mismatch_count: u64,
    pub open_orders_sha256: String,
    pub open_orders: Vec<PmReadOnlyOrderEvidence>,
    pub trade_page_count: u64,
    pub trade_terminal_cursor_seen: bool,
    pub trade_count: u64,
    pub trade_owner_bound_count: u64,
    pub trade_scope_bound_count: u64,
    pub trade_owner_mismatch_count: u64,
    pub trade_scope_mismatch_count: u64,
    pub trades_sha256: String,
    pub trades: Vec<PmReadOnlyTradeEvidence>,
}

/// Authenticated user-stream lifecycle counters. Raw frames are never
/// persisted. `bound_observation_count` is re-derived as correlated
/// post-subscription PONGs plus scope-bound business events; only the latter
/// establish credential-owner evidence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PmReadOnlyUserStreamEvidence {
    pub connection_attempt_count: u64,
    pub connection_open_count: u64,
    pub subscription_count: u64,
    pub reconnect_attempt_count: u64,
    pub retirement_count: u64,
    pub retry_exhausted_count: u64,
    pub ping_count: u64,
    pub correlated_pong_count: u64,
    pub frame_count: u64,
    pub event_count: u64,
    pub order_event_count: u64,
    pub trade_event_count: u64,
    pub owner_bound_event_count: u64,
    pub scope_bound_event_count: u64,
    pub owner_mismatch_count: u64,
    pub scope_mismatch_count: u64,
    pub bound_observation_count: u64,
    pub dwell_ms: u64,
    pub shutdown_event_count: u64,
    pub run_completed_without_transport_error: bool,
    pub lifecycle_fingerprint_sha256: String,
}

/// Positive evidence that every secret-holding or socket-owning task reached
/// its explicit shutdown and join boundary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PmReadOnlyTeardownEvidence {
    pub user_stream_task_started: bool,
    pub user_stream_shutdown_requested: bool,
    pub user_stream_abort_requested: bool,
    pub user_stream_task_joined: bool,
    pub user_stream_task_completed_cleanly: bool,
    pub credential_authority_task_started: bool,
    pub credential_authority_shutdown_requested: bool,
    pub credential_authority_abort_requested: bool,
    pub credential_authority_task_joined: bool,
    pub credential_authority_task_completed_cleanly: bool,
    pub credentials_loaded: bool,
    pub credentials_dropped_before_return: bool,
    pub all_tasks_joined: bool,
    pub mutation_roles_constructed: bool,
    pub mutation_requests: u64,
}

/// Fully derived per-gate result.  A collector cannot select these values:
/// construction and offline verification derive every field again.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PmReadOnlySmokeSummary {
    pub provenance_valid: bool,
    pub config_valid: bool,
    pub authorization_closed: bool,
    pub public_metadata_valid: bool,
    pub account_balances_observed: bool,
    pub position_observed: bool,
    pub required_allowances_complete: bool,
    pub open_orders_complete: bool,
    pub trades_complete: bool,
    pub owner_evidence_non_vacuous: bool,
    pub user_stream_authenticated: bool,
    pub teardown_complete: bool,
    pub limitations_explicit: bool,
    pub passed: bool,
}

/// Self-contained, secret-free V1 evidence artifact.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PmReadOnlySmokeArtifact {
    pub schema_version: u32,
    pub product: String,
    pub coverage: String,
    pub contract_version: String,
    pub binary_name: String,
    pub binary_version: String,
    pub binary_sha256: String,
    pub binary_fingerprint_sha256: String,
    pub host_name: String,
    pub host_os: String,
    pub host_arch: String,
    pub host_fingerprint_sha256: String,
    pub config: PmReadOnlyConfigEvidence,
    pub config_fingerprint_sha256: String,
    pub credential_slot_nonsecret_fingerprint_sha256: String,
    pub started_unix_ms: u64,
    pub finished_unix_ms: u64,
    pub production_order_entry_authorized: bool,
    pub mutation_roles_constructed: bool,
    pub mutation_requests: u64,
    pub collection_failure: Option<PmReadOnlyCollectionFailureEvidence>,
    pub metadata: Option<PmReadOnlyMetadataEvidence>,
    pub account: Option<PmReadOnlyAccountEvidence>,
    pub reconciliation: Option<PmReadOnlyReconciliationEvidence>,
    pub user_stream: Option<PmReadOnlyUserStreamEvidence>,
    pub teardown: PmReadOnlyTeardownEvidence,
    pub limitations: Vec<String>,
    pub summary: PmReadOnlySmokeSummary,
    pub evidence_fingerprint_sha256: String,
}
