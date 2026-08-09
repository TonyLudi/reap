//! Closed evidence schema for the narrow account-only read workflow.

use serde::{Deserialize, Serialize};

use crate::{
    PmReadOnlyAllowanceEvidence, PmReadOnlyCollectionFailureEvidence, PmReadOnlyConfigEvidence,
};

pub const PM_READ_ONLY_ACCOUNT_ARTIFACT_SCHEMA_VERSION: u32 = 1;
pub const PM_READ_ONLY_ACCOUNT_PRODUCT: &str = "polymarket";
pub const PM_READ_ONLY_ACCOUNT_COVERAGE: &str = "credentialed_account_read_only";
pub const PM_READ_ONLY_ACCOUNT_CONTRACT_VERSION: &str = "pm-read-only-account-v1";

/// Exact projection of the two authenticated balance/allowance GET responses.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PmReadOnlyAccountSnapshotEvidence {
    pub authenticated_response_count: u64,
    pub collateral_balance: String,
    pub conditional_balance: String,
    pub token_id: String,
    pub allowances: Vec<PmReadOnlyAllowanceEvidence>,
    pub allowance_count: u64,
    pub canonical_sha256: String,
}

/// Positive teardown and exact request-surface evidence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PmReadOnlyAccountTeardownEvidence {
    pub public_time_attempt_count: u64,
    pub authenticated_balance_attempt_count: u64,
    pub private_reconciliation_request_count: u64,
    pub user_stream_connection_count: u64,
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PmReadOnlyAccountSummary {
    pub provenance_valid: bool,
    pub config_valid: bool,
    pub configured_profile_structurally_valid: bool,
    pub authorization_closed: bool,
    pub public_time_observed: bool,
    pub balances_observed: bool,
    pub required_allowances_complete: bool,
    pub request_surface_exact: bool,
    pub teardown_complete: bool,
    pub limitations_explicit: bool,
    pub passed: bool,
}

/// Self-contained, secret-free evidence for exactly one account read attempt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PmReadOnlyAccountArtifact {
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
    pub signer_address: String,
    pub funder_address: String,
    pub signature_type: u8,
    pub production_order_entry_authorized: bool,
    pub mutation_roles_constructed: bool,
    pub mutation_requests: u64,
    pub collection_failure: Option<PmReadOnlyCollectionFailureEvidence>,
    pub account: Option<PmReadOnlyAccountSnapshotEvidence>,
    pub teardown: PmReadOnlyAccountTeardownEvidence,
    pub limitations: Vec<String>,
    pub summary: PmReadOnlyAccountSummary,
    pub evidence_fingerprint_sha256: String,
}
