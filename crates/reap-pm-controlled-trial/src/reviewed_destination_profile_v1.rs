//! Exact, offline-only reviewed production destination profile V1.
//!
//! This additive sidecar binds six exact TLS names to one reviewer-captured
//! peer IP each. Loading or verifying it performs no DNS lookup and proves no
//! live DNS answer, DNSSEC result, TTL, authoritative resolver path, route,
//! connected socket, remote peer, or network-namespace fact. The captured
//! peers are reviewer evidence only. A later source-owned actor must connect
//! directly to the one reviewed peer for a role while preserving the exact
//! TLS server name and HTTP Host, and must not fall back to DNS or another
//! address.
//!
//! The profile makes no destination-independent NAT or common-public-IP
//! claim. In particular, the narrower two-host NAT assumption in the frozen
//! online-authorization V2 record does not expand to these six destinations.
//! Its validity window neither resets nor extends the V2 status-history quiet
//! window. This reusable sidecar is not consumed by the frozen V2 ledger and
//! grants no mutation authority; a later live integration needs an additive,
//! versioned conjunction that binds this exact profile fingerprint.

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
    CanonicalOnlineAuthorizationV2, CanonicalOnlinePolicyV2, CanonicalTrialConfig,
    OfflineAuthorizationState, OnlinePolicyPinsV2, V1ConfigPinsV2,
    online_policy_v2::validate_online_authorization_contract_v2,
    protected_file::{ProtectedFileKind, read_one},
};

pub const REVIEWED_PRODUCTION_DESTINATION_PROFILE_V1_SCHEMA_VERSION: u32 = 1;
pub const MAX_REVIEWED_DNS_ANSWER_AGE_SECONDS_V1: i64 = 300;
pub const PM_T2_REVIEWED_PRODUCTION_DESTINATION_PROFILE_FILE_V1: &str =
    "pm-t2-reviewed-production-destinations-v1.json";

pub const REVIEWED_GEOBLOCK_HTTPS_HOST_V1: &str = "polymarket.com";
pub const REVIEWED_CLOB_HTTPS_HOST_V1: &str = "clob.polymarket.com";
pub const REVIEWED_STATUS_HTTPS_HOST_V1: &str = "status.polymarket.com";
pub const REVIEWED_DATA_API_HTTPS_HOST_V1: &str = "data-api.polymarket.com";
pub const REVIEWED_POLYGON_RPC_HTTPS_HOST_V1: &str = "polygon.drpc.org";
pub const REVIEWED_CLOB_WEBSOCKET_WSS_HOST_V1: &str = "ws-subscriptions-clob.polymarket.com";
pub const REVIEWED_CLOB_WEBSOCKET_PUBLIC_PATH_V1: &str = "/ws/market";
pub const REVIEWED_CLOB_WEBSOCKET_USER_PATH_V1: &str = "/ws/user";

const REVIEWED_TLS_PORT_V1: u16 = 443;
const MAX_CANONICAL_REVIEWED_DESTINATION_PROFILE_BYTES_V1: usize = 64 * 1024;
const REVIEWED_PRODUCTION_DESTINATION_PROFILE_V1_FINGERPRINT_DOMAIN: &[u8] =
    b"reap.pm-t2.controlled-trial.reviewed-production-destinations.v1\0";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewedOnlineAuthorizationPinsV1 {
    pub authorization_id: String,
    pub canonical_sha256: String,
    pub canonical_length: u64,
    pub fingerprint: String,
}

impl ReviewedOnlineAuthorizationPinsV1 {
    fn validate(&self) -> Result<(), PmReviewedProductionDestinationProfileV1Error> {
        validate_token(
            &self.authorization_id,
            128,
            "reviewed destination online-authorization ID is invalid",
        )?;
        validate_sha256(&self.canonical_sha256)?;
        validate_sha256(&self.fingerprint)?;
        if self.canonical_length == 0 {
            return Err(invalid(
                "reviewed destination online-authorization canonical length must be nonzero",
            ));
        }
        Ok(())
    }

    fn matches(&self, authorization: &CanonicalOnlineAuthorizationV2) -> bool {
        self.authorization_id == authorization.value().authorization_id
            && self.canonical_sha256 == authorization.canonical_sha256()
            && self.canonical_length == authorization.canonical_length()
            && self.fingerprint == authorization.fingerprint()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewedDnsAnswerSourceV1 {
    ReviewerCapturedFixedAnswers,
}

/// Reviewer-owned evidence for the six scalar peer selections below.
///
/// `resolved_at_utc` is when the reviewer captured the answers. It is not a
/// runtime lookup timestamp, and `review_sha256` is not DNSSEC validation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewedDnsAnswerEvidenceV1 {
    pub source_kind: ReviewedDnsAnswerSourceV1,
    pub resolved_at_utc: String,
    pub review_reference: String,
    pub review_sha256: String,
}

/// One exact HTTPS TLS identity and exactly one reviewed peer IP.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewedFixedTlsDestinationV1 {
    pub dns_name: String,
    pub tcp_port: u16,
    pub tls_server_name: String,
    pub http_host: String,
    pub peer_ip: String,
}

/// One exact WSS TLS identity, its two fixed paths, and exactly one peer IP.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewedFixedWebSocketDestinationV1 {
    pub dns_name: String,
    pub tcp_port: u16,
    pub tls_server_name: String,
    pub http_host: String,
    pub peer_ip: String,
    pub public_path: String,
    pub user_path: String,
}

/// Closed six-role map. Scalar `peer_ip` fields deliberately prevent an
/// address list, resolver callback, or implicit alternate-address policy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewedProductionDestinationsV1 {
    pub geoblock_https: ReviewedFixedTlsDestinationV1,
    pub clob_https: ReviewedFixedTlsDestinationV1,
    pub status_https: ReviewedFixedTlsDestinationV1,
    pub data_api_https: ReviewedFixedTlsDestinationV1,
    pub polygon_rpc_https: ReviewedFixedTlsDestinationV1,
    pub clob_websocket_wss: ReviewedFixedWebSocketDestinationV1,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewedProductionDestinationProfileV1 {
    pub schema_version: u32,
    pub profile_id: String,
    pub issuing_reviewer: String,
    pub reviewed_at_utc: String,
    pub valid_not_before_utc: String,
    pub valid_not_after_utc: String,
    pub v1_config: V1ConfigPinsV2,
    pub online_policy: OnlinePolicyPinsV2,
    pub online_authorization: ReviewedOnlineAuthorizationPinsV1,
    pub dns_review: ReviewedDnsAnswerEvidenceV1,
    pub destinations: ReviewedProductionDestinationsV1,
}

struct ValidatedProfileTimesV1 {
    reviewed_at: DateTime<Utc>,
    valid_not_before: DateTime<Utc>,
    valid_not_after: DateTime<Utc>,
    dns_resolved_at: DateTime<Utc>,
}

impl ReviewedProductionDestinationProfileV1 {
    fn validate_intrinsic(
        &self,
    ) -> Result<ValidatedProfileTimesV1, PmReviewedProductionDestinationProfileV1Error> {
        if self.schema_version != REVIEWED_PRODUCTION_DESTINATION_PROFILE_V1_SCHEMA_VERSION {
            return Err(invalid(
                "unsupported reviewed production destination profile V1 schema",
            ));
        }
        validate_token(
            &self.profile_id,
            128,
            "reviewed production destination profile ID is invalid",
        )?;
        validate_reference(
            &self.issuing_reviewer,
            "reviewed production destination issuing reviewer is invalid",
        )?;
        validate_v1_config_pins(&self.v1_config)?;
        validate_online_policy_pins(&self.online_policy)?;
        self.online_authorization.validate()?;
        validate_reference(
            &self.dns_review.review_reference,
            "reviewed DNS answer reference is invalid",
        )?;
        validate_sha256(&self.dns_review.review_sha256)?;

        let reviewed_at = parse_utc(&self.reviewed_at_utc)?;
        let valid_not_before = parse_utc(&self.valid_not_before_utc)?;
        let valid_not_after = parse_utc(&self.valid_not_after_utc)?;
        let dns_resolved_at = parse_utc(&self.dns_review.resolved_at_utc)?;
        if dns_resolved_at > reviewed_at
            || reviewed_at > valid_not_before
            || valid_not_before >= valid_not_after
        {
            return Err(invalid(
                "reviewed production destination profile time envelope is invalid",
            ));
        }

        self.destinations.validate()?;
        Ok(ValidatedProfileTimesV1 {
            reviewed_at,
            valid_not_before,
            valid_not_after,
            dns_resolved_at,
        })
    }
}

impl ReviewedProductionDestinationsV1 {
    fn validate(&self) -> Result<(), PmReviewedProductionDestinationProfileV1Error> {
        validate_tls_destination(&self.geoblock_https, REVIEWED_GEOBLOCK_HTTPS_HOST_V1)?;
        validate_tls_destination(&self.clob_https, REVIEWED_CLOB_HTTPS_HOST_V1)?;
        validate_tls_destination(&self.status_https, REVIEWED_STATUS_HTTPS_HOST_V1)?;
        validate_tls_destination(&self.data_api_https, REVIEWED_DATA_API_HTTPS_HOST_V1)?;
        validate_tls_destination(&self.polygon_rpc_https, REVIEWED_POLYGON_RPC_HTTPS_HOST_V1)?;
        validate_websocket_destination(&self.clob_websocket_wss)?;
        Ok(())
    }

    fn peer_ips(&self) -> Result<[IpAddr; 6], PmReviewedProductionDestinationProfileV1Error> {
        Ok([
            parse_peer_ip(&self.geoblock_https.peer_ip)?,
            parse_peer_ip(&self.clob_https.peer_ip)?,
            parse_peer_ip(&self.status_https.peer_ip)?,
            parse_peer_ip(&self.data_api_https.peer_ip)?,
            parse_peer_ip(&self.polygon_rpc_https.peer_ip)?,
            parse_peer_ip(&self.clob_websocket_wss.peer_ip)?,
        ])
    }
}

/// Move-only, redacted holder of the exact protected canonical sidecar bytes.
pub struct CanonicalReviewedProductionDestinationProfileV1 {
    value: ReviewedProductionDestinationProfileV1,
    canonical_bytes: Vec<u8>,
    canonical_sha256: String,
    fingerprint: String,
}

impl CanonicalReviewedProductionDestinationProfileV1 {
    #[must_use]
    pub const fn value(&self) -> &ReviewedProductionDestinationProfileV1 {
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

impl fmt::Debug for CanonicalReviewedProductionDestinationProfileV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(
            "CanonicalReviewedProductionDestinationProfileV1(<reviewed-evidence; exact-canonical-bytes>)",
        )
    }
}

/// Offline structural display only. All production mutation authority remains
/// DENIED, and no live lookup, NAT equivalence, consumption, or actor custody
/// is inferred from this result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ReviewedProductionDestinationProfileVerificationV1 {
    pub schema_version: u32,
    pub profile_id: String,
    pub config_fingerprint: String,
    pub online_policy_fingerprint: String,
    pub online_authorization_fingerprint: String,
    pub reviewed_destination_profile_fingerprint: String,
    pub exact_v2_bindings_structurally_valid: bool,
    pub fixed_six_destination_profile_structurally_valid: bool,
    pub live_dns_observation_checked: bool,
    pub destination_nat_equivalence_checked: bool,
    pub authorization_consumption_checked: bool,
    #[serde(flatten)]
    pub authorization: OfflineAuthorizationState,
}

pub fn load_canonical_reviewed_production_destination_profile_v1(
    path: &Path,
) -> Result<
    CanonicalReviewedProductionDestinationProfileV1,
    PmReviewedProductionDestinationProfileV1Error,
> {
    let bytes = read_one(
        path,
        ProtectedFileKind::ReviewedProductionDestinationProfileV1,
        MAX_CANONICAL_REVIEWED_DESTINATION_PROFILE_BYTES_V1,
    )
    .map_err(|_| {
        invalid("reviewed production destination profile protection or stability check failed")
    })?;
    let value: ReviewedProductionDestinationProfileV1 = parse_exact_canonical(&bytes)?;
    let _ = value.validate_intrinsic()?;
    let canonical_bytes = bytes.to_vec();
    Ok(CanonicalReviewedProductionDestinationProfileV1 {
        canonical_sha256: hash_bytes(&[], &canonical_bytes),
        fingerprint: hash_bytes(
            REVIEWED_PRODUCTION_DESTINATION_PROFILE_V1_FINGERPRINT_DOMAIN,
            &canonical_bytes,
        ),
        canonical_bytes,
        value,
    })
}

/// Verify exact immutable bindings without consulting a caller clock.
///
/// The eventual actor must enforce the source-owned current time against this
/// profile and the authorization on every transition/reconnect. This check
/// intentionally does not turn a caller-supplied clock into freshness proof.
pub fn verify_reviewed_production_destination_profile_v1(
    config: &CanonicalTrialConfig,
    policy: &CanonicalOnlinePolicyV2,
    authorization: &CanonicalOnlineAuthorizationV2,
    profile: &CanonicalReviewedProductionDestinationProfileV1,
) -> Result<
    ReviewedProductionDestinationProfileVerificationV1,
    PmReviewedProductionDestinationProfileV1Error,
> {
    let authorization_times =
        validate_online_authorization_contract_v2(config, policy, authorization)
            .map_err(|_| invalid("reviewed destination bound online V2 contract is invalid"))?;
    let profile_times = profile.value.validate_intrinsic()?;

    if !v1_config_pins_match(&profile.value.v1_config, config)
        || !online_policy_pins_match(&profile.value.online_policy, policy)
        || !profile.value.online_authorization.matches(authorization)
    {
        return Err(invalid(
            "reviewed destination exact config, policy, or authorization binding mismatched",
        ));
    }
    if profile_times.reviewed_at < authorization_times.reviewed_at
        || profile_times.reviewed_at > authorization_times.not_before
        || profile_times.valid_not_before != authorization_times.not_before
        || profile_times.valid_not_after != authorization_times.cleanup_not_after
    {
        return Err(invalid(
            "reviewed destination validity does not match the bound authorization envelope",
        ));
    }
    let dns_answer_age_seconds = authorization_times
        .not_before
        .timestamp()
        .checked_sub(profile_times.dns_resolved_at.timestamp())
        .ok_or_else(|| invalid("reviewed DNS answer age arithmetic overflowed"))?;
    if !(0..=MAX_REVIEWED_DNS_ANSWER_AGE_SECONDS_V1).contains(&dns_answer_age_seconds) {
        return Err(invalid(
            "reviewed DNS answers are older than 300 seconds at authorization start",
        ));
    }

    let local_source_ip = IpAddr::from_str(&authorization.value().host.egress.local_source_ip)
        .map_err(|_| invalid("bound authorization local source IP is invalid"))?;
    if profile
        .value
        .destinations
        .peer_ips()?
        .into_iter()
        .any(|peer_ip| !same_address_family(local_source_ip, peer_ip))
    {
        return Err(invalid(
            "reviewed destination peer family differs from the authorized local source family",
        ));
    }

    Ok(ReviewedProductionDestinationProfileVerificationV1 {
        schema_version: profile.value.schema_version,
        profile_id: profile.value.profile_id.clone(),
        config_fingerprint: config.fingerprint().to_owned(),
        online_policy_fingerprint: policy.fingerprint().to_owned(),
        online_authorization_fingerprint: authorization.fingerprint().to_owned(),
        reviewed_destination_profile_fingerprint: profile.fingerprint.clone(),
        exact_v2_bindings_structurally_valid: true,
        fixed_six_destination_profile_structurally_valid: true,
        live_dns_observation_checked: false,
        destination_nat_equivalence_checked: false,
        authorization_consumption_checked: false,
        authorization: OfflineAuthorizationState::DENIED,
    })
}

#[derive(Debug, Error)]
pub enum PmReviewedProductionDestinationProfileV1Error {
    #[error("controlled-trial reviewed production destination profile V1 is invalid: {0}")]
    Invalid(&'static str),
}

fn invalid(message: &'static str) -> PmReviewedProductionDestinationProfileV1Error {
    PmReviewedProductionDestinationProfileV1Error::Invalid(message)
}

fn parse_exact_canonical<T: DeserializeOwned + Serialize>(
    bytes: &[u8],
) -> Result<T, PmReviewedProductionDestinationProfileV1Error> {
    let value: T = serde_json::from_slice(bytes).map_err(|_| {
        invalid(
            "reviewed production destination JSON is malformed, duplicated, unknown, or trailing",
        )
    })?;
    let canonical = serde_json::to_vec(&value).map_err(|_| {
        invalid("reviewed production destination record cannot be serialized canonically")
    })?;
    if canonical != bytes {
        return Err(invalid(
            "reviewed production destination bytes are not exact canonical compact JSON",
        ));
    }
    Ok(value)
}

fn validate_v1_config_pins(
    pins: &V1ConfigPinsV2,
) -> Result<(), PmReviewedProductionDestinationProfileV1Error> {
    validate_sha256(&pins.canonical_config_sha256)?;
    validate_sha256(&pins.canonical_config_fingerprint)?;
    validate_sha256(&pins.trial_plan_fingerprint)?;
    if pins.canonical_config_length == 0 {
        return Err(invalid(
            "reviewed destination V1 canonical config length must be nonzero",
        ));
    }
    Ok(())
}

fn validate_online_policy_pins(
    pins: &OnlinePolicyPinsV2,
) -> Result<(), PmReviewedProductionDestinationProfileV1Error> {
    validate_sha256(&pins.canonical_sha256)?;
    validate_sha256(&pins.fingerprint)?;
    if pins.canonical_length == 0 {
        return Err(invalid(
            "reviewed destination online-policy canonical length must be nonzero",
        ));
    }
    Ok(())
}

fn v1_config_pins_match(pins: &V1ConfigPinsV2, config: &CanonicalTrialConfig) -> bool {
    pins.canonical_config_sha256 == config.canonical_sha256()
        && pins.canonical_config_length == config.canonical_length()
        && pins.canonical_config_fingerprint == config.fingerprint()
        && pins.trial_plan_fingerprint == config.plan_fingerprint()
}

fn online_policy_pins_match(pins: &OnlinePolicyPinsV2, policy: &CanonicalOnlinePolicyV2) -> bool {
    pins.canonical_sha256 == policy.canonical_sha256()
        && pins.canonical_length == policy.canonical_length()
        && pins.fingerprint == policy.fingerprint()
}

fn validate_tls_destination(
    destination: &ReviewedFixedTlsDestinationV1,
    expected_host: &'static str,
) -> Result<(), PmReviewedProductionDestinationProfileV1Error> {
    validate_fixed_tls_identity(
        &destination.dns_name,
        destination.tcp_port,
        &destination.tls_server_name,
        &destination.http_host,
        expected_host,
    )?;
    let _ = parse_peer_ip(&destination.peer_ip)?;
    Ok(())
}

fn validate_websocket_destination(
    destination: &ReviewedFixedWebSocketDestinationV1,
) -> Result<(), PmReviewedProductionDestinationProfileV1Error> {
    validate_fixed_tls_identity(
        &destination.dns_name,
        destination.tcp_port,
        &destination.tls_server_name,
        &destination.http_host,
        REVIEWED_CLOB_WEBSOCKET_WSS_HOST_V1,
    )?;
    let _ = parse_peer_ip(&destination.peer_ip)?;
    if destination.public_path != REVIEWED_CLOB_WEBSOCKET_PUBLIC_PATH_V1
        || destination.user_path != REVIEWED_CLOB_WEBSOCKET_USER_PATH_V1
    {
        return Err(invalid(
            "reviewed CLOB WebSocket public or user path is invalid",
        ));
    }
    Ok(())
}

fn validate_fixed_tls_identity(
    dns_name: &str,
    tcp_port: u16,
    tls_server_name: &str,
    http_host: &str,
    expected_host: &'static str,
) -> Result<(), PmReviewedProductionDestinationProfileV1Error> {
    if dns_name != expected_host
        || tls_server_name != expected_host
        || http_host != expected_host
        || tcp_port != REVIEWED_TLS_PORT_V1
    {
        return Err(invalid(
            "reviewed destination DNS name, TLS name, HTTP Host, or port is invalid",
        ));
    }
    Ok(())
}

fn parse_peer_ip(value: &str) -> Result<IpAddr, PmReviewedProductionDestinationProfileV1Error> {
    let parsed =
        IpAddr::from_str(value).map_err(|_| invalid("reviewed destination peer IP is invalid"))?;
    if parsed.to_string() != value || !is_public_global_unicast(parsed) {
        return Err(invalid("reviewed destination peer IP is invalid"));
    }
    Ok(parsed)
}

fn same_address_family(left: IpAddr, right: IpAddr) -> bool {
    matches!(
        (left, right),
        (IpAddr::V4(_), IpAddr::V4(_)) | (IpAddr::V6(_), IpAddr::V6(_))
    )
}

/// Conservative, explicit public-global-unicast predicate. This deliberately
/// rejects private, loopback, link-local, shared, benchmark, documentation,
/// multicast, mapped/compatible IPv6, and other special-purpose ranges.
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
    segments[0] & 0xe000 == 0x2000
        && !(segments[0] == 0x2001 && segments[1] <= 0x01ff)
        && !(segments[0] == 0x2001 && segments[1] == 0x0db8)
        && segments[0] != 0x2002
        && !(segments[0] == 0x3fff && segments[1] & 0xf000 == 0)
}

fn validate_sha256(value: &str) -> Result<(), PmReviewedProductionDestinationProfileV1Error> {
    if value.len() != 64
        || value
            .bytes()
            .any(|byte| !byte.is_ascii_digit() && !(b'a'..=b'f').contains(&byte))
    {
        return Err(invalid(
            "reviewed production destination SHA-256 value is invalid",
        ));
    }
    Ok(())
}

fn validate_token(
    value: &str,
    maximum: usize,
    message: &'static str,
) -> Result<(), PmReviewedProductionDestinationProfileV1Error> {
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

fn validate_reference(
    value: &str,
    message: &'static str,
) -> Result<(), PmReviewedProductionDestinationProfileV1Error> {
    if value.is_empty()
        || value.len() > 512
        || value.bytes().any(|byte| !(0x21..=0x7e).contains(&byte))
    {
        return Err(invalid(message));
    }
    Ok(())
}

fn parse_utc(value: &str) -> Result<DateTime<Utc>, PmReviewedProductionDestinationProfileV1Error> {
    let parsed = DateTime::parse_from_rfc3339(value)
        .map_err(|_| invalid("reviewed production destination timestamp is invalid"))?
        .with_timezone(&Utc);
    if parsed.to_rfc3339_opts(chrono::SecondsFormat::Secs, true) != value {
        return Err(invalid(
            "reviewed production destination timestamp is not canonical UTC seconds",
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
