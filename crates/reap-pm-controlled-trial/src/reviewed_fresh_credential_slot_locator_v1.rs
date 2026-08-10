//! Exact, offline-only reviewed fresh credential-slot locator V1.
//!
//! This additive sidecar joins one exact protected-directory spelling and four
//! fixed fresh-place entry basenames to the non-secret credential-slot labels
//! already present in the canonical PM-T2 config. It never reads credential
//! material and deliberately does not derive a fingerprint from an API key,
//! HMAC secret, passphrase, or private key.
//!
//! The protected sidecar and absolute directory remain caller-supplied and
//! unattested by the frozen V2 lineage. Protected-file mode, ownership, and
//! stability checks establish local custody only; they do not attest reviewer
//! authorship, and `issuing_reviewer` is only a reviewer label. Verification
//! proves only record-internal and lexical locator bindings. It does not prove
//! that a filesystem object currently exists at the locator, that any loaded
//! bundle is the operator-labeled credential generation, that an API key is
//! accepted or owned by the configured signer, or that the proxy funder is
//! remotely controlled by that signer. A later actor must perform
//! descriptor-pinned protected-file custody, and a future positive gate needs
//! a separately versioned authorization/delivery conjunction that binds this
//! exact sidecar fingerprint. The frozen online-authorization V2 consumption
//! ledger neither consumes nor authenticates this sidecar.

use std::{
    ffi::OsStr,
    fmt,
    path::{Component, Path, PathBuf},
};

use chrono::{DateTime, Utc};
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

pub const REVIEWED_FRESH_CREDENTIAL_SLOT_LOCATOR_V1_SCHEMA_VERSION: u32 = 1;
pub const PM_T2_REVIEWED_FRESH_CREDENTIAL_SLOT_LOCATOR_FILE_V1: &str =
    "pm-t2-reviewed-fresh-credential-slot-locator-v1.json";
pub const PM_T2_FRESH_PRIVATE_KEY_ENTRY_V1: &str = "private-key";
pub const PM_T2_FRESH_API_KEY_ENTRY_V1: &str = "api-key";
pub const PM_T2_FRESH_L2_SECRET_ENTRY_V1: &str = "l2-secret";
pub const PM_T2_FRESH_PASSPHRASE_ENTRY_V1: &str = "passphrase";

const MAX_CANONICAL_REVIEWED_FRESH_CREDENTIAL_SLOT_LOCATOR_BYTES_V1: usize = 64 * 1024;
const MAX_PROTECTED_FRESH_CREDENTIAL_DIRECTORY_BYTES_V1: usize = 1_024;
const REVIEWED_FRESH_CREDENTIAL_SLOT_LOCATOR_V1_FINGERPRINT_DOMAIN: &[u8] =
    b"reap.pm-t2.controlled-trial.reviewed-fresh-credential-slot-locator.v1\0";

/// Closed role-to-basename map for fresh-place custody.
///
/// Each scalar is present in the canonical record for reviewer visibility but
/// validates to one compile-time literal, so the caller cannot select a
/// non-fixed basename. This does not make the sidecar or directory
/// caller-independent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewedFreshCredentialFilesV1 {
    pub private_key_entry: String,
    pub api_key_entry: String,
    pub l2_secret_entry: String,
    pub passphrase_entry: String,
}

impl ReviewedFreshCredentialFilesV1 {
    fn validate(&self) -> Result<(), PmReviewedFreshCredentialSlotLocatorV1Error> {
        if self.private_key_entry != PM_T2_FRESH_PRIVATE_KEY_ENTRY_V1
            || self.api_key_entry != PM_T2_FRESH_API_KEY_ENTRY_V1
            || self.l2_secret_entry != PM_T2_FRESH_L2_SECRET_ENTRY_V1
            || self.passphrase_entry != PM_T2_FRESH_PASSPHRASE_ENTRY_V1
        {
            return Err(invalid(
                "reviewed fresh credential role basenames differ from the fixed V1 layout",
            ));
        }
        Ok(())
    }
}

/// Reviewer-labeled, non-authenticated, non-secret lexical locator for one
/// exact fresh slot.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewedFreshCredentialSlotLocatorV1 {
    pub schema_version: u32,
    pub locator_id: String,
    pub issuing_reviewer: String,
    pub reviewed_at_utc: String,
    pub valid_not_before_utc: String,
    pub valid_not_after_utc: String,
    pub v1_config: V1ConfigPinsV2,
    pub online_policy: OnlinePolicyPinsV2,
    pub online_authorization: ReviewedOnlineAuthorizationPinsV1,
    pub credential_slot_id: String,
    pub credential_slot_nonsecret_fingerprint_sha256: String,
    pub protected_fresh_credential_directory: String,
    pub files: ReviewedFreshCredentialFilesV1,
}

impl fmt::Debug for ReviewedFreshCredentialSlotLocatorV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(
            "ReviewedFreshCredentialSlotLocatorV1(<reviewed-nonsecret-locator; redacted>)",
        )
    }
}

struct ValidatedFreshCredentialSlotLocatorV1 {
    reviewed_at: DateTime<Utc>,
    valid_not_before: DateTime<Utc>,
    valid_not_after: DateTime<Utc>,
    protected_fresh_credential_directory: PathBuf,
}

impl ReviewedFreshCredentialSlotLocatorV1 {
    fn validate_intrinsic(
        &self,
    ) -> Result<ValidatedFreshCredentialSlotLocatorV1, PmReviewedFreshCredentialSlotLocatorV1Error>
    {
        if self.schema_version != REVIEWED_FRESH_CREDENTIAL_SLOT_LOCATOR_V1_SCHEMA_VERSION {
            return Err(invalid(
                "unsupported reviewed fresh credential-slot locator V1 schema",
            ));
        }
        validate_token(
            &self.locator_id,
            128,
            "reviewed fresh credential-slot locator ID is invalid",
        )?;
        validate_reference(
            &self.issuing_reviewer,
            "reviewed fresh credential-slot locator issuing reviewer is invalid",
        )?;
        validate_v1_config_pins(&self.v1_config)?;
        validate_online_policy_pins(&self.online_policy)?;
        validate_online_authorization_pins(&self.online_authorization)?;
        validate_token(
            &self.credential_slot_id,
            128,
            "reviewed fresh credential slot ID is invalid",
        )?;
        validate_sha256(&self.credential_slot_nonsecret_fingerprint_sha256)?;
        self.files.validate()?;

        let reviewed_at = parse_utc(&self.reviewed_at_utc)?;
        let valid_not_before = parse_utc(&self.valid_not_before_utc)?;
        let valid_not_after = parse_utc(&self.valid_not_after_utc)?;
        if reviewed_at > valid_not_before || valid_not_before >= valid_not_after {
            return Err(invalid(
                "reviewed fresh credential-slot locator time envelope is invalid",
            ));
        }
        let protected_fresh_credential_directory =
            validate_absolute_lexical_directory(&self.protected_fresh_credential_directory)?;

        Ok(ValidatedFreshCredentialSlotLocatorV1 {
            reviewed_at,
            valid_not_before,
            valid_not_after,
            protected_fresh_credential_directory,
        })
    }
}

/// Move-only, redacted holder of the exact protected canonical sidecar bytes.
pub struct CanonicalReviewedFreshCredentialSlotLocatorV1 {
    value: ReviewedFreshCredentialSlotLocatorV1,
    canonical_bytes: Vec<u8>,
    canonical_sha256: String,
    fingerprint: String,
    protected_fresh_credential_directory: PathBuf,
}

impl CanonicalReviewedFreshCredentialSlotLocatorV1 {
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

impl fmt::Debug for CanonicalReviewedFreshCredentialSlotLocatorV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(
            "CanonicalReviewedFreshCredentialSlotLocatorV1(<reviewed-locator-evidence; exact-canonical-bytes; redacted>)",
        )
    }
}

/// Move-only retained canonical evidence after the one local load token has
/// been split from a verified holder.
///
/// The exact record and bytes remain private so this evidence can be
/// structurally reverified without exposing a directory, file layout, signer,
/// or another load capability. It is not V2 authorization, reviewer-authorship
/// attestation, credential-delivery evidence, or globally consumed state.
pub struct CanonicalReviewedFreshCredentialSlotLocatorEvidenceV1 {
    value: ReviewedFreshCredentialSlotLocatorV1,
    canonical_bytes: Vec<u8>,
    canonical_sha256: String,
    fingerprint: String,
}

impl CanonicalReviewedFreshCredentialSlotLocatorEvidenceV1 {
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

impl fmt::Debug for CanonicalReviewedFreshCredentialSlotLocatorEvidenceV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(
            "CanonicalReviewedFreshCredentialSlotLocatorEvidenceV1(<retained-canonical-evidence; no-directory-authority; redacted>)",
        )
    }
}

/// One-shot local projection capability for one loaded canonical holder.
///
/// The token carries the validated lexical directory and the signer string
/// derived inside the exact conjunction binder from the canonical config. It
/// has no getters, is not cloneable or serializable, and yields both values
/// only by being consumed. `into_parts` returns ordinary cloneable `PathBuf`
/// and `String` values; it does not prevent their later copying or arbitrary
/// filesystem access. Actual single-load composition is enforced only when
/// the runner's private staged loader immediately consumes this token. This is
/// a local structural capability, not V2 authorization, credential delivery,
/// reviewer-authorship proof, durable consumption, or global uniqueness.
/// Loading the protected sidecar again can issue another independently denied
/// token until a future V3 lineage binds and consumes the exact locator
/// fingerprint.
#[must_use = "a reviewed fresh credential load token must be consumed or deliberately dropped"]
pub struct ReviewedFreshCredentialLoadTokenV1 {
    protected_fresh_credential_directory: PathBuf,
    configured_signer: String,
}

impl ReviewedFreshCredentialLoadTokenV1 {
    /// Consume the sole projection boundary for this loaded canonical holder.
    #[must_use]
    pub fn into_parts(self) -> (PathBuf, String) {
        (
            self.protected_fresh_credential_directory,
            self.configured_signer,
        )
    }
}

impl fmt::Debug for ReviewedFreshCredentialLoadTokenV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(
            "ReviewedFreshCredentialLoadTokenV1(<one-shot-local-load; path-and-signer-redacted; denied>)",
        )
    }
}

/// Offline structural display only. Production mutation authority remains
/// DENIED, and no filesystem, secret-generation, or remote-owner fact is
/// inferred from this result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ReviewedFreshCredentialSlotLocatorVerificationV1 {
    pub schema_version: u32,
    pub locator_id: String,
    pub config_fingerprint: String,
    pub online_policy_fingerprint: String,
    pub online_authorization_fingerprint: String,
    pub reviewed_fresh_credential_slot_locator_fingerprint: String,
    pub exact_v2_bindings_structurally_valid: bool,
    pub canonical_credential_slot_binding_structurally_valid: bool,
    pub fixed_fresh_credential_locator_structurally_valid: bool,
    pub source_owned_current_time_checked: bool,
    pub protected_credential_directory_and_four_files_checked: bool,
    pub loaded_bundle_matches_credential_slot_generation: bool,
    pub remote_api_key_owner_attested: bool,
    pub locator_fingerprint_pinned_by_v2: bool,
    pub reviewer_authorship_attested: bool,
    pub load_token_consumption_durably_recorded: bool,
    pub authorization_consumption_checked: bool,
    #[serde(flatten)]
    pub authorization: OfflineAuthorizationState,
}

pub fn load_canonical_reviewed_fresh_credential_slot_locator_v1(
    path: &Path,
) -> Result<
    CanonicalReviewedFreshCredentialSlotLocatorV1,
    PmReviewedFreshCredentialSlotLocatorV1Error,
> {
    let bytes = read_one(
        path,
        ProtectedFileKind::ReviewedFreshCredentialSlotLocatorV1,
        MAX_CANONICAL_REVIEWED_FRESH_CREDENTIAL_SLOT_LOCATOR_BYTES_V1,
    )
    .map_err(|_| {
        invalid("reviewed fresh credential-slot locator protection or stability check failed")
    })?;
    let value: ReviewedFreshCredentialSlotLocatorV1 = parse_exact_canonical(&bytes)?;
    let validated = value.validate_intrinsic()?;
    let canonical_bytes = bytes.to_vec();
    Ok(CanonicalReviewedFreshCredentialSlotLocatorV1 {
        canonical_sha256: hash_bytes(&[], &canonical_bytes),
        fingerprint: hash_bytes(
            REVIEWED_FRESH_CREDENTIAL_SLOT_LOCATOR_V1_FINGERPRINT_DOMAIN,
            &canonical_bytes,
        ),
        canonical_bytes,
        protected_fresh_credential_directory: validated.protected_fresh_credential_directory,
        value,
    })
}

/// Verify exact immutable reviewer bindings without consulting a caller clock.
///
/// A later source-owned actor must enforce current time and protected-file
/// custody. The frozen V2 consumption ledger does not consume this sidecar.
pub fn verify_reviewed_fresh_credential_slot_locator_v1(
    config: &CanonicalTrialConfig,
    policy: &CanonicalOnlinePolicyV2,
    authorization: &CanonicalOnlineAuthorizationV2,
    locator: &CanonicalReviewedFreshCredentialSlotLocatorV1,
) -> Result<
    ReviewedFreshCredentialSlotLocatorVerificationV1,
    PmReviewedFreshCredentialSlotLocatorV1Error,
> {
    verify_reviewed_fresh_credential_slot_locator_value_v1(
        config,
        policy,
        authorization,
        &locator.value,
        &locator.fingerprint,
    )
}

/// Consume one exact canonical locator into retained non-projecting evidence
/// and one one-shot local projection token.
///
/// The conjunction is checked before either output is created. The token's
/// signer comes only from the exact canonical config; no caller signer input is
/// accepted. This split is not durable consumption or V2 authorization. A
/// separately loaded protected sidecar can be bound into another denied token,
/// and the projected scalar parts are ordinarily cloneable, until a future V3
/// lineage pins and consumes the exact locator fingerprint.
pub fn bind_reviewed_fresh_credential_slot_locator_v1(
    config: &CanonicalTrialConfig,
    policy: &CanonicalOnlinePolicyV2,
    authorization: &CanonicalOnlineAuthorizationV2,
    locator: CanonicalReviewedFreshCredentialSlotLocatorV1,
) -> Result<
    (
        CanonicalReviewedFreshCredentialSlotLocatorEvidenceV1,
        ReviewedFreshCredentialLoadTokenV1,
    ),
    PmReviewedFreshCredentialSlotLocatorV1Error,
> {
    let _ =
        verify_reviewed_fresh_credential_slot_locator_v1(config, policy, authorization, &locator)?;
    let configured_signer = config.value().account.signer.clone();
    let CanonicalReviewedFreshCredentialSlotLocatorV1 {
        value,
        canonical_bytes,
        canonical_sha256,
        fingerprint,
        protected_fresh_credential_directory,
    } = locator;
    Ok((
        CanonicalReviewedFreshCredentialSlotLocatorEvidenceV1 {
            value,
            canonical_bytes,
            canonical_sha256,
            fingerprint,
        },
        ReviewedFreshCredentialLoadTokenV1 {
            protected_fresh_credential_directory,
            configured_signer,
        },
    ))
}

/// Reverify the exact retained conjunction after the one path-bearing token
/// has been split away.
///
/// This returns the same denied structural display and cannot project a path,
/// signer, file layout, or new load token from the evidence.
pub fn verify_reviewed_fresh_credential_slot_locator_evidence_v1(
    config: &CanonicalTrialConfig,
    policy: &CanonicalOnlinePolicyV2,
    authorization: &CanonicalOnlineAuthorizationV2,
    evidence: &CanonicalReviewedFreshCredentialSlotLocatorEvidenceV1,
) -> Result<
    ReviewedFreshCredentialSlotLocatorVerificationV1,
    PmReviewedFreshCredentialSlotLocatorV1Error,
> {
    verify_reviewed_fresh_credential_slot_locator_value_v1(
        config,
        policy,
        authorization,
        &evidence.value,
        &evidence.fingerprint,
    )
}

fn verify_reviewed_fresh_credential_slot_locator_value_v1(
    config: &CanonicalTrialConfig,
    policy: &CanonicalOnlinePolicyV2,
    authorization: &CanonicalOnlineAuthorizationV2,
    locator: &ReviewedFreshCredentialSlotLocatorV1,
    locator_fingerprint: &str,
) -> Result<
    ReviewedFreshCredentialSlotLocatorVerificationV1,
    PmReviewedFreshCredentialSlotLocatorV1Error,
> {
    let authorization_times =
        validate_online_authorization_contract_v2(config, policy, authorization).map_err(|_| {
            invalid("reviewed fresh credential-slot locator bound online V2 contract is invalid")
        })?;
    let locator_times = locator.validate_intrinsic()?;

    if !v1_config_pins_match(&locator.v1_config, config)
        || !online_policy_pins_match(&locator.online_policy, policy)
        || !online_authorization_pins_match(&locator.online_authorization, authorization)
    {
        return Err(invalid(
            "reviewed fresh credential-slot locator exact config, policy, or authorization binding mismatched",
        ));
    }
    if locator_times.reviewed_at < authorization_times.reviewed_at
        || locator_times.reviewed_at > authorization_times.not_before
        || locator_times.valid_not_before != authorization_times.not_before
        || locator_times.valid_not_after != authorization_times.cleanup_not_after
    {
        return Err(invalid(
            "reviewed fresh credential-slot locator validity does not match the bound authorization envelope",
        ));
    }
    if locator.credential_slot_id != config.value().credential_slot.slot_id
        || locator.credential_slot_nonsecret_fingerprint_sha256
            != config.value().credential_slot.nonsecret_fingerprint_sha256
    {
        return Err(invalid(
            "reviewed fresh credential-slot locator labels do not match the canonical config",
        ));
    }

    Ok(ReviewedFreshCredentialSlotLocatorVerificationV1 {
        schema_version: locator.schema_version,
        locator_id: locator.locator_id.clone(),
        config_fingerprint: config.fingerprint().to_owned(),
        online_policy_fingerprint: policy.fingerprint().to_owned(),
        online_authorization_fingerprint: authorization.fingerprint().to_owned(),
        reviewed_fresh_credential_slot_locator_fingerprint: locator_fingerprint.to_owned(),
        exact_v2_bindings_structurally_valid: true,
        canonical_credential_slot_binding_structurally_valid: true,
        fixed_fresh_credential_locator_structurally_valid: true,
        source_owned_current_time_checked: false,
        protected_credential_directory_and_four_files_checked: false,
        loaded_bundle_matches_credential_slot_generation: false,
        remote_api_key_owner_attested: false,
        locator_fingerprint_pinned_by_v2: false,
        reviewer_authorship_attested: false,
        load_token_consumption_durably_recorded: false,
        authorization_consumption_checked: false,
        authorization: OfflineAuthorizationState::DENIED,
    })
}

#[derive(Debug, Error)]
pub enum PmReviewedFreshCredentialSlotLocatorV1Error {
    #[error("controlled-trial reviewed fresh credential-slot locator V1 is invalid: {0}")]
    Invalid(&'static str),
}

fn invalid(message: &'static str) -> PmReviewedFreshCredentialSlotLocatorV1Error {
    PmReviewedFreshCredentialSlotLocatorV1Error::Invalid(message)
}

fn parse_exact_canonical<T: DeserializeOwned + Serialize>(
    bytes: &[u8],
) -> Result<T, PmReviewedFreshCredentialSlotLocatorV1Error> {
    let value: T = serde_json::from_slice(bytes).map_err(|_| {
        invalid(
            "reviewed fresh credential-slot locator JSON is malformed, duplicated, unknown, or trailing",
        )
    })?;
    let canonical = serde_json::to_vec(&value).map_err(|_| {
        invalid("reviewed fresh credential-slot locator cannot be serialized canonically")
    })?;
    if canonical != bytes {
        return Err(invalid(
            "reviewed fresh credential-slot locator bytes are not exact canonical compact JSON",
        ));
    }
    Ok(value)
}

fn validate_v1_config_pins(
    pins: &V1ConfigPinsV2,
) -> Result<(), PmReviewedFreshCredentialSlotLocatorV1Error> {
    validate_sha256(&pins.canonical_config_sha256)?;
    validate_sha256(&pins.canonical_config_fingerprint)?;
    validate_sha256(&pins.trial_plan_fingerprint)?;
    if pins.canonical_config_length == 0 {
        return Err(invalid(
            "reviewed fresh credential-slot locator V1 canonical config length must be nonzero",
        ));
    }
    Ok(())
}

fn validate_online_policy_pins(
    pins: &OnlinePolicyPinsV2,
) -> Result<(), PmReviewedFreshCredentialSlotLocatorV1Error> {
    validate_sha256(&pins.canonical_sha256)?;
    validate_sha256(&pins.fingerprint)?;
    if pins.canonical_length == 0 {
        return Err(invalid(
            "reviewed fresh credential-slot locator online-policy canonical length must be nonzero",
        ));
    }
    Ok(())
}

fn validate_online_authorization_pins(
    pins: &ReviewedOnlineAuthorizationPinsV1,
) -> Result<(), PmReviewedFreshCredentialSlotLocatorV1Error> {
    validate_token(
        &pins.authorization_id,
        128,
        "reviewed fresh credential-slot locator online-authorization ID is invalid",
    )?;
    validate_sha256(&pins.canonical_sha256)?;
    validate_sha256(&pins.fingerprint)?;
    if pins.canonical_length == 0 {
        return Err(invalid(
            "reviewed fresh credential-slot locator online-authorization canonical length must be nonzero",
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

fn validate_absolute_lexical_directory(
    value: &str,
) -> Result<PathBuf, PmReviewedFreshCredentialSlotLocatorV1Error> {
    if value.is_empty()
        || value.len() > MAX_PROTECTED_FRESH_CREDENTIAL_DIRECTORY_BYTES_V1
        || value.chars().any(char::is_control)
    {
        return Err(invalid(
            "reviewed fresh credential directory is not one canonical absolute UTF-8 path",
        ));
    }
    let path = Path::new(value);
    let mut components = path.components();
    if !path.is_absolute()
        || path == Path::new("/")
        || components.next() != Some(Component::RootDir)
        || components.any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(invalid(
            "reviewed fresh credential directory is not one canonical absolute UTF-8 path",
        ));
    }
    let normalized: PathBuf = path.components().collect();
    if normalized.as_os_str() != OsStr::new(value) {
        return Err(invalid(
            "reviewed fresh credential directory is not one canonical absolute UTF-8 path",
        ));
    }
    Ok(normalized)
}

fn validate_sha256(value: &str) -> Result<(), PmReviewedFreshCredentialSlotLocatorV1Error> {
    if value.len() != 64
        || value
            .bytes()
            .any(|byte| !byte.is_ascii_digit() && !(b'a'..=b'f').contains(&byte))
    {
        return Err(invalid(
            "reviewed fresh credential-slot locator SHA-256 value is invalid",
        ));
    }
    Ok(())
}

fn validate_token(
    value: &str,
    maximum: usize,
    message: &'static str,
) -> Result<(), PmReviewedFreshCredentialSlotLocatorV1Error> {
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
) -> Result<(), PmReviewedFreshCredentialSlotLocatorV1Error> {
    if value.is_empty()
        || value.len() > 512
        || value.bytes().any(|byte| !(0x21..=0x7e).contains(&byte))
    {
        return Err(invalid(message));
    }
    Ok(())
}

fn parse_utc(value: &str) -> Result<DateTime<Utc>, PmReviewedFreshCredentialSlotLocatorV1Error> {
    let parsed = DateTime::parse_from_rfc3339(value)
        .map_err(|_| invalid("reviewed fresh credential-slot locator timestamp is invalid"))?
        .with_timezone(&Utc);
    if parsed.to_rfc3339_opts(chrono::SecondsFormat::Secs, true) != value {
        return Err(invalid(
            "reviewed fresh credential-slot locator timestamp is not canonical UTC seconds",
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
