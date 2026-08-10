//! Unsigned, protected fresh-credential delivery binding V1.
//!
//! This additive controlled-trial sidecar joins the exact canonical reviewed
//! fresh credential-slot locator to caller-recorded provider/generation labels
//! and expected Linux metadata for one directory plus the four fixed roles.
//! Its schema defines no designated secret/value, credential-file-length, or
//! credential-content-digest field. Arbitrary labels cannot be proven
//! secret-free: callers and operators must never place secret material or
//! secret-derived values in any label field.
//!
//! The provider, generation, delivery, time, and Linux-object fields are
//! explicitly unattested labels. Exact protected-file custody establishes only
//! local custody of these canonical JSON bytes; there is no provider signature
//! or lease. Verification neither opens the described directory nor compares
//! the labels to loaded file descriptors. The combined binder consumes the
//! canonical locator itself and issues one local token carrying the
//! path-bearing locator token plus the expected non-secret metadata. Retained
//! evidence keeps the exact canonical locator bytes, including its private path
//! field, but offers no public path projection and has a redacted `Debug`.
//! Initial combined construction needs no process-local `Arc` correlation;
//! after the outputs split, they do not prove same-invocation or same-load
//! identity.
//!
//! A future positive gate needs descriptor-pinned comparison against the
//! actually loaded directory and four files, an authenticated provider
//! signature or lease (or provider-owned descriptor delivery), source-owned
//! time and revocation checks, exact V3 pins, and durable consumption. The
//! signature, lease, or authenticated descriptor-delivery receipt must commit
//! this exact binding fingerprint and ultimately the exact V3 Prepared
//! actor/generation audience; a coexisting generic provider proof is not this
//! conjunction. The
//! frozen V2 authorization and consumption lineages do not pin or consume this
//! sidecar. Every mutation-authority result in this module is permanently
//! DENIED.
//!
//! The only time conjunction is structural nesting: the caller-recorded time
//! starts no earlier than V2 review/start and ends no later than V2
//! `cleanup_not_after`. It is intentionally not an active placement-window,
//! current-time, provider-freshness, or unrevoked-lease check. A future V3
//! must separately pin both locator and delivery records by schema/ID/exact
//! canonical SHA-256/length/domain fingerprint before any positive use.

use std::{fmt, path::Path};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use sha2::{Digest as _, Sha256};
use thiserror::Error;

use crate::{
    CanonicalOnlineAuthorizationV2, CanonicalOnlinePolicyV2,
    CanonicalReviewedFreshCredentialSlotLocatorEvidenceV1,
    CanonicalReviewedFreshCredentialSlotLocatorV1, CanonicalTrialConfig, OfflineAuthorizationState,
    ReviewedFreshCredentialLoadTokenV1, bind_reviewed_fresh_credential_slot_locator_v1,
    online_policy_v2::validate_online_authorization_contract_v2,
    protected_file::{ProtectedFileKind, read_one},
    reviewed_fresh_credential_slot_locator_v1::{
        verify_reviewed_fresh_credential_slot_locator_evidence_v1,
        verify_reviewed_fresh_credential_slot_locator_v1,
    },
};

pub const FRESH_CREDENTIAL_DELIVERY_BINDING_V1_SCHEMA_VERSION: u32 = 1;
pub const PM_T2_FRESH_CREDENTIAL_DELIVERY_BINDING_FILE_V1: &str =
    "pm-t2-fresh-credential-delivery-binding-v1.json";

const MAX_CANONICAL_FRESH_CREDENTIAL_DELIVERY_BINDING_BYTES_V1: usize = 64 * 1024;
const FRESH_CREDENTIAL_DELIVERY_BINDING_V1_FINGERPRINT_DOMAIN: &[u8] =
    b"reap.pm-t2.controlled-trial.fresh-credential-delivery-binding.v1\0";

/// Exact canonical-byte pins for one reviewed fresh credential-slot locator.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FreshCredentialSlotLocatorPinsV1 {
    pub locator_id: String,
    pub canonical_sha256: String,
    pub canonical_length: u64,
    pub fingerprint: String,
}

impl FreshCredentialSlotLocatorPinsV1 {
    fn validate(&self) -> Result<(), PmFreshCredentialDeliveryBindingV1Error> {
        validate_token(
            &self.locator_id,
            128,
            "fresh credential delivery binding locator ID is invalid",
        )?;
        validate_sha256(&self.canonical_sha256)?;
        validate_sha256(&self.fingerprint)?;
        if self.canonical_length == 0 {
            return Err(invalid(
                "fresh credential delivery binding locator canonical length must be nonzero",
            ));
        }
        Ok(())
    }

    fn matches(&self, pins: &FreshCredentialSlotLocatorPinsViewV1<'_>) -> bool {
        self.locator_id == pins.locator_id
            && self.canonical_sha256 == pins.canonical_sha256
            && self.canonical_length == pins.canonical_length
            && self.fingerprint == pins.fingerprint
    }
}

struct FreshCredentialSlotLocatorPinsViewV1<'a> {
    locator_id: &'a str,
    canonical_sha256: &'a str,
    canonical_length: u64,
    fingerprint: &'a str,
}

/// Caller-recorded provider and delivery-generation labels.
///
/// There is deliberately no signature, certificate, lease, or provider-owned
/// transport attached to these values. Structural validation is not provider
/// authorship, generation truth, freshness, revocation, or global uniqueness.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UnattestedFreshCredentialProviderGenerationV1 {
    pub provider_id: String,
    pub provider_key_id: String,
    pub rotation_namespace_id: String,
    pub delivery_id: String,
    pub rotation_generation: u64,
}

impl UnattestedFreshCredentialProviderGenerationV1 {
    fn validate(&self) -> Result<(), PmFreshCredentialDeliveryBindingV1Error> {
        for (value, message) in [
            (
                self.provider_id.as_str(),
                "fresh credential delivery binding provider ID label is invalid",
            ),
            (
                self.provider_key_id.as_str(),
                "fresh credential delivery binding provider key ID label is invalid",
            ),
            (
                self.rotation_namespace_id.as_str(),
                "fresh credential delivery binding rotation namespace label is invalid",
            ),
            (
                self.delivery_id.as_str(),
                "fresh credential delivery binding delivery ID label is invalid",
            ),
        ] {
            validate_token(value, 128, message)?;
        }
        if self.rotation_generation == 0 {
            return Err(invalid(
                "fresh credential delivery binding rotation generation label must be nonzero",
            ));
        }
        Ok(())
    }
}

/// Caller-recorded Linux identity metadata for the protected directory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FreshCredentialLinuxDirectoryIdentityV1 {
    pub filesystem_device: u64,
    pub inode: u64,
    pub owner_uid: u32,
    pub permission_mode: u32,
    pub modified_seconds: i64,
    pub modified_nanoseconds: i64,
    pub status_changed_seconds: i64,
    pub status_changed_nanoseconds: i64,
}

impl FreshCredentialLinuxDirectoryIdentityV1 {
    fn validate(&self) -> Result<(), PmFreshCredentialDeliveryBindingV1Error> {
        validate_linux_identity(
            self.inode,
            self.permission_mode,
            0o700,
            FreshCredentialLinuxTimesViewV1 {
                modified_seconds: self.modified_seconds,
                modified_nanoseconds: self.modified_nanoseconds,
                status_changed_seconds: self.status_changed_seconds,
                status_changed_nanoseconds: self.status_changed_nanoseconds,
            },
            "fresh credential delivery binding directory metadata identity is invalid",
        )
    }

    fn inode_key(&self) -> (u64, u64) {
        (self.filesystem_device, self.inode)
    }
}

/// Caller-recorded Linux identity metadata for one fixed credential role.
///
/// File length and credential/content hashes are deliberately absent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FreshCredentialLinuxFileIdentityV1 {
    pub filesystem_device: u64,
    pub inode: u64,
    pub owner_uid: u32,
    pub permission_mode: u32,
    pub hard_link_count: u64,
    pub modified_seconds: i64,
    pub modified_nanoseconds: i64,
    pub status_changed_seconds: i64,
    pub status_changed_nanoseconds: i64,
}

impl FreshCredentialLinuxFileIdentityV1 {
    fn validate(&self) -> Result<(), PmFreshCredentialDeliveryBindingV1Error> {
        validate_linux_identity(
            self.inode,
            self.permission_mode,
            0o600,
            FreshCredentialLinuxTimesViewV1 {
                modified_seconds: self.modified_seconds,
                modified_nanoseconds: self.modified_nanoseconds,
                status_changed_seconds: self.status_changed_seconds,
                status_changed_nanoseconds: self.status_changed_nanoseconds,
            },
            "fresh credential delivery binding file metadata identity is invalid",
        )?;
        if self.hard_link_count != 1 {
            return Err(invalid(
                "fresh credential delivery binding file hard-link count must be exactly one",
            ));
        }
        Ok(())
    }

    fn inode_key(&self) -> (u64, u64) {
        (self.filesystem_device, self.inode)
    }
}

/// Closed role-keyed metadata map matching the locator's four fixed basenames.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FreshCredentialLinuxFileIdentitiesV1 {
    pub private_key: FreshCredentialLinuxFileIdentityV1,
    pub api_key: FreshCredentialLinuxFileIdentityV1,
    pub l2_secret: FreshCredentialLinuxFileIdentityV1,
    pub passphrase: FreshCredentialLinuxFileIdentityV1,
}

/// Caller-recorded expected Linux object metadata carried toward a later
/// descriptor-pinned loader. This is not an observation or attestation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FreshCredentialLinuxObjectSetV1 {
    pub directory: FreshCredentialLinuxDirectoryIdentityV1,
    pub files: FreshCredentialLinuxFileIdentitiesV1,
}

impl FreshCredentialLinuxObjectSetV1 {
    fn validate(&self) -> Result<(), PmFreshCredentialDeliveryBindingV1Error> {
        self.directory.validate()?;
        let files = [
            &self.files.private_key,
            &self.files.api_key,
            &self.files.l2_secret,
            &self.files.passphrase,
        ];
        for file in files {
            file.validate()?;
            if file.filesystem_device != self.directory.filesystem_device
                || file.owner_uid != self.directory.owner_uid
                || file.inode_key() == self.directory.inode_key()
            {
                return Err(invalid(
                    "fresh credential delivery binding Linux object metadata is not one same-owner filesystem set",
                ));
            }
        }
        for left in 0..files.len() {
            for right in left + 1..files.len() {
                if files[left].inode_key() == files[right].inode_key() {
                    return Err(invalid(
                        "fresh credential delivery binding credential roles reuse one inode identity",
                    ));
                }
            }
        }
        Ok(())
    }
}

/// Unsigned canonical record for one exact locator and one caller-labeled
/// provider generation / expected Linux object set.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FreshCredentialDeliveryBindingV1 {
    pub schema_version: u32,
    pub binding_id: String,
    pub unattested_delivery_recorded_at_utc: String,
    pub unattested_valid_not_before_utc: String,
    pub unattested_valid_not_after_utc: String,
    pub reviewed_fresh_credential_slot_locator: FreshCredentialSlotLocatorPinsV1,
    pub unattested_provider_generation: UnattestedFreshCredentialProviderGenerationV1,
    pub unattested_linux_objects: FreshCredentialLinuxObjectSetV1,
}

impl fmt::Debug for FreshCredentialDeliveryBindingV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(
            "FreshCredentialDeliveryBindingV1(<unsigned-provider-generation-and-linux-metadata-labels; redacted; denied>)",
        )
    }
}

struct ValidatedFreshCredentialDeliveryBindingTimesV1 {
    recorded_at: DateTime<Utc>,
    valid_not_before: DateTime<Utc>,
    valid_not_after: DateTime<Utc>,
}

impl FreshCredentialDeliveryBindingV1 {
    fn validate_intrinsic(
        &self,
    ) -> Result<
        ValidatedFreshCredentialDeliveryBindingTimesV1,
        PmFreshCredentialDeliveryBindingV1Error,
    > {
        if self.schema_version != FRESH_CREDENTIAL_DELIVERY_BINDING_V1_SCHEMA_VERSION {
            return Err(invalid(
                "unsupported fresh credential delivery binding V1 schema",
            ));
        }
        validate_token(
            &self.binding_id,
            128,
            "fresh credential delivery binding ID is invalid",
        )?;
        self.reviewed_fresh_credential_slot_locator.validate()?;
        self.unattested_provider_generation.validate()?;
        self.unattested_linux_objects.validate()?;

        let recorded_at = parse_utc(&self.unattested_delivery_recorded_at_utc)?;
        let valid_not_before = parse_utc(&self.unattested_valid_not_before_utc)?;
        let valid_not_after = parse_utc(&self.unattested_valid_not_after_utc)?;
        if recorded_at > valid_not_before || valid_not_before >= valid_not_after {
            return Err(invalid(
                "fresh credential delivery binding unattested time labels are invalid",
            ));
        }
        Ok(ValidatedFreshCredentialDeliveryBindingTimesV1 {
            recorded_at,
            valid_not_before,
            valid_not_after,
        })
    }
}

/// Move-only, redacted holder of the exact protected canonical binding bytes.
pub struct CanonicalFreshCredentialDeliveryBindingV1 {
    value: FreshCredentialDeliveryBindingV1,
    canonical_bytes: Vec<u8>,
    canonical_sha256: String,
    fingerprint: String,
}

impl CanonicalFreshCredentialDeliveryBindingV1 {
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

impl fmt::Debug for CanonicalFreshCredentialDeliveryBindingV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(
            "CanonicalFreshCredentialDeliveryBindingV1(<unsigned-exact-canonical-bytes; redacted; denied>)",
        )
    }
}

/// Move-only retained evidence after the path-bearing token has been split.
///
/// Exact binding bytes and exact locator evidence remain private for
/// reverification. No path, signer, Linux metadata, provider label, generation
/// label, or load capability can be projected from this holder.
pub struct CanonicalFreshCredentialDeliveryBindingEvidenceV1 {
    value: FreshCredentialDeliveryBindingV1,
    canonical_bytes: Vec<u8>,
    canonical_sha256: String,
    fingerprint: String,
    reviewed_fresh_credential_slot_locator_evidence:
        CanonicalReviewedFreshCredentialSlotLocatorEvidenceV1,
}

impl CanonicalFreshCredentialDeliveryBindingEvidenceV1 {
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

impl fmt::Debug for CanonicalFreshCredentialDeliveryBindingEvidenceV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(
            "CanonicalFreshCredentialDeliveryBindingEvidenceV1(<retained-exact-evidence; no-path-or-provider-projection; redacted; denied>)",
        )
    }
}

/// One-shot local carrier toward a future descriptor-pinned loader.
///
/// The nested locator token is the only path/signer projection and its signer
/// was derived solely from the exact canonical config. Consuming this token
/// returns ordinary cloneable metadata and a fingerprint, so it provides no
/// durable consumption, same-holder guarantee, global uniqueness, or mutation
/// authority by itself.
#[must_use = "a fresh credential delivery load token must be consumed or deliberately dropped"]
pub struct FreshCredentialDeliveryLoadTokenV1 {
    reviewed_fresh_credential_load_token: ReviewedFreshCredentialLoadTokenV1,
    expected_linux_objects: FreshCredentialLinuxObjectSetV1,
    fresh_credential_delivery_binding_fingerprint: String,
}

impl FreshCredentialDeliveryLoadTokenV1 {
    #[must_use = "consuming the delivery token yields the exact locator token, expected metadata, and binding fingerprint"]
    pub fn into_parts(
        self,
    ) -> (
        ReviewedFreshCredentialLoadTokenV1,
        FreshCredentialLinuxObjectSetV1,
        String,
    ) {
        (
            self.reviewed_fresh_credential_load_token,
            self.expected_linux_objects,
            self.fresh_credential_delivery_binding_fingerprint,
        )
    }
}

impl fmt::Debug for FreshCredentialDeliveryLoadTokenV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(
            "FreshCredentialDeliveryLoadTokenV1(<one-shot-local-load; path-signer-and-metadata-redacted; denied>)",
        )
    }
}

/// Offline structural display only. Every provider, live-object, freshness,
/// consumption, and mutation-authority claim remains false.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FreshCredentialDeliveryBindingVerificationV1 {
    pub schema_version: u32,
    pub binding_id: String,
    pub config_fingerprint: String,
    pub online_policy_fingerprint: String,
    pub online_authorization_fingerprint: String,
    pub reviewed_fresh_credential_slot_locator_fingerprint: String,
    pub fresh_credential_delivery_binding_fingerprint: String,
    pub exact_reviewed_locator_pins_structurally_valid: bool,
    pub unattested_provider_generation_labels_structurally_valid: bool,
    pub unattested_linux_object_metadata_labels_structurally_valid: bool,
    pub unattested_validity_labels_nested_within_v2: bool,
    pub source_owned_current_time_checked: bool,
    pub protected_credential_directory_and_four_files_checked: bool,
    pub loaded_linux_objects_match_unattested_binding: bool,
    pub same_loaded_holder_attested: bool,
    pub globally_unique_delivery_attested: bool,
    pub provider_authorship_attested: bool,
    pub provider_signature_verified: bool,
    pub provider_lease_fresh_and_unrevoked: bool,
    pub rotation_generation_attested: bool,
    pub delivery_freshness_attested: bool,
    pub loaded_bundle_matches_credential_slot_generation: bool,
    pub remote_api_key_owner_attested: bool,
    pub locator_fingerprint_pinned_by_v2: bool,
    pub delivery_binding_fingerprint_pinned_by_v2: bool,
    pub delivery_consumption_durably_recorded: bool,
    pub authorization_consumption_checked: bool,
    pub credential_mutation_authority_attested: bool,
    #[serde(flatten)]
    pub authorization: OfflineAuthorizationState,
}

pub fn load_canonical_fresh_credential_delivery_binding_v1(
    path: &Path,
) -> Result<CanonicalFreshCredentialDeliveryBindingV1, PmFreshCredentialDeliveryBindingV1Error> {
    let bytes = read_one(
        path,
        ProtectedFileKind::FreshCredentialDeliveryBindingV1,
        MAX_CANONICAL_FRESH_CREDENTIAL_DELIVERY_BINDING_BYTES_V1,
    )
    .map_err(|_| {
        invalid("fresh credential delivery binding protection or stability check failed")
    })?;
    let value: FreshCredentialDeliveryBindingV1 = parse_exact_canonical(&bytes)?;
    let _ = value.validate_intrinsic()?;
    let canonical_bytes = bytes.to_vec();
    Ok(CanonicalFreshCredentialDeliveryBindingV1 {
        canonical_sha256: hash_bytes(&[], &canonical_bytes),
        fingerprint: hash_bytes(
            FRESH_CREDENTIAL_DELIVERY_BINDING_V1_FINGERPRINT_DOMAIN,
            &canonical_bytes,
        ),
        canonical_bytes,
        value,
    })
}

/// Verify the unsigned exact-byte conjunction without consulting a caller
/// clock or opening any described credential object.
pub fn verify_fresh_credential_delivery_binding_v1(
    config: &CanonicalTrialConfig,
    policy: &CanonicalOnlinePolicyV2,
    authorization: &CanonicalOnlineAuthorizationV2,
    locator: &CanonicalReviewedFreshCredentialSlotLocatorV1,
    binding: &CanonicalFreshCredentialDeliveryBindingV1,
) -> Result<FreshCredentialDeliveryBindingVerificationV1, PmFreshCredentialDeliveryBindingV1Error> {
    let locator_verification =
        verify_reviewed_fresh_credential_slot_locator_v1(config, policy, authorization, locator)
            .map_err(|_| {
                invalid("fresh credential delivery binding reviewed locator is invalid")
            })?;
    let locator_pins = FreshCredentialSlotLocatorPinsViewV1 {
        locator_id: &locator_verification.locator_id,
        canonical_sha256: locator.canonical_sha256(),
        canonical_length: locator.canonical_length(),
        fingerprint: locator.fingerprint(),
    };
    verify_fresh_credential_delivery_binding_value_v1(
        config,
        policy,
        authorization,
        locator_pins,
        &binding.value,
        &binding.fingerprint,
    )
}

/// Consume the exact canonical locator and binding into non-projecting retained
/// evidence plus one local load token.
///
/// No separately supplied locator evidence, locator token, signer, provider
/// proof, or load-instance correlation value is accepted.
pub fn bind_fresh_credential_delivery_binding_v1(
    config: &CanonicalTrialConfig,
    policy: &CanonicalOnlinePolicyV2,
    authorization: &CanonicalOnlineAuthorizationV2,
    locator: CanonicalReviewedFreshCredentialSlotLocatorV1,
    binding: CanonicalFreshCredentialDeliveryBindingV1,
) -> Result<
    (
        CanonicalFreshCredentialDeliveryBindingEvidenceV1,
        FreshCredentialDeliveryLoadTokenV1,
    ),
    PmFreshCredentialDeliveryBindingV1Error,
> {
    let _ = verify_fresh_credential_delivery_binding_v1(
        config,
        policy,
        authorization,
        &locator,
        &binding,
    )?;
    let expected_linux_objects = binding.value.unattested_linux_objects.clone();
    let delivery_fingerprint_for_token = binding.fingerprint.clone();
    let (locator_evidence, reviewed_fresh_credential_load_token) =
        bind_reviewed_fresh_credential_slot_locator_v1(config, policy, authorization, locator)
            .map_err(|_| {
                invalid("fresh credential delivery binding could not consume exact locator")
            })?;
    let CanonicalFreshCredentialDeliveryBindingV1 {
        value,
        canonical_bytes,
        canonical_sha256,
        fingerprint,
    } = binding;
    Ok((
        CanonicalFreshCredentialDeliveryBindingEvidenceV1 {
            value,
            canonical_bytes,
            canonical_sha256,
            fingerprint,
            reviewed_fresh_credential_slot_locator_evidence: locator_evidence,
        },
        FreshCredentialDeliveryLoadTokenV1 {
            reviewed_fresh_credential_load_token,
            expected_linux_objects,
            fresh_credential_delivery_binding_fingerprint: delivery_fingerprint_for_token,
        },
    ))
}

/// Reverify retained exact-byte evidence without minting another path-bearing
/// token or projecting provider/generation/Linux metadata labels.
pub fn verify_fresh_credential_delivery_binding_evidence_v1(
    config: &CanonicalTrialConfig,
    policy: &CanonicalOnlinePolicyV2,
    authorization: &CanonicalOnlineAuthorizationV2,
    evidence: &CanonicalFreshCredentialDeliveryBindingEvidenceV1,
) -> Result<FreshCredentialDeliveryBindingVerificationV1, PmFreshCredentialDeliveryBindingV1Error> {
    let locator_verification = verify_reviewed_fresh_credential_slot_locator_evidence_v1(
        config,
        policy,
        authorization,
        &evidence.reviewed_fresh_credential_slot_locator_evidence,
    )
    .map_err(|_| invalid("fresh credential delivery binding retained locator is invalid"))?;
    let locator_pins = FreshCredentialSlotLocatorPinsViewV1 {
        locator_id: &locator_verification.locator_id,
        canonical_sha256: evidence
            .reviewed_fresh_credential_slot_locator_evidence
            .canonical_sha256(),
        canonical_length: evidence
            .reviewed_fresh_credential_slot_locator_evidence
            .canonical_length(),
        fingerprint: evidence
            .reviewed_fresh_credential_slot_locator_evidence
            .fingerprint(),
    };
    verify_fresh_credential_delivery_binding_value_v1(
        config,
        policy,
        authorization,
        locator_pins,
        &evidence.value,
        &evidence.fingerprint,
    )
}

fn verify_fresh_credential_delivery_binding_value_v1(
    config: &CanonicalTrialConfig,
    policy: &CanonicalOnlinePolicyV2,
    authorization: &CanonicalOnlineAuthorizationV2,
    locator_pins: FreshCredentialSlotLocatorPinsViewV1<'_>,
    binding: &FreshCredentialDeliveryBindingV1,
    binding_fingerprint: &str,
) -> Result<FreshCredentialDeliveryBindingVerificationV1, PmFreshCredentialDeliveryBindingV1Error> {
    let authorization_times =
        validate_online_authorization_contract_v2(config, policy, authorization).map_err(|_| {
            invalid("fresh credential delivery binding bound online V2 contract is invalid")
        })?;
    let binding_times = binding.validate_intrinsic()?;
    if !binding
        .reviewed_fresh_credential_slot_locator
        .matches(&locator_pins)
    {
        return Err(invalid(
            "fresh credential delivery binding exact reviewed locator pins mismatched",
        ));
    }
    if binding_times.recorded_at < authorization_times.reviewed_at
        || binding_times.valid_not_before < authorization_times.not_before
        || binding_times.valid_not_after > authorization_times.cleanup_not_after
    {
        return Err(invalid(
            "fresh credential delivery binding unattested validity labels are outside the V2 envelope",
        ));
    }
    if binding.unattested_linux_objects.directory.owner_uid != authorization.value().host.linux_euid
    {
        return Err(invalid(
            "fresh credential delivery binding Linux object owner label differs from the exact authorized EUID",
        ));
    }

    Ok(FreshCredentialDeliveryBindingVerificationV1 {
        schema_version: binding.schema_version,
        binding_id: binding.binding_id.clone(),
        config_fingerprint: config.fingerprint().to_owned(),
        online_policy_fingerprint: policy.fingerprint().to_owned(),
        online_authorization_fingerprint: authorization.fingerprint().to_owned(),
        reviewed_fresh_credential_slot_locator_fingerprint: locator_pins.fingerprint.to_owned(),
        fresh_credential_delivery_binding_fingerprint: binding_fingerprint.to_owned(),
        exact_reviewed_locator_pins_structurally_valid: true,
        unattested_provider_generation_labels_structurally_valid: true,
        unattested_linux_object_metadata_labels_structurally_valid: true,
        unattested_validity_labels_nested_within_v2: true,
        source_owned_current_time_checked: false,
        protected_credential_directory_and_four_files_checked: false,
        loaded_linux_objects_match_unattested_binding: false,
        same_loaded_holder_attested: false,
        globally_unique_delivery_attested: false,
        provider_authorship_attested: false,
        provider_signature_verified: false,
        provider_lease_fresh_and_unrevoked: false,
        rotation_generation_attested: false,
        delivery_freshness_attested: false,
        loaded_bundle_matches_credential_slot_generation: false,
        remote_api_key_owner_attested: false,
        locator_fingerprint_pinned_by_v2: false,
        delivery_binding_fingerprint_pinned_by_v2: false,
        delivery_consumption_durably_recorded: false,
        authorization_consumption_checked: false,
        credential_mutation_authority_attested: false,
        authorization: OfflineAuthorizationState::DENIED,
    })
}

#[derive(Debug, Error)]
pub enum PmFreshCredentialDeliveryBindingV1Error {
    #[error("controlled-trial fresh credential delivery binding V1 is invalid: {0}")]
    Invalid(&'static str),
}

fn invalid(message: &'static str) -> PmFreshCredentialDeliveryBindingV1Error {
    PmFreshCredentialDeliveryBindingV1Error::Invalid(message)
}

fn parse_exact_canonical<T: DeserializeOwned + Serialize>(
    bytes: &[u8],
) -> Result<T, PmFreshCredentialDeliveryBindingV1Error> {
    let value: T = serde_json::from_slice(bytes).map_err(|_| {
        invalid(
            "fresh credential delivery binding JSON is malformed, duplicated, unknown, or trailing",
        )
    })?;
    let canonical = serde_json::to_vec(&value).map_err(|_| {
        invalid("fresh credential delivery binding cannot be serialized canonically")
    })?;
    if canonical != bytes {
        return Err(invalid(
            "fresh credential delivery binding bytes are not exact canonical compact JSON",
        ));
    }
    Ok(value)
}

struct FreshCredentialLinuxTimesViewV1 {
    modified_seconds: i64,
    modified_nanoseconds: i64,
    status_changed_seconds: i64,
    status_changed_nanoseconds: i64,
}

fn validate_linux_identity(
    inode: u64,
    permission_mode: u32,
    required_permission_mode: u32,
    times: FreshCredentialLinuxTimesViewV1,
    message: &'static str,
) -> Result<(), PmFreshCredentialDeliveryBindingV1Error> {
    if inode == 0
        || permission_mode != required_permission_mode
        || times.modified_seconds < 0
        || !(0..1_000_000_000).contains(&times.modified_nanoseconds)
        || times.status_changed_seconds < 0
        || !(0..1_000_000_000).contains(&times.status_changed_nanoseconds)
    {
        return Err(invalid(message));
    }
    Ok(())
}

fn validate_sha256(value: &str) -> Result<(), PmFreshCredentialDeliveryBindingV1Error> {
    if value.len() != 64
        || value
            .bytes()
            .any(|byte| !byte.is_ascii_digit() && !(b'a'..=b'f').contains(&byte))
    {
        return Err(invalid(
            "fresh credential delivery binding SHA-256 value is invalid",
        ));
    }
    Ok(())
}

fn validate_token(
    value: &str,
    maximum: usize,
    message: &'static str,
) -> Result<(), PmFreshCredentialDeliveryBindingV1Error> {
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

fn parse_utc(value: &str) -> Result<DateTime<Utc>, PmFreshCredentialDeliveryBindingV1Error> {
    let parsed = DateTime::parse_from_rfc3339(value)
        .map_err(|_| invalid("fresh credential delivery binding timestamp is invalid"))?
        .with_timezone(&Utc);
    if parsed.to_rfc3339_opts(chrono::SecondsFormat::Secs, true) != value {
        return Err(invalid(
            "fresh credential delivery binding timestamp is not canonical UTC seconds",
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
