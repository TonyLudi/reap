//! Exact, offline-only V2 review records for the future online Phase-A gate.
//!
//! These records are evidence sidecars. They neither observe a live source nor
//! authorize a request. In particular, reviewed market classification and
//! reviewed status history remain reviewer evidence; current status
//! announcements, fixed CLOB `GET /ok` liveness/health, and the exact
//! account's signer-authenticated closed-only state are separate source
//! observations. None of these records asserts that a global matching-engine
//! restart, restricted mode, or order-admission mode is absent.
//!
//! The V2 authorization is a distinct future authorization lineage, not a
//! wrapper around or proof of any V1 authorization. It binds the historical
//! V1 *config* only. The separate V2 consumption ledger consumes and
//! fingerprints these exact V2 authorization bytes directly. A later V2 A3
//! integration must consume that exact V2 authorization and its take-once
//! evidence; pairing this record with or converting from V1 is forbidden, as is
//! falling back to an arbitrary V1 authorization is forbidden.

use std::{
    fmt,
    net::{IpAddr, Ipv4Addr, Ipv6Addr},
    path::Path,
    str::FromStr as _,
};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use sha2::{Digest as _, Sha256};
use thiserror::Error;

use crate::{
    CanonicalTrialConfig, OfflineAuthorizationState, TrialPhase,
    protected_file::{ProtectedFileKind, read_one},
};

pub const ONLINE_POLICY_V2_SCHEMA_VERSION: u32 = 2;
pub const ONLINE_AUTHORIZATION_V2_SCHEMA_VERSION: u32 = 2;
pub const MAX_ONLINE_OBSERVATION_AGE_MS_V2: u64 = 5_000;
pub const MIN_STATUS_NOTICE_HISTORY_QUIET_INTERVAL_SECONDS_V2: u64 = 86_400;
pub const MAX_STATUS_NOTICE_HISTORY_QUIET_INTERVAL_SECONDS_V2: u64 = 7_776_000;

const MAX_CANONICAL_ONLINE_RECORD_BYTES_V2: usize = 128 * 1024;
const MAX_ONLINE_AUTHORIZATION_LIFETIME_SECONDS_V2: i64 = 15 * 60;
const ONLINE_POLICY_V2_FINGERPRINT_DOMAIN: &[u8] =
    b"reap.pm-t2.controlled-trial.online-policy.v2\0";
const ONLINE_AUTHORIZATION_V2_FINGERPRINT_DOMAIN: &[u8] =
    b"reap.pm-t2.controlled-trial.online-authorization.v2\0";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct V1ConfigPinsV2 {
    pub canonical_config_sha256: String,
    pub canonical_config_length: u64,
    pub canonical_config_fingerprint: String,
    pub trial_plan_fingerprint: String,
}

impl V1ConfigPinsV2 {
    fn validate(&self) -> Result<(), PmOnlinePolicyV2Error> {
        validate_sha256(&self.canonical_config_sha256)?;
        validate_sha256(&self.canonical_config_fingerprint)?;
        validate_sha256(&self.trial_plan_fingerprint)?;
        if self.canonical_config_length == 0 {
            return Err(invalid("V1 canonical config length must be nonzero"));
        }
        Ok(())
    }

    fn matches(&self, config: &CanonicalTrialConfig) -> bool {
        self.canonical_config_sha256 == config.canonical_sha256()
            && self.canonical_config_length == config.canonical_length()
            && self.canonical_config_fingerprint == config.fingerprint()
            && self.trial_plan_fingerprint == config.plan_fingerprint()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewedMarketClassificationV2 {
    NonSports,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewedMarketEvidenceV2 {
    pub classification: ReviewedMarketClassificationV2,
    pub review_reference: String,
    pub review_sha256: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewedMarketObservationRequirementV2 {
    RequireReviewedNonSports,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewedStatusHistoryObservationRequirementV2 {
    RequireReviewedExactClobComponentHistory,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FreshStatusAnnouncementObservationRequirementV2 {
    RequireFreshSummaryAndComponents,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClobLivenessHealthObservationRequirementV2 {
    RequireFixedClobGetOkLivenessOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SameAccountClosedOnlyObservationRequirementV2 {
    RequireSignerAuthenticatedFalseForExactAccount,
}

/// Five purpose-distinct requirements. No one field can substitute for
/// another, and none is a global order-admission assertion.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OperationalObservationProfileV2 {
    pub reviewed_market_classification: ReviewedMarketObservationRequirementV2,
    pub reviewed_official_status_notice_history: ReviewedStatusHistoryObservationRequirementV2,
    pub fresh_official_status_announcements: FreshStatusAnnouncementObservationRequirementV2,
    pub clob_ok_liveness_health: ClobLivenessHealthObservationRequirementV2,
    pub same_account_closed_only: SameAccountClosedOnlyObservationRequirementV2,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewedStatusClobComponentV2 {
    pub component_id: String,
    pub component_name: String,
}

impl ReviewedStatusClobComponentV2 {
    fn validate(&self) -> Result<(), PmOnlinePolicyV2Error> {
        validate_status_component_id(&self.component_id)?;
        // Preserve the reviewed name byte-for-byte, including legitimate
        // leading spaces present in the production component name.
        if self.component_name.is_empty()
            || self.component_name.len() > 512
            || self.component_name.chars().any(char::is_control)
        {
            return Err(invalid("reviewed CLOB component name is invalid"));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OnlinePolicyV2 {
    pub schema_version: u32,
    pub policy_id: String,
    pub issuing_reviewer: String,
    pub reviewed_at_utc: String,
    pub phase: TrialPhase,
    pub v1_config: V1ConfigPinsV2,
    pub reviewed_market: ReviewedMarketEvidenceV2,
    pub operational_observations: OperationalObservationProfileV2,
    pub reviewed_status_clob_component: ReviewedStatusClobComponentV2,
    pub maximum_observation_age_ms: u64,
    pub minimum_notice_history_quiet_interval_seconds: u64,
}

impl OnlinePolicyV2 {
    fn validate_intrinsic(&self) -> Result<(), PmOnlinePolicyV2Error> {
        if self.schema_version != ONLINE_POLICY_V2_SCHEMA_VERSION {
            return Err(invalid("unsupported online-policy V2 schema"));
        }
        validate_token(&self.policy_id, 128, "online-policy V2 ID is invalid")?;
        validate_reference(
            &self.issuing_reviewer,
            "online-policy V2 issuing reviewer is invalid",
        )?;
        let _ = parse_utc(&self.reviewed_at_utc)?;
        if self.phase != TrialPhase::APlaceCancel {
            return Err(invalid("online-policy V2 is Phase-A-only"));
        }
        self.v1_config.validate()?;
        validate_reference(
            &self.reviewed_market.review_reference,
            "reviewed market reference is invalid",
        )?;
        validate_sha256(&self.reviewed_market.review_sha256)?;
        self.reviewed_status_clob_component.validate()?;
        if self.maximum_observation_age_ms == 0
            || self.maximum_observation_age_ms > MAX_ONLINE_OBSERVATION_AGE_MS_V2
        {
            return Err(invalid(
                "online-policy V2 observation age is outside the one-to-five-second bound",
            ));
        }
        if !(MIN_STATUS_NOTICE_HISTORY_QUIET_INTERVAL_SECONDS_V2
            ..=MAX_STATUS_NOTICE_HISTORY_QUIET_INTERVAL_SECONDS_V2)
            .contains(&self.minimum_notice_history_quiet_interval_seconds)
        {
            return Err(invalid(
                "online-policy V2 notice-history quiet interval is outside the one-to-ninety-day bound",
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OnlinePolicyPinsV2 {
    pub canonical_sha256: String,
    pub canonical_length: u64,
    pub fingerprint: String,
}

impl OnlinePolicyPinsV2 {
    fn validate(&self) -> Result<(), PmOnlinePolicyV2Error> {
        validate_sha256(&self.canonical_sha256)?;
        validate_sha256(&self.fingerprint)?;
        if self.canonical_length == 0 {
            return Err(invalid("online-policy V2 canonical length must be nonzero"));
        }
        Ok(())
    }

    fn matches(&self, policy: &CanonicalOnlinePolicyV2) -> bool {
        self.canonical_sha256 == policy.canonical_sha256
            && self.canonical_length == policy.canonical_bytes.len() as u64
            && self.fingerprint == policy.fingerprint
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewedRepositoryStateV2 {
    ExactCleanCommit,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OnlineAuthorizationBuildBindingV2 {
    pub repository_commit: String,
    pub repository_state: ReviewedRepositoryStateV2,
    pub cargo_lock_sha256: String,
    pub release_binary_sha256: String,
    pub release_binary_length: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OnlineAuthorizationHostBindingV2 {
    /// Exact UTF-8 Linux UTS nodename; no DNS or `/etc/hostname` aliasing.
    pub uts_nodename: String,
    /// Exact lowercase Linux boot UUID read from the current boot namespace.
    pub boot_id: String,
    /// Exact NSS account name returned for `linux_euid`.
    pub nss_username: String,
    /// Numeric effective Linux UID. Root is forbidden for this run profile.
    pub linux_euid: u32,
    /// Reviewer-owned Linux routing and same-local-egress profile.
    ///
    /// The destination-independent NAT statement below is a reviewed
    /// tunnel/gateway assumption, not an observation that two destinations
    /// exposed one identical public address. Current source evidence may only
    /// claim the narrower `SameLocalEgressSelection` fact until a signed
    /// gateway flow attestor exists.
    pub egress: ReviewedLinuxEgressProfileV2,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewedDestinationIndependentNatV2 {
    OnePublicIpForPolymarketComAndClobPolymarketCom,
}

/// Reviewed Linux egress identity for the future current-runtime gate.
///
/// `authorized_geoblock_reported_public_ip` is the canonical global-unicast
/// public IP reported by the reviewed geoblock source. The namespace,
/// interface, and local source IP identify same-local-egress selection; the
/// dedicated tunnel/gateway profile is separate reviewer evidence. It supplies
/// the reviewed destination-independent NAT assumption.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewedLinuxEgressProfileV2 {
    pub network_namespace_device: u64,
    pub network_namespace_inode: u64,
    pub interface_name: String,
    pub interface_index: u32,
    pub local_source_ip: String,
    pub dedicated_tunnel_or_gateway_profile_reference: String,
    pub dedicated_tunnel_or_gateway_profile_sha256: String,
    pub destination_independent_nat_assumption: ReviewedDestinationIndependentNatV2,
    pub authorized_geoblock_reported_public_ip: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewedStatusNoticeHistorySourceV2 {
    OfficialPolymarketStatusHistory,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewedStatusNoticeHistoryFindingV2 {
    NoExactComponentLinkedIncidentOrMaintenanceInWindow,
}

/// Reviewer-owned cut of official history for one exact component.
///
/// Its finding is deliberately component-scoped. It is not evidence that any
/// global matching-engine restart, restricted mode, or order-admission mode is
/// absent, and it cannot replace current source observations.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewedStatusNoticeHistoryCutV2 {
    pub source_kind: ReviewedStatusNoticeHistorySourceV2,
    pub review_reference: String,
    pub review_sha256: String,
    pub history_window_start_utc: String,
    pub reviewed_through_utc: String,
    pub clob_component: ReviewedStatusClobComponentV2,
    pub finding: ReviewedStatusNoticeHistoryFindingV2,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OnlineAuthorizationPurposeV2 {
    OneExactPhaseAPlaceCancelAttempt,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OnlinePhaseScopeApprovalV2 {
    OnlyExactPhaseA,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OnlineAttemptScopeApprovalV2 {
    ExactlyOnePlaceDispatch,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OnlineFillRiskApprovalV2 {
    OnePossibleFillWithinExactV1LossCap,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OnlinePostOnlySemanticsApprovalV2 {
    MayFill,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OnlineProxyConcurrencyApprovalV2 {
    NoConcurrentProxyTrading,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OnlineCleanupApprovalV2 {
    IndependentCleanupMethodReviewed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OnlineSourceSeparationApprovalV2 {
    FiveDistinctEvidenceClassesRequired,
}

/// Closed reviewer statements. There are no caller-selected sufficiency
/// booleans and no positive mutation-authority field.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OnlineAuthorizationApprovalV2 {
    pub phase_scope: OnlinePhaseScopeApprovalV2,
    pub attempt_scope: OnlineAttemptScopeApprovalV2,
    pub fill_risk: OnlineFillRiskApprovalV2,
    pub post_only_semantics: OnlinePostOnlySemanticsApprovalV2,
    pub proxy_concurrency: OnlineProxyConcurrencyApprovalV2,
    pub cleanup: OnlineCleanupApprovalV2,
    pub source_separation: OnlineSourceSeparationApprovalV2,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OnlineAuthorizationV2 {
    pub schema_version: u32,
    pub authorization_id: String,
    pub issuing_reviewer: String,
    pub reviewed_at_utc: String,
    pub phase: TrialPhase,
    pub purpose: OnlineAuthorizationPurposeV2,
    pub not_before_utc: String,
    pub expires_at_utc: String,
    pub cleanup_not_after_utc: String,
    pub v1_config: V1ConfigPinsV2,
    pub online_policy: OnlinePolicyPinsV2,
    pub build: OnlineAuthorizationBuildBindingV2,
    pub host: OnlineAuthorizationHostBindingV2,
    pub status_notice_history: ReviewedStatusNoticeHistoryCutV2,
    pub approval: OnlineAuthorizationApprovalV2,
}

impl OnlineAuthorizationV2 {
    fn validate_intrinsic(
        &self,
    ) -> Result<ValidatedOnlineAuthorizationTimesV2, PmOnlinePolicyV2Error> {
        if self.schema_version != ONLINE_AUTHORIZATION_V2_SCHEMA_VERSION {
            return Err(invalid("unsupported online-authorization V2 schema"));
        }
        validate_token(
            &self.authorization_id,
            128,
            "online-authorization V2 ID is invalid",
        )?;
        validate_reference(
            &self.issuing_reviewer,
            "online-authorization V2 issuing reviewer is invalid",
        )?;
        if self.phase != TrialPhase::APlaceCancel {
            return Err(invalid("online-authorization V2 is Phase-A-only"));
        }
        self.v1_config.validate()?;
        self.online_policy.validate()?;
        self.validate_build()?;
        self.validate_host()?;
        self.validate_status_history()?;

        let reviewed_at = parse_utc(&self.reviewed_at_utc)?;
        let not_before = parse_utc(&self.not_before_utc)?;
        let expires_at = parse_utc(&self.expires_at_utc)?;
        let cleanup_not_after = parse_utc(&self.cleanup_not_after_utc)?;
        let history_window_start = parse_utc(&self.status_notice_history.history_window_start_utc)?;
        let history_reviewed_through = parse_utc(&self.status_notice_history.reviewed_through_utc)?;
        let lifetime_seconds = expires_at
            .timestamp()
            .checked_sub(not_before.timestamp())
            .ok_or_else(|| invalid("online-authorization V2 time arithmetic overflowed"))?;
        if reviewed_at > not_before
            || lifetime_seconds <= 0
            || lifetime_seconds > MAX_ONLINE_AUTHORIZATION_LIFETIME_SECONDS_V2
            || cleanup_not_after < expires_at
            || history_reviewed_through != reviewed_at
            || history_window_start >= history_reviewed_through
        {
            return Err(invalid("online-authorization V2 time envelope is invalid"));
        }

        Ok(ValidatedOnlineAuthorizationTimesV2 {
            reviewed_at,
            not_before,
            expires_at,
            cleanup_not_after,
            history_window_start,
            history_reviewed_through,
        })
    }

    fn validate_build(&self) -> Result<(), PmOnlinePolicyV2Error> {
        validate_lower_hex(
            &self.build.repository_commit,
            40,
            "online-authorization repository commit is invalid",
        )?;
        validate_sha256(&self.build.cargo_lock_sha256)?;
        validate_sha256(&self.build.release_binary_sha256)?;
        if self.build.release_binary_length == 0 {
            return Err(invalid(
                "online-authorization release binary length must be nonzero",
            ));
        }
        Ok(())
    }

    fn validate_host(&self) -> Result<(), PmOnlinePolicyV2Error> {
        validate_uts_nodename(&self.host.uts_nodename)?;
        validate_boot_id(&self.host.boot_id)?;
        validate_nss_username(&self.host.nss_username)?;
        if self.host.linux_euid == 0 || self.host.linux_euid == u32::MAX {
            return Err(invalid(
                "online-authorization Linux EUID is invalid for the controlled-run profile",
            ));
        }
        let egress = &self.host.egress;
        if egress.network_namespace_device == 0 || egress.network_namespace_inode == 0 {
            return Err(invalid(
                "online-authorization network namespace identity is invalid",
            ));
        }
        if egress.interface_name.is_empty()
            || egress.interface_name.len() > 15
            || egress
                .interface_name
                .bytes()
                .any(|byte| !byte.is_ascii_alphanumeric() && !matches!(byte, b'_' | b'.' | b'-'))
        {
            return Err(invalid(
                "online-authorization Linux egress interface name is invalid",
            ));
        }
        if egress.interface_index == 0 || egress.interface_index > 2_147_483_647 {
            return Err(invalid(
                "online-authorization Linux egress interface index is invalid",
            ));
        }
        validate_egress_ip(
            &egress.local_source_ip,
            "online-authorization local source IP is invalid",
        )?;
        validate_reference(
            &egress.dedicated_tunnel_or_gateway_profile_reference,
            "online-authorization tunnel or gateway profile reference is invalid",
        )?;
        validate_sha256(&egress.dedicated_tunnel_or_gateway_profile_sha256)?;
        validate_public_egress_ip(
            &egress.authorized_geoblock_reported_public_ip,
            "online-authorization geoblock-reported public IP is invalid",
        )?;
        Ok(())
    }

    fn validate_status_history(&self) -> Result<(), PmOnlinePolicyV2Error> {
        validate_reference(
            &self.status_notice_history.review_reference,
            "reviewed status-history reference is invalid",
        )?;
        validate_sha256(&self.status_notice_history.review_sha256)?;
        self.status_notice_history.clob_component.validate()
    }
}

pub(crate) struct ValidatedOnlineAuthorizationTimesV2 {
    pub(crate) reviewed_at: DateTime<Utc>,
    pub(crate) not_before: DateTime<Utc>,
    pub(crate) expires_at: DateTime<Utc>,
    pub(crate) cleanup_not_after: DateTime<Utc>,
    pub(crate) history_window_start: DateTime<Utc>,
    pub(crate) history_reviewed_through: DateTime<Utc>,
}

/// Move-only, redacted holder of exact canonical online-policy V2 bytes.
pub struct CanonicalOnlinePolicyV2 {
    value: OnlinePolicyV2,
    canonical_bytes: Vec<u8>,
    canonical_sha256: String,
    fingerprint: String,
}

impl CanonicalOnlinePolicyV2 {
    #[must_use]
    pub const fn value(&self) -> &OnlinePolicyV2 {
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
    pub fn canonical_length(&self) -> u64 {
        self.canonical_bytes.len() as u64
    }
}

impl fmt::Debug for CanonicalOnlinePolicyV2 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("CanonicalOnlinePolicyV2(<reviewed-evidence; exact-canonical-bytes>)")
    }
}

/// Move-only, redacted holder of exact canonical online-authorization V2
/// bytes. Structural verification still returns denied mutation authority.
pub struct CanonicalOnlineAuthorizationV2 {
    value: OnlineAuthorizationV2,
    canonical_bytes: Vec<u8>,
    canonical_sha256: String,
    fingerprint: String,
}

impl CanonicalOnlineAuthorizationV2 {
    #[must_use]
    pub const fn value(&self) -> &OnlineAuthorizationV2 {
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
    pub fn canonical_length(&self) -> u64 {
        self.canonical_bytes.len() as u64
    }
}

impl fmt::Debug for CanonicalOnlineAuthorizationV2 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .write_str("CanonicalOnlineAuthorizationV2(<reviewed-evidence; exact-canonical-bytes>)")
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OnlinePolicyVerificationV2 {
    pub schema_version: u32,
    pub phase: TrialPhase,
    pub config_fingerprint: String,
    pub online_policy_fingerprint: String,
    pub exact_v1_config_binding_structurally_valid: bool,
    pub five_source_profile_structurally_valid: bool,
    #[serde(flatten)]
    pub authorization: OfflineAuthorizationState,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OnlineAuthorizationVerificationV2 {
    pub schema_version: u32,
    pub phase: TrialPhase,
    pub authorization_id: String,
    pub config_fingerprint: String,
    pub online_policy_fingerprint: String,
    pub online_authorization_fingerprint: String,
    pub exact_bindings_structurally_valid: bool,
    /// Offline display of a caller-supplied clock comparison only. This is not
    /// live freshness and must never be accepted by consumption, A3, a permit,
    /// or transport.
    pub within_short_lived_window_at_verification: bool,
    pub component_scoped_history_window_structurally_valid: bool,
    /// Durable consumption is a separate gate that this display does not
    /// inspect.
    pub authorization_consumption_checked: bool,
    #[serde(flatten)]
    pub authorization: OfflineAuthorizationState,
}

pub fn load_canonical_online_policy_v2(
    path: &Path,
) -> Result<CanonicalOnlinePolicyV2, PmOnlinePolicyV2Error> {
    let bytes = read_one(
        path,
        ProtectedFileKind::OnlinePolicyV2,
        MAX_CANONICAL_ONLINE_RECORD_BYTES_V2,
    )
    .map_err(|_| invalid("online-policy V2 protection or stability check failed"))?;
    let value: OnlinePolicyV2 = parse_exact_canonical(&bytes, "online-policy V2")?;
    value.validate_intrinsic()?;
    let canonical_bytes = bytes.to_vec();
    Ok(CanonicalOnlinePolicyV2 {
        canonical_sha256: hash_bytes(&[], &canonical_bytes),
        fingerprint: hash_bytes(ONLINE_POLICY_V2_FINGERPRINT_DOMAIN, &canonical_bytes),
        canonical_bytes,
        value,
    })
}

pub fn load_canonical_online_authorization_v2(
    path: &Path,
) -> Result<CanonicalOnlineAuthorizationV2, PmOnlinePolicyV2Error> {
    let bytes = read_one(
        path,
        ProtectedFileKind::OnlineAuthorizationV2,
        MAX_CANONICAL_ONLINE_RECORD_BYTES_V2,
    )
    .map_err(|_| invalid("online-authorization V2 protection or stability check failed"))?;
    let value: OnlineAuthorizationV2 = parse_exact_canonical(&bytes, "online-authorization V2")?;
    let _ = value.validate_intrinsic()?;
    let canonical_bytes = bytes.to_vec();
    Ok(CanonicalOnlineAuthorizationV2 {
        canonical_sha256: hash_bytes(&[], &canonical_bytes),
        fingerprint: hash_bytes(ONLINE_AUTHORIZATION_V2_FINGERPRINT_DOMAIN, &canonical_bytes),
        canonical_bytes,
        value,
    })
}

pub fn verify_online_policy_v2(
    config: &CanonicalTrialConfig,
    policy: &CanonicalOnlinePolicyV2,
) -> Result<OnlinePolicyVerificationV2, PmOnlinePolicyV2Error> {
    policy.value.validate_intrinsic()?;
    if config.value().phase != TrialPhase::APlaceCancel
        || policy.value.phase != TrialPhase::APlaceCancel
        || !policy.value.v1_config.matches(config)
    {
        return Err(invalid(
            "online-policy V2 does not bind the exact canonical Phase-A V1 config",
        ));
    }
    if policy.value.maximum_observation_age_ms
        > config
            .value()
            .time_limits
            .maximum_preflight_observation_age_ms
    {
        return Err(invalid(
            "online-policy V2 observation age exceeds the exact V1 config cap",
        ));
    }
    Ok(OnlinePolicyVerificationV2 {
        schema_version: policy.value.schema_version,
        phase: policy.value.phase,
        config_fingerprint: config.fingerprint().to_owned(),
        online_policy_fingerprint: policy.fingerprint.clone(),
        exact_v1_config_binding_structurally_valid: true,
        five_source_profile_structurally_valid: true,
        authorization: OfflineAuthorizationState::DENIED,
    })
}

/// Perform an offline reviewer/CLI structural display check.
///
/// `now` is caller supplied. This check cannot establish live freshness or
/// satisfy consumption/A3/permit. The eventual live gate must use a
/// source-owned current-runtime witness and consume and fingerprint the exact
/// canonical V2 authorization directly.
pub fn verify_online_authorization_v2(
    config: &CanonicalTrialConfig,
    policy: &CanonicalOnlinePolicyV2,
    authorization: &CanonicalOnlineAuthorizationV2,
    now: DateTime<Utc>,
) -> Result<OnlineAuthorizationVerificationV2, PmOnlinePolicyV2Error> {
    let times = validate_online_authorization_contract_v2(config, policy, authorization)?;
    if now < times.not_before || now >= times.expires_at {
        return Err(invalid(
            "online-authorization V2 is early or expired at verification time",
        ));
    }

    Ok(OnlineAuthorizationVerificationV2 {
        schema_version: authorization.value.schema_version,
        phase: authorization.value.phase,
        authorization_id: authorization.value.authorization_id.clone(),
        config_fingerprint: config.fingerprint().to_owned(),
        online_policy_fingerprint: policy.fingerprint.clone(),
        online_authorization_fingerprint: authorization.fingerprint.clone(),
        exact_bindings_structurally_valid: true,
        within_short_lived_window_at_verification: true,
        component_scoped_history_window_structurally_valid: true,
        authorization_consumption_checked: false,
        authorization: OfflineAuthorizationState::DENIED,
    })
}

/// Validate the immutable V2 reviewer contract without consulting a caller
/// clock. The V2 consumption ledger uses this helper and then checks its own
/// source-owned runtime observation. It must not call the public offline
/// `verify_online_authorization_v2(..., now)` display check.
pub(crate) fn validate_online_authorization_contract_v2(
    config: &CanonicalTrialConfig,
    policy: &CanonicalOnlinePolicyV2,
    authorization: &CanonicalOnlineAuthorizationV2,
) -> Result<ValidatedOnlineAuthorizationTimesV2, PmOnlinePolicyV2Error> {
    let _ = verify_online_policy_v2(config, policy)?;
    let times = authorization.value.validate_intrinsic()?;
    if authorization.value.phase != TrialPhase::APlaceCancel
        || authorization.value.v1_config != policy.value.v1_config
        || !authorization.value.v1_config.matches(config)
        || !authorization.value.online_policy.matches(policy)
    {
        return Err(invalid(
            "online-authorization V2 exact config or policy binding mismatched",
        ));
    }
    let policy_reviewed_at = parse_utc(&policy.value.reviewed_at_utc)?;
    if policy_reviewed_at > times.reviewed_at || times.reviewed_at > times.not_before {
        return Err(invalid(
            "online-authorization V2 review ordering is invalid",
        ));
    }
    if authorization.value.status_notice_history.clob_component
        != policy.value.reviewed_status_clob_component
    {
        return Err(invalid(
            "online-authorization V2 status component differs from reviewed policy",
        ));
    }
    let reviewed_history_seconds = times
        .history_reviewed_through
        .timestamp()
        .checked_sub(times.history_window_start.timestamp())
        .ok_or_else(|| invalid("status notice-history window arithmetic overflowed"))?;
    let required_history_seconds =
        i64::try_from(policy.value.minimum_notice_history_quiet_interval_seconds)
            .map_err(|_| invalid("status notice-history quiet interval is invalid"))?;
    if reviewed_history_seconds < required_history_seconds {
        return Err(invalid(
            "reviewed status notice history does not cover the configured quiet interval",
        ));
    }
    Ok(times)
}

#[derive(Debug, Error)]
pub enum PmOnlinePolicyV2Error {
    #[error("controlled-trial online V2 record is invalid: {0}")]
    Invalid(&'static str),
}

fn invalid(message: &'static str) -> PmOnlinePolicyV2Error {
    PmOnlinePolicyV2Error::Invalid(message)
}

fn parse_exact_canonical<T: DeserializeOwned + Serialize>(
    bytes: &[u8],
    label: &'static str,
) -> Result<T, PmOnlinePolicyV2Error> {
    let value: T = serde_json::from_slice(bytes)
        .map_err(|_| invalid("online V2 JSON is malformed, duplicated, unknown, or trailing"))?;
    let canonical = serde_json::to_vec(&value)
        .map_err(|_| invalid("online V2 record cannot be serialized canonically"))?;
    if canonical != bytes {
        return Err(match label {
            "online-policy V2" => {
                invalid("online-policy V2 bytes are not exact canonical compact JSON")
            }
            _ => invalid("online-authorization V2 bytes are not exact canonical compact JSON"),
        });
    }
    Ok(value)
}

fn validate_sha256(value: &str) -> Result<(), PmOnlinePolicyV2Error> {
    validate_lower_hex(value, 64, "online V2 SHA-256 value is invalid")
}

fn validate_lower_hex(
    value: &str,
    length: usize,
    message: &'static str,
) -> Result<(), PmOnlinePolicyV2Error> {
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
) -> Result<(), PmOnlinePolicyV2Error> {
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

fn validate_reference(value: &str, message: &'static str) -> Result<(), PmOnlinePolicyV2Error> {
    if value.is_empty()
        || value.len() > 512
        || value.bytes().any(|byte| !(0x21..=0x7e).contains(&byte))
    {
        return Err(invalid(message));
    }
    Ok(())
}

fn validate_boot_id(value: &str) -> Result<(), PmOnlinePolicyV2Error> {
    if value.len() != 36
        || value.bytes().enumerate().any(|(index, byte)| {
            if matches!(index, 8 | 13 | 18 | 23) {
                byte != b'-'
            } else {
                !byte.is_ascii_digit() && !(b'a'..=b'f').contains(&byte)
            }
        })
    {
        return Err(invalid("online-authorization Linux boot ID is invalid"));
    }
    Ok(())
}

fn validate_nss_username(value: &str) -> Result<(), PmOnlinePolicyV2Error> {
    if value.is_empty()
        || value.len() > 128
        || value
            .bytes()
            .any(|byte| !byte.is_ascii_alphanumeric() && !matches!(byte, b'-' | b'_' | b'.' | b'/'))
    {
        return Err(invalid("online-authorization NSS username is invalid"));
    }
    Ok(())
}

fn validate_uts_nodename(value: &str) -> Result<(), PmOnlinePolicyV2Error> {
    if value.is_empty()
        || value.len() > 64
        || value.trim() != value
        || value.chars().any(char::is_control)
    {
        return Err(invalid("online-authorization UTS nodename is invalid"));
    }
    Ok(())
}

fn validate_status_component_id(value: &str) -> Result<(), PmOnlinePolicyV2Error> {
    if value.is_empty()
        || value.len() > 128
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
    {
        return Err(invalid("reviewed status component ID is invalid"));
    }
    Ok(())
}

fn validate_egress_ip(value: &str, message: &'static str) -> Result<(), PmOnlinePolicyV2Error> {
    let parsed = IpAddr::from_str(value).map_err(|_| invalid(message))?;
    if parsed.to_string() != value
        || parsed.is_unspecified()
        || parsed.is_loopback()
        || parsed.is_multicast()
    {
        return Err(invalid(message));
    }
    Ok(())
}

/// Require a canonical, globally routable unicast address for the reviewed
/// geoblock-reported identity. This predicate is deliberately explicit and
/// conservative instead of inheriting toolchain-dependent `is_global`
/// semantics. Tunnel-local `local_source_ip` uses the distinct, less
/// restrictive validator above and may remain private.
fn validate_public_egress_ip(
    value: &str,
    message: &'static str,
) -> Result<(), PmOnlinePolicyV2Error> {
    let parsed = IpAddr::from_str(value).map_err(|_| invalid(message))?;
    if parsed.to_string() != value || !is_public_global_unicast(parsed) {
        return Err(invalid(message));
    }
    Ok(())
}

fn is_public_global_unicast(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(address) => is_public_global_unicast_v4(address),
        IpAddr::V6(address) => is_public_global_unicast_v6(address),
    }
}

fn is_public_global_unicast_v4(address: Ipv4Addr) -> bool {
    let [a, b, c, _] = address.octets();
    !(a == 0
        || a == 10
        || (a == 100 && (64..=127).contains(&b))
        || a == 127
        || (a == 169 && b == 254)
        || (a == 172 && (16..=31).contains(&b))
        || (a == 192 && b == 0 && c == 0)
        || (a == 192 && b == 0 && c == 2)
        || (a == 192 && b == 88 && c == 99)
        || (a == 192 && b == 168)
        || (a == 198 && matches!(b, 18 | 19))
        || (a == 198 && b == 51 && c == 100)
        || (a == 203 && b == 0 && c == 113)
        || a >= 224)
}

fn is_public_global_unicast_v6(address: Ipv6Addr) -> bool {
    let segments = address.segments();
    // Ordinary IPv6 global unicast is 2000::/3. Conservatively exclude the
    // IETF special-purpose 2001:0000::/23 block, both documentation ranges,
    // and deprecated 6to4 even though they sit within that aggregate.
    segments[0] & 0xe000 == 0x2000
        && !(segments[0] == 0x2001 && segments[1] <= 0x01ff)
        && !(segments[0] == 0x2001 && segments[1] == 0x0db8)
        && segments[0] != 0x2002
        && !(segments[0] == 0x3fff && segments[1] & 0xf000 == 0)
}

fn parse_utc(value: &str) -> Result<DateTime<Utc>, PmOnlinePolicyV2Error> {
    let parsed = DateTime::parse_from_rfc3339(value)
        .map_err(|_| invalid("online V2 timestamp is invalid"))?
        .with_timezone(&Utc);
    if parsed.to_rfc3339_opts(chrono::SecondsFormat::Secs, true) != value {
        return Err(invalid("online V2 timestamp is not canonical UTC seconds"));
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
