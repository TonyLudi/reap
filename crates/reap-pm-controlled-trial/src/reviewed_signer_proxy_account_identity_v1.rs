//! Exact, offline-only reviewed signer/proxy account-identity V1.
//!
//! This additive controlled-trial sidecar joins the exact canonical config,
//! online policy, online authorization, and committed official-source manifest
//! pin to one caller-supplied reviewed-account evidence label and one claimed
//! Polygon type-1 signer/proxy tuple. It loads neither the official manifest
//! nor the reviewed-account evidence payload and performs no network, TLS,
//! JSON-RPC, contract, factory, finality, credential, or API-key operation.
//!
//! The official-source manifest pin is closed to the locally committed REAP
//! PM-T2 freeze manifest family, version, retrieval timestamp, raw pretty-JSON
//! byte length, and SHA-256. It is not publisher-signed Polymarket evidence.
//! Matching its SHA-256 to the canonical config is still only exact label
//! correlation here: neither byte set is supplied for comparison. Likewise,
//! matching `source_reference_label` and the
//! `reviewed-account-record:{identity_id}` spelling to the config's
//! signer-to-proxy reference loads no evidence and proves no authorship.
//! General official documentation about proxy wallets cannot prove this
//! specific signer/proxy relationship.
//!
//! Strict nonzero EIP-55 parsing establishes address spelling only. Applying
//! that syntax parser to the proxy does not call it an EOA, prove it is an
//! on-chain contract, identify a factory, or establish ownership. Protected
//! custody establishes only local custody of the exact sidecar bytes; reviewer
//! and issuer fields are explicitly labels, and no signature is present.
//!
//! The schema defines no credential, private-key, API-key, L2 secret,
//! passphrase, header, cryptographic signature bytes or material, or
//! authenticated-body field. Arbitrary label and payload-digest inputs cannot
//! be proven secret-free, so callers must never place secrets or secret-derived
//! material in them. A future positive gate requires independently loaded and
//! authenticated evidence, live
//! descriptor/source and on-chain checks, source-owned time, and an exact V3
//! conjunction that pins and consumes this record fingerprint. Every mutation
//! authority result in this module is permanently DENIED. The frozen online
//! authorization V2 points nowhere to, and does not consume, this sidecar.

use std::{fmt, path::Path};

use chrono::{DateTime, Utc};
use reap_polymarket_auth::EoaAddress;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use sha2::{Digest as _, Sha256};
use thiserror::Error;

use crate::{
    CanonicalOnlineAuthorizationV2, CanonicalOnlinePolicyV2, CanonicalTrialConfig,
    OfflineAuthorizationState, OnlinePolicyPinsV2, ReviewedOnlineAuthorizationPinsV1,
    V1ConfigPinsV2,
    online_policy_v2::validate_online_authorization_contract_v2,
    protected_file::{ProtectedFileKind, read_one},
};

pub const REVIEWED_SIGNER_PROXY_ACCOUNT_IDENTITY_V1_SCHEMA_VERSION: u32 = 1;
pub const PM_T2_REVIEWED_SIGNER_PROXY_ACCOUNT_IDENTITY_FILE_V1: &str =
    "pm-t2-reviewed-signer-proxy-account-identity-v1.json";
pub const PM_T2_OFFICIAL_SOURCE_MANIFEST_SCHEMA_FAMILY_V1: &str =
    "reap-pm-controlled-trial-official-sources";
pub const PM_T2_OFFICIAL_SOURCE_MANIFEST_SCHEMA_VERSION_V1: u32 = 1;
pub const PM_T2_OFFICIAL_SOURCE_MANIFEST_RETRIEVED_AT_UTC_V1: &str = "2026-08-09T10:17:00Z";
pub const PM_T2_OFFICIAL_SOURCE_MANIFEST_BYTE_LENGTH_V1: u64 = 9_103;
pub const PM_T2_OFFICIAL_SOURCE_MANIFEST_SHA256_V1: &str =
    "ebd07e0dfbb7ee0dd825b7b435b303826130761d156e2f23b6c3428f1486e910";

const MAX_CANONICAL_REVIEWED_SIGNER_PROXY_ACCOUNT_IDENTITY_BYTES_V1: usize = 64 * 1024;
const REVIEWED_SIGNER_PROXY_ACCOUNT_IDENTITY_V1_FINGERPRINT_DOMAIN: &[u8] =
    b"reap.pm-t2.controlled-trial.reviewed-signer-proxy-account-identity.v1\0";
const PM_T2_ACCOUNT_CHAIN_ID_V1: u64 = 137;
const PM_T2_ACCOUNT_SIGNATURE_TYPE_V1: u8 = 1;
const PM_T2_ACCOUNT_WALLET_PROFILE_V1: &str = "poly_proxy";

/// Closed identification of the committed official-source manifest bytes.
///
/// These fields are exact constants, not a free manifest label. Verification
/// still does not load or hash the manifest bytes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewedOfficialSourceManifestPinsV1 {
    pub schema_family: String,
    pub schema_version: u32,
    pub retrieved_at_utc: String,
    pub byte_length: u64,
    pub sha256: String,
}

impl ReviewedOfficialSourceManifestPinsV1 {
    fn validate(&self) -> Result<DateTime<Utc>, PmReviewedSignerProxyAccountIdentityV1Error> {
        if self.schema_family != PM_T2_OFFICIAL_SOURCE_MANIFEST_SCHEMA_FAMILY_V1
            || self.schema_version != PM_T2_OFFICIAL_SOURCE_MANIFEST_SCHEMA_VERSION_V1
            || self.retrieved_at_utc != PM_T2_OFFICIAL_SOURCE_MANIFEST_RETRIEVED_AT_UTC_V1
            || self.byte_length != PM_T2_OFFICIAL_SOURCE_MANIFEST_BYTE_LENGTH_V1
            || self.sha256 != PM_T2_OFFICIAL_SOURCE_MANIFEST_SHA256_V1
        {
            return Err(invalid(
                "reviewed account official-source manifest pin differs from the exact committed V1 manifest",
            ));
        }
        parse_utc(&self.retrieved_at_utc)
    }
}

/// The only V1 source kind. Its serialized name deliberately states that the
/// source is unattested.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReviewedSignerProxyAccountEvidenceKindV1 {
    #[serde(rename = "unattested_reviewed_account_source_v1")]
    UnattestedReviewedAccountSourceV1,
}

/// Exact public claimed account tuple found in the unattested evidence label.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewedSignerProxyClaimedAccountV1 {
    pub chain_id: u64,
    pub wallet_profile: String,
    pub signature_type: u8,
    pub signer: String,
    pub proxy_funder: String,
}

impl ReviewedSignerProxyClaimedAccountV1 {
    fn validate(&self) -> Result<(), PmReviewedSignerProxyAccountIdentityV1Error> {
        if self.chain_id != PM_T2_ACCOUNT_CHAIN_ID_V1
            || self.wallet_profile != PM_T2_ACCOUNT_WALLET_PROFILE_V1
            || self.signature_type != PM_T2_ACCOUNT_SIGNATURE_TYPE_V1
        {
            return Err(invalid(
                "reviewed account claim is not the fixed Polygon type-1 proxy profile",
            ));
        }
        let signer = parse_eip55_address_syntax(&self.signer)?;
        let proxy_funder = parse_eip55_address_syntax(&self.proxy_funder)?;
        if signer == proxy_funder {
            return Err(invalid(
                "reviewed account signer and proxy funder must be distinct",
            ));
        }
        Ok(())
    }
}

/// Caller-supplied metadata and digest for an account-source payload that this
/// module never loads. All identity/source scalars are labels, not signatures.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UnattestedReviewedSignerProxyAccountEvidenceV1 {
    pub evidence_kind: ReviewedSignerProxyAccountEvidenceKindV1,
    pub evidence_id_label: String,
    pub issuer_label: String,
    pub source_reference_label: String,
    pub observed_at_utc: String,
    pub payload_media_type_label: String,
    pub payload_byte_length: u64,
    pub payload_sha256: String,
    pub claimed_account: ReviewedSignerProxyClaimedAccountV1,
}

impl UnattestedReviewedSignerProxyAccountEvidenceV1 {
    fn validate(&self) -> Result<DateTime<Utc>, PmReviewedSignerProxyAccountIdentityV1Error> {
        validate_token(
            &self.evidence_id_label,
            128,
            "reviewed account evidence ID label is invalid",
        )?;
        validate_reference(
            &self.issuer_label,
            "reviewed account issuer label is invalid",
        )?;
        validate_reference(
            &self.source_reference_label,
            "reviewed account source-reference label is invalid",
        )?;
        validate_reference(
            &self.payload_media_type_label,
            "reviewed account payload media-type label is invalid",
        )?;
        if self.payload_byte_length == 0 {
            return Err(invalid(
                "reviewed account payload byte-length label must be nonzero",
            ));
        }
        validate_sha256(&self.payload_sha256)?;
        self.claimed_account.validate()?;
        parse_utc(&self.observed_at_utc)
    }
}

/// Reviewer-labeled, unsigned account-identity sidecar.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewedSignerProxyAccountIdentityV1 {
    pub schema_version: u32,
    pub identity_id: String,
    pub reviewer_label: String,
    pub reviewed_at_utc: String,
    pub valid_not_before_utc: String,
    pub valid_not_after_utc: String,
    pub v1_config: V1ConfigPinsV2,
    pub online_policy: OnlinePolicyPinsV2,
    pub online_authorization: ReviewedOnlineAuthorizationPinsV1,
    pub official_source_manifest: ReviewedOfficialSourceManifestPinsV1,
    pub evidence: UnattestedReviewedSignerProxyAccountEvidenceV1,
}

impl fmt::Debug for ReviewedSignerProxyAccountIdentityV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(
            "ReviewedSignerProxyAccountIdentityV1(<unsigned-reviewer-and-issuer-labels; account-and-evidence-redacted; denied>)",
        )
    }
}

struct ValidatedReviewedSignerProxyAccountTimesV1 {
    reviewed_at: DateTime<Utc>,
    valid_not_before: DateTime<Utc>,
    valid_not_after: DateTime<Utc>,
}

impl ReviewedSignerProxyAccountIdentityV1 {
    fn validate_intrinsic(
        &self,
    ) -> Result<
        ValidatedReviewedSignerProxyAccountTimesV1,
        PmReviewedSignerProxyAccountIdentityV1Error,
    > {
        if self.schema_version != REVIEWED_SIGNER_PROXY_ACCOUNT_IDENTITY_V1_SCHEMA_VERSION {
            return Err(invalid(
                "unsupported reviewed signer/proxy account-identity V1 schema",
            ));
        }
        validate_token(
            &self.identity_id,
            128,
            "reviewed signer/proxy account identity ID is invalid",
        )?;
        validate_reference(
            &self.reviewer_label,
            "reviewed signer/proxy account reviewer label is invalid",
        )?;
        validate_v1_config_pins(&self.v1_config)?;
        validate_online_policy_pins(&self.online_policy)?;
        validate_online_authorization_pins(&self.online_authorization)?;
        let manifest_retrieved_at = self.official_source_manifest.validate()?;
        let evidence_observed_at = self.evidence.validate()?;
        let reviewed_at = parse_utc(&self.reviewed_at_utc)?;
        let valid_not_before = parse_utc(&self.valid_not_before_utc)?;
        let valid_not_after = parse_utc(&self.valid_not_after_utc)?;
        if manifest_retrieved_at > reviewed_at
            || evidence_observed_at > reviewed_at
            || reviewed_at > valid_not_before
            || valid_not_before >= valid_not_after
        {
            return Err(invalid(
                "reviewed signer/proxy account identity time envelope is invalid",
            ));
        }
        Ok(ValidatedReviewedSignerProxyAccountTimesV1 {
            reviewed_at,
            valid_not_before,
            valid_not_after,
        })
    }
}

/// Move-only, non-serializable, non-projecting holder of the exact protected
/// canonical sidecar bytes.
pub struct CanonicalReviewedSignerProxyAccountIdentityV1 {
    value: ReviewedSignerProxyAccountIdentityV1,
    canonical_bytes: Vec<u8>,
    canonical_sha256: String,
    fingerprint: String,
}

impl CanonicalReviewedSignerProxyAccountIdentityV1 {
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

impl fmt::Debug for CanonicalReviewedSignerProxyAccountIdentityV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(
            "CanonicalReviewedSignerProxyAccountIdentityV1(<exact-protected-canonical-bytes; no-value-address-reference-or-path-projection; redacted; denied>)",
        )
    }
}

/// Offline structural display only. Every source/authorship, live-chain,
/// ownership, currentness, consumption, and mutation-authority fact is false.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ReviewedSignerProxyAccountIdentityVerificationV1 {
    pub schema_version: u32,
    pub identity_id: String,
    pub config_fingerprint: String,
    pub online_policy_fingerprint: String,
    pub online_authorization_fingerprint: String,
    pub reviewed_signer_proxy_account_identity_fingerprint: String,
    pub exact_config_policy_authorization_pins_structurally_valid: bool,
    pub exact_official_source_manifest_pin_structurally_valid: bool,
    pub official_source_manifest_sha256_matches_config_label: bool,
    pub exact_claimed_account_tuple_matches_config: bool,
    pub identity_id_matches_config_evidence_reference_label: bool,
    pub source_reference_matches_config_label: bool,
    pub official_source_manifest_bytes_loaded_and_hash_verified: bool,
    pub reviewed_account_evidence_bytes_loaded_and_hash_verified: bool,
    pub official_source_manifest_publisher_authorship_attested: bool,
    pub reviewer_authorship_attested: bool,
    pub source_authorship_attested: bool,
    pub issuer_signature_verified: bool,
    pub evidence_source_tls_and_server_identity_verified: bool,
    pub signer_on_chain_eoa_status_verified: bool,
    pub proxy_on_chain_contract_status_verified: bool,
    pub on_chain_account_state_checked: bool,
    pub on_chain_finality_checked: bool,
    pub proxy_factory_semantics_verified: bool,
    pub signer_controls_proxy_attested: bool,
    pub signer_proxy_relationship_current: bool,
    pub signer_proxy_relationship_unrevoked: bool,
    pub account_specific_evidence_reference_resolved_and_authenticated: bool,
    pub source_owned_current_time_checked: bool,
    pub remote_api_key_owner_attested: bool,
    pub private_key_derived_signer_matches_config_checked: bool,
    pub l2_credentials_match_configured_signer_checked: bool,
    pub identity_fingerprint_pinned_by_online_authorization_v2: bool,
    pub identity_fingerprint_pinned_by_v3: bool,
    pub identity_consumption_durably_recorded: bool,
    pub authorization_consumption_checked: bool,
    pub credential_mutation_authority_attested: bool,
    #[serde(flatten)]
    pub authorization: OfflineAuthorizationState,
}

pub fn load_canonical_reviewed_signer_proxy_account_identity_v1(
    path: &Path,
) -> Result<
    CanonicalReviewedSignerProxyAccountIdentityV1,
    PmReviewedSignerProxyAccountIdentityV1Error,
> {
    let bytes = read_one(
        path,
        ProtectedFileKind::ReviewedSignerProxyAccountIdentityV1,
        MAX_CANONICAL_REVIEWED_SIGNER_PROXY_ACCOUNT_IDENTITY_BYTES_V1,
    )
    .map_err(|_| {
        invalid("reviewed signer/proxy account identity protection or stability check failed")
    })?;
    let value: ReviewedSignerProxyAccountIdentityV1 = parse_exact_canonical(&bytes)?;
    let _ = value.validate_intrinsic()?;
    let canonical_bytes = bytes.to_vec();
    Ok(CanonicalReviewedSignerProxyAccountIdentityV1 {
        canonical_sha256: hash_bytes(&[], &canonical_bytes),
        fingerprint: hash_bytes(
            REVIEWED_SIGNER_PROXY_ACCOUNT_IDENTITY_V1_FINGERPRINT_DOMAIN,
            &canonical_bytes,
        ),
        canonical_bytes,
        value,
    })
}

/// Verify exact structural bindings without consulting a caller clock or
/// loading either pinned source byte set.
pub fn verify_reviewed_signer_proxy_account_identity_v1(
    config: &CanonicalTrialConfig,
    policy: &CanonicalOnlinePolicyV2,
    authorization: &CanonicalOnlineAuthorizationV2,
    identity: &CanonicalReviewedSignerProxyAccountIdentityV1,
) -> Result<
    ReviewedSignerProxyAccountIdentityVerificationV1,
    PmReviewedSignerProxyAccountIdentityV1Error,
> {
    let authorization_times =
        validate_online_authorization_contract_v2(config, policy, authorization).map_err(|_| {
            invalid("reviewed signer/proxy account identity bound online V2 contract is invalid")
        })?;
    let identity_times = identity.value.validate_intrinsic()?;
    if !v1_config_pins_match(&identity.value.v1_config, config)
        || !online_policy_pins_match(&identity.value.online_policy, policy)
        || !online_authorization_pins_match(&identity.value.online_authorization, authorization)
    {
        return Err(invalid(
            "reviewed signer/proxy account identity exact config, policy, or authorization pin mismatched",
        ));
    }
    if identity_times.reviewed_at < authorization_times.reviewed_at
        || identity_times.reviewed_at > authorization_times.not_before
        || identity_times.valid_not_before != authorization_times.not_before
        || identity_times.valid_not_after != authorization_times.cleanup_not_after
    {
        return Err(invalid(
            "reviewed signer/proxy account identity validity differs from the bound authorization envelope",
        ));
    }
    if identity.value.official_source_manifest.sha256 != config.value().source_pin_manifest_sha256 {
        return Err(invalid(
            "reviewed signer/proxy account official-source manifest SHA label differs from config",
        ));
    }
    let claimed_account = &identity.value.evidence.claimed_account;
    let configured_account = &config.value().account;
    if claimed_account.chain_id != configured_account.chain_id
        || claimed_account.wallet_profile != configured_account.wallet_profile
        || claimed_account.signature_type != configured_account.signature_type
        || claimed_account.signer != configured_account.signer
        || claimed_account.proxy_funder != configured_account.funder
    {
        return Err(invalid(
            "reviewed signer/proxy claimed account tuple differs from canonical config",
        ));
    }
    let configured_evidence_reference = &config
        .value()
        .credential_slot
        .signer_to_proxy_evidence_reference;
    let expected_evidence_reference =
        format!("reviewed-account-record:{}", identity.value.identity_id);
    if configured_evidence_reference.as_str() != expected_evidence_reference.as_str()
        || identity.value.evidence.source_reference_label != configured_evidence_reference.as_str()
    {
        return Err(invalid(
            "reviewed signer/proxy evidence-reference label differs from canonical config",
        ));
    }

    Ok(ReviewedSignerProxyAccountIdentityVerificationV1 {
        schema_version: identity.value.schema_version,
        identity_id: identity.value.identity_id.clone(),
        config_fingerprint: config.fingerprint().to_owned(),
        online_policy_fingerprint: policy.fingerprint().to_owned(),
        online_authorization_fingerprint: authorization.fingerprint().to_owned(),
        reviewed_signer_proxy_account_identity_fingerprint: identity.fingerprint.clone(),
        exact_config_policy_authorization_pins_structurally_valid: true,
        exact_official_source_manifest_pin_structurally_valid: true,
        official_source_manifest_sha256_matches_config_label: true,
        exact_claimed_account_tuple_matches_config: true,
        identity_id_matches_config_evidence_reference_label: true,
        source_reference_matches_config_label: true,
        official_source_manifest_bytes_loaded_and_hash_verified: false,
        reviewed_account_evidence_bytes_loaded_and_hash_verified: false,
        official_source_manifest_publisher_authorship_attested: false,
        reviewer_authorship_attested: false,
        source_authorship_attested: false,
        issuer_signature_verified: false,
        evidence_source_tls_and_server_identity_verified: false,
        signer_on_chain_eoa_status_verified: false,
        proxy_on_chain_contract_status_verified: false,
        on_chain_account_state_checked: false,
        on_chain_finality_checked: false,
        proxy_factory_semantics_verified: false,
        signer_controls_proxy_attested: false,
        signer_proxy_relationship_current: false,
        signer_proxy_relationship_unrevoked: false,
        account_specific_evidence_reference_resolved_and_authenticated: false,
        source_owned_current_time_checked: false,
        remote_api_key_owner_attested: false,
        private_key_derived_signer_matches_config_checked: false,
        l2_credentials_match_configured_signer_checked: false,
        identity_fingerprint_pinned_by_online_authorization_v2: false,
        identity_fingerprint_pinned_by_v3: false,
        identity_consumption_durably_recorded: false,
        authorization_consumption_checked: false,
        credential_mutation_authority_attested: false,
        authorization: OfflineAuthorizationState::DENIED,
    })
}

#[derive(Debug, Error)]
pub enum PmReviewedSignerProxyAccountIdentityV1Error {
    #[error("controlled-trial reviewed signer/proxy account identity V1 is invalid: {0}")]
    Invalid(&'static str),
}

fn invalid(message: &'static str) -> PmReviewedSignerProxyAccountIdentityV1Error {
    PmReviewedSignerProxyAccountIdentityV1Error::Invalid(message)
}

fn parse_exact_canonical<T: DeserializeOwned + Serialize>(
    bytes: &[u8],
) -> Result<T, PmReviewedSignerProxyAccountIdentityV1Error> {
    let value: T = serde_json::from_slice(bytes).map_err(|_| {
        invalid(
            "reviewed signer/proxy account identity JSON is malformed, duplicated, unknown, or trailing",
        )
    })?;
    let canonical = serde_json::to_vec(&value).map_err(|_| {
        invalid("reviewed signer/proxy account identity cannot be serialized canonically")
    })?;
    if canonical != bytes {
        return Err(invalid(
            "reviewed signer/proxy account identity bytes are not exact canonical compact JSON",
        ));
    }
    Ok(value)
}

fn validate_v1_config_pins(
    pins: &V1ConfigPinsV2,
) -> Result<(), PmReviewedSignerProxyAccountIdentityV1Error> {
    validate_sha256(&pins.canonical_config_sha256)?;
    validate_sha256(&pins.canonical_config_fingerprint)?;
    validate_sha256(&pins.trial_plan_fingerprint)?;
    if pins.canonical_config_length == 0 {
        return Err(invalid(
            "reviewed signer/proxy account canonical config length must be nonzero",
        ));
    }
    Ok(())
}

fn validate_online_policy_pins(
    pins: &OnlinePolicyPinsV2,
) -> Result<(), PmReviewedSignerProxyAccountIdentityV1Error> {
    validate_sha256(&pins.canonical_sha256)?;
    validate_sha256(&pins.fingerprint)?;
    if pins.canonical_length == 0 {
        return Err(invalid(
            "reviewed signer/proxy account online-policy length must be nonzero",
        ));
    }
    Ok(())
}

fn validate_online_authorization_pins(
    pins: &ReviewedOnlineAuthorizationPinsV1,
) -> Result<(), PmReviewedSignerProxyAccountIdentityV1Error> {
    validate_token(
        &pins.authorization_id,
        128,
        "reviewed signer/proxy account online-authorization ID is invalid",
    )?;
    validate_sha256(&pins.canonical_sha256)?;
    validate_sha256(&pins.fingerprint)?;
    if pins.canonical_length == 0 {
        return Err(invalid(
            "reviewed signer/proxy account online-authorization length must be nonzero",
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

fn online_authorization_pins_match(
    pins: &ReviewedOnlineAuthorizationPinsV1,
    authorization: &CanonicalOnlineAuthorizationV2,
) -> bool {
    pins.authorization_id == authorization.value().authorization_id
        && pins.canonical_sha256 == authorization.canonical_sha256()
        && pins.canonical_length == authorization.canonical_length()
        && pins.fingerprint == authorization.fingerprint()
}

fn parse_eip55_address_syntax(
    value: &str,
) -> Result<EoaAddress, PmReviewedSignerProxyAccountIdentityV1Error> {
    // `EoaAddress` is used solely for strict nonzero EIP-55 spelling. The
    // proxy field is not asserted to be an EOA, contract, deployed object, or
    // factory product by this parser.
    EoaAddress::parse(value).map_err(|_| {
        invalid("reviewed signer/proxy account address is not canonical nonzero EIP-55")
    })
}

fn validate_sha256(value: &str) -> Result<(), PmReviewedSignerProxyAccountIdentityV1Error> {
    if value.len() != 64
        || value
            .bytes()
            .any(|byte| !byte.is_ascii_digit() && !(b'a'..=b'f').contains(&byte))
    {
        return Err(invalid(
            "reviewed signer/proxy account SHA-256 value is invalid",
        ));
    }
    Ok(())
}

fn validate_token(
    value: &str,
    maximum: usize,
    message: &'static str,
) -> Result<(), PmReviewedSignerProxyAccountIdentityV1Error> {
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
) -> Result<(), PmReviewedSignerProxyAccountIdentityV1Error> {
    if value.is_empty()
        || value.len() > 512
        || value.bytes().any(|byte| !(0x21..=0x7e).contains(&byte))
    {
        return Err(invalid(message));
    }
    Ok(())
}

fn parse_utc(value: &str) -> Result<DateTime<Utc>, PmReviewedSignerProxyAccountIdentityV1Error> {
    let parsed = DateTime::parse_from_rfc3339(value)
        .map_err(|_| invalid("reviewed signer/proxy account timestamp is invalid"))?
        .with_timezone(&Utc);
    if parsed.to_rfc3339_opts(chrono::SecondsFormat::Secs, true) != value {
        return Err(invalid(
            "reviewed signer/proxy account timestamp is not canonical UTC seconds",
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
