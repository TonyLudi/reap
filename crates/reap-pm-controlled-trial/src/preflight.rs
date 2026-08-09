//! Canonical, network-free evidence contract for the closed PM-T2 preflight.
//!
//! This module validates observations already produced by separately scoped
//! readers. It owns no transport, credential, signature, request body, journal
//! writer, mutation capability, or dispatch permit.

use std::{net::IpAddr, path::Path, str::FromStr as _};

use chrono::{DateTime, SecondsFormat, TimeDelta, Utc};
use reap_pm_core::{PmPrice, PmTick, U256};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use thiserror::Error;

use crate::{
    AuthorizationHostBinding, CanonicalAuthorization, CanonicalTrialConfig,
    OfflineAuthorizationState, TrialDomain, TrialJournalBinding, TrialPhase, TrialSide,
    verify_authorization,
};

pub const TRIAL_PREFLIGHT_SCHEMA_VERSION: u32 = 1;

const MAX_CANONICAL_PREFLIGHT_BYTES: usize = 256 * 1024;
const MAX_SERVER_TIME_SKEW_MS: u64 = 5_000;
const MAX_GEOBLOCK_AGE_NS: u64 = 5_000_000_000;
const MAX_FINALIZED_BLOCK_AGE_MS: u64 = 30_000;
const MAX_FINALIZED_BLOCK_FUTURE_MS: u64 = 5_000;
const MAX_ACCOUNT_WIDE_CUT_PAGES: u16 = 1_024;
const MAX_ACCOUNT_WIDE_ROWS: u32 = 65_536;
const MAX_POSITION_PAGES: u8 = 100;
const MAX_POSITION_ROWS: u16 = 50_000;
const PREFLIGHT_FINGERPRINT_DOMAIN: &[u8] = b"reap.pm-t2.controlled-trial.preflight.v1\0";

const GEOBLOCK_ENDPOINT: &str = "https://polymarket.com/api/geoblock";
const CLOB_TIME_ENDPOINT: &str = "https://clob.polymarket.com/time";
const CLOSED_ONLY_ENDPOINT: &str = "https://clob.polymarket.com/auth/ban-status/closed-only";
const POLYGON_RPC_ORIGIN: &str = "https://polygon.drpc.org";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrialObservationStamp {
    pub observed_at_monotonic_ns: u64,
    pub response_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrialPreflightWindow {
    pub observation_started_at_utc: String,
    pub observation_completed_at_utc: String,
    pub validated_at_utc: String,
    pub maximum_observation_age_ms: u64,
    pub dispatch_deadline_at_utc: String,
    pub observation_started_monotonic_ns: u64,
    pub observation_completed_monotonic_ns: u64,
    pub validated_at_monotonic_ns: u64,
    pub dispatch_deadline_monotonic_ns: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrialAuthorizationConsumptionLeaseState {
    PreparedUnconsumed,
    Consumed,
    Ambiguous,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrialJournalLeaseEvidence {
    pub owner_process_identity: String,
    pub owner_process_count: u8,
    pub artifact_directory: String,
    pub artifact_directory_lease_fingerprint: String,
    pub artifact_directory_exclusive: bool,
    pub product_journal_path: String,
    pub product_journal_schema_version: u32,
    pub product_journal_scope_fingerprint: String,
    pub product_journal_exclusive: bool,
    pub authenticated_journal_path: String,
    pub authenticated_journal_schema_version: u32,
    pub authenticated_journal_scope_fingerprint: String,
    pub authenticated_journal_exclusive: bool,
    pub leases_held_continuously: bool,
    pub recovery_state_unambiguous: bool,
    pub authorization_consumption_state: TrialAuthorizationConsumptionLeaseState,
    pub authorization_consumption_binding_fingerprint: String,
    pub authorization_consumption_ledger_record_count: u8,
    pub authorization_consumption_claim_absent: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrialPreflightBinding {
    pub phase: TrialPhase,
    pub canonical_config_sha256: String,
    pub canonical_config_length: u64,
    pub canonical_config_fingerprint: String,
    pub trial_plan_fingerprint: String,
    pub authorization_id: String,
    pub authorization_fingerprint: String,
    pub source_pin_manifest_sha256: String,
    pub runbook_revision: String,
    pub runbook_sha256: String,
    pub repository_commit: String,
    pub cargo_lock_sha256: String,
    pub clean_tree_attested: bool,
    pub release_binary_sha256: String,
    pub release_binary_length: u64,
    pub host: AuthorizationHostBinding,
    pub credential_slot_id: String,
    pub credential_slot_nonsecret_fingerprint_sha256: String,
    pub signer_to_proxy_evidence_reference: String,
    pub journal: TrialJournalBinding,
    pub leases: TrialJournalLeaseEvidence,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrialGeoblockEvidence {
    pub endpoint: String,
    pub egress_identity: String,
    pub reported_ip: String,
    pub country: String,
    pub region: String,
    pub blocked: bool,
    pub same_egress_as_clob: bool,
    pub stamp: TrialObservationStamp,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrialServerTimeEvidence {
    pub endpoint: String,
    pub server_unix_ms: u64,
    pub previous_server_unix_ms: u64,
    pub local_wall_receive_unix_ms: u64,
    pub maximum_absolute_skew_ms: u64,
    pub product_clock_epoch_fingerprint: String,
    pub previous_product_clock_epoch_fingerprint: String,
    pub timestamp_regression_absent: bool,
    pub epoch_unchanged: bool,
    pub stamp: TrialObservationStamp,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrialEnvironmentPreflight {
    pub geoblock: TrialGeoblockEvidence,
    pub server_time: TrialServerTimeEvidence,
    pub clob_health_response_sha256: String,
    pub clob_health_green: bool,
    pub matching_engine_restart_reported: bool,
    pub restricted_mode_observed: bool,
    pub health_stamp: TrialObservationStamp,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrialMarketPreflight {
    pub condition_id: String,
    pub question_id: String,
    pub token_id: String,
    pub outcome_label: String,
    pub token_count: u8,
    pub exact_token_membership: bool,
    pub domain: TrialDomain,
    pub exchange: String,
    pub pusd_contract: String,
    pub conditional_tokens_contract: String,
    pub active: bool,
    pub closed: bool,
    pub archived: bool,
    pub accepting_orders: bool,
    pub order_book_enabled: bool,
    pub tick: String,
    pub minimum_order_size: String,
    pub maker_base_fee_bps: u64,
    pub taker_base_fee_bps: u64,
    pub fee_rate: Option<String>,
    pub fee_exponent: Option<String>,
    pub fee_taker_only: Option<bool>,
    pub fee_policy_reviewed_and_supported: bool,
    pub take_only_delay_enabled: bool,
    pub seconds_delay: u64,
    pub cancel_book_on_start: bool,
    pub minimum_order_age_seconds: u64,
    pub sports_market: bool,
    pub accepting_order_timestamp_utc: Option<String>,
    pub game_start_time_utc: Option<String>,
    pub end_time_utc: Option<String>,
    pub resolution_not_imminent: bool,
    pub long_market_response_sha256: String,
    pub clob_market_response_sha256: String,
    pub stamp: TrialObservationStamp,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrialBookPreflight {
    pub condition_id: String,
    pub token_id: String,
    pub tick: String,
    pub exact_order_price: String,
    pub best_bid: String,
    pub best_ask: String,
    pub snapshot_sequence: u64,
    pub stream_epoch_fingerprint: String,
    pub public_stream_established: bool,
    pub two_sided: bool,
    pub not_crossed: bool,
    pub tick_unchanged: bool,
    pub passive_at_dispatch: bool,
    pub stamp: TrialObservationStamp,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrialClosedOnlyEvidence {
    pub endpoint: String,
    pub signer: String,
    pub closed_only: bool,
    pub stamp: TrialObservationStamp,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrialPrivateAccountCut {
    pub signature_type: u8,
    pub signer: String,
    pub funder: String,
    pub private_key_derived_signer_matches: bool,
    pub l2_signer_matches: bool,
    pub remote_api_key_owner_matches_signer: bool,
    pub signer_to_proxy_evidence_reviewed: bool,
    pub collateral_response_sha256: String,
    pub configured_token_response_sha256: String,
    pub balance_response_count: u8,
    pub allowance_entry_count: u16,
    pub collateral_balance_sufficient_for_order: bool,
    pub collateral_allowance_sufficient_for_order: bool,
    pub configured_token_balance_sufficient_for_order: bool,
    pub configured_token_allowance_sufficient_for_order: bool,
    pub all_remote_reservations_accounted: bool,
    pub stamp: TrialObservationStamp,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrialAccountPreflight {
    pub closed_only: TrialClosedOnlyEvidence,
    pub private_cut: TrialPrivateAccountCut,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrialFinalizedChainPreflight {
    pub rpc_origin: String,
    pub chain_id: u64,
    pub proxy_funder: String,
    pub exchange_spender: String,
    pub pusd_contract: String,
    pub conditional_tokens_contract: String,
    pub finalized_block_number: u64,
    pub finalized_block_hash: String,
    pub finalized_block_unix_seconds: u64,
    pub rpc_request_count: u8,
    pub chain_id_response_sha256: String,
    pub finalized_block_response_sha256: String,
    pub allowance_response_sha256: String,
    pub operator_approval_response_sha256: String,
    pub block_reread_response_sha256: String,
    pub one_finalized_block_for_all_calls: bool,
    pub finalized_block_reread_matched: bool,
    pub finalized_block_fresh: bool,
    pub pusd_allowance_sufficient: bool,
    pub conditional_tokens_operator_approved: bool,
    pub stamp: TrialObservationStamp,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub enum TrialConfiguredPositionState {
    Absent,
    Present {
        position_row_sha256: String,
        size_is_zero: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrialDataApiPositionPreflight {
    pub proxy_funder: String,
    pub condition_id: String,
    pub token_id: String,
    pub pages_observed: u8,
    pub rows_observed: u16,
    pub configured_token_row_count: u8,
    pub page_walk_response_sha256: String,
    pub complete_bounded_page_walk: bool,
    pub configured_position: TrialConfiguredPositionState,
    pub position_consistent_with_account_cut: bool,
    pub stamp: TrialObservationStamp,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrialCompleteCutEvidence {
    pub pages_observed: u16,
    pub rows_observed: u32,
    pub terminal_cursor_observed: bool,
    pub complete: bool,
    pub response_set_sha256: String,
    pub stamp: TrialObservationStamp,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrialExactDetailCutEvidence {
    pub implicated_order_id_count: u32,
    pub exact_detail_count: u32,
    pub complete: bool,
    pub response_set_sha256: String,
    pub stamp: TrialObservationStamp,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrialUserStreamPreflight {
    pub signer: String,
    pub condition_id: String,
    pub epoch_fingerprint: String,
    pub business_event_set_sha256: String,
    pub authenticated: bool,
    pub bounded_subscription_acknowledged: bool,
    pub application_heartbeat_observed: bool,
    pub correlated_pong_observed: bool,
    pub socket_not_retired: bool,
    pub business_event_count: u32,
    pub business_event_scope_exact: bool,
    pub same_credential_rest_owner_evidence_joined: bool,
    pub epoch_unchanged: bool,
    pub reconnect_count: u16,
    pub latest_reconnect_monotonic_ns: Option<u64>,
    pub complete_cuts_refreshed_after_latest_reconnect: bool,
    pub stamp: TrialObservationStamp,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrialReconciliationPreflight {
    pub user_stream: TrialUserStreamPreflight,
    pub account_wide_open_orders: TrialCompleteCutEvidence,
    pub account_wide_trades: TrialCompleteCutEvidence,
    pub exact_details: TrialExactDetailCutEvidence,
    pub no_existing_trial_order: bool,
    pub no_unmanaged_order: bool,
    pub no_unresolved_fill: bool,
    pub no_ambiguous_state: bool,
    pub no_concurrent_proxy_trading_attested: bool,
    pub credential_visible_scope_not_funder_wide: bool,
    pub reconciliation_commitment_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrialRiskPreflight {
    pub side: TrialSide,
    pub price: String,
    pub quantity: String,
    pub maker_amount: String,
    pub taker_amount: String,
    pub maximum_loss_pusd_base_units: String,
    pub reservation_pusd_base_units: String,
    pub sell_outcome_share_payout_risk_cap_base_units: Option<String>,
    pub unsigned_order_semantic_commitment_sha256: String,
    pub risk_decision_commitment_sha256: String,
    pub reservation_commitment_sha256: String,
    pub loss_commitment_sha256: String,
    pub remote_reservation_set_sha256: String,
    pub remote_reservation_count: u32,
    pub exact_amounts_recomputed: bool,
    pub risk_within_exact_cap: bool,
    pub reservation_is_exact_and_durable: bool,
    pub all_remote_reservations_deducted: bool,
    pub available_after_reservations_sufficient: bool,
    pub one_possible_fill_within_loss_cap: bool,
    pub no_fee_or_rebate_credit_in_loss_bound: bool,
    pub one_exact_order_commitment: bool,
    pub replacement_or_reprice_disabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrialPhaseGateEvidence {
    pub phase_a_implementation_gates_green: bool,
    pub accepted_phase_a_evidence_sha256: Option<String>,
    pub separate_phase_b_authorization: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrialPreflightEvidence {
    pub schema_version: u32,
    pub binding: TrialPreflightBinding,
    pub window: TrialPreflightWindow,
    pub environment: TrialEnvironmentPreflight,
    pub market: TrialMarketPreflight,
    pub book: TrialBookPreflight,
    pub account: TrialAccountPreflight,
    pub finalized_chain: TrialFinalizedChainPreflight,
    pub data_api_position: TrialDataApiPositionPreflight,
    pub reconciliation: TrialReconciliationPreflight,
    pub risk: TrialRiskPreflight,
    pub phase_gate: TrialPhaseGateEvidence,
    pub authorization: OfflineAuthorizationState,
}

/// Move-only proof that exact canonical preflight evidence validated.
///
/// This type deliberately has no `Clone`, serialization, signing, request,
/// placement, cancellation, or dispatch API. It is structural evidence only.
pub struct CanonicalTrialPreflight {
    value: TrialPreflightEvidence,
    canonical_bytes: Box<[u8]>,
    fingerprint: String,
}

impl CanonicalTrialPreflight {
    #[must_use]
    pub const fn value(&self) -> &TrialPreflightEvidence {
        &self.value
    }

    #[must_use]
    pub fn fingerprint(&self) -> &str {
        &self.fingerprint
    }

    #[must_use]
    pub fn canonical_bytes(&self) -> &[u8] {
        &self.canonical_bytes
    }

    #[must_use]
    pub const fn production_order_entry_authorized(&self) -> bool {
        false
    }

    #[must_use]
    pub const fn real_order_submission_authorized(&self) -> bool {
        false
    }

    #[must_use]
    pub const fn place_dispatch_allowance(&self) -> u8 {
        0
    }
}

/// Validates one exact, already-collected PM-T2 preflight record.
///
/// `now` and `now_monotonic_ns` must equal the validation clock embedded in
/// the record. Supplying them independently prevents stale bytes from becoming
/// current merely by being deserialized again.
pub fn validate_canonical_trial_preflight(
    config: &CanonicalTrialConfig,
    authorization: &CanonicalAuthorization,
    canonical_bytes: &[u8],
    now: DateTime<Utc>,
    now_monotonic_ns: u64,
) -> Result<CanonicalTrialPreflight, PmTrialPreflightError> {
    if canonical_bytes.is_empty() || canonical_bytes.len() > MAX_CANONICAL_PREFLIGHT_BYTES {
        return Err(invalid("canonical preflight byte length is invalid"));
    }
    let value: TrialPreflightEvidence = serde_json::from_slice(canonical_bytes)
        .map_err(|_| invalid("preflight JSON is malformed, duplicated, unknown, or trailing"))?;
    let reencoded = serde_json::to_vec(&value)
        .map_err(|_| invalid("preflight evidence cannot be serialized canonically"))?;
    if reencoded != canonical_bytes {
        return Err(invalid(
            "preflight bytes are not exact canonical compact JSON",
        ));
    }
    value.validate(config, authorization, now, now_monotonic_ns)?;
    Ok(CanonicalTrialPreflight {
        fingerprint: hash_bytes(PREFLIGHT_FINGERPRINT_DOMAIN, canonical_bytes),
        canonical_bytes: canonical_bytes.into(),
        value,
    })
}

impl TrialPreflightEvidence {
    fn validate(
        &self,
        config: &CanonicalTrialConfig,
        authorization: &CanonicalAuthorization,
        now: DateTime<Utc>,
        now_monotonic_ns: u64,
    ) -> Result<(), PmTrialPreflightError> {
        if self.schema_version != TRIAL_PREFLIGHT_SCHEMA_VERSION {
            return Err(invalid("unsupported controlled-trial preflight schema"));
        }
        verify_authorization(config, authorization, now)
            .map_err(|_| invalid("authorization is invalid at preflight validation time"))?;
        self.validate_binding(config, authorization)?;
        let bounds = self.window.validate(
            config
                .value()
                .time_limits
                .maximum_preflight_observation_age_ms,
            now,
            now_monotonic_ns,
        )?;
        self.validate_environment(config, &bounds)?;
        self.validate_market(config, &bounds)?;
        self.validate_book(config, &bounds)?;
        self.validate_account(config, &bounds)?;
        self.validate_chain(config, &bounds)?;
        self.validate_position(config, &bounds)?;
        self.validate_reconciliation(config, authorization, &bounds)?;
        self.validate_risk(config)?;
        self.validate_phase_gate(config)?;
        if self.authorization != OfflineAuthorizationState::DENIED {
            return Err(invalid(
                "preflight evidence must not grant mutation authority",
            ));
        }
        Ok(())
    }

    fn validate_binding(
        &self,
        config: &CanonicalTrialConfig,
        authorization: &CanonicalAuthorization,
    ) -> Result<(), PmTrialPreflightError> {
        let expected = config.value();
        let approved = authorization.value();
        let binding = &self.binding;
        if binding.phase != expected.phase
            || binding.phase != approved.phase
            || binding.canonical_config_sha256 != config.canonical_sha256()
            || binding.canonical_config_length != config.canonical_length()
            || binding.canonical_config_fingerprint != config.fingerprint()
            || binding.trial_plan_fingerprint != config.plan_fingerprint()
            || binding.authorization_id != approved.authorization_id
            || binding.authorization_fingerprint != authorization.fingerprint()
            || binding.source_pin_manifest_sha256 != expected.source_pin_manifest_sha256
            || binding.runbook_revision != expected.runbook_revision
            || binding.runbook_sha256 != expected.runbook_sha256
            || binding.repository_commit != approved.build.repository_commit
            || binding.cargo_lock_sha256 != approved.build.cargo_lock_sha256
            || !binding.clean_tree_attested
            || !approved.build.clean_tree_attested
            || binding.release_binary_sha256 != approved.build.release_binary_sha256
            || binding.release_binary_length != approved.build.release_binary_length
            || binding.host != approved.host
            || binding.credential_slot_id != expected.credential_slot.slot_id
            || binding.credential_slot_nonsecret_fingerprint_sha256
                != expected.credential_slot.nonsecret_fingerprint_sha256
            || binding.signer_to_proxy_evidence_reference
                != expected.credential_slot.signer_to_proxy_evidence_reference
            || binding.journal != expected.journal
        {
            return Err(invalid(
                "preflight does not bind the exact config, plan, authorization, source, build, host, credential slot, and journal",
            ));
        }
        validate_sha256(&binding.canonical_config_sha256)?;
        validate_sha256(&binding.canonical_config_fingerprint)?;
        validate_sha256(&binding.trial_plan_fingerprint)?;
        validate_sha256(&binding.authorization_fingerprint)?;
        validate_sha256(&binding.source_pin_manifest_sha256)?;
        validate_sha256(&binding.runbook_sha256)?;
        validate_sha256(&binding.cargo_lock_sha256)?;
        validate_sha256(&binding.release_binary_sha256)?;
        binding.leases.validate(&expected.journal)
    }

    fn validate_environment(
        &self,
        config: &CanonicalTrialConfig,
        bounds: &ObservationBounds,
    ) -> Result<(), PmTrialPreflightError> {
        let environment = &self.environment;
        bounds.validate_stamp(&environment.health_stamp)?;
        validate_sha256(&environment.clob_health_response_sha256)?;
        if environment.health_stamp.response_sha256 != environment.clob_health_response_sha256
            || !environment.clob_health_green
            || environment.matching_engine_restart_reported
            || environment.restricted_mode_observed
        {
            return Err(invalid("CLOB health or matching-engine mode is not green"));
        }
        environment
            .geoblock
            .validate(&config.value().account, &self.binding.host, bounds)?;
        environment.server_time.validate(bounds)
    }

    fn validate_market(
        &self,
        config: &CanonicalTrialConfig,
        bounds: &ObservationBounds,
    ) -> Result<(), PmTrialPreflightError> {
        let expected = config.value();
        let market = &self.market;
        bounds.validate_stamp(&market.stamp)?;
        for hash in [
            &market.long_market_response_sha256,
            &market.clob_market_response_sha256,
        ] {
            validate_sha256(hash)?;
        }
        if market.condition_id != expected.market.condition_id
            || market.question_id != expected.market.question_id
            || market.token_id != expected.market.token_id
            || market.outcome_label != expected.market.outcome_label
            || market.domain != expected.market.domain
            || market.exchange != expected.market.exchange
            || market.pusd_contract != expected.market.pusd_contract
            || market.conditional_tokens_contract != expected.market.conditional_tokens_contract
            || market.token_count != 2
            || !market.exact_token_membership
            || !market.active
            || market.closed
            || market.archived
            || !market.accepting_orders
            || !market.order_book_enabled
            || market.tick != expected.order.tick
            || market.minimum_order_size != expected.order.minimum_order_size
        {
            return Err(invalid(
                "market identity, lifecycle, domain, or numerics drifted",
            ));
        }
        canonical_price(&market.tick)?;
        canonical_quantity(&market.minimum_order_size)?;
        validate_fee_decimal(market.fee_rate.as_deref())?;
        validate_fee_decimal(market.fee_exponent.as_deref())?;
        if market.maker_base_fee_bps != 0
            || market.fee_rate.is_none()
            || market.fee_exponent.is_none()
            || market.fee_taker_only.is_none()
            || !market.fee_policy_reviewed_and_supported
            || market.take_only_delay_enabled
            || market.seconds_delay != 0
            || market.cancel_book_on_start
            || market.minimum_order_age_seconds != 0
            || market.sports_market
            || !market.resolution_not_imminent
        {
            return Err(invalid(
                "market fee, delay, order-age, or lifecycle policy is unsupported",
            ));
        }
        for timestamp in [
            market.accepting_order_timestamp_utc.as_deref(),
            market.game_start_time_utc.as_deref(),
            market.end_time_utc.as_deref(),
        ]
        .into_iter()
        .flatten()
        {
            parse_canonical_utc(timestamp)?;
        }
        Ok(())
    }

    fn validate_book(
        &self,
        config: &CanonicalTrialConfig,
        bounds: &ObservationBounds,
    ) -> Result<(), PmTrialPreflightError> {
        let expected = config.value();
        let book = &self.book;
        bounds.validate_stamp(&book.stamp)?;
        if book.condition_id != expected.market.condition_id
            || book.token_id != expected.market.token_id
            || book.tick != expected.order.tick
            || book.exact_order_price != expected.order.price
            || book.snapshot_sequence == 0
            || !book.public_stream_established
            || !book.two_sided
            || !book.not_crossed
            || !book.tick_unchanged
            || !book.passive_at_dispatch
        {
            return Err(invalid("book profile or exact scope is not green"));
        }
        validate_sha256(&book.stream_epoch_fingerprint)?;
        let tick =
            PmTick::parse_decimal(&book.tick).map_err(|_| invalid("book tick is invalid"))?;
        let price = canonical_price(&book.exact_order_price)?;
        let bid = canonical_price(&book.best_bid)?;
        let ask = canonical_price(&book.best_ask)?;
        price
            .validate_tick(tick)
            .map_err(|_| invalid("exact order price is no longer tick aligned"))?;
        if bid >= ask {
            return Err(invalid("book is crossed or locked"));
        }
        let passive = match expected.order.side {
            TrialSide::Buy => price < ask,
            TrialSide::Sell => price > bid,
        };
        if !passive {
            return Err(invalid("exact order price is not passive at dispatch"));
        }
        Ok(())
    }

    fn validate_account(
        &self,
        config: &CanonicalTrialConfig,
        bounds: &ObservationBounds,
    ) -> Result<(), PmTrialPreflightError> {
        let expected = config.value();
        self.account
            .closed_only
            .validate(&expected.account.signer, bounds)?;
        let cut = &self.account.private_cut;
        bounds.validate_stamp(&cut.stamp)?;
        for hash in [
            &cut.collateral_response_sha256,
            &cut.configured_token_response_sha256,
        ] {
            validate_sha256(hash)?;
        }
        if cut.signature_type != 1
            || cut.signer != expected.account.signer
            || cut.funder != expected.account.funder
            || !cut.private_key_derived_signer_matches
            || !cut.l2_signer_matches
            || !cut.remote_api_key_owner_matches_signer
            || !cut.signer_to_proxy_evidence_reviewed
            || cut.balance_response_count != 2
            || cut.allowance_entry_count == 0
            || !cut.all_remote_reservations_accounted
        {
            return Err(invalid(
                "private account profile or observation is incomplete",
            ));
        }
        let side_green = match expected.order.side {
            TrialSide::Buy => {
                cut.collateral_balance_sufficient_for_order
                    && cut.collateral_allowance_sufficient_for_order
            }
            TrialSide::Sell => {
                cut.configured_token_balance_sufficient_for_order
                    && cut.configured_token_allowance_sufficient_for_order
            }
        };
        if !side_green {
            return Err(invalid(
                "side-appropriate balance or allowance is insufficient",
            ));
        }
        Ok(())
    }

    fn validate_chain(
        &self,
        config: &CanonicalTrialConfig,
        bounds: &ObservationBounds,
    ) -> Result<(), PmTrialPreflightError> {
        let expected = config.value();
        let chain = &self.finalized_chain;
        bounds.validate_stamp(&chain.stamp)?;
        for hash in [
            &chain.finalized_block_hash,
            &chain.chain_id_response_sha256,
            &chain.finalized_block_response_sha256,
            &chain.allowance_response_sha256,
            &chain.operator_approval_response_sha256,
            &chain.block_reread_response_sha256,
        ] {
            validate_sha256(hash)?;
        }
        let block_unix_ms = chain
            .finalized_block_unix_seconds
            .checked_mul(1_000)
            .ok_or_else(|| invalid("finalized block timestamp overflows"))?;
        let block_not_too_old = bounds
            .validated_unix_ms
            .checked_sub(block_unix_ms)
            .is_some_and(|age| age <= MAX_FINALIZED_BLOCK_AGE_MS);
        let block_not_too_far_future = block_unix_ms
            .checked_sub(bounds.validated_unix_ms)
            .is_none_or(|lead| lead <= MAX_FINALIZED_BLOCK_FUTURE_MS);
        if chain.rpc_origin != POLYGON_RPC_ORIGIN
            || chain.chain_id != 137
            || chain.chain_id != expected.account.chain_id
            || chain.proxy_funder != expected.account.funder
            || chain.exchange_spender != expected.market.exchange
            || chain.pusd_contract != expected.market.pusd_contract
            || chain.conditional_tokens_contract != expected.market.conditional_tokens_contract
            || chain.finalized_block_number == 0
            || chain.finalized_block_unix_seconds == 0
            || chain.rpc_request_count != 5
            || !chain.one_finalized_block_for_all_calls
            || !chain.finalized_block_reread_matched
            || !chain.finalized_block_fresh
            || !block_not_too_old
            || !block_not_too_far_future
            || !chain.pusd_allowance_sufficient
            || !chain.conditional_tokens_operator_approved
        {
            return Err(invalid("finalized Polygon authorization cut is not green"));
        }
        Ok(())
    }

    fn validate_position(
        &self,
        config: &CanonicalTrialConfig,
        bounds: &ObservationBounds,
    ) -> Result<(), PmTrialPreflightError> {
        let expected = config.value();
        let position = &self.data_api_position;
        bounds.validate_stamp(&position.stamp)?;
        validate_sha256(&position.page_walk_response_sha256)?;
        if position.proxy_funder != expected.account.funder
            || position.condition_id != expected.market.condition_id
            || position.token_id != expected.market.token_id
            || position.pages_observed == 0
            || position.pages_observed > MAX_POSITION_PAGES
            || position.rows_observed > MAX_POSITION_ROWS
            || u16::from(position.configured_token_row_count) > position.rows_observed
            || !position.complete_bounded_page_walk
            || !position.position_consistent_with_account_cut
        {
            return Err(invalid(
                "Data API position cut is incomplete, stale, or out of scope",
            ));
        }
        match &position.configured_position {
            TrialConfiguredPositionState::Absent => {
                if position.configured_token_row_count != 0 {
                    return Err(invalid("absent configured position has a row count"));
                }
                if expected.order.side == TrialSide::Sell {
                    return Err(invalid(
                        "SELL requires a corroborating configured position row",
                    ));
                }
            }
            TrialConfiguredPositionState::Present {
                position_row_sha256,
                size_is_zero,
            } => {
                validate_sha256(position_row_sha256)?;
                if position.configured_token_row_count != 1
                    || (expected.order.side == TrialSide::Sell && *size_is_zero)
                {
                    return Err(invalid(
                        "configured position row is ambiguous or insufficient",
                    ));
                }
            }
        }
        Ok(())
    }

    fn validate_reconciliation(
        &self,
        config: &CanonicalTrialConfig,
        authorization: &CanonicalAuthorization,
        bounds: &ObservationBounds,
    ) -> Result<(), PmTrialPreflightError> {
        let reconciliation = &self.reconciliation;
        reconciliation
            .account_wide_open_orders
            .validate(bounds, "open-order")?;
        reconciliation
            .account_wide_trades
            .validate(bounds, "trade")?;
        reconciliation.exact_details.validate(bounds)?;
        reconciliation.user_stream.validate(
            config,
            bounds,
            &[
                self.account.closed_only.stamp.observed_at_monotonic_ns,
                self.account.private_cut.stamp.observed_at_monotonic_ns,
                self.data_api_position.stamp.observed_at_monotonic_ns,
                reconciliation
                    .account_wide_open_orders
                    .stamp
                    .observed_at_monotonic_ns,
                reconciliation
                    .account_wide_trades
                    .stamp
                    .observed_at_monotonic_ns,
                reconciliation.exact_details.stamp.observed_at_monotonic_ns,
            ],
        )?;
        validate_sha256(&reconciliation.reconciliation_commitment_sha256)?;
        let quiet_stream_join_is_nonvacuous = reconciliation
            .user_stream
            .same_credential_rest_owner_evidence_joined
            && self.account.closed_only.signer == config.value().account.signer
            && !self.account.closed_only.closed_only
            && self.account.private_cut.signature_type == 1
            && self.account.private_cut.signer == config.value().account.signer
            && self.account.private_cut.balance_response_count == 2
            && reconciliation.account_wide_open_orders.complete
            && reconciliation.account_wide_open_orders.rows_observed == 0
            && reconciliation.account_wide_trades.complete
            && reconciliation.account_wide_trades.rows_observed == 0
            && reconciliation.exact_details.complete
            && reconciliation.exact_details.implicated_order_id_count == 0
            && reconciliation.exact_details.exact_detail_count == 0
            && reconciliation.no_concurrent_proxy_trading_attested;
        let observed_rows = reconciliation
            .account_wide_open_orders
            .rows_observed
            .checked_add(reconciliation.account_wide_trades.rows_observed)
            .ok_or_else(|| invalid("reconciliation row count overflows"))?;
        if reconciliation.account_wide_open_orders.rows_observed != 0
            || reconciliation.exact_details.implicated_order_id_count
                != reconciliation.exact_details.exact_detail_count
            || reconciliation.exact_details.implicated_order_id_count > observed_rows
            || !reconciliation.no_existing_trial_order
            || !reconciliation.no_unmanaged_order
            || !reconciliation.no_unresolved_fill
            || !reconciliation.no_ambiguous_state
            || !reconciliation.no_concurrent_proxy_trading_attested
            || !authorization
                .value()
                .approval
                .no_concurrent_proxy_trading_attested
            || !reconciliation.credential_visible_scope_not_funder_wide
            || (reconciliation.user_stream.business_event_count == 0
                && !quiet_stream_join_is_nonvacuous)
        {
            return Err(invalid("private reconciliation is incomplete or ambiguous"));
        }
        Ok(())
    }

    fn validate_risk(&self, config: &CanonicalTrialConfig) -> Result<(), PmTrialPreflightError> {
        let expected = &config.value().order;
        let risk = &self.risk;
        if risk.side != expected.side
            || risk.price != expected.price
            || risk.quantity != expected.quantity
            || risk.maker_amount != expected.maker_amount
            || risk.taker_amount != expected.taker_amount
            || risk.maximum_loss_pusd_base_units != expected.maximum_loss_pusd_base_units
            || risk.reservation_pusd_base_units != expected.reservation_pusd_base_units
            || risk.sell_outcome_share_payout_risk_cap_base_units
                != expected.sell_outcome_share_payout_risk_cap_base_units
            || risk.remote_reservation_count
                != self.reconciliation.account_wide_open_orders.rows_observed
            || !risk.exact_amounts_recomputed
            || !risk.risk_within_exact_cap
            || !risk.reservation_is_exact_and_durable
            || !risk.all_remote_reservations_deducted
            || !risk.available_after_reservations_sufficient
            || !risk.one_possible_fill_within_loss_cap
            || !risk.no_fee_or_rebate_credit_in_loss_bound
            || !risk.one_exact_order_commitment
            || !risk.replacement_or_reprice_disabled
        {
            return Err(invalid(
                "risk, reservation, loss, or order commitment is not exact",
            ));
        }
        canonical_u256(&risk.maker_amount, false)?;
        canonical_u256(&risk.taker_amount, false)?;
        canonical_u256(&risk.maximum_loss_pusd_base_units, false)?;
        canonical_u256(&risk.reservation_pusd_base_units, true)?;
        if let Some(value) = &risk.sell_outcome_share_payout_risk_cap_base_units {
            canonical_u256(value, false)?;
        }
        for hash in [
            &risk.unsigned_order_semantic_commitment_sha256,
            &risk.risk_decision_commitment_sha256,
            &risk.reservation_commitment_sha256,
            &risk.loss_commitment_sha256,
            &risk.remote_reservation_set_sha256,
        ] {
            validate_sha256(hash)?;
        }
        Ok(())
    }

    fn validate_phase_gate(
        &self,
        config: &CanonicalTrialConfig,
    ) -> Result<(), PmTrialPreflightError> {
        if !self.phase_gate.phase_a_implementation_gates_green {
            return Err(invalid("Phase A implementation gates are not green"));
        }
        match config.value().phase {
            TrialPhase::APlaceCancel => {
                if self.phase_gate.accepted_phase_a_evidence_sha256.is_some()
                    || self.phase_gate.separate_phase_b_authorization
                {
                    return Err(invalid("Phase A evidence carries Phase B prerequisites"));
                }
            }
            TrialPhase::BFillPosition => {
                let accepted = self
                    .phase_gate
                    .accepted_phase_a_evidence_sha256
                    .as_deref()
                    .ok_or_else(|| invalid("Phase B lacks accepted Phase A evidence"))?;
                validate_sha256(accepted)?;
                if !self.phase_gate.separate_phase_b_authorization {
                    return Err(invalid("Phase B lacks a separate authorization"));
                }
            }
        }
        Ok(())
    }
}

impl TrialPreflightWindow {
    fn validate(
        &self,
        expected_maximum_age_ms: u64,
        now: DateTime<Utc>,
        now_monotonic_ns: u64,
    ) -> Result<ObservationBounds, PmTrialPreflightError> {
        let started = parse_canonical_utc(&self.observation_started_at_utc)?;
        let completed = parse_canonical_utc(&self.observation_completed_at_utc)?;
        let validated = parse_canonical_utc(&self.validated_at_utc)?;
        let deadline = parse_canonical_utc(&self.dispatch_deadline_at_utc)?;
        let delta_ms = i64::try_from(self.maximum_observation_age_ms)
            .map_err(|_| invalid("preflight maximum age overflows the clock"))?;
        let expected_deadline = started
            .checked_add_signed(TimeDelta::milliseconds(delta_ms))
            .ok_or_else(|| invalid("preflight wall-clock deadline overflows"))?;
        let maximum_age_ns = self
            .maximum_observation_age_ms
            .checked_mul(1_000_000)
            .ok_or_else(|| invalid("preflight monotonic deadline overflows"))?;
        let expected_monotonic_deadline = self
            .observation_started_monotonic_ns
            .checked_add(maximum_age_ns)
            .ok_or_else(|| invalid("preflight monotonic deadline overflows"))?;
        if self.maximum_observation_age_ms == 0
            || self.maximum_observation_age_ms != expected_maximum_age_ms
            || deadline != expected_deadline
            || self.dispatch_deadline_monotonic_ns != expected_monotonic_deadline
            || started > completed
            || completed > validated
            || validated > deadline
            || self.observation_started_monotonic_ns > self.observation_completed_monotonic_ns
            || self.observation_completed_monotonic_ns > self.validated_at_monotonic_ns
            || self.validated_at_monotonic_ns > self.dispatch_deadline_monotonic_ns
            || validated != now
            || self.validated_at_monotonic_ns != now_monotonic_ns
        {
            return Err(invalid(
                "preflight observation window is stale or inconsistent",
            ));
        }
        Ok(ObservationBounds {
            started_monotonic_ns: self.observation_started_monotonic_ns,
            completed_monotonic_ns: self.observation_completed_monotonic_ns,
            validated_monotonic_ns: self.validated_at_monotonic_ns,
            started_unix_ms: positive_unix_millis(started)?,
            completed_unix_ms: positive_unix_millis(completed)?,
            validated_unix_ms: positive_unix_millis(validated)?,
        })
    }
}

impl TrialJournalLeaseEvidence {
    fn validate(&self, journal: &TrialJournalBinding) -> Result<(), PmTrialPreflightError> {
        for hash in [
            &self.artifact_directory_lease_fingerprint,
            &self.product_journal_scope_fingerprint,
            &self.authenticated_journal_scope_fingerprint,
            &self.authorization_consumption_binding_fingerprint,
        ] {
            validate_sha256(hash)?;
        }
        let product_path =
            validate_journal_path(&self.product_journal_path, &journal.artifact_directory)?;
        let authenticated_path = validate_journal_path(
            &self.authenticated_journal_path,
            &journal.artifact_directory,
        )?;
        let product_name = product_path.file_name().and_then(|name| name.to_str());
        let authenticated_name = authenticated_path
            .file_name()
            .and_then(|name| name.to_str());
        if self.owner_process_identity.is_empty()
            || self.owner_process_identity.len() > 256
            || self.owner_process_count != 1
            || self.artifact_directory != journal.artifact_directory
            || !self.artifact_directory_exclusive
            || product_path == authenticated_path
            || product_name == Some(&journal.authorization_consumption_ledger_file)
            || product_name == Some(&journal.authorization_consumption_claim_file)
            || authenticated_name == Some(&journal.authorization_consumption_ledger_file)
            || authenticated_name == Some(&journal.authorization_consumption_claim_file)
            || self.product_journal_schema_version == 0
            || self.authenticated_journal_schema_version == 0
            || !self.product_journal_exclusive
            || !self.authenticated_journal_exclusive
            || !self.leases_held_continuously
            || !self.recovery_state_unambiguous
            || self.authorization_consumption_state
                != TrialAuthorizationConsumptionLeaseState::PreparedUnconsumed
            || self.authorization_consumption_ledger_record_count != 1
            || !self.authorization_consumption_claim_absent
        {
            return Err(invalid(
                "exclusive journal and artifact leases are not exact",
            ));
        }
        Ok(())
    }
}

impl TrialGeoblockEvidence {
    fn validate(
        &self,
        account: &crate::TrialAccount,
        host: &AuthorizationHostBinding,
        bounds: &ObservationBounds,
    ) -> Result<(), PmTrialPreflightError> {
        bounds.validate_stamp(&self.stamp)?;
        let reported_ip = self
            .reported_ip
            .parse::<IpAddr>()
            .map_err(|_| invalid("geoblock IP is invalid"))?;
        let age = bounds
            .validated_monotonic_ns
            .checked_sub(self.stamp.observed_at_monotonic_ns)
            .ok_or_else(|| invalid("geoblock observation is from the future"))?;
        if self.endpoint != GEOBLOCK_ENDPOINT
            || self.egress_identity != host.egress_identity
            || self.reported_ip != host.egress_identity
            || reported_ip.to_string() != self.reported_ip
            || self.country.len() != 2
            || !self.country.bytes().all(|byte| byte.is_ascii_uppercase())
            || self.region.len() > 16
            || !self
                .region
                .bytes()
                .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'-')
            || self.blocked
            || !self.same_egress_as_clob
            || age > MAX_GEOBLOCK_AGE_NS
            || account.chain_id != 137
        {
            return Err(invalid("geoblock or same-egress gate is not green"));
        }
        Ok(())
    }
}

impl TrialServerTimeEvidence {
    fn validate(&self, bounds: &ObservationBounds) -> Result<(), PmTrialPreflightError> {
        bounds.validate_stamp(&self.stamp)?;
        validate_sha256(&self.product_clock_epoch_fingerprint)?;
        validate_sha256(&self.previous_product_clock_epoch_fingerprint)?;
        let skew = self
            .server_unix_ms
            .abs_diff(self.local_wall_receive_unix_ms);
        if self.endpoint != CLOB_TIME_ENDPOINT
            || self.server_unix_ms == 0
            || self.previous_server_unix_ms == 0
            || self.local_wall_receive_unix_ms < bounds.started_unix_ms
            || self.local_wall_receive_unix_ms > bounds.completed_unix_ms
            || self.maximum_absolute_skew_ms != MAX_SERVER_TIME_SKEW_MS
            || skew > self.maximum_absolute_skew_ms
            || self.server_unix_ms < self.previous_server_unix_ms
            || self.product_clock_epoch_fingerprint != self.previous_product_clock_epoch_fingerprint
            || !self.timestamp_regression_absent
            || !self.epoch_unchanged
        {
            return Err(invalid(
                "server time, skew, regression, or epoch gate is not green",
            ));
        }
        Ok(())
    }
}

impl TrialClosedOnlyEvidence {
    fn validate(
        &self,
        expected_signer: &str,
        bounds: &ObservationBounds,
    ) -> Result<(), PmTrialPreflightError> {
        bounds.validate_stamp(&self.stamp)?;
        if self.endpoint != CLOSED_ONLY_ENDPOINT
            || self.signer != expected_signer
            || self.closed_only
        {
            return Err(invalid(
                "closed-only gate is not green for the exact signer",
            ));
        }
        Ok(())
    }
}

impl TrialCompleteCutEvidence {
    fn validate(
        &self,
        bounds: &ObservationBounds,
        label: &'static str,
    ) -> Result<(), PmTrialPreflightError> {
        bounds.validate_stamp(&self.stamp)?;
        validate_sha256(&self.response_set_sha256)?;
        if self.pages_observed == 0
            || self.pages_observed > MAX_ACCOUNT_WIDE_CUT_PAGES
            || self.rows_observed > MAX_ACCOUNT_WIDE_ROWS
            || !self.terminal_cursor_observed
            || !self.complete
        {
            return Err(match label {
                "open-order" => invalid("account-wide open-order cut is incomplete"),
                _ => invalid("account-wide trade cut is incomplete"),
            });
        }
        Ok(())
    }
}

impl TrialExactDetailCutEvidence {
    fn validate(&self, bounds: &ObservationBounds) -> Result<(), PmTrialPreflightError> {
        bounds.validate_stamp(&self.stamp)?;
        validate_sha256(&self.response_set_sha256)?;
        if self.implicated_order_id_count > MAX_ACCOUNT_WIDE_ROWS
            || self.exact_detail_count > MAX_ACCOUNT_WIDE_ROWS
            || !self.complete
        {
            return Err(invalid("exact-order detail cut is incomplete"));
        }
        Ok(())
    }
}

impl TrialUserStreamPreflight {
    fn validate(
        &self,
        config: &CanonicalTrialConfig,
        bounds: &ObservationBounds,
        refreshed_cut_times: &[u64],
    ) -> Result<(), PmTrialPreflightError> {
        bounds.validate_stamp(&self.stamp)?;
        validate_sha256(&self.epoch_fingerprint)?;
        validate_sha256(&self.business_event_set_sha256)?;
        if self.signer != config.value().account.signer
            || self.condition_id != config.value().market.condition_id
            || !self.authenticated
            || !self.bounded_subscription_acknowledged
            || !self.application_heartbeat_observed
            || !self.correlated_pong_observed
            || !self.socket_not_retired
            || !self.business_event_scope_exact
            || !self.epoch_unchanged
        {
            return Err(invalid("authenticated user-stream gate is incomplete"));
        }
        // A quiescent exclusive account can have no pre-place business event.
        // In that case the socket proof is non-vacuous only when it joins the
        // already-validated signer-authenticated closed-only read and both
        // fixed type-1 account responses from this same evidence record.
        if self.business_event_count == 0 && !self.same_credential_rest_owner_evidence_joined {
            return Err(invalid(
                "quiet user stream lacks non-vacuous same-credential REST owner evidence",
            ));
        }
        match (self.reconnect_count, self.latest_reconnect_monotonic_ns) {
            (0, None) => {
                if !self.complete_cuts_refreshed_after_latest_reconnect {
                    return Err(invalid("user-stream cut freshness flag is false"));
                }
            }
            (0, Some(_)) | (_, None) => {
                return Err(invalid("user-stream reconnect state is inconsistent"));
            }
            (_, Some(reconnected_at)) => {
                if !self.complete_cuts_refreshed_after_latest_reconnect
                    || reconnected_at < bounds.started_monotonic_ns
                    || reconnected_at > bounds.completed_monotonic_ns
                    || self.stamp.observed_at_monotonic_ns <= reconnected_at
                    || refreshed_cut_times
                        .iter()
                        .any(|observed_at| *observed_at <= reconnected_at)
                {
                    return Err(invalid(
                        "user-stream reconnect was not followed by complete fresh cuts",
                    ));
                }
            }
        }
        Ok(())
    }
}

struct ObservationBounds {
    started_monotonic_ns: u64,
    completed_monotonic_ns: u64,
    validated_monotonic_ns: u64,
    started_unix_ms: u64,
    completed_unix_ms: u64,
    validated_unix_ms: u64,
}

impl ObservationBounds {
    fn validate_stamp(&self, stamp: &TrialObservationStamp) -> Result<(), PmTrialPreflightError> {
        validate_sha256(&stamp.response_sha256)?;
        if stamp.observed_at_monotonic_ns < self.started_monotonic_ns
            || stamp.observed_at_monotonic_ns > self.completed_monotonic_ns
        {
            return Err(invalid(
                "preflight observation is outside the canonical window",
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum PmTrialPreflightError {
    #[error("controlled-trial preflight is invalid: {0}")]
    Invalid(&'static str),
}

fn invalid(message: &'static str) -> PmTrialPreflightError {
    PmTrialPreflightError::Invalid(message)
}

fn parse_canonical_utc(value: &str) -> Result<DateTime<Utc>, PmTrialPreflightError> {
    let parsed = DateTime::parse_from_rfc3339(value)
        .map_err(|_| invalid("preflight timestamp is invalid"))?
        .with_timezone(&Utc);
    if parsed.to_rfc3339_opts(SecondsFormat::AutoSi, true) != value {
        return Err(invalid("preflight timestamp is not canonical UTC"));
    }
    Ok(parsed)
}

fn positive_unix_millis(value: DateTime<Utc>) -> Result<u64, PmTrialPreflightError> {
    u64::try_from(value.timestamp_millis())
        .ok()
        .filter(|millis| *millis > 0)
        .ok_or_else(|| invalid("preflight timestamp is before the positive Unix epoch"))
}

fn validate_sha256(value: &str) -> Result<(), PmTrialPreflightError> {
    if value.len() != 64
        || value
            .bytes()
            .any(|byte| !byte.is_ascii_digit() && !(b'a'..=b'f').contains(&byte))
    {
        return Err(invalid("preflight SHA-256 value is invalid"));
    }
    Ok(())
}

fn validate_fee_decimal(value: Option<&str>) -> Result<(), PmTrialPreflightError> {
    let Some(value) = value else {
        return Ok(());
    };
    if value.is_empty()
        || value.len() > 64
        || !value.as_bytes()[0].is_ascii_digit()
        || serde_json::from_str::<serde_json::Number>(value).is_err()
    {
        return Err(invalid(
            "fee decimal is not a bounded nonnegative JSON number",
        ));
    }
    Ok(())
}

fn canonical_price(value: &str) -> Result<PmPrice, PmTrialPreflightError> {
    let parsed = PmPrice::parse_decimal(value).map_err(|_| invalid("book price is invalid"))?;
    if parsed.to_string() != value {
        return Err(invalid("book price is not canonical"));
    }
    Ok(parsed)
}

fn canonical_quantity(value: &str) -> Result<(), PmTrialPreflightError> {
    let parsed = reap_pm_core::PmQuantity::parse_decimal(value)
        .map_err(|_| invalid("market quantity is invalid"))?;
    if parsed.to_string() != value {
        return Err(invalid("market quantity is not canonical"));
    }
    Ok(())
}

fn canonical_u256(value: &str, allow_zero: bool) -> Result<U256, PmTrialPreflightError> {
    let parsed = U256::from_str(value).map_err(|_| invalid("risk amount is invalid"))?;
    if parsed.to_string() != value || (!allow_zero && parsed.is_zero()) {
        return Err(invalid("risk amount is not canonical or unexpectedly zero"));
    }
    Ok(parsed)
}

fn validate_journal_path<'a>(
    value: &'a str,
    artifact_directory: &str,
) -> Result<&'a Path, PmTrialPreflightError> {
    let path = Path::new(value);
    let parent = path.parent();
    if !path.is_absolute()
        || value.len() > 1_024
        || parent != Some(Path::new(artifact_directory))
        || path.file_name().is_none()
        || path
            .components()
            .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        return Err(invalid(
            "journal lease path is outside the exact artifact directory",
        ));
    }
    Ok(path)
}

fn hash_bytes(domain: &[u8], bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(domain);
    hasher.update(bytes);
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use std::{fs, os::unix::fs::PermissionsExt as _, path::Path};

    use tempfile::TempDir;

    use super::*;
    use crate::{
        AuthorizationApproval, AuthorizationBuildBinding, TrialAccount, TrialAuthorization,
        TrialConfig, TrialCredentialSlot, TrialMarket, TrialOrder, TrialOrderType, TrialTimeLimits,
        load_canonical_authorization, load_canonical_trial_config,
    };

    const SIGNER: &str = "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266";
    const FUNDER: &str = "0x70997970C51812dc3A010C7d01b50e0d17dc79C8";
    const CONDITION: &str = "0x3333333333333333333333333333333333333333333333333333333333333333";
    const QUESTION: &str = "0x4444444444444444444444444444444444444444444444444444444444444444";

    #[test]
    fn phase_a_and_phase_b_canonical_preflight_are_structural_and_move_only() {
        for phase in [TrialPhase::APlaceCancel, TrialPhase::BFillPosition] {
            let fixture = Fixture::new(phase);
            let evidence = evidence(&fixture.config, &fixture.authorization);
            let bytes = serde_json::to_vec(&evidence).unwrap();
            let canonical = validate_canonical_trial_preflight(
                &fixture.config,
                &fixture.authorization,
                &bytes,
                time("2026-08-09T12:05:04Z"),
                4_000_000_100,
            )
            .unwrap();
            assert_eq!(canonical.canonical_bytes(), bytes);
            assert_eq!(
                canonical.fingerprint(),
                hash_bytes(PREFLIGHT_FINGERPRINT_DOMAIN, &bytes)
            );
            assert!(!canonical.production_order_entry_authorized());
            assert!(!canonical.real_order_submission_authorized());
            assert_eq!(canonical.place_dispatch_allowance(), 0);
            assert_eq!(
                canonical.value().authorization,
                OfflineAuthorizationState::DENIED
            );
        }
    }

    #[test]
    fn preflight_rejects_noncanonical_unknown_and_trailing_bytes() {
        let fixture = Fixture::new(TrialPhase::APlaceCancel);
        let evidence = evidence(&fixture.config, &fixture.authorization);
        let canonical = serde_json::to_vec(&evidence).unwrap();

        let mut trailing = canonical.clone();
        trailing.push(b'\n');
        assert!(validate(&fixture, &trailing).is_err());

        let mut unknown = canonical[..canonical.len() - 1].to_vec();
        unknown.extend_from_slice(b",\"place_request\":false}");
        assert!(validate(&fixture, &unknown).is_err());

        let mut value: serde_json::Value = serde_json::from_slice(&canonical).unwrap();
        let object = value.as_object_mut().unwrap();
        let schema = object.remove("schema_version").unwrap();
        object.insert("schema_version".into(), schema);
        let reordered = serde_json::to_vec(&value).unwrap();
        assert_ne!(reordered, canonical);
        assert!(validate(&fixture, &reordered).is_err());
    }

    #[test]
    fn time_fingerprint_profile_scope_and_staleness_fail_closed() {
        let mutations: [fn(&mut TrialPreflightEvidence); 5] = [
            |value| value.window.dispatch_deadline_monotonic_ns += 1,
            |value| value.binding.canonical_config_fingerprint = hex(0x91),
            |value| value.account.private_cut.signature_type = 0,
            |value| value.market.token_id = "987654321".into(),
            |value| value.book.stamp.observed_at_monotonic_ns = 99,
        ];
        assert_mutations_rejected(TrialPhase::APlaceCancel, &mutations);
    }

    #[test]
    fn reconnect_fee_order_age_book_and_chain_fail_closed() {
        let mutations: [fn(&mut TrialPreflightEvidence); 7] = [
            |value| {
                value.reconciliation.user_stream.reconnect_count = 1;
                value
                    .reconciliation
                    .user_stream
                    .latest_reconnect_monotonic_ns = Some(1_500_000_100);
                value
                    .reconciliation
                    .user_stream
                    .complete_cuts_refreshed_after_latest_reconnect = false;
            },
            |value| value.market.maker_base_fee_bps = 1,
            |value| value.market.fee_policy_reviewed_and_supported = false,
            |value| value.market.minimum_order_age_seconds = 1,
            |value| value.book.best_ask = "0.5".into(),
            |value| value.finalized_chain.finalized_block_reread_matched = false,
            |value| value.finalized_chain.conditional_tokens_operator_approved = false,
        ];
        assert_mutations_rejected(TrialPhase::APlaceCancel, &mutations);
    }

    #[test]
    fn position_reconciliation_and_risk_fail_closed() {
        let mutations: [fn(&mut TrialPreflightEvidence); 7] = [
            |value| value.data_api_position.configured_token_row_count = 1,
            |value| value.data_api_position.position_consistent_with_account_cut = false,
            |value| value.reconciliation.account_wide_open_orders.complete = false,
            |value| value.reconciliation.no_ambiguous_state = false,
            |value| {
                value
                    .reconciliation
                    .credential_visible_scope_not_funder_wide = false
            },
            |value| value.risk.reservation_is_exact_and_durable = false,
            |value| value.risk.unsigned_order_semantic_commitment_sha256 = "not-a-hash".into(),
        ];
        assert_mutations_rejected(TrialPhase::APlaceCancel, &mutations);
    }

    #[test]
    fn absent_position_is_distinct_from_present_zero_and_sell_requires_nonzero_row() {
        let fixture = Fixture::new(TrialPhase::APlaceCancel);
        let absent = evidence(&fixture.config, &fixture.authorization);
        assert!(validate_evidence(&fixture, &absent).is_ok());

        let mut zero = absent;
        zero.data_api_position.rows_observed = 1;
        zero.data_api_position.configured_token_row_count = 1;
        zero.data_api_position.configured_position = TrialConfiguredPositionState::Present {
            position_row_sha256: hex(0x61),
            size_is_zero: true,
        };
        assert!(validate_evidence(&fixture, &zero).is_ok());

        let sell_fixture = Fixture::new_sell();
        let sell_absent = evidence(&sell_fixture.config, &sell_fixture.authorization);
        assert!(validate_evidence(&sell_fixture, &sell_absent).is_err());

        let mut sell_zero = sell_absent;
        sell_zero.data_api_position.rows_observed = 1;
        sell_zero.data_api_position.configured_token_row_count = 1;
        sell_zero.data_api_position.configured_position = TrialConfiguredPositionState::Present {
            position_row_sha256: hex(0x62),
            size_is_zero: true,
        };
        assert!(validate_evidence(&sell_fixture, &sell_zero).is_err());

        let mut sell_nonzero = sell_zero;
        sell_nonzero.data_api_position.configured_position =
            TrialConfiguredPositionState::Present {
                position_row_sha256: hex(0x62),
                size_is_zero: false,
            };
        assert!(validate_evidence(&sell_fixture, &sell_nonzero).is_ok());
    }

    #[test]
    fn quiet_user_stream_needs_nonvacuous_same_credential_rest_owner_proof() {
        let fixture = Fixture::new(TrialPhase::APlaceCancel);
        let mut evidence = evidence(&fixture.config, &fixture.authorization);
        assert_eq!(evidence.reconciliation.user_stream.business_event_count, 0);
        assert!(validate_evidence(&fixture, &evidence).is_ok());

        evidence
            .reconciliation
            .user_stream
            .same_credential_rest_owner_evidence_joined = false;
        assert!(validate_evidence(&fixture, &evidence).is_err());

        evidence.reconciliation.user_stream.business_event_count = 1;
        assert!(validate_evidence(&fixture, &evidence).is_ok());
    }

    #[test]
    fn refreshed_complete_cuts_are_required_after_reconnect() {
        let fixture = Fixture::new(TrialPhase::APlaceCancel);
        let mut evidence = evidence(&fixture.config, &fixture.authorization);
        evidence.reconciliation.user_stream.reconnect_count = 1;
        evidence
            .reconciliation
            .user_stream
            .latest_reconnect_monotonic_ns = Some(1_500_000_100);
        assert!(validate_evidence(&fixture, &evidence).is_ok());

        evidence
            .reconciliation
            .account_wide_trades
            .stamp
            .observed_at_monotonic_ns = 1_400_000_100;
        assert!(validate_evidence(&fixture, &evidence).is_err());
    }

    fn assert_mutations_rejected(phase: TrialPhase, mutations: &[fn(&mut TrialPreflightEvidence)]) {
        for mutate in mutations {
            let fixture = Fixture::new(phase);
            let mut value = evidence(&fixture.config, &fixture.authorization);
            mutate(&mut value);
            assert!(validate_evidence(&fixture, &value).is_err());
        }
    }

    fn validate_evidence(
        fixture: &Fixture,
        evidence: &TrialPreflightEvidence,
    ) -> Result<CanonicalTrialPreflight, PmTrialPreflightError> {
        validate(fixture, &serde_json::to_vec(evidence).unwrap())
    }

    fn validate(
        fixture: &Fixture,
        bytes: &[u8],
    ) -> Result<CanonicalTrialPreflight, PmTrialPreflightError> {
        validate_canonical_trial_preflight(
            &fixture.config,
            &fixture.authorization,
            bytes,
            time("2026-08-09T12:05:04Z"),
            4_000_000_100,
        )
    }

    fn evidence(
        config: &CanonicalTrialConfig,
        authorization: &CanonicalAuthorization,
    ) -> TrialPreflightEvidence {
        let value = config.value();
        let approval = authorization.value();
        let common_stamp = || TrialObservationStamp {
            observed_at_monotonic_ns: 2_000_000_100,
            response_sha256: hex(0xa0),
        };
        TrialPreflightEvidence {
            schema_version: TRIAL_PREFLIGHT_SCHEMA_VERSION,
            binding: TrialPreflightBinding {
                phase: value.phase,
                canonical_config_sha256: config.canonical_sha256().into(),
                canonical_config_length: config.canonical_length(),
                canonical_config_fingerprint: config.fingerprint().into(),
                trial_plan_fingerprint: config.plan_fingerprint().into(),
                authorization_id: approval.authorization_id.clone(),
                authorization_fingerprint: authorization.fingerprint().into(),
                source_pin_manifest_sha256: value.source_pin_manifest_sha256.clone(),
                runbook_revision: value.runbook_revision.clone(),
                runbook_sha256: value.runbook_sha256.clone(),
                repository_commit: approval.build.repository_commit.clone(),
                cargo_lock_sha256: approval.build.cargo_lock_sha256.clone(),
                clean_tree_attested: true,
                release_binary_sha256: approval.build.release_binary_sha256.clone(),
                release_binary_length: approval.build.release_binary_length,
                host: approval.host.clone(),
                credential_slot_id: value.credential_slot.slot_id.clone(),
                credential_slot_nonsecret_fingerprint_sha256: value
                    .credential_slot
                    .nonsecret_fingerprint_sha256
                    .clone(),
                signer_to_proxy_evidence_reference: value
                    .credential_slot
                    .signer_to_proxy_evidence_reference
                    .clone(),
                journal: value.journal.clone(),
                leases: TrialJournalLeaseEvidence {
                    owner_process_identity: "pid:4242:boot:fixture".into(),
                    owner_process_count: 1,
                    artifact_directory: value.journal.artifact_directory.clone(),
                    artifact_directory_lease_fingerprint: hex(0x01),
                    artifact_directory_exclusive: true,
                    product_journal_path: format!(
                        "{}/product-journal.jsonl",
                        value.journal.artifact_directory
                    ),
                    product_journal_schema_version: 1,
                    product_journal_scope_fingerprint: hex(0x02),
                    product_journal_exclusive: true,
                    authenticated_journal_path: format!(
                        "{}/authenticated-journal.jsonl",
                        value.journal.artifact_directory
                    ),
                    authenticated_journal_schema_version: 2,
                    authenticated_journal_scope_fingerprint: hex(0x03),
                    authenticated_journal_exclusive: true,
                    leases_held_continuously: true,
                    recovery_state_unambiguous: true,
                    authorization_consumption_state:
                        TrialAuthorizationConsumptionLeaseState::PreparedUnconsumed,
                    authorization_consumption_binding_fingerprint: hex(0x04),
                    authorization_consumption_ledger_record_count: 1,
                    authorization_consumption_claim_absent: true,
                },
            },
            window: TrialPreflightWindow {
                observation_started_at_utc: "2026-08-09T12:05:00Z".into(),
                observation_completed_at_utc: "2026-08-09T12:05:03Z".into(),
                validated_at_utc: "2026-08-09T12:05:04Z".into(),
                maximum_observation_age_ms: 5_000,
                dispatch_deadline_at_utc: "2026-08-09T12:05:05Z".into(),
                observation_started_monotonic_ns: 100,
                observation_completed_monotonic_ns: 3_000_000_100,
                validated_at_monotonic_ns: 4_000_000_100,
                dispatch_deadline_monotonic_ns: 5_000_000_100,
            },
            environment: TrialEnvironmentPreflight {
                geoblock: TrialGeoblockEvidence {
                    endpoint: GEOBLOCK_ENDPOINT.into(),
                    egress_identity: approval.host.egress_identity.clone(),
                    reported_ip: approval.host.egress_identity.clone(),
                    country: "US".into(),
                    region: "NY".into(),
                    blocked: false,
                    same_egress_as_clob: true,
                    stamp: common_stamp(),
                },
                server_time: TrialServerTimeEvidence {
                    endpoint: CLOB_TIME_ENDPOINT.into(),
                    server_unix_ms: 1_786_277_102_000,
                    previous_server_unix_ms: 1_786_277_101_000,
                    local_wall_receive_unix_ms: 1_786_277_102_100,
                    maximum_absolute_skew_ms: MAX_SERVER_TIME_SKEW_MS,
                    product_clock_epoch_fingerprint: hex(0x05),
                    previous_product_clock_epoch_fingerprint: hex(0x05),
                    timestamp_regression_absent: true,
                    epoch_unchanged: true,
                    stamp: common_stamp(),
                },
                clob_health_response_sha256: hex(0x06),
                clob_health_green: true,
                matching_engine_restart_reported: false,
                restricted_mode_observed: false,
                health_stamp: TrialObservationStamp {
                    observed_at_monotonic_ns: 2_000_000_100,
                    response_sha256: hex(0x06),
                },
            },
            market: TrialMarketPreflight {
                condition_id: value.market.condition_id.clone(),
                question_id: value.market.question_id.clone(),
                token_id: value.market.token_id.clone(),
                outcome_label: value.market.outcome_label.clone(),
                token_count: 2,
                exact_token_membership: true,
                domain: value.market.domain,
                exchange: value.market.exchange.clone(),
                pusd_contract: value.market.pusd_contract.clone(),
                conditional_tokens_contract: value.market.conditional_tokens_contract.clone(),
                active: true,
                closed: false,
                archived: false,
                accepting_orders: true,
                order_book_enabled: true,
                tick: value.order.tick.clone(),
                minimum_order_size: value.order.minimum_order_size.clone(),
                maker_base_fee_bps: 0,
                taker_base_fee_bps: 0,
                fee_rate: Some("0.020".into()),
                fee_exponent: Some("2.0".into()),
                fee_taker_only: Some(true),
                fee_policy_reviewed_and_supported: true,
                take_only_delay_enabled: false,
                seconds_delay: 0,
                cancel_book_on_start: false,
                minimum_order_age_seconds: 0,
                sports_market: false,
                accepting_order_timestamp_utc: Some("2026-08-09T12:00:00Z".into()),
                game_start_time_utc: None,
                end_time_utc: Some("2026-08-10T12:00:00Z".into()),
                resolution_not_imminent: true,
                long_market_response_sha256: hex(0x07),
                clob_market_response_sha256: hex(0x08),
                stamp: common_stamp(),
            },
            book: TrialBookPreflight {
                condition_id: value.market.condition_id.clone(),
                token_id: value.market.token_id.clone(),
                tick: value.order.tick.clone(),
                exact_order_price: value.order.price.clone(),
                best_bid: "0.49".into(),
                best_ask: "0.51".into(),
                snapshot_sequence: 1,
                stream_epoch_fingerprint: hex(0x09),
                public_stream_established: true,
                two_sided: true,
                not_crossed: true,
                tick_unchanged: true,
                passive_at_dispatch: true,
                stamp: common_stamp(),
            },
            account: TrialAccountPreflight {
                closed_only: TrialClosedOnlyEvidence {
                    endpoint: CLOSED_ONLY_ENDPOINT.into(),
                    signer: value.account.signer.clone(),
                    closed_only: false,
                    stamp: common_stamp(),
                },
                private_cut: TrialPrivateAccountCut {
                    signature_type: 1,
                    signer: value.account.signer.clone(),
                    funder: value.account.funder.clone(),
                    private_key_derived_signer_matches: true,
                    l2_signer_matches: true,
                    remote_api_key_owner_matches_signer: true,
                    signer_to_proxy_evidence_reviewed: true,
                    collateral_response_sha256: hex(0x0a),
                    configured_token_response_sha256: hex(0x0b),
                    balance_response_count: 2,
                    allowance_entry_count: 2,
                    collateral_balance_sufficient_for_order: true,
                    collateral_allowance_sufficient_for_order: true,
                    configured_token_balance_sufficient_for_order: true,
                    configured_token_allowance_sufficient_for_order: true,
                    all_remote_reservations_accounted: true,
                    stamp: common_stamp(),
                },
            },
            finalized_chain: TrialFinalizedChainPreflight {
                rpc_origin: POLYGON_RPC_ORIGIN.into(),
                chain_id: 137,
                proxy_funder: value.account.funder.clone(),
                exchange_spender: value.market.exchange.clone(),
                pusd_contract: value.market.pusd_contract.clone(),
                conditional_tokens_contract: value.market.conditional_tokens_contract.clone(),
                finalized_block_number: 77_000_000,
                finalized_block_hash: hex(0x0c),
                finalized_block_unix_seconds: 1_786_277_100,
                rpc_request_count: 5,
                chain_id_response_sha256: hex(0x0d),
                finalized_block_response_sha256: hex(0x0e),
                allowance_response_sha256: hex(0x0f),
                operator_approval_response_sha256: hex(0x10),
                block_reread_response_sha256: hex(0x11),
                one_finalized_block_for_all_calls: true,
                finalized_block_reread_matched: true,
                finalized_block_fresh: true,
                pusd_allowance_sufficient: true,
                conditional_tokens_operator_approved: true,
                stamp: common_stamp(),
            },
            data_api_position: TrialDataApiPositionPreflight {
                proxy_funder: value.account.funder.clone(),
                condition_id: value.market.condition_id.clone(),
                token_id: value.market.token_id.clone(),
                pages_observed: 1,
                rows_observed: 0,
                configured_token_row_count: 0,
                page_walk_response_sha256: hex(0x12),
                complete_bounded_page_walk: true,
                configured_position: TrialConfiguredPositionState::Absent,
                position_consistent_with_account_cut: true,
                stamp: common_stamp(),
            },
            reconciliation: TrialReconciliationPreflight {
                user_stream: TrialUserStreamPreflight {
                    signer: value.account.signer.clone(),
                    condition_id: value.market.condition_id.clone(),
                    epoch_fingerprint: hex(0x13),
                    business_event_set_sha256: hex(0x14),
                    authenticated: true,
                    bounded_subscription_acknowledged: true,
                    application_heartbeat_observed: true,
                    correlated_pong_observed: true,
                    socket_not_retired: true,
                    business_event_count: 0,
                    business_event_scope_exact: true,
                    same_credential_rest_owner_evidence_joined: true,
                    epoch_unchanged: true,
                    reconnect_count: 0,
                    latest_reconnect_monotonic_ns: None,
                    complete_cuts_refreshed_after_latest_reconnect: true,
                    stamp: common_stamp(),
                },
                account_wide_open_orders: TrialCompleteCutEvidence {
                    pages_observed: 1,
                    rows_observed: 0,
                    terminal_cursor_observed: true,
                    complete: true,
                    response_set_sha256: hex(0x15),
                    stamp: common_stamp(),
                },
                account_wide_trades: TrialCompleteCutEvidence {
                    pages_observed: 1,
                    rows_observed: 0,
                    terminal_cursor_observed: true,
                    complete: true,
                    response_set_sha256: hex(0x16),
                    stamp: common_stamp(),
                },
                exact_details: TrialExactDetailCutEvidence {
                    implicated_order_id_count: 0,
                    exact_detail_count: 0,
                    complete: true,
                    response_set_sha256: hex(0x17),
                    stamp: common_stamp(),
                },
                no_existing_trial_order: true,
                no_unmanaged_order: true,
                no_unresolved_fill: true,
                no_ambiguous_state: true,
                no_concurrent_proxy_trading_attested: true,
                credential_visible_scope_not_funder_wide: true,
                reconciliation_commitment_sha256: hex(0x18),
            },
            risk: TrialRiskPreflight {
                side: value.order.side,
                price: value.order.price.clone(),
                quantity: value.order.quantity.clone(),
                maker_amount: value.order.maker_amount.clone(),
                taker_amount: value.order.taker_amount.clone(),
                maximum_loss_pusd_base_units: value.order.maximum_loss_pusd_base_units.clone(),
                reservation_pusd_base_units: value.order.reservation_pusd_base_units.clone(),
                sell_outcome_share_payout_risk_cap_base_units: value
                    .order
                    .sell_outcome_share_payout_risk_cap_base_units
                    .clone(),
                unsigned_order_semantic_commitment_sha256: hex(0x19),
                risk_decision_commitment_sha256: hex(0x1a),
                reservation_commitment_sha256: hex(0x1b),
                loss_commitment_sha256: hex(0x1c),
                remote_reservation_set_sha256: hex(0x1d),
                remote_reservation_count: 0,
                exact_amounts_recomputed: true,
                risk_within_exact_cap: true,
                reservation_is_exact_and_durable: true,
                all_remote_reservations_deducted: true,
                available_after_reservations_sufficient: true,
                one_possible_fill_within_loss_cap: true,
                no_fee_or_rebate_credit_in_loss_bound: true,
                one_exact_order_commitment: true,
                replacement_or_reprice_disabled: true,
            },
            phase_gate: match value.phase {
                TrialPhase::APlaceCancel => TrialPhaseGateEvidence {
                    phase_a_implementation_gates_green: true,
                    accepted_phase_a_evidence_sha256: None,
                    separate_phase_b_authorization: false,
                },
                TrialPhase::BFillPosition => TrialPhaseGateEvidence {
                    phase_a_implementation_gates_green: true,
                    accepted_phase_a_evidence_sha256: Some(hex(0x1e)),
                    separate_phase_b_authorization: true,
                },
            },
            authorization: OfflineAuthorizationState::DENIED,
        }
    }

    struct Fixture {
        _directory: TempDir,
        config: CanonicalTrialConfig,
        authorization: CanonicalAuthorization,
    }

    impl Fixture {
        fn new(phase: TrialPhase) -> Self {
            Self::from_config(trial_config(phase))
        }

        fn new_sell() -> Self {
            let mut config = trial_config(TrialPhase::APlaceCancel);
            config.order.side = TrialSide::Sell;
            config.order.maker_amount = "5000000".into();
            config.order.taker_amount = "2500000".into();
            config.order.reservation_pusd_base_units = "0".into();
            config.order.sell_outcome_share_payout_risk_cap_base_units = Some("5000000".into());
            Self::from_config(config)
        }

        fn from_config(mut raw_config: TrialConfig) -> Self {
            let directory = protected_dir();
            raw_config.journal.artifact_directory = directory.path().to_str().unwrap().into();
            let config_path = directory.path().join("config.json");
            write_0600(&config_path, &serde_json::to_vec(&raw_config).unwrap());
            let config = load_canonical_trial_config(&config_path).unwrap();
            let authorization_path = directory.path().join("authorization.json");
            let raw_authorization = trial_authorization(&config);
            write_0600(
                &authorization_path,
                &serde_json::to_vec(&raw_authorization).unwrap(),
            );
            let authorization = load_canonical_authorization(&authorization_path).unwrap();
            Self {
                _directory: directory,
                config,
                authorization,
            }
        }
    }

    fn trial_config(phase: TrialPhase) -> TrialConfig {
        TrialConfig {
            schema_version: 1,
            profile: "pm_t2_type1_proxy_offline_a0".into(),
            phase,
            source_pin_manifest_sha256: hex(0x21),
            runbook_revision: "pm-t2-runbook-v1".into(),
            runbook_sha256: hex(0x22),
            account: TrialAccount {
                chain_id: 137,
                signature_type: 1,
                wallet_profile: "poly_proxy".into(),
                signer: SIGNER.into(),
                funder: FUNDER.into(),
            },
            market: TrialMarket {
                condition_id: CONDITION.into(),
                question_id: QUESTION.into(),
                token_id: "123456789".into(),
                outcome_label: "YES".into(),
                domain: TrialDomain::Standard,
                exchange: "0xE111180000d2663C0091e4f400237545B87B996B".into(),
                pusd_contract: "0xC011a7E12a19f7B1f670d46F03B03f3342E82DFB".into(),
                conditional_tokens_contract: "0x4D97DCd97eC945f40cF65F87097ACe5EA0476045".into(),
            },
            order: TrialOrder {
                salt: 1,
                timestamp_ms: 1_800_000_000_000,
                side: TrialSide::Buy,
                price: "0.5".into(),
                quantity: "5".into(),
                tick: "0.01".into(),
                minimum_order_size: "5".into(),
                maker_amount: "2500000".into(),
                taker_amount: "5000000".into(),
                maximum_loss_pusd_base_units: "2500000".into(),
                reservation_pusd_base_units: "2500000".into(),
                sell_outcome_share_payout_risk_cap_base_units: None,
                order_type: TrialOrderType::Gtc,
                post_only: true,
                defer_exec: false,
                expiration: "0".into(),
                metadata: format!("0x{}", "00".repeat(32)),
                builder: format!("0x{}", "00".repeat(32)),
                no_fee_or_rebate_credit_in_loss_bound: true,
                place_dispatch_allowance: 1,
                replacement_or_reprice_allowed: false,
                primary_cancel_dispatch_budget: 1,
                recovery_cancel_dispatch_budget: 2,
            },
            time_limits: TrialTimeLimits {
                maximum_preflight_observation_age_ms: 5_000,
                maximum_resting_duration_ms: match phase {
                    TrialPhase::APlaceCancel => 30_000,
                    TrialPhase::BFillPosition => 60_000,
                },
                primary_cancel_deadline_ms: 65_000,
                cleanup_not_after_ms: 120_000,
                maximum_remediation_duration_ms: 90_000,
            },
            credential_slot: TrialCredentialSlot {
                slot_id: "pm-t2-slot-1".into(),
                nonsecret_fingerprint_sha256: hex(0x23),
                signer_to_proxy_evidence_reference: "reviewed-account-record:pm-t2-account-v1"
                    .into(),
            },
            journal: TrialJournalBinding {
                artifact_directory: "/tmp/reap-pm-t2-artifacts".into(),
                journal_family: "pm-t2-controlled-trial".into(),
                journal_version: 1,
                authorization_consumption_ledger_file: "authorization-consumption.jsonl".into(),
                authorization_consumption_claim_file: "authorization-consumed.claim".into(),
            },
        }
    }

    fn trial_authorization(config: &CanonicalTrialConfig) -> TrialAuthorization {
        let phase = config.value().phase;
        TrialAuthorization {
            schema_version: 1,
            authorization_id: match phase {
                TrialPhase::APlaceCancel => "pm-t2-a-authorization-1",
                TrialPhase::BFillPosition => "pm-t2-b-authorization-1",
            }
            .into(),
            phase,
            issuing_reviewer: "operator-reviewer".into(),
            reviewed_at_utc: "2026-08-09T11:59:00Z".into(),
            purpose: match phase {
                TrialPhase::APlaceCancel => "one_exact_pm_t2_phase_a_passive_place_cancel_attempt",
                TrialPhase::BFillPosition => {
                    "one_exact_pm_t2_phase_b_passive_buy_fill_position_attempt"
                }
            }
            .into(),
            not_before_utc: "2026-08-09T12:00:00Z".into(),
            expires_at_utc: "2026-08-09T12:15:00Z".into(),
            cleanup_not_after_utc: "2026-08-09T12:20:00Z".into(),
            build: AuthorizationBuildBinding {
                repository_commit: "66".repeat(20),
                clean_tree_attested: true,
                cargo_lock_sha256: hex(0x77),
                release_binary_sha256: hex(0x88),
                release_binary_length: 1_000_000,
                canonical_config_sha256: config.canonical_sha256().into(),
                canonical_config_length: config.canonical_length(),
                canonical_config_fingerprint: config.fingerprint().into(),
            },
            host: AuthorizationHostBinding {
                host_identity: "trial-host-1".into(),
                boot_identity: "01234567-89ab-cdef-0123-456789abcdef".into(),
                runtime_user: "reap-trial".into(),
                egress_identity: "203.0.113.7".into(),
            },
            trial: config.value().clone(),
            trial_plan_fingerprint: config.plan_fingerprint().into(),
            approval: AuthorizationApproval {
                only_named_phase: true,
                exactly_one_attempt: true,
                one_possible_fill_is_within_loss_cap: true,
                post_only_does_not_mean_no_fill: true,
                no_concurrent_proxy_trading_attested: true,
                independent_cleanup_method_reviewed: true,
            },
        }
    }

    fn protected_dir() -> TempDir {
        let directory = tempfile::tempdir().unwrap();
        fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700)).unwrap();
        directory
    }

    fn write_0600(path: &Path, bytes: &[u8]) {
        fs::write(path, bytes).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
    }

    fn time(value: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(value)
            .unwrap()
            .with_timezone(&Utc)
    }

    fn hex(byte: u8) -> String {
        format!("{byte:02x}").repeat(32)
    }
}
