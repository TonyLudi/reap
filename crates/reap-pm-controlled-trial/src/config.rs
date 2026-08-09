use std::{net::IpAddr, path::Path, str::FromStr as _};

use chrono::{DateTime, Utc};
use reap_pm_core::{
    EvmAddress, PmOrderSalt, PmOrderSide, PmPrice, PmQuantity, PmTick, PmTokenId, U256,
};
use reap_polymarket_auth::{
    EoaAddress, PlacePublicRequestIdentity, PmClobDomain, derive_place_public_request_identity,
};
use reap_polymarket_wire::{PM_CLOB_V2_EMPTY_BYTES32, PmUnsignedClobV2Order};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use sha2::{Digest as _, Sha256};
use thiserror::Error;

use crate::{
    OfflineAuthorizationState,
    protected_file::{ProtectedFileKind, read_one},
};

pub const TRIAL_CONFIG_SCHEMA_VERSION: u32 = 1;
pub const TRIAL_AUTHORIZATION_SCHEMA_VERSION: u32 = 1;
pub const PM_T2_JOURNAL_FAMILY_V1: &str = "pm-t2-controlled-trial";
pub const PM_T2_JOURNAL_VERSION_V1: u32 = 1;
pub const PM_T2_AUTHORIZATION_CONSUMPTION_LEDGER_FILE_V1: &str = "authorization-consumption.jsonl";
pub const PM_T2_AUTHORIZATION_CONSUMPTION_CLAIM_FILE_V1: &str = "authorization-consumed.claim";
pub const PM_T2_LIVE_INTENT_JOURNAL_FILE_V1: &str = "pm-t2-controlled-trial-live-intent-v1.jsonl";
pub const PM_T2_LIVE_DISPATCH_JOURNAL_FILE_V1: &str =
    "pm-t2-controlled-trial-live-dispatch-v1.jsonl";
const MAX_CANONICAL_RECORD_BYTES: usize = 128 * 1024;
const MAX_AUTHORIZATION_LIFETIME_SECONDS: i64 = 15 * 60;
pub(crate) const MAX_PREFLIGHT_OBSERVATION_AGE_MS: u64 = 5_000;
const MAX_BASE_FEE_BPS: u64 = 10_000;
const CONFIG_FINGERPRINT_DOMAIN: &[u8] = b"reap.pm-t2.controlled-trial.config.v1\0";
const PLAN_FINGERPRINT_DOMAIN: &[u8] = b"reap.pm-t2.controlled-trial.plan.v1\0";
const AUTHORIZATION_FINGERPRINT_DOMAIN: &[u8] = b"reap.pm-t2.controlled-trial.authorization.v1\0";

const PROFILE: &str = "pm_t2_type1_proxy_offline_a0";
const WALLET_PROFILE: &str = "poly_proxy";
const STANDARD_EXCHANGE: &str = "0xE111180000d2663C0091e4f400237545B87B996B";
const NEGATIVE_RISK_EXCHANGE: &str = "0xe2222d279d744050d28e00520010520000310F59";
const PUSD: &str = "0xC011a7E12a19f7B1f670d46F03B03f3342E82DFB";
const CONDITIONAL_TOKENS: &str = "0x4D97DCd97eC945f40cF65F87097ACe5EA0476045";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrialPhase {
    APlaceCancel,
    BFillPosition,
}

impl TrialPhase {
    const fn purpose(self) -> &'static str {
        match self {
            Self::APlaceCancel => "one_exact_pm_t2_phase_a_passive_place_cancel_attempt",
            Self::BFillPosition => "one_exact_pm_t2_phase_b_passive_buy_fill_position_attempt",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum TrialSide {
    Buy,
    Sell,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrialDomain {
    Standard,
    NegativeRisk,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TrialOrderType {
    #[serde(rename = "GTC")]
    Gtc,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrialAccount {
    pub chain_id: u64,
    pub signature_type: u8,
    pub wallet_profile: String,
    pub signer: String,
    pub funder: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrialMarket {
    pub condition_id: String,
    pub question_id: String,
    pub token_id: String,
    pub outcome_label: String,
    pub domain: TrialDomain,
    pub exchange: String,
    pub pusd_contract: String,
    pub conditional_tokens_contract: String,
    pub maker_base_fee_bps: u64,
    pub taker_base_fee_bps: u64,
    pub fee_rate: String,
    pub fee_exponent: String,
    pub fee_taker_only: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrialOrder {
    pub salt: u64,
    pub timestamp_ms: u64,
    pub side: TrialSide,
    pub price: String,
    pub quantity: String,
    pub tick: String,
    pub minimum_order_size: String,
    pub maker_amount: String,
    pub taker_amount: String,
    pub maximum_loss_pusd_base_units: String,
    pub reservation_pusd_base_units: String,
    pub sell_outcome_share_payout_risk_cap_base_units: Option<String>,
    pub order_type: TrialOrderType,
    pub post_only: bool,
    pub defer_exec: bool,
    pub expiration: String,
    pub metadata: String,
    pub builder: String,
    pub no_fee_or_rebate_credit_in_loss_bound: bool,
    pub place_dispatch_allowance: u8,
    pub replacement_or_reprice_allowed: bool,
    pub primary_cancel_dispatch_budget: u8,
    pub recovery_cancel_dispatch_budget: u8,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrialTimeLimits {
    pub maximum_preflight_observation_age_ms: u64,
    pub maximum_resting_duration_ms: u64,
    pub primary_cancel_deadline_ms: u64,
    pub cleanup_not_after_ms: u64,
    pub maximum_remediation_duration_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrialCredentialSlot {
    pub slot_id: String,
    pub nonsecret_fingerprint_sha256: String,
    pub signer_to_proxy_evidence_reference: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrialJournalBinding {
    pub artifact_directory: String,
    pub journal_family: String,
    pub journal_version: u32,
    pub authorization_consumption_ledger_file: String,
    pub authorization_consumption_claim_file: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrialConfig {
    pub schema_version: u32,
    pub profile: String,
    pub phase: TrialPhase,
    pub source_pin_manifest_sha256: String,
    pub runbook_revision: String,
    pub runbook_sha256: String,
    pub account: TrialAccount,
    pub market: TrialMarket,
    pub order: TrialOrder,
    pub time_limits: TrialTimeLimits,
    pub credential_slot: TrialCredentialSlot,
    pub journal: TrialJournalBinding,
}

impl TrialConfig {
    pub fn validate(&self) -> Result<(), PmTrialConfigError> {
        if self.schema_version != TRIAL_CONFIG_SCHEMA_VERSION {
            return Err(invalid("unsupported controlled-trial config schema"));
        }
        if self.profile != PROFILE {
            return Err(invalid(
                "controlled-trial profile is not the offline PM-T2 profile",
            ));
        }
        validate_sha256(&self.source_pin_manifest_sha256)?;
        validate_token(&self.runbook_revision, 128, "runbook revision is invalid")?;
        validate_sha256(&self.runbook_sha256)?;
        self.validate_account()?;
        let unsigned = self.validate_market_and_order()?;
        if unsigned.maker_amount().to_string() != self.order.maker_amount
            || unsigned.taker_amount().to_string() != self.order.taker_amount
        {
            return Err(invalid(
                "configured maker/taker amounts do not match exact order lowering",
            ));
        }
        self.validate_risk(unsigned.maker_amount(), unsigned.taker_amount())?;
        self.validate_time()?;
        validate_token(
            &self.credential_slot.slot_id,
            128,
            "credential slot ID is invalid",
        )?;
        validate_sha256(&self.credential_slot.nonsecret_fingerprint_sha256)?;
        validate_reference(
            &self.credential_slot.signer_to_proxy_evidence_reference,
            "signer-to-proxy evidence reference is invalid",
        )?;
        validate_absolute_path(&self.journal.artifact_directory)?;
        if self.journal.journal_family != PM_T2_JOURNAL_FAMILY_V1
            || self.journal.journal_version != PM_T2_JOURNAL_VERSION_V1
            || self.journal.authorization_consumption_ledger_file
                != PM_T2_AUTHORIZATION_CONSUMPTION_LEDGER_FILE_V1
            || self.journal.authorization_consumption_claim_file
                != PM_T2_AUTHORIZATION_CONSUMPTION_CLAIM_FILE_V1
        {
            return Err(invalid(
                "journal family, version, or fixed filenames drifted",
            ));
        }
        validate_direct_entry(
            &self.journal.authorization_consumption_ledger_file,
            "authorization-consumption ledger filename is invalid",
        )?;
        validate_direct_entry(
            &self.journal.authorization_consumption_claim_file,
            "authorization-consumption claim filename is invalid",
        )?;
        Ok(())
    }

    fn validate_account(&self) -> Result<(), PmTrialConfigError> {
        if self.account.chain_id != 137
            || self.account.signature_type != 1
            || self.account.wallet_profile != WALLET_PROFILE
        {
            return Err(invalid(
                "account is not the fixed Polygon type-1 proxy profile",
            ));
        }
        let signer = parse_profile_address(&self.account.signer)?;
        let funder = parse_profile_address(&self.account.funder)?;
        if signer == funder {
            return Err(invalid("type-1 signer and proxy funder must be distinct"));
        }
        Ok(())
    }

    fn validate_market_and_order(&self) -> Result<PmUnsignedClobV2Order, PmTrialConfigError> {
        validate_lower_hex32(&self.market.condition_id, "condition ID is invalid")?;
        validate_lower_hex32(&self.market.question_id, "question ID is invalid")?;
        let token_units = canonical_u256(&self.market.token_id, false)?;
        let token = PmTokenId::new(token_units).map_err(|_| invalid("token ID must be nonzero"))?;
        validate_reference(&self.market.outcome_label, "outcome label is invalid")?;
        let expected_exchange = match self.market.domain {
            TrialDomain::Standard => STANDARD_EXCHANGE,
            TrialDomain::NegativeRisk => NEGATIVE_RISK_EXCHANGE,
        };
        if self.market.exchange != expected_exchange
            || self.market.pusd_contract != PUSD
            || self.market.conditional_tokens_contract != CONDITIONAL_TOKENS
        {
            return Err(invalid(
                "market contract identities are outside the frozen PM-T2 profile",
            ));
        }
        parse_address(&self.market.exchange)?;
        parse_address(&self.market.pusd_contract)?;
        parse_address(&self.market.conditional_tokens_contract)?;
        validate_fee_lexeme(&self.market.fee_rate)?;
        validate_fee_lexeme(&self.market.fee_exponent)?;
        if self.market.maker_base_fee_bps != 0
            || self.market.taker_base_fee_bps > MAX_BASE_FEE_BPS
            || !self.market.fee_taker_only
        {
            return Err(invalid(
                "reviewed market fee tuple is outside the maker-zero taker-only profile",
            ));
        }

        let price = PmPrice::parse_decimal(&self.order.price)
            .map_err(|_| invalid("order price is not canonical"))?;
        let quantity = PmQuantity::parse_decimal(&self.order.quantity)
            .map_err(|_| invalid("order quantity is not canonical"))?;
        let tick = PmTick::parse_decimal(&self.order.tick)
            .map_err(|_| invalid("order tick is not canonical"))?;
        let minimum = PmQuantity::parse_decimal(&self.order.minimum_order_size)
            .map_err(|_| invalid("minimum order size is not canonical"))?;
        if price.to_string() != self.order.price
            || quantity.to_string() != self.order.quantity
            || tick.to_string() != self.order.tick
            || minimum.to_string() != self.order.minimum_order_size
        {
            return Err(invalid(
                "order decimals are not in canonical six-decimal form",
            ));
        }
        let salt = PmOrderSalt::from_u64(self.order.salt)
            .map_err(|_| invalid("order salt is outside the JSON-safe range"))?;
        let side = match self.order.side {
            TrialSide::Buy => PmOrderSide::Buy,
            TrialSide::Sell => PmOrderSide::Sell,
        };
        let unsigned = PmUnsignedClobV2Order::new_pm_t2_proxy(
            salt,
            parse_profile_address(&self.account.funder)?,
            parse_profile_address(&self.account.signer)?,
            token,
            side,
            price,
            quantity,
            tick,
            minimum,
            self.order.timestamp_ms,
        )
        .map_err(|_| invalid("order terms do not lower into the fixed PM-T2 proxy order"))?;
        if self.order.order_type != TrialOrderType::Gtc
            || !self.order.post_only
            || self.order.defer_exec
            || self.order.expiration != "0"
            || self.order.metadata != PM_CLOB_V2_EMPTY_BYTES32
            || self.order.builder != PM_CLOB_V2_EMPTY_BYTES32
            || !self.order.no_fee_or_rebate_credit_in_loss_bound
            || self.order.place_dispatch_allowance != 1
            || self.order.replacement_or_reprice_allowed
            || self.order.primary_cancel_dispatch_budget != 1
            || self.order.recovery_cancel_dispatch_budget > 2
        {
            return Err(invalid(
                "order execution envelope is outside the fixed one-attempt profile",
            ));
        }
        if self.phase == TrialPhase::BFillPosition && self.order.side != TrialSide::Buy {
            return Err(invalid("the first Phase B trial is BUY-only"));
        }
        Ok(unsigned)
    }

    fn validate_risk(&self, maker: U256, _taker: U256) -> Result<(), PmTrialConfigError> {
        let maximum_loss = canonical_u256(&self.order.maximum_loss_pusd_base_units, false)?;
        let reservation = canonical_u256(&self.order.reservation_pusd_base_units, true)?;
        match self.order.side {
            TrialSide::Buy => {
                if reservation != maker || maximum_loss < maker {
                    return Err(invalid(
                        "BUY reservation/loss cap does not cover exact maker pUSD",
                    ));
                }
                if self
                    .order
                    .sell_outcome_share_payout_risk_cap_base_units
                    .is_some()
                {
                    return Err(invalid("BUY must not carry a SELL share/payout-risk cap"));
                }
            }
            TrialSide::Sell => {
                if !reservation.is_zero() {
                    return Err(invalid("SELL must have zero pUSD reservation"));
                }
                let share_cap = self
                    .order
                    .sell_outcome_share_payout_risk_cap_base_units
                    .as_deref()
                    .ok_or_else(|| invalid("SELL requires an exact share/payout-risk cap"))?;
                if canonical_u256(share_cap, false)? < maker {
                    return Err(invalid(
                        "SELL share/payout-risk cap is below exact maker shares",
                    ));
                }
            }
        }
        Ok(())
    }

    fn validate_time(&self) -> Result<(), PmTrialConfigError> {
        let limits = &self.time_limits;
        if limits.maximum_preflight_observation_age_ms == 0
            || limits.maximum_preflight_observation_age_ms > MAX_PREFLIGHT_OBSERVATION_AGE_MS
            || limits.maximum_resting_duration_ms == 0
            || limits.primary_cancel_deadline_ms == 0
            || limits.cleanup_not_after_ms == 0
            || limits.maximum_remediation_duration_ms == 0
            || limits.primary_cancel_deadline_ms > limits.cleanup_not_after_ms
            || limits.maximum_resting_duration_ms > limits.primary_cancel_deadline_ms
            || limits.maximum_remediation_duration_ms > limits.cleanup_not_after_ms
        {
            return Err(invalid("trial time limits are inconsistent"));
        }
        let maximum_rest = match self.phase {
            TrialPhase::APlaceCancel => 30_000,
            TrialPhase::BFillPosition => 300_000,
        };
        if limits.maximum_resting_duration_ms > maximum_rest {
            return Err(invalid(
                "maximum resting duration exceeds the phase hard cap",
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthorizationBuildBinding {
    pub repository_commit: String,
    pub clean_tree_attested: bool,
    pub cargo_lock_sha256: String,
    pub release_binary_sha256: String,
    pub release_binary_length: u64,
    pub canonical_config_sha256: String,
    pub canonical_config_length: u64,
    pub canonical_config_fingerprint: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthorizationHostBinding {
    pub host_identity: String,
    pub boot_identity: String,
    pub runtime_user: String,
    pub egress_identity: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthorizationApproval {
    pub only_named_phase: bool,
    pub exactly_one_attempt: bool,
    pub one_possible_fill_is_within_loss_cap: bool,
    pub post_only_does_not_mean_no_fill: bool,
    pub no_concurrent_proxy_trading_attested: bool,
    pub independent_cleanup_method_reviewed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrialAuthorization {
    pub schema_version: u32,
    pub authorization_id: String,
    pub phase: TrialPhase,
    pub issuing_reviewer: String,
    pub reviewed_at_utc: String,
    pub purpose: String,
    pub not_before_utc: String,
    pub expires_at_utc: String,
    pub cleanup_not_after_utc: String,
    pub build: AuthorizationBuildBinding,
    pub host: AuthorizationHostBinding,
    pub trial: TrialConfig,
    pub trial_plan_fingerprint: String,
    pub approval: AuthorizationApproval,
}

impl TrialAuthorization {
    fn validate(
        &self,
        config: &CanonicalTrialConfig,
        now: DateTime<Utc>,
    ) -> Result<(), PmTrialConfigError> {
        if self.schema_version != TRIAL_AUTHORIZATION_SCHEMA_VERSION {
            return Err(invalid("unsupported controlled-trial authorization schema"));
        }
        validate_token(&self.authorization_id, 128, "authorization ID is invalid")?;
        validate_reference(&self.issuing_reviewer, "issuing reviewer is invalid")?;
        if self.phase != config.value.phase
            || self.purpose != self.phase.purpose()
            || self.trial != config.value
        {
            return Err(invalid(
                "authorization does not bind the exact reviewed trial config",
            ));
        }
        self.trial.validate()?;
        if self.trial_plan_fingerprint != config.plan_fingerprint {
            return Err(invalid("authorization trial-plan fingerprint mismatch"));
        }
        self.validate_build(config)?;
        self.validate_host()?;
        if !self.approval.only_named_phase
            || !self.approval.exactly_one_attempt
            || !self.approval.one_possible_fill_is_within_loss_cap
            || !self.approval.post_only_does_not_mean_no_fill
            || !self.approval.no_concurrent_proxy_trading_attested
            || !self.approval.independent_cleanup_method_reviewed
        {
            return Err(invalid("authorization approval statements are incomplete"));
        }
        let reviewed = parse_utc(&self.reviewed_at_utc)?;
        let not_before = parse_utc(&self.not_before_utc)?;
        let expires = parse_utc(&self.expires_at_utc)?;
        let cleanup = parse_utc(&self.cleanup_not_after_utc)?;
        let lifetime_seconds = expires.timestamp() - not_before.timestamp();
        if reviewed > not_before
            || not_before >= expires
            || lifetime_seconds > MAX_AUTHORIZATION_LIFETIME_SECONDS
            || cleanup < expires
        {
            return Err(invalid("authorization time envelope is invalid"));
        }
        if now < not_before || now >= expires {
            return Err(invalid(
                "authorization is early or expired at the supplied verification time",
            ));
        }
        Ok(())
    }

    fn validate_build(&self, config: &CanonicalTrialConfig) -> Result<(), PmTrialConfigError> {
        validate_lower_hex(
            &self.build.repository_commit,
            40,
            "repository commit is invalid",
        )?;
        validate_sha256(&self.build.cargo_lock_sha256)?;
        validate_sha256(&self.build.release_binary_sha256)?;
        if !self.build.clean_tree_attested || self.build.release_binary_length == 0 {
            return Err(invalid("build binding is incomplete"));
        }
        if self.build.canonical_config_sha256 != config.canonical_sha256
            || self.build.canonical_config_length != config.canonical_bytes.len() as u64
            || self.build.canonical_config_fingerprint != config.fingerprint
        {
            return Err(invalid(
                "authorization build binding does not match canonical config bytes",
            ));
        }
        Ok(())
    }

    fn validate_host(&self) -> Result<(), PmTrialConfigError> {
        validate_reference(&self.host.host_identity, "host identity is invalid")?;
        validate_token(&self.host.boot_identity, 128, "boot identity is invalid")?;
        validate_token(&self.host.runtime_user, 128, "runtime user is invalid")?;
        IpAddr::from_str(&self.host.egress_identity)
            .map_err(|_| invalid("egress identity must be one exact IP address"))?;
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct CanonicalTrialConfig {
    value: TrialConfig,
    canonical_bytes: Vec<u8>,
    canonical_sha256: String,
    fingerprint: String,
    plan_fingerprint: String,
}

impl CanonicalTrialConfig {
    #[must_use]
    pub fn value(&self) -> &TrialConfig {
        &self.value
    }

    #[must_use]
    pub fn canonical_sha256(&self) -> &str {
        &self.canonical_sha256
    }

    #[must_use]
    pub fn fingerprint(&self) -> &str {
        &self.fingerprint
    }

    #[must_use]
    pub fn plan_fingerprint(&self) -> &str {
        &self.plan_fingerprint
    }

    #[must_use]
    pub fn canonical_length(&self) -> u64 {
        self.canonical_bytes.len() as u64
    }

    /// Exact no-secret place identity derived from the already validated
    /// domain and unsigned PM-T2 order. This is journal-safe public identity,
    /// never an authenticated-body commitment or mutation grant.
    #[must_use]
    pub fn exact_place_public_request_identity(&self) -> PlacePublicRequestIdentity {
        let domain = match self.value.market.domain {
            TrialDomain::Standard => PmClobDomain::Standard,
            TrialDomain::NegativeRisk => PmClobDomain::NegativeRisk,
        };
        let unsigned = self
            .value
            .validate_market_and_order()
            .expect("canonical config already validated exact unsigned order");
        derive_place_public_request_identity(domain, unsigned)
    }
}

#[derive(Debug, Clone)]
pub struct CanonicalAuthorization {
    value: TrialAuthorization,
    fingerprint: String,
}

impl CanonicalAuthorization {
    #[must_use]
    pub fn value(&self) -> &TrialAuthorization {
        &self.value
    }

    #[must_use]
    pub fn fingerprint(&self) -> &str {
        &self.fingerprint
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PlanVerification {
    pub schema_version: u32,
    pub phase: TrialPhase,
    pub config_sha256: String,
    pub config_fingerprint: String,
    pub plan_fingerprint: String,
    pub configured_profile_structurally_valid: bool,
    #[serde(flatten)]
    pub authorization: OfflineAuthorizationState,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AuthorizationVerification {
    pub schema_version: u32,
    pub phase: TrialPhase,
    pub authorization_id: String,
    pub authorization_fingerprint: String,
    pub config_fingerprint: String,
    pub exact_bindings_structurally_valid: bool,
    pub within_short_lived_window_at_verification: bool,
    /// This structural command does not inspect the separately bound durable
    /// consumption ledger and therefore makes no consumed/unconsumed claim.
    pub authorization_consumption_checked: bool,
    #[serde(flatten)]
    pub authorization: OfflineAuthorizationState,
}

pub fn load_canonical_trial_config(
    path: &Path,
) -> Result<CanonicalTrialConfig, PmTrialConfigError> {
    let bytes = read_one(path, ProtectedFileKind::Config, MAX_CANONICAL_RECORD_BYTES)
        .map_err(|_| invalid("canonical config file protection or stability check failed"))?;
    let value: TrialConfig = parse_exact_canonical(&bytes, "config")?;
    value.validate()?;
    let canonical_bytes = bytes.to_vec();
    Ok(CanonicalTrialConfig {
        canonical_sha256: hash_bytes(&[], &canonical_bytes),
        fingerprint: hash_bytes(CONFIG_FINGERPRINT_DOMAIN, &canonical_bytes),
        plan_fingerprint: hash_bytes(PLAN_FINGERPRINT_DOMAIN, &canonical_bytes),
        canonical_bytes,
        value,
    })
}

pub fn load_canonical_authorization(
    path: &Path,
) -> Result<CanonicalAuthorization, PmTrialConfigError> {
    let bytes = read_one(
        path,
        ProtectedFileKind::Authorization,
        MAX_CANONICAL_RECORD_BYTES,
    )
    .map_err(|_| invalid("authorization file protection or stability check failed"))?;
    let value: TrialAuthorization = parse_exact_canonical(&bytes, "authorization")?;
    Ok(CanonicalAuthorization {
        fingerprint: hash_bytes(AUTHORIZATION_FINGERPRINT_DOMAIN, &bytes),
        value,
    })
}

#[must_use]
pub fn verify_plan(config: &CanonicalTrialConfig) -> PlanVerification {
    PlanVerification {
        schema_version: config.value.schema_version,
        phase: config.value.phase,
        config_sha256: config.canonical_sha256.clone(),
        config_fingerprint: config.fingerprint.clone(),
        plan_fingerprint: config.plan_fingerprint.clone(),
        configured_profile_structurally_valid: true,
        authorization: OfflineAuthorizationState::DENIED,
    }
}

pub fn verify_authorization(
    config: &CanonicalTrialConfig,
    authorization: &CanonicalAuthorization,
    now: DateTime<Utc>,
) -> Result<AuthorizationVerification, PmTrialConfigError> {
    authorization.value.validate(config, now)?;
    Ok(AuthorizationVerification {
        schema_version: authorization.value.schema_version,
        phase: authorization.value.phase,
        authorization_id: authorization.value.authorization_id.clone(),
        authorization_fingerprint: authorization.fingerprint.clone(),
        config_fingerprint: config.fingerprint.clone(),
        exact_bindings_structurally_valid: true,
        within_short_lived_window_at_verification: true,
        authorization_consumption_checked: false,
        authorization: OfflineAuthorizationState::DENIED,
    })
}

#[derive(Debug, Error)]
pub enum PmTrialConfigError {
    #[error("controlled-trial record is invalid: {0}")]
    Invalid(&'static str),
}

fn invalid(message: &'static str) -> PmTrialConfigError {
    PmTrialConfigError::Invalid(message)
}

fn parse_exact_canonical<T: DeserializeOwned + Serialize>(
    bytes: &[u8],
    label: &'static str,
) -> Result<T, PmTrialConfigError> {
    let value: T = serde_json::from_slice(bytes).map_err(|_| match label {
        "config" => invalid("config JSON is malformed, duplicated, unknown, or trailing"),
        _ => invalid("authorization JSON is malformed, duplicated, unknown, or trailing"),
    })?;
    let canonical = serde_json::to_vec(&value)
        .map_err(|_| invalid("record cannot be serialized canonically"))?;
    if canonical != bytes {
        return Err(invalid("record bytes are not exact canonical compact JSON"));
    }
    Ok(value)
}

fn parse_address(value: &str) -> Result<EvmAddress, PmTrialConfigError> {
    EvmAddress::parse(value).map_err(|_| invalid("EVM address is invalid"))
}

fn parse_profile_address(value: &str) -> Result<EvmAddress, PmTrialConfigError> {
    // `EoaAddress` supplies the closed EIP-55 spelling check. Here it is used
    // only as address syntax validation; the funder remains a Proxy Wallet,
    // while only the separately named signer is asserted to be an EOA.
    EoaAddress::parse(value)
        .map(EoaAddress::as_core)
        .map_err(|_| invalid("account address is not canonical nonzero EIP-55"))
}

fn canonical_u256(value: &str, allow_zero: bool) -> Result<U256, PmTrialConfigError> {
    let parsed = U256::from_str(value).map_err(|_| invalid("base-unit amount is invalid"))?;
    if parsed.to_string() != value || (!allow_zero && parsed.is_zero()) {
        return Err(invalid(
            "base-unit amount is not canonical or is unexpectedly zero",
        ));
    }
    Ok(parsed)
}

fn validate_sha256(value: &str) -> Result<(), PmTrialConfigError> {
    validate_lower_hex(value, 64, "SHA-256 value is invalid")
}

fn validate_lower_hex32(value: &str, message: &'static str) -> Result<(), PmTrialConfigError> {
    if value.len() != 66 || !value.starts_with("0x") {
        return Err(invalid(message));
    }
    validate_lower_hex(&value[2..], 64, message)
}

fn validate_lower_hex(
    value: &str,
    length: usize,
    message: &'static str,
) -> Result<(), PmTrialConfigError> {
    if value.len() != length
        || value
            .bytes()
            .any(|byte| !byte.is_ascii_digit() && !(b'a'..=b'f').contains(&byte))
    {
        return Err(invalid(message));
    }
    Ok(())
}

fn validate_token(
    value: &str,
    maximum: usize,
    message: &'static str,
) -> Result<(), PmTrialConfigError> {
    if value.is_empty()
        || value.len() > maximum
        || value.bytes().any(|byte| {
            !(byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':' | b'/'))
        })
    {
        return Err(invalid(message));
    }
    Ok(())
}

fn validate_reference(value: &str, message: &'static str) -> Result<(), PmTrialConfigError> {
    if value.is_empty()
        || value.len() > 512
        || value.bytes().any(|byte| !(0x21..=0x7e).contains(&byte))
    {
        return Err(invalid(message));
    }
    Ok(())
}

fn validate_fee_lexeme(value: &str) -> Result<(), PmTrialConfigError> {
    let mut dot_seen = false;
    if value.is_empty()
        || value.len() > 64
        || !value.as_bytes()[0].is_ascii_digit()
        || !value.as_bytes()[value.len() - 1].is_ascii_digit()
        || serde_json::from_str::<serde_json::Number>(value).is_err()
        || value.bytes().any(|byte| {
            if byte == b'.' && !dot_seen {
                dot_seen = true;
                false
            } else {
                !byte.is_ascii_digit()
            }
        })
    {
        return Err(invalid("reviewed market fee lexeme is invalid"));
    }
    Ok(())
}

fn validate_direct_entry(value: &str, message: &'static str) -> Result<(), PmTrialConfigError> {
    if value.is_empty()
        || value.len() > 128
        || value == "."
        || value == ".."
        || value
            .bytes()
            .any(|byte| !byte.is_ascii_alphanumeric() && !matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err(invalid(message));
    }
    Ok(())
}

fn validate_absolute_path(value: &str) -> Result<(), PmTrialConfigError> {
    let path = Path::new(value);
    if !path.is_absolute()
        || value.len() > 1024
        || path
            .components()
            .any(|part| matches!(part, std::path::Component::ParentDir))
    {
        return Err(invalid("artifact directory is not one exact absolute path"));
    }
    Ok(())
}

fn parse_utc(value: &str) -> Result<DateTime<Utc>, PmTrialConfigError> {
    let parsed = DateTime::parse_from_rfc3339(value)
        .map_err(|_| invalid("authorization timestamp is invalid"))?
        .with_timezone(&Utc);
    if parsed.to_rfc3339_opts(chrono::SecondsFormat::Secs, true) != value {
        return Err(invalid(
            "authorization timestamp is not canonical UTC seconds",
        ));
    }
    Ok(parsed)
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
