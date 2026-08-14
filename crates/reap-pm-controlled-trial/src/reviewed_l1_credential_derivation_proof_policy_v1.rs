//! Offline reviewed L1 credential-derivation proof policy V1.
//!
//! This additive sidecar is a permanently denied structural policy for one
//! future `GET /auth/derive-api-key` L1-control proof and a local equality
//! association between the exact three returned credential fields and one
//! already loaded L2 credential holder. It is deliberately distinct from the
//! frozen same-holder L2 remote-acceptance policy: neither policy is response
//! evidence for the other, and this policy does not amend any V1, V2, V3, V4,
//! local-custody, proxy-control, or remote-acceptance source.
//!
//! All artifact commitments are derived inside this module from borrowed
//! canonical holders. There is no caller digest, verification report, proof
//! DTO, boolean, clock reading, observed timestamp, signature, header bytes,
//! body bytes, API UUID, API key, L2 secret, passphrase, secret/signature
//! digest, response hash, response length, or response value in the record.
//! The canonical holder is move-only, non-Serde, redacted, and projects only
//! its SHA-256, byte length, and domain-separated fingerprint.
//!
//! The official raw-manifest and `api_authentication` entry pins below are
//! labels copied from the frozen source manifest. This module does not load or
//! hash those source bytes and does not authenticate their publisher or this
//! policy's reviewer. Those pins do not establish no-cache response semantics.
//! They do not establish nonshared response semantics.
//! They do not establish authentication before handler execution.
//! They do not establish credential current-and-unrevoked semantics.
//!
//! Every protocol field is a requirement for a future sealed actor, not an
//! observation or performed operation. A future source-owned `/time` sample
//! or an equivalent source-owned proof must create the ten-digit timestamp,
//! and one monotonic interval must remain at most 5,000 ms through both send
//! and receive. This module has no current clock, signer, credential value,
//! parser input, socket, TLS client, transport, runtime actor, journal writer,
//! consumption claim, or mutation capability. Authorization is always DENIED
//! and every dispatch allowance is zero.

use std::{fmt, net::IpAddr, path::Path, str::FromStr as _};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use sha2::{Digest as _, Sha256};
use thiserror::Error;

use crate::{
    CanonicalReviewedLocalOperatorCooperativeCustodyProfileV1,
    CanonicalReviewedPhaseAEligibilityEnvelopeV4,
    FRESH_CREDENTIAL_DELIVERY_BINDING_V1_SCHEMA_VERSION, ONLINE_AUTHORIZATION_V2_SCHEMA_VERSION,
    ONLINE_POLICY_V2_SCHEMA_VERSION, OfflineAuthorizationState,
    PM_T2_OFFICIAL_SOURCE_MANIFEST_BYTE_LENGTH_V1,
    PM_T2_OFFICIAL_SOURCE_MANIFEST_RETRIEVED_AT_UTC_V1,
    PM_T2_OFFICIAL_SOURCE_MANIFEST_SCHEMA_FAMILY_V1,
    PM_T2_OFFICIAL_SOURCE_MANIFEST_SCHEMA_VERSION_V1, PM_T2_OFFICIAL_SOURCE_MANIFEST_SHA256_V1,
    REVIEWED_LOCAL_OPERATOR_COOPERATIVE_CUSTODY_PROFILE_V1_SCHEMA_VERSION,
    REVIEWED_PHASE_A_ELIGIBILITY_ENVELOPE_V4_SCHEMA_VERSION,
    REVIEWED_PRODUCTION_DESTINATION_PROFILE_V1_SCHEMA_VERSION,
    REVIEWED_REMOTE_CREDENTIAL_PROOF_POLICY_V1_SCHEMA_VERSION,
    REVIEWED_SIGNER_PROXY_ACCOUNT_IDENTITY_V1_SCHEMA_VERSION,
    REVIEWED_STATIC_ONLINE_AUTHORIZATION_V3_SCHEMA_VERSION,
    ReviewedLocalOperatorCooperativeCustodyProfileContextV1, ReviewedOfficialSourceManifestPinsV1,
    ReviewedPhaseAEligibilityEnvelopeContextV4, TrialPhase,
    config::TRIAL_CONFIG_SCHEMA_VERSION,
    protected_file::{ProtectedFileKind, read_one},
    verify_reviewed_local_operator_cooperative_custody_profile_v1,
    verify_reviewed_phase_a_eligibility_envelope_v4,
};

pub const REVIEWED_L1_CREDENTIAL_DERIVATION_PROOF_POLICY_V1_SCHEMA_VERSION: u32 = 1;
pub const PM_T2_REVIEWED_L1_CREDENTIAL_DERIVATION_PROOF_POLICY_FILE_V1: &str =
    "pm-t2-reviewed-l1-credential-derivation-proof-policy-v1.json";

const REVIEWED_L1_CREDENTIAL_DERIVATION_PROOF_POLICY_ID_V1: &str =
    "pm-t2-reviewed-l1-credential-derivation-proof-policy-v1";
const MAX_CANONICAL_REVIEWED_L1_CREDENTIAL_DERIVATION_PROOF_POLICY_BYTES_V1: usize = 160 * 1024;
const REVIEWED_L1_CREDENTIAL_DERIVATION_PROOF_POLICY_V1_FINGERPRINT_DOMAIN: &[u8] =
    b"reap.pm-t2.controlled-trial.reviewed-l1-credential-derivation-proof-policy.v1\0";

const API_AUTHENTICATION_SOURCE_ID_V1: &str = "api_authentication";
const API_AUTHENTICATION_SOURCE_URL_V1: &str = "https://docs.polymarket.com/getting-started/api.md";
const API_AUTHENTICATION_SOURCE_BYTE_LENGTH_V1: u64 = 9_391;
const API_AUTHENTICATION_SOURCE_SHA256_V1: &str =
    "6c397c66109852220b3f5d8033ea274061b3fc44b426edc9faa60673ecbef8fc";
const OFFICIAL_SOURCE_CONTENT_TYPE_V1: &str = "text/markdown; charset=utf-8";

const PROTOCOL_SCHEME_V1: &str = "https";
const CLOB_DNS_NAME_V1: &str = "clob.polymarket.com";
const CLOB_TCP_PORT_V1: u16 = 443;
const DERIVE_METHOD_V1: &str = "GET";
const DERIVE_PATH_V1: &str = "/auth/derive-api-key";
const SERVER_TIME_PATH_V1: &str = "/time";
const ABSENT_COMPONENT_V1: &str = "absent";
const ACCEPT_APPLICATION_JSON_V1: &str = "application/json";
const ACCEPT_ENCODING_IDENTITY_V1: &str = "identity";
const EIP712_STANDARD_V1: &str = "EIP-712";
const CLOB_AUTH_DOMAIN_NAME_V1: &str = "ClobAuthDomain";
const CLOB_AUTH_DOMAIN_VERSION_V1: &str = "1";
const CLOB_AUTH_CHAIN_ID_V1: u64 = 137;
const CLOB_AUTH_PRIMARY_TYPE_V1: &str = "ClobAuth";
const CLOB_AUTH_DOMAIN_TYPE_V1: &str = "EIP712Domain(string name,string version,uint256 chainId)";
const CLOB_AUTH_STRUCT_TYPE_V1: &str =
    "ClobAuth(address address,string timestamp,uint256 nonce,string message)";
const CLOB_AUTH_MESSAGE_V1: &str = "This message attests that I control the given wallet";
const CLOB_AUTH_ADDRESS_SOURCE_V1: &str = "canonical_config_account_signer_eip55";
const CLOB_AUTH_TIMESTAMP_SOURCE_V1: &str =
    "future_source_owned_clob_time_or_equivalent_unix_seconds_decimal_ascii";
const CLOB_AUTH_SIGNATURE_SOURCE_V1: &str = "future_exact_eip712_clob_auth_signature";
const CLOB_AUTH_NONCE_SOURCE_V1: &str = "exact_zero_explicit_reviewed_v1_not_inferred";
const TIMESTAMP_DECIMAL_DIGITS_V1: u8 = 10;
const MINIMUM_TIMESTAMP_UNIX_SECONDS_V1: u64 = 1_000_000_000;
const MAXIMUM_TIMESTAMP_UNIX_SECONDS_V1: u64 = 9_999_999_999;
const EXACT_NONCE_V1: u64 = 0;
const MAXIMUM_TIMESTAMP_CREATION_TO_SEND_MS_V1: u64 = 5_000;
const MAXIMUM_TIMESTAMP_CREATION_TO_RECEIVE_MS_V1: u64 = 5_000;
const CONNECT_TIMEOUT_MS_V1: u64 = 3_000;
const REQUEST_TIMEOUT_MS_V1: u64 = 5_000;
const REQUIRED_CONTENT_TYPE_ESSENCE_V1: &str = "application/json";
const ALLOWED_CONTENT_TYPE_CHARSET_V1: &str =
    "none_or_exactly_one_charset_utf8_ascii_case_insensitive";
const ALLOWED_CONTENT_ENCODING_V1: &str = "absent_or_exactly_one_identity_ascii_case_insensitive";
const MAXIMUM_BODY_BYTES_V1: u64 = 1_024;
const RESPONSE_API_KEY_FIELD_V1: &str = "apiKey";
const RESPONSE_SECRET_FIELD_V1: &str = "secret";
const RESPONSE_PASSPHRASE_FIELD_V1: &str = "passphrase";
const RESPONSE_FIELD_TYPE_V1: &str = "json_string";
const RESPONSE_API_KEY_ASSOCIATION_V1: &str =
    "returned_apiKey_exact_local_equality_to_same_loaded_l2_holder_api_key";
const RESPONSE_SECRET_ASSOCIATION_V1: &str =
    "returned_secret_exact_local_equality_to_same_loaded_l2_holder_secret";
const RESPONSE_PASSPHRASE_ASSOCIATION_V1: &str =
    "returned_passphrase_exact_local_equality_to_same_loaded_l2_holder_passphrase";

/// Exact V1 config role. The type cannot be substituted for another pin role.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewedL1CredentialDerivationConfigPinsV1 {
    pub schema_version: u32,
    pub canonical_sha256: String,
    pub canonical_length: u64,
    pub fingerprint: String,
    pub plan_fingerprint: String,
}

/// Exact online-policy V2 role.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewedL1CredentialDerivationOnlinePolicyPinsV1 {
    pub schema_version: u32,
    pub canonical_sha256: String,
    pub canonical_length: u64,
    pub fingerprint: String,
}

/// Exact online-authorization V2 role.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewedL1CredentialDerivationOnlineAuthorizationPinsV1 {
    pub schema_version: u32,
    pub canonical_sha256: String,
    pub canonical_length: u64,
    pub fingerprint: String,
}

/// Exact reviewed production-destination V1 role.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewedL1CredentialDerivationDestinationPinsV1 {
    pub schema_version: u32,
    pub canonical_sha256: String,
    pub canonical_length: u64,
    pub fingerprint: String,
}

/// Exact fresh-credential delivery-binding V1 role.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewedL1CredentialDerivationDeliveryPinsV1 {
    pub schema_version: u32,
    pub canonical_sha256: String,
    pub canonical_length: u64,
    pub fingerprint: String,
}

/// Exact reviewed signer/proxy account-identity V1 role.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewedL1CredentialDerivationAccountIdentityPinsV1 {
    pub schema_version: u32,
    pub canonical_sha256: String,
    pub canonical_length: u64,
    pub fingerprint: String,
}

/// Exact frozen same-holder L2 remote-proof policy V1 role.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewedL1CredentialDerivationRemoteProofPolicyPinsV1 {
    pub schema_version: u32,
    pub canonical_sha256: String,
    pub canonical_length: u64,
    pub fingerprint: String,
}

/// Exact reviewed static-online-authorization V3 role.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewedL1CredentialDerivationStaticAuthorizationPinsV1 {
    pub schema_version: u32,
    pub canonical_sha256: String,
    pub canonical_length: u64,
    pub fingerprint: String,
}

/// Exact reviewed Phase-A eligibility-envelope V4 role.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewedL1CredentialDerivationEligibilityEnvelopePinsV1 {
    pub schema_version: u32,
    pub canonical_sha256: String,
    pub canonical_length: u64,
    pub fingerprint: String,
}

/// Exact reviewed local cooperative-custody profile V1 role.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewedL1CredentialDerivationLocalCustodyProfilePinsV1 {
    pub schema_version: u32,
    pub canonical_sha256: String,
    pub canonical_length: u64,
    pub fingerprint: String,
}

struct ExactPinViewV1<'a> {
    schema_version: u32,
    expected_schema_version: u32,
    canonical_sha256: &'a str,
    canonical_length: u64,
    fingerprint: &'a str,
}

macro_rules! impl_exact_role_pin_validation {
    ($pin:ty, $schema:expr, $message:literal) => {
        impl $pin {
            fn validate(&self) -> Result<(), PmReviewedL1CredentialDerivationProofPolicyV1Error> {
                validate_exact_pin(
                    ExactPinViewV1 {
                        schema_version: self.schema_version,
                        expected_schema_version: $schema,
                        canonical_sha256: &self.canonical_sha256,
                        canonical_length: self.canonical_length,
                        fingerprint: &self.fingerprint,
                    },
                    $message,
                )
            }
        }
    };
}

impl_exact_role_pin_validation!(
    ReviewedL1CredentialDerivationOnlinePolicyPinsV1,
    ONLINE_POLICY_V2_SCHEMA_VERSION,
    "reviewed L1 credential-derivation online-policy V2 pin is invalid"
);
impl_exact_role_pin_validation!(
    ReviewedL1CredentialDerivationOnlineAuthorizationPinsV1,
    ONLINE_AUTHORIZATION_V2_SCHEMA_VERSION,
    "reviewed L1 credential-derivation online-authorization V2 pin is invalid"
);
impl_exact_role_pin_validation!(
    ReviewedL1CredentialDerivationDestinationPinsV1,
    REVIEWED_PRODUCTION_DESTINATION_PROFILE_V1_SCHEMA_VERSION,
    "reviewed L1 credential-derivation destination V1 pin is invalid"
);
impl_exact_role_pin_validation!(
    ReviewedL1CredentialDerivationDeliveryPinsV1,
    FRESH_CREDENTIAL_DELIVERY_BINDING_V1_SCHEMA_VERSION,
    "reviewed L1 credential-derivation delivery V1 pin is invalid"
);
impl_exact_role_pin_validation!(
    ReviewedL1CredentialDerivationAccountIdentityPinsV1,
    REVIEWED_SIGNER_PROXY_ACCOUNT_IDENTITY_V1_SCHEMA_VERSION,
    "reviewed L1 credential-derivation account-identity V1 pin is invalid"
);
impl_exact_role_pin_validation!(
    ReviewedL1CredentialDerivationRemoteProofPolicyPinsV1,
    REVIEWED_REMOTE_CREDENTIAL_PROOF_POLICY_V1_SCHEMA_VERSION,
    "reviewed L1 credential-derivation remote-proof policy V1 pin is invalid"
);
impl_exact_role_pin_validation!(
    ReviewedL1CredentialDerivationStaticAuthorizationPinsV1,
    REVIEWED_STATIC_ONLINE_AUTHORIZATION_V3_SCHEMA_VERSION,
    "reviewed L1 credential-derivation static-authorization V3 pin is invalid"
);
impl_exact_role_pin_validation!(
    ReviewedL1CredentialDerivationEligibilityEnvelopePinsV1,
    REVIEWED_PHASE_A_ELIGIBILITY_ENVELOPE_V4_SCHEMA_VERSION,
    "reviewed L1 credential-derivation eligibility-envelope V4 pin is invalid"
);
impl_exact_role_pin_validation!(
    ReviewedL1CredentialDerivationLocalCustodyProfilePinsV1,
    REVIEWED_LOCAL_OPERATOR_COOPERATIVE_CUSTODY_PROFILE_V1_SCHEMA_VERSION,
    "reviewed L1 credential-derivation local-custody V1 pin is invalid"
);

impl ReviewedL1CredentialDerivationConfigPinsV1 {
    fn validate(&self) -> Result<(), PmReviewedL1CredentialDerivationProofPolicyV1Error> {
        validate_exact_pin(
            ExactPinViewV1 {
                schema_version: self.schema_version,
                expected_schema_version: TRIAL_CONFIG_SCHEMA_VERSION,
                canonical_sha256: &self.canonical_sha256,
                canonical_length: self.canonical_length,
                fingerprint: &self.fingerprint,
            },
            "reviewed L1 credential-derivation config V1 pin is invalid",
        )?;
        validate_sha256(&self.plan_fingerprint)
    }
}

/// Exact raw-manifest entry label. No source bytes are carried or loaded.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewedL1CredentialDerivationSourceEntryPinsV1 {
    pub id: String,
    pub requested_url: String,
    pub final_url: String,
    pub retrieved_at_utc: String,
    pub content_type: String,
    pub byte_length: u64,
    pub sha256: String,
}

impl ReviewedL1CredentialDerivationSourceEntryPinsV1 {
    fn validate_exact(
        &self,
    ) -> Result<DateTime<Utc>, PmReviewedL1CredentialDerivationProofPolicyV1Error> {
        if self.id != API_AUTHENTICATION_SOURCE_ID_V1
            || self.requested_url != API_AUTHENTICATION_SOURCE_URL_V1
            || self.final_url != API_AUTHENTICATION_SOURCE_URL_V1
            || self.retrieved_at_utc != PM_T2_OFFICIAL_SOURCE_MANIFEST_RETRIEVED_AT_UTC_V1
            || self.content_type != OFFICIAL_SOURCE_CONTENT_TYPE_V1
            || self.byte_length != API_AUTHENTICATION_SOURCE_BYTE_LENGTH_V1
            || self.sha256 != API_AUTHENTICATION_SOURCE_SHA256_V1
        {
            return Err(invalid(
                "reviewed L1 credential-derivation api_authentication source entry differs from the exact frozen V1 pin",
            ));
        }
        parse_utc(&self.retrieved_at_utc)
    }
}

/// The official raw manifest and only source entry used by this policy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewedL1CredentialDerivationOfficialSourcesV1 {
    pub manifest: ReviewedOfficialSourceManifestPinsV1,
    pub api_authentication: ReviewedL1CredentialDerivationSourceEntryPinsV1,
}

impl ReviewedL1CredentialDerivationOfficialSourcesV1 {
    fn validate(
        &self,
    ) -> Result<[DateTime<Utc>; 2], PmReviewedL1CredentialDerivationProofPolicyV1Error> {
        if self.manifest.schema_family != PM_T2_OFFICIAL_SOURCE_MANIFEST_SCHEMA_FAMILY_V1
            || self.manifest.schema_version != PM_T2_OFFICIAL_SOURCE_MANIFEST_SCHEMA_VERSION_V1
            || self.manifest.retrieved_at_utc != PM_T2_OFFICIAL_SOURCE_MANIFEST_RETRIEVED_AT_UTC_V1
            || self.manifest.byte_length != PM_T2_OFFICIAL_SOURCE_MANIFEST_BYTE_LENGTH_V1
            || self.manifest.sha256 != PM_T2_OFFICIAL_SOURCE_MANIFEST_SHA256_V1
        {
            return Err(invalid(
                "reviewed L1 credential-derivation official raw-manifest pin drifted",
            ));
        }
        Ok([
            parse_utc(&self.manifest.retrieved_at_utc)?,
            self.api_authentication.validate_exact()?,
        ])
    }
}

/// Closed status: the named semantic contract is not established by pins.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReviewedL1CredentialDerivationUnavailableSourceContractV1 {
    #[serde(rename = "not_established_by_frozen_official_source_pins_v1")]
    NotEstablishedByFrozenOfficialSourcePinsV1,
}

/// Explicit limits on what the official manifest and entry labels establish.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewedL1CredentialDerivationOfficialSourceLimitationsV1 {
    pub no_cache_response_semantics: ReviewedL1CredentialDerivationUnavailableSourceContractV1,
    pub nonshared_response_semantics: ReviewedL1CredentialDerivationUnavailableSourceContractV1,
    pub authentication_before_handler_semantics:
        ReviewedL1CredentialDerivationUnavailableSourceContractV1,
    pub credential_current_and_unrevoked_semantics:
        ReviewedL1CredentialDerivationUnavailableSourceContractV1,
}

/// The record's only role: structural L1 grammar plus local tuple association.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReviewedL1CredentialDerivationRecordRoleV1 {
    #[serde(
        rename = "offline_l1_control_and_returned_tuple_local_association_policy_only_no_response_evidence_v1"
    )]
    OfflineL1ControlAndReturnedTupleLocalAssociationPolicyOnlyNoResponseEvidenceV1,
}

/// Exact origin and reviewed peer/local-egress correlation labels.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewedL1CredentialDerivationEndpointPolicyV1 {
    pub scheme: String,
    pub dns_name: String,
    pub tls_server_name: String,
    pub http_host: String,
    pub tcp_port: u16,
    pub selected_peer_ip: String,
    pub network_namespace_device: u64,
    pub network_namespace_inode: u64,
    pub interface_name: String,
    pub interface_index: u32,
    pub local_egress_ip: String,
    pub dedicated_tunnel_or_gateway_profile_reference: String,
    pub dedicated_tunnel_or_gateway_profile_sha256: String,
    pub correlation: ReviewedL1CredentialDerivationEndpointCorrelationV1,
}

/// Only the exact destination-V1/authorization-V2 route-label correlation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReviewedL1CredentialDerivationEndpointCorrelationV1 {
    #[serde(rename = "exact_reviewed_clob_peer_and_exact_online_v2_local_egress_labels_only_v1")]
    ExactReviewedClobPeerAndExactOnlineV2LocalEgressLabelsOnlyV1,
}

impl ReviewedL1CredentialDerivationEndpointPolicyV1 {
    fn validate(&self) -> Result<(), PmReviewedL1CredentialDerivationProofPolicyV1Error> {
        if self.scheme != PROTOCOL_SCHEME_V1
            || self.dns_name != CLOB_DNS_NAME_V1
            || self.tls_server_name != CLOB_DNS_NAME_V1
            || self.http_host != CLOB_DNS_NAME_V1
            || self.tcp_port != CLOB_TCP_PORT_V1
            || self.network_namespace_device == 0
            || self.network_namespace_inode == 0
            || self.interface_index == 0
        {
            return Err(invalid(
                "reviewed L1 credential-derivation exact HTTPS endpoint grammar drifted",
            ));
        }
        let peer = IpAddr::from_str(&self.selected_peer_ip).map_err(|_| {
            invalid("reviewed L1 credential-derivation selected peer IP is invalid")
        })?;
        let egress = IpAddr::from_str(&self.local_egress_ip)
            .map_err(|_| invalid("reviewed L1 credential-derivation local egress IP is invalid"))?;
        if !same_address_family(peer, egress) {
            return Err(invalid(
                "reviewed L1 credential-derivation peer and egress address families differ",
            ));
        }
        validate_token(
            &self.interface_name,
            64,
            "reviewed L1 credential-derivation interface name is invalid",
        )?;
        validate_reference(
            &self.dedicated_tunnel_or_gateway_profile_reference,
            "reviewed L1 credential-derivation tunnel or gateway reference is invalid",
        )?;
        validate_sha256(&self.dedicated_tunnel_or_gateway_profile_sha256)
    }
}

/// Exactly four header names; the record contains no corresponding values.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewedL1CredentialDerivationHeaderNamesV1 {
    pub poly_address: String,
    pub poly_signature: String,
    pub poly_timestamp: String,
    pub poly_nonce: String,
}

impl ReviewedL1CredentialDerivationHeaderNamesV1 {
    fn validate(&self) -> Result<(), PmReviewedL1CredentialDerivationProofPolicyV1Error> {
        if self.poly_address != "POLY_ADDRESS"
            || self.poly_signature != "POLY_SIGNATURE"
            || self.poly_timestamp != "POLY_TIMESTAMP"
            || self.poly_nonce != "POLY_NONCE"
        {
            return Err(invalid(
                "reviewed L1 credential-derivation header-name grammar is not the exact four-name set",
            ));
        }
        Ok(())
    }
}

/// The single reviewed EIP-712 field order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReviewedL1CredentialDerivationEip712FieldOrderV1 {
    #[serde(rename = "address_then_timestamp_then_nonce_then_message_v1")]
    AddressThenTimestampThenNonceThenMessageV1,
}

/// The exact-zero nonce is an explicit V1 review choice, never an inference.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReviewedL1CredentialDerivationNoncePolicyV1 {
    #[serde(rename = "exact_zero_explicit_reviewed_choice_not_inferred_v1")]
    ExactZeroExplicitReviewedChoiceNotInferredV1,
}

/// Public typed-data grammar only; there is no signature or timestamp value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewedL1CredentialDerivationEip712PolicyV1 {
    pub standard: String,
    pub domain_type: String,
    pub domain_name: String,
    pub domain_version: String,
    pub domain_chain_id: u64,
    pub primary_type: String,
    pub struct_type: String,
    pub field_order: ReviewedL1CredentialDerivationEip712FieldOrderV1,
    pub address_source: String,
    pub timestamp_source: String,
    pub timestamp_decimal_digits: u8,
    pub minimum_timestamp_unix_seconds: u64,
    pub maximum_timestamp_unix_seconds: u64,
    pub nonce_value: u64,
    pub nonce_source: String,
    pub nonce_policy: ReviewedL1CredentialDerivationNoncePolicyV1,
    pub message: String,
    pub signature_source: String,
}

impl ReviewedL1CredentialDerivationEip712PolicyV1 {
    fn validate(&self) -> Result<(), PmReviewedL1CredentialDerivationProofPolicyV1Error> {
        if self.standard != EIP712_STANDARD_V1
            || self.domain_type != CLOB_AUTH_DOMAIN_TYPE_V1
            || self.domain_name != CLOB_AUTH_DOMAIN_NAME_V1
            || self.domain_version != CLOB_AUTH_DOMAIN_VERSION_V1
            || self.domain_chain_id != CLOB_AUTH_CHAIN_ID_V1
            || self.primary_type != CLOB_AUTH_PRIMARY_TYPE_V1
            || self.struct_type != CLOB_AUTH_STRUCT_TYPE_V1
            || self.address_source != CLOB_AUTH_ADDRESS_SOURCE_V1
            || self.timestamp_source != CLOB_AUTH_TIMESTAMP_SOURCE_V1
            || self.timestamp_decimal_digits != TIMESTAMP_DECIMAL_DIGITS_V1
            || self.minimum_timestamp_unix_seconds != MINIMUM_TIMESTAMP_UNIX_SECONDS_V1
            || self.maximum_timestamp_unix_seconds != MAXIMUM_TIMESTAMP_UNIX_SECONDS_V1
            || self.nonce_value != EXACT_NONCE_V1
            || self.nonce_source != CLOB_AUTH_NONCE_SOURCE_V1
            || self.message != CLOB_AUTH_MESSAGE_V1
            || self.signature_source != CLOB_AUTH_SIGNATURE_SOURCE_V1
        {
            return Err(invalid(
                "reviewed L1 credential-derivation exact EIP-712 ClobAuth grammar drifted",
            ));
        }
        Ok(())
    }
}

/// Exact fixed GET grammar with no query, body, or Content-Type.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewedL1CredentialDerivationRequestPolicyV1 {
    pub method: String,
    pub path: String,
    pub query: String,
    pub body: String,
    pub content_type: String,
    pub accept: String,
    pub accept_encoding: String,
    pub exact_header_count: u8,
    pub header_names: ReviewedL1CredentialDerivationHeaderNamesV1,
    pub eip712: ReviewedL1CredentialDerivationEip712PolicyV1,
}

impl ReviewedL1CredentialDerivationRequestPolicyV1 {
    fn validate(&self) -> Result<(), PmReviewedL1CredentialDerivationProofPolicyV1Error> {
        if self.method != DERIVE_METHOD_V1
            || self.path != DERIVE_PATH_V1
            || self.query != ABSENT_COMPONENT_V1
            || self.body != ABSENT_COMPONENT_V1
            || self.content_type != ABSENT_COMPONENT_V1
            || self.accept != ACCEPT_APPLICATION_JSON_V1
            || self.accept_encoding != ACCEPT_ENCODING_IDENTITY_V1
            || self.exact_header_count != 4
        {
            return Err(invalid(
                "reviewed L1 credential-derivation exact request grammar drifted",
            ));
        }
        self.header_names.validate()?;
        self.eip712.validate()
    }
}

/// Closed alternatives for a future source-owned timestamp proof.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReviewedL1CredentialDerivationSourceOwnedTimeRequirementV1 {
    #[serde(rename = "clob_time_endpoint_or_equivalent_source_owned_proof_required_v1")]
    ClobTimeEndpointOrEquivalentSourceOwnedProofRequiredV1,
}

/// One monotonic interval covers creation, send, and receive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReviewedL1CredentialDerivationMonotonicFreshnessRequirementV1 {
    #[serde(rename = "one_monotonic_origin_timestamp_creation_through_send_and_receive_v1")]
    OneMonotonicOriginTimestampCreationThroughSendAndReceiveV1,
}

/// Separate future time-proof and monotonic freshness requirements.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewedL1CredentialDerivationTimePolicyV1 {
    pub clob_time_path: String,
    pub source_owned_time_requirement: ReviewedL1CredentialDerivationSourceOwnedTimeRequirementV1,
    pub monotonic_freshness_requirement:
        ReviewedL1CredentialDerivationMonotonicFreshnessRequirementV1,
    pub maximum_timestamp_creation_to_request_send_ms: u64,
    pub maximum_timestamp_creation_to_response_receive_ms: u64,
}

impl ReviewedL1CredentialDerivationTimePolicyV1 {
    fn validate(&self) -> Result<(), PmReviewedL1CredentialDerivationProofPolicyV1Error> {
        if self.clob_time_path != SERVER_TIME_PATH_V1
            || self.maximum_timestamp_creation_to_request_send_ms
                != MAXIMUM_TIMESTAMP_CREATION_TO_SEND_MS_V1
            || self.maximum_timestamp_creation_to_response_receive_ms
                != MAXIMUM_TIMESTAMP_CREATION_TO_RECEIVE_MS_V1
        {
            return Err(invalid(
                "reviewed L1 credential-derivation source-owned time or monotonic freshness policy drifted",
            ));
        }
        Ok(())
    }
}

/// One exact closed transport disposition for a future actor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReviewedL1CredentialDerivationTransportDispositionV1 {
    #[serde(rename = "no_redirect_retry_forward_proxy_or_destination_fallback_v1")]
    NoRedirectRetryForwardProxyOrDestinationFallbackV1,
}

/// Transport requirements only. No transport state or capability exists.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewedL1CredentialDerivationDispatchPolicyV1 {
    pub maximum_request_count: u8,
    pub connect_timeout_ms: u64,
    pub request_timeout_ms: u64,
    pub redirects_allowed: bool,
    pub retries_allowed: bool,
    pub forward_proxy_allowed: bool,
    pub destination_fallback_allowed: bool,
    pub disposition: ReviewedL1CredentialDerivationTransportDispositionV1,
    pub connected_peer_check_before_status_and_body_required: bool,
}

impl ReviewedL1CredentialDerivationDispatchPolicyV1 {
    fn validate(&self) -> Result<(), PmReviewedL1CredentialDerivationProofPolicyV1Error> {
        if self.maximum_request_count != 1
            || self.connect_timeout_ms != CONNECT_TIMEOUT_MS_V1
            || self.request_timeout_ms != REQUEST_TIMEOUT_MS_V1
            || self.redirects_allowed
            || self.retries_allowed
            || self.forward_proxy_allowed
            || self.destination_fallback_allowed
            || !self.connected_peer_check_before_status_and_body_required
        {
            return Err(invalid(
                "reviewed L1 credential-derivation one-request dispatch grammar drifted",
            ));
        }
        Ok(())
    }
}

/// Exact JSON object grammar for the three returned string fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReviewedL1CredentialDerivationResponseObjectGrammarV1 {
    #[serde(rename = "exact_three_strings_no_missing_unknown_duplicate_or_trailing_input_v1")]
    ExactThreeStringsNoMissingUnknownDuplicateOrTrailingInputV1,
}

/// Pure local equality requirements; no returned value appears here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewedL1CredentialDerivationTupleAssociationPolicyV1 {
    pub api_key_association: String,
    pub secret_association: String,
    pub passphrase_association: String,
}

impl ReviewedL1CredentialDerivationTupleAssociationPolicyV1 {
    fn validate(&self) -> Result<(), PmReviewedL1CredentialDerivationProofPolicyV1Error> {
        if self.api_key_association != RESPONSE_API_KEY_ASSOCIATION_V1
            || self.secret_association != RESPONSE_SECRET_ASSOCIATION_V1
            || self.passphrase_association != RESPONSE_PASSPHRASE_ASSOCIATION_V1
        {
            return Err(invalid(
                "reviewed L1 credential-derivation same-loaded-holder tuple association drifted",
            ));
        }
        Ok(())
    }
}

/// Strict HTTP metadata, bounded exact JSON shape, and local association.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewedL1CredentialDerivationResponsePolicyV1 {
    pub required_status_code: u16,
    pub content_type_header_required: bool,
    pub required_content_type_essence: String,
    pub allowed_content_type_charset: String,
    pub allowed_content_encoding: String,
    pub maximum_body_bytes: u64,
    pub required_json_object_field_count: u8,
    pub api_key_field_name: String,
    pub api_key_field_type: String,
    pub secret_field_name: String,
    pub secret_field_type: String,
    pub passphrase_field_name: String,
    pub passphrase_field_type: String,
    pub object_grammar: ReviewedL1CredentialDerivationResponseObjectGrammarV1,
    pub tuple_association: ReviewedL1CredentialDerivationTupleAssociationPolicyV1,
}

impl ReviewedL1CredentialDerivationResponsePolicyV1 {
    fn validate(&self) -> Result<(), PmReviewedL1CredentialDerivationProofPolicyV1Error> {
        if self.required_status_code != 200
            || !self.content_type_header_required
            || self.required_content_type_essence != REQUIRED_CONTENT_TYPE_ESSENCE_V1
            || self.allowed_content_type_charset != ALLOWED_CONTENT_TYPE_CHARSET_V1
            || self.allowed_content_encoding != ALLOWED_CONTENT_ENCODING_V1
            || self.maximum_body_bytes != MAXIMUM_BODY_BYTES_V1
            || self.required_json_object_field_count != 3
            || self.api_key_field_name != RESPONSE_API_KEY_FIELD_V1
            || self.api_key_field_type != RESPONSE_FIELD_TYPE_V1
            || self.secret_field_name != RESPONSE_SECRET_FIELD_V1
            || self.secret_field_type != RESPONSE_FIELD_TYPE_V1
            || self.passphrase_field_name != RESPONSE_PASSPHRASE_FIELD_V1
            || self.passphrase_field_type != RESPONSE_FIELD_TYPE_V1
        {
            return Err(invalid(
                "reviewed L1 credential-derivation strict response grammar drifted",
            ));
        }
        self.tuple_association.validate()
    }
}

/// Complete closed protocol conjunction.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewedL1CredentialDerivationProtocolPolicyV1 {
    pub endpoint: ReviewedL1CredentialDerivationEndpointPolicyV1,
    pub request: ReviewedL1CredentialDerivationRequestPolicyV1,
    pub time: ReviewedL1CredentialDerivationTimePolicyV1,
    pub dispatch: ReviewedL1CredentialDerivationDispatchPolicyV1,
    pub response: ReviewedL1CredentialDerivationResponsePolicyV1,
}

impl ReviewedL1CredentialDerivationProtocolPolicyV1 {
    fn validate(&self) -> Result<(), PmReviewedL1CredentialDerivationProofPolicyV1Error> {
        self.endpoint.validate()?;
        self.request.validate()?;
        self.time.validate()?;
        self.dispatch.validate()?;
        self.response.validate()
    }
}

/// Exact additive policy record. It contains public grammar and commitments.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewedL1CredentialDerivationProofPolicyV1 {
    pub schema_version: u32,
    pub policy_id: String,
    pub reviewer_label: String,
    pub reviewed_at_utc: String,
    pub not_before_utc: String,
    pub expires_at_utc: String,
    pub cleanup_not_after_utc: String,
    pub record_role: ReviewedL1CredentialDerivationRecordRoleV1,
    pub v1_config: ReviewedL1CredentialDerivationConfigPinsV1,
    pub online_policy_v2: ReviewedL1CredentialDerivationOnlinePolicyPinsV1,
    pub online_authorization_v2: ReviewedL1CredentialDerivationOnlineAuthorizationPinsV1,
    pub reviewed_production_destination_v1: ReviewedL1CredentialDerivationDestinationPinsV1,
    pub fresh_credential_delivery_binding_v1: ReviewedL1CredentialDerivationDeliveryPinsV1,
    pub reviewed_signer_proxy_account_identity_v1:
        ReviewedL1CredentialDerivationAccountIdentityPinsV1,
    pub reviewed_remote_credential_proof_policy_v1:
        ReviewedL1CredentialDerivationRemoteProofPolicyPinsV1,
    pub reviewed_static_online_authorization_v3:
        ReviewedL1CredentialDerivationStaticAuthorizationPinsV1,
    pub reviewed_phase_a_eligibility_envelope_v4:
        ReviewedL1CredentialDerivationEligibilityEnvelopePinsV1,
    pub reviewed_local_operator_cooperative_custody_profile_v1:
        ReviewedL1CredentialDerivationLocalCustodyProfilePinsV1,
    pub official_sources: ReviewedL1CredentialDerivationOfficialSourcesV1,
    pub official_source_limitations: ReviewedL1CredentialDerivationOfficialSourceLimitationsV1,
    pub protocol: ReviewedL1CredentialDerivationProtocolPolicyV1,
}

impl fmt::Debug for ReviewedL1CredentialDerivationProofPolicyV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(
            "ReviewedL1CredentialDerivationProofPolicyV1(<exact-structural-pins-and-public-grammar; no-response-values-or-evidence; redacted; denied>)",
        )
    }
}

struct ValidatedReviewedL1CredentialDerivationTimesV1 {
    reviewed_at: DateTime<Utc>,
    not_before: DateTime<Utc>,
    expires_at: DateTime<Utc>,
    cleanup_not_after: DateTime<Utc>,
}

impl ReviewedL1CredentialDerivationProofPolicyV1 {
    fn validate_intrinsic(
        &self,
    ) -> Result<
        ValidatedReviewedL1CredentialDerivationTimesV1,
        PmReviewedL1CredentialDerivationProofPolicyV1Error,
    > {
        if self.schema_version != REVIEWED_L1_CREDENTIAL_DERIVATION_PROOF_POLICY_V1_SCHEMA_VERSION
            || self.policy_id != REVIEWED_L1_CREDENTIAL_DERIVATION_PROOF_POLICY_ID_V1
        {
            return Err(invalid(
                "unsupported reviewed L1 credential-derivation proof policy V1 identity",
            ));
        }
        validate_reference(
            &self.reviewer_label,
            "reviewed L1 credential-derivation reviewer label is invalid",
        )?;
        self.v1_config.validate()?;
        self.online_policy_v2.validate()?;
        self.online_authorization_v2.validate()?;
        self.reviewed_production_destination_v1.validate()?;
        self.fresh_credential_delivery_binding_v1.validate()?;
        self.reviewed_signer_proxy_account_identity_v1.validate()?;
        self.reviewed_remote_credential_proof_policy_v1.validate()?;
        self.reviewed_static_online_authorization_v3.validate()?;
        self.reviewed_phase_a_eligibility_envelope_v4.validate()?;
        self.reviewed_local_operator_cooperative_custody_profile_v1
            .validate()?;
        let source_times = self.official_sources.validate()?;
        self.protocol.validate()?;

        let reviewed_at = parse_utc(&self.reviewed_at_utc)?;
        let not_before = parse_utc(&self.not_before_utc)?;
        let expires_at = parse_utc(&self.expires_at_utc)?;
        let cleanup_not_after = parse_utc(&self.cleanup_not_after_utc)?;
        if source_times.into_iter().any(|source| source > reviewed_at)
            || reviewed_at > not_before
            || not_before >= expires_at
            || expires_at > cleanup_not_after
        {
            return Err(invalid(
                "reviewed L1 credential-derivation review and validity envelope is invalid",
            ));
        }
        Ok(ValidatedReviewedL1CredentialDerivationTimesV1 {
            reviewed_at,
            not_before,
            expires_at,
            cleanup_not_after,
        })
    }
}

/// Move-only, non-Serde, non-projecting canonical protected record.
pub struct CanonicalReviewedL1CredentialDerivationProofPolicyV1 {
    value: ReviewedL1CredentialDerivationProofPolicyV1,
    canonical_bytes: Vec<u8>,
    canonical_sha256: String,
    fingerprint: String,
}

impl CanonicalReviewedL1CredentialDerivationProofPolicyV1 {
    #[must_use]
    pub fn canonical_sha256(&self) -> &str {
        &self.canonical_sha256
    }

    #[must_use]
    pub fn canonical_length(&self) -> u64 {
        self.canonical_bytes.len() as u64
    }

    #[must_use]
    pub fn fingerprint(&self) -> &str {
        &self.fingerprint
    }
}

impl fmt::Debug for CanonicalReviewedL1CredentialDerivationProofPolicyV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(
            "CanonicalReviewedL1CredentialDerivationProofPolicyV1(<exact-protected-canonical-bytes; sha-length-fingerprint-only; no-value-proof-runtime-or-authority-projection; redacted; denied>)",
        )
    }
}

/// Borrowed canonical-holder chain. No digest, report, boolean, clock, proof,
/// credential, actor, attempt, token, or capability is accepted from callers.
pub struct ReviewedL1CredentialDerivationProofPolicyContextV1<'a> {
    pub phase_a_eligibility_context: ReviewedPhaseAEligibilityEnvelopeContextV4<'a>,
    pub reviewed_phase_a_eligibility_envelope_v4: &'a CanonicalReviewedPhaseAEligibilityEnvelopeV4,
    pub reviewed_local_operator_cooperative_custody_profile_v1:
        &'a CanonicalReviewedLocalOperatorCooperativeCustodyProfileV1,
}

/// Exhaustive offline report. Only structural one-way pins, closed grammar,
/// and nested time-envelope facts can be true.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ReviewedL1CredentialDerivationProofPolicyVerificationV1 {
    pub schema_version: u32,
    pub v1_config_fingerprint: String,
    pub online_policy_v2_fingerprint: String,
    pub online_authorization_v2_fingerprint: String,
    pub reviewed_production_destination_v1_fingerprint: String,
    pub fresh_credential_delivery_binding_v1_fingerprint: String,
    pub reviewed_signer_proxy_account_identity_v1_fingerprint: String,
    pub reviewed_remote_credential_proof_policy_v1_fingerprint: String,
    pub reviewed_static_online_authorization_v3_fingerprint: String,
    pub reviewed_phase_a_eligibility_envelope_v4_fingerprint: String,
    pub reviewed_local_operator_cooperative_custody_profile_v1_fingerprint: String,
    pub reviewed_l1_credential_derivation_proof_policy_v1_fingerprint: String,
    pub exact_v1_config_pin_structurally_valid: bool,
    pub exact_online_policy_v2_pin_structurally_valid: bool,
    pub exact_online_authorization_v2_pin_structurally_valid: bool,
    pub exact_reviewed_production_destination_v1_pin_structurally_valid: bool,
    pub exact_fresh_credential_delivery_binding_v1_pin_structurally_valid: bool,
    pub exact_reviewed_signer_proxy_account_identity_v1_pin_structurally_valid: bool,
    pub exact_reviewed_remote_credential_proof_policy_v1_pin_structurally_valid: bool,
    pub exact_reviewed_static_online_authorization_v3_pin_structurally_valid: bool,
    pub exact_reviewed_phase_a_eligibility_envelope_v4_pin_structurally_valid: bool,
    pub exact_reviewed_local_operator_cooperative_custody_profile_v1_pin_structurally_valid: bool,
    pub exact_official_manifest_pin_structurally_valid: bool,
    pub exact_api_authentication_source_entry_pin_structurally_valid: bool,
    pub closed_endpoint_and_route_correlation_grammar_structurally_valid: bool,
    pub closed_request_grammar_structurally_valid: bool,
    pub closed_eip712_timestamp_and_nonce_grammar_structurally_valid: bool,
    pub source_owned_time_requirement_structurally_valid: bool,
    pub monotonic_freshness_envelope_structurally_valid: bool,
    pub one_request_transport_grammar_structurally_valid: bool,
    pub strict_response_grammar_structurally_valid: bool,
    pub same_loaded_l2_holder_tuple_association_requirement_structurally_valid: bool,
    pub review_time_envelope_nested_within_online_authorization_v2: bool,
    pub official_source_manifest_bytes_loaded: bool,
    pub official_source_manifest_hash_verified: bool,
    pub api_authentication_source_bytes_loaded: bool,
    pub api_authentication_source_hash_verified: bool,
    pub official_source_publisher_authorship_attested: bool,
    pub policy_reviewer_authorship_attested: bool,
    pub response_no_cache_semantics_attested: bool,
    pub response_nonshared_semantics_attested: bool,
    pub authentication_before_handler_attested: bool,
    pub credential_derivation_response_evidence_established: bool,
    pub request_constructed: bool,
    pub eip712_signature_created: bool,
    pub request_signed: bool,
    pub request_dispatched: bool,
    pub request_received_by_provider: bool,
    pub response_received: bool,
    pub fixed_socket_created: bool,
    pub tls_handshake_observed: bool,
    pub tls_server_identity_observed: bool,
    pub fixed_reviewed_peer_observed: bool,
    pub fixed_local_egress_observed: bool,
    pub response_status_observed: bool,
    pub response_mime_observed: bool,
    pub response_content_type_observed: bool,
    pub response_content_encoding_observed: bool,
    pub response_body_observed: bool,
    pub response_parser_executed: bool,
    pub exact_three_field_response_object_observed: bool,
    pub returned_tuple_matched_same_loaded_l2_holder: bool,
    pub signer_private_key_control_attested: bool,
    pub signer_control_current: bool,
    pub remote_api_key_owner_attested: bool,
    pub later_l2_remote_acceptance_verified: bool,
    pub l2_credential_current: bool,
    pub l2_credential_unrevoked: bool,
    pub same_holder_across_derivation_proof_and_place_attested: bool,
    pub credential_provider_origin_attested: bool,
    pub credential_provider_delivery_attested: bool,
    pub credential_delivery_lease_verified: bool,
    pub credential_delivery_lease_current_and_unrevoked: bool,
    pub credential_provider_generation_attested: bool,
    pub signer_proxy_relationship_attested: bool,
    pub signer_proxy_relationship_current: bool,
    pub signer_proxy_relationship_unrevoked: bool,
    pub selected_actor_bound: bool,
    pub source_owned_runtime_attempt_bound: bool,
    pub source_clock_owner_bound: bool,
    pub source_owned_current_time_observed: bool,
    pub source_owned_time_proof_verified: bool,
    pub timestamp_creation_observed: bool,
    pub monotonic_timestamp_creation_to_send_checked: bool,
    pub monotonic_timestamp_creation_to_receive_checked: bool,
    pub durable_preparation_record_created: bool,
    pub atomic_consumption_claim_created: bool,
    pub authorization_burn_performed: bool,
    pub durable_no_resend_established: bool,
    pub online_authorization_v2_reverse_pin_established: bool,
    pub static_online_authorization_v3_reverse_pin_established: bool,
    pub phase_a_eligibility_envelope_v4_reverse_pin_established: bool,
    pub online_authorization_v2_consumed: bool,
    pub static_online_authorization_v3_consumed: bool,
    pub phase_a_eligibility_envelope_v4_consumed: bool,
    pub online_authorization_v2_current: bool,
    pub reviewed_static_online_authorization_v3_current: bool,
    pub reviewed_phase_a_eligibility_envelope_v4_current: bool,
    pub reviewed_local_operator_cooperative_custody_profile_v1_current: bool,
    pub reviewed_l1_credential_derivation_proof_policy_v1_current: bool,
    pub place_request_constructed: bool,
    pub cancel_request_constructed: bool,
    pub hmac_constructed: bool,
    pub place_dispatch_owner_or_grant_minted: bool,
    pub network_mutation_performed: bool,
    pub credential_mutation_authority_attested: bool,
    pub credential_derivation_dispatch_allowance: u8,
    pub cancel_dispatch_allowance: u8,
    #[serde(flatten)]
    pub authorization: OfflineAuthorizationState,
}

/// Pure offline drafting from canonical holders. All pins and route labels are
/// internally derived and all protocol fields are closed constants.
pub fn draft_non_authorizing_reviewed_l1_credential_derivation_proof_policy_v1(
    context: &ReviewedL1CredentialDerivationProofPolicyContextV1<'_>,
) -> Result<
    ReviewedL1CredentialDerivationProofPolicyV1,
    PmReviewedL1CredentialDerivationProofPolicyV1Error,
> {
    verify_denied_context(context)?;
    let joined = &context.phase_a_eligibility_context;
    let config = joined.v1_config;
    let policy = joined.online_policy_v2;
    let authorization = joined.online_authorization_v2;
    let destination = joined.reviewed_production_destination_v1;
    let delivery = joined.fresh_credential_delivery_binding_v1;
    let identity = joined.reviewed_signer_proxy_account_identity_v1;
    let remote = joined.reviewed_remote_credential_proof_policy_v1;
    let static_v3 = joined.reviewed_static_online_authorization_v3;
    let v4 = context.reviewed_phase_a_eligibility_envelope_v4;
    let custody = context.reviewed_local_operator_cooperative_custody_profile_v1;
    let online = authorization.value();
    let clob = &destination.value().destinations.clob_https;
    let egress = &online.host.egress;

    let record = ReviewedL1CredentialDerivationProofPolicyV1 {
        schema_version: REVIEWED_L1_CREDENTIAL_DERIVATION_PROOF_POLICY_V1_SCHEMA_VERSION,
        policy_id: REVIEWED_L1_CREDENTIAL_DERIVATION_PROOF_POLICY_ID_V1.to_owned(),
        reviewer_label: online.issuing_reviewer.clone(),
        reviewed_at_utc: online.reviewed_at_utc.clone(),
        not_before_utc: online.not_before_utc.clone(),
        expires_at_utc: online.expires_at_utc.clone(),
        cleanup_not_after_utc: online.cleanup_not_after_utc.clone(),
        record_role:
            ReviewedL1CredentialDerivationRecordRoleV1::OfflineL1ControlAndReturnedTupleLocalAssociationPolicyOnlyNoResponseEvidenceV1,
        v1_config: ReviewedL1CredentialDerivationConfigPinsV1 {
            schema_version: TRIAL_CONFIG_SCHEMA_VERSION,
            canonical_sha256: config.canonical_sha256().to_owned(),
            canonical_length: config.canonical_length(),
            fingerprint: config.fingerprint().to_owned(),
            plan_fingerprint: config.plan_fingerprint().to_owned(),
        },
        online_policy_v2: ReviewedL1CredentialDerivationOnlinePolicyPinsV1 {
            schema_version: ONLINE_POLICY_V2_SCHEMA_VERSION,
            canonical_sha256: policy.canonical_sha256().to_owned(),
            canonical_length: policy.canonical_length(),
            fingerprint: policy.fingerprint().to_owned(),
        },
        online_authorization_v2: ReviewedL1CredentialDerivationOnlineAuthorizationPinsV1 {
            schema_version: ONLINE_AUTHORIZATION_V2_SCHEMA_VERSION,
            canonical_sha256: authorization.canonical_sha256().to_owned(),
            canonical_length: authorization.canonical_length(),
            fingerprint: authorization.fingerprint().to_owned(),
        },
        reviewed_production_destination_v1: ReviewedL1CredentialDerivationDestinationPinsV1 {
            schema_version: REVIEWED_PRODUCTION_DESTINATION_PROFILE_V1_SCHEMA_VERSION,
            canonical_sha256: destination.canonical_sha256().to_owned(),
            canonical_length: destination.canonical_length(),
            fingerprint: destination.fingerprint().to_owned(),
        },
        fresh_credential_delivery_binding_v1: ReviewedL1CredentialDerivationDeliveryPinsV1 {
            schema_version: FRESH_CREDENTIAL_DELIVERY_BINDING_V1_SCHEMA_VERSION,
            canonical_sha256: delivery.canonical_sha256().to_owned(),
            canonical_length: delivery.canonical_length(),
            fingerprint: delivery.fingerprint().to_owned(),
        },
        reviewed_signer_proxy_account_identity_v1:
            ReviewedL1CredentialDerivationAccountIdentityPinsV1 {
                schema_version: REVIEWED_SIGNER_PROXY_ACCOUNT_IDENTITY_V1_SCHEMA_VERSION,
                canonical_sha256: identity.canonical_sha256().to_owned(),
                canonical_length: identity.canonical_length(),
                fingerprint: identity.fingerprint().to_owned(),
            },
        reviewed_remote_credential_proof_policy_v1:
            ReviewedL1CredentialDerivationRemoteProofPolicyPinsV1 {
                schema_version: REVIEWED_REMOTE_CREDENTIAL_PROOF_POLICY_V1_SCHEMA_VERSION,
                canonical_sha256: remote.canonical_sha256().to_owned(),
                canonical_length: remote.canonical_length(),
                fingerprint: remote.fingerprint().to_owned(),
            },
        reviewed_static_online_authorization_v3:
            ReviewedL1CredentialDerivationStaticAuthorizationPinsV1 {
                schema_version: REVIEWED_STATIC_ONLINE_AUTHORIZATION_V3_SCHEMA_VERSION,
                canonical_sha256: static_v3.canonical_sha256().to_owned(),
                canonical_length: static_v3.canonical_length(),
                fingerprint: static_v3.fingerprint().to_owned(),
            },
        reviewed_phase_a_eligibility_envelope_v4:
            ReviewedL1CredentialDerivationEligibilityEnvelopePinsV1 {
                schema_version: REVIEWED_PHASE_A_ELIGIBILITY_ENVELOPE_V4_SCHEMA_VERSION,
                canonical_sha256: v4.canonical_sha256().to_owned(),
                canonical_length: v4.canonical_length(),
                fingerprint: v4.fingerprint().to_owned(),
            },
        reviewed_local_operator_cooperative_custody_profile_v1:
            ReviewedL1CredentialDerivationLocalCustodyProfilePinsV1 {
                schema_version:
                    REVIEWED_LOCAL_OPERATOR_COOPERATIVE_CUSTODY_PROFILE_V1_SCHEMA_VERSION,
                canonical_sha256: custody.canonical_sha256().to_owned(),
                canonical_length: custody.canonical_length(),
                fingerprint: custody.fingerprint().to_owned(),
            },
        official_sources: exact_official_sources(),
        official_source_limitations: unavailable_official_source_contracts(),
        protocol: ReviewedL1CredentialDerivationProtocolPolicyV1 {
            endpoint: ReviewedL1CredentialDerivationEndpointPolicyV1 {
                scheme: PROTOCOL_SCHEME_V1.to_owned(),
                dns_name: CLOB_DNS_NAME_V1.to_owned(),
                tls_server_name: CLOB_DNS_NAME_V1.to_owned(),
                http_host: CLOB_DNS_NAME_V1.to_owned(),
                tcp_port: CLOB_TCP_PORT_V1,
                selected_peer_ip: clob.peer_ip.clone(),
                network_namespace_device: egress.network_namespace_device,
                network_namespace_inode: egress.network_namespace_inode,
                interface_name: egress.interface_name.clone(),
                interface_index: egress.interface_index,
                local_egress_ip: egress.local_source_ip.clone(),
                dedicated_tunnel_or_gateway_profile_reference: egress
                    .dedicated_tunnel_or_gateway_profile_reference
                    .clone(),
                dedicated_tunnel_or_gateway_profile_sha256: egress
                    .dedicated_tunnel_or_gateway_profile_sha256
                    .clone(),
                correlation:
                    ReviewedL1CredentialDerivationEndpointCorrelationV1::ExactReviewedClobPeerAndExactOnlineV2LocalEgressLabelsOnlyV1,
            },
            request: ReviewedL1CredentialDerivationRequestPolicyV1 {
                method: DERIVE_METHOD_V1.to_owned(),
                path: DERIVE_PATH_V1.to_owned(),
                query: ABSENT_COMPONENT_V1.to_owned(),
                body: ABSENT_COMPONENT_V1.to_owned(),
                content_type: ABSENT_COMPONENT_V1.to_owned(),
                accept: ACCEPT_APPLICATION_JSON_V1.to_owned(),
                accept_encoding: ACCEPT_ENCODING_IDENTITY_V1.to_owned(),
                exact_header_count: 4,
                header_names: ReviewedL1CredentialDerivationHeaderNamesV1 {
                    poly_address: "POLY_ADDRESS".to_owned(),
                    poly_signature: "POLY_SIGNATURE".to_owned(),
                    poly_timestamp: "POLY_TIMESTAMP".to_owned(),
                    poly_nonce: "POLY_NONCE".to_owned(),
                },
                eip712: ReviewedL1CredentialDerivationEip712PolicyV1 {
                    standard: EIP712_STANDARD_V1.to_owned(),
                    domain_type: CLOB_AUTH_DOMAIN_TYPE_V1.to_owned(),
                    domain_name: CLOB_AUTH_DOMAIN_NAME_V1.to_owned(),
                    domain_version: CLOB_AUTH_DOMAIN_VERSION_V1.to_owned(),
                    domain_chain_id: CLOB_AUTH_CHAIN_ID_V1,
                    primary_type: CLOB_AUTH_PRIMARY_TYPE_V1.to_owned(),
                    struct_type: CLOB_AUTH_STRUCT_TYPE_V1.to_owned(),
                    field_order:
                        ReviewedL1CredentialDerivationEip712FieldOrderV1::AddressThenTimestampThenNonceThenMessageV1,
                    address_source: CLOB_AUTH_ADDRESS_SOURCE_V1.to_owned(),
                    timestamp_source: CLOB_AUTH_TIMESTAMP_SOURCE_V1.to_owned(),
                    timestamp_decimal_digits: TIMESTAMP_DECIMAL_DIGITS_V1,
                    minimum_timestamp_unix_seconds: MINIMUM_TIMESTAMP_UNIX_SECONDS_V1,
                    maximum_timestamp_unix_seconds: MAXIMUM_TIMESTAMP_UNIX_SECONDS_V1,
                    nonce_value: EXACT_NONCE_V1,
                    nonce_source: CLOB_AUTH_NONCE_SOURCE_V1.to_owned(),
                    nonce_policy:
                        ReviewedL1CredentialDerivationNoncePolicyV1::ExactZeroExplicitReviewedChoiceNotInferredV1,
                    message: CLOB_AUTH_MESSAGE_V1.to_owned(),
                    signature_source: CLOB_AUTH_SIGNATURE_SOURCE_V1.to_owned(),
                },
            },
            time: ReviewedL1CredentialDerivationTimePolicyV1 {
                clob_time_path: SERVER_TIME_PATH_V1.to_owned(),
                source_owned_time_requirement:
                    ReviewedL1CredentialDerivationSourceOwnedTimeRequirementV1::ClobTimeEndpointOrEquivalentSourceOwnedProofRequiredV1,
                monotonic_freshness_requirement:
                    ReviewedL1CredentialDerivationMonotonicFreshnessRequirementV1::OneMonotonicOriginTimestampCreationThroughSendAndReceiveV1,
                maximum_timestamp_creation_to_request_send_ms:
                    MAXIMUM_TIMESTAMP_CREATION_TO_SEND_MS_V1,
                maximum_timestamp_creation_to_response_receive_ms:
                    MAXIMUM_TIMESTAMP_CREATION_TO_RECEIVE_MS_V1,
            },
            dispatch: ReviewedL1CredentialDerivationDispatchPolicyV1 {
                maximum_request_count: 1,
                connect_timeout_ms: CONNECT_TIMEOUT_MS_V1,
                request_timeout_ms: REQUEST_TIMEOUT_MS_V1,
                redirects_allowed: false,
                retries_allowed: false,
                forward_proxy_allowed: false,
                destination_fallback_allowed: false,
                disposition:
                    ReviewedL1CredentialDerivationTransportDispositionV1::NoRedirectRetryForwardProxyOrDestinationFallbackV1,
                connected_peer_check_before_status_and_body_required: true,
            },
            response: ReviewedL1CredentialDerivationResponsePolicyV1 {
                required_status_code: 200,
                content_type_header_required: true,
                required_content_type_essence: REQUIRED_CONTENT_TYPE_ESSENCE_V1.to_owned(),
                allowed_content_type_charset: ALLOWED_CONTENT_TYPE_CHARSET_V1.to_owned(),
                allowed_content_encoding: ALLOWED_CONTENT_ENCODING_V1.to_owned(),
                maximum_body_bytes: MAXIMUM_BODY_BYTES_V1,
                required_json_object_field_count: 3,
                api_key_field_name: RESPONSE_API_KEY_FIELD_V1.to_owned(),
                api_key_field_type: RESPONSE_FIELD_TYPE_V1.to_owned(),
                secret_field_name: RESPONSE_SECRET_FIELD_V1.to_owned(),
                secret_field_type: RESPONSE_FIELD_TYPE_V1.to_owned(),
                passphrase_field_name: RESPONSE_PASSPHRASE_FIELD_V1.to_owned(),
                passphrase_field_type: RESPONSE_FIELD_TYPE_V1.to_owned(),
                object_grammar:
                    ReviewedL1CredentialDerivationResponseObjectGrammarV1::ExactThreeStringsNoMissingUnknownDuplicateOrTrailingInputV1,
                tuple_association: ReviewedL1CredentialDerivationTupleAssociationPolicyV1 {
                    api_key_association: RESPONSE_API_KEY_ASSOCIATION_V1.to_owned(),
                    secret_association: RESPONSE_SECRET_ASSOCIATION_V1.to_owned(),
                    passphrase_association: RESPONSE_PASSPHRASE_ASSOCIATION_V1.to_owned(),
                },
            },
        },
    };
    let times = record.validate_intrinsic()?;
    validate_context_time_envelope(context, &record, &times)?;
    Ok(record)
}

/// Load one exact compact JSON record through the protected-file reader.
pub fn load_canonical_reviewed_l1_credential_derivation_proof_policy_v1(
    path: &Path,
) -> Result<
    CanonicalReviewedL1CredentialDerivationProofPolicyV1,
    PmReviewedL1CredentialDerivationProofPolicyV1Error,
> {
    let bytes = read_one(
        path,
        ProtectedFileKind::ReviewedL1CredentialDerivationProofPolicyV1,
        MAX_CANONICAL_REVIEWED_L1_CREDENTIAL_DERIVATION_PROOF_POLICY_BYTES_V1,
    )
    .map_err(|_| {
        invalid(
            "reviewed L1 credential-derivation proof policy protection or stability check failed",
        )
    })?;
    let value: ReviewedL1CredentialDerivationProofPolicyV1 = parse_exact_canonical(&bytes)?;
    let _ = value.validate_intrinsic()?;
    let canonical_bytes = bytes.to_vec();
    Ok(CanonicalReviewedL1CredentialDerivationProofPolicyV1 {
        canonical_sha256: hash_bytes(&[], &canonical_bytes),
        fingerprint: hash_bytes(
            REVIEWED_L1_CREDENTIAL_DERIVATION_PROOF_POLICY_V1_FINGERPRINT_DOMAIN,
            &canonical_bytes,
        ),
        canonical_bytes,
        value,
    })
}

/// Verify the exact derived record and return a permanently denied report.
pub fn verify_reviewed_l1_credential_derivation_proof_policy_v1(
    context: &ReviewedL1CredentialDerivationProofPolicyContextV1<'_>,
    reviewed: &CanonicalReviewedL1CredentialDerivationProofPolicyV1,
) -> Result<
    ReviewedL1CredentialDerivationProofPolicyVerificationV1,
    PmReviewedL1CredentialDerivationProofPolicyV1Error,
> {
    let expected =
        draft_non_authorizing_reviewed_l1_credential_derivation_proof_policy_v1(context)?;
    if reviewed.value != expected {
        return Err(invalid(
            "reviewed L1 credential-derivation exact pin, route, grammar, or time envelope mismatched",
        ));
    }
    let joined = &context.phase_a_eligibility_context;
    Ok(ReviewedL1CredentialDerivationProofPolicyVerificationV1 {
        schema_version: reviewed.value.schema_version,
        v1_config_fingerprint: joined.v1_config.fingerprint().to_owned(),
        online_policy_v2_fingerprint: joined.online_policy_v2.fingerprint().to_owned(),
        online_authorization_v2_fingerprint: joined
            .online_authorization_v2
            .fingerprint()
            .to_owned(),
        reviewed_production_destination_v1_fingerprint: joined
            .reviewed_production_destination_v1
            .fingerprint()
            .to_owned(),
        fresh_credential_delivery_binding_v1_fingerprint: joined
            .fresh_credential_delivery_binding_v1
            .fingerprint()
            .to_owned(),
        reviewed_signer_proxy_account_identity_v1_fingerprint: joined
            .reviewed_signer_proxy_account_identity_v1
            .fingerprint()
            .to_owned(),
        reviewed_remote_credential_proof_policy_v1_fingerprint: joined
            .reviewed_remote_credential_proof_policy_v1
            .fingerprint()
            .to_owned(),
        reviewed_static_online_authorization_v3_fingerprint: joined
            .reviewed_static_online_authorization_v3
            .fingerprint()
            .to_owned(),
        reviewed_phase_a_eligibility_envelope_v4_fingerprint: context
            .reviewed_phase_a_eligibility_envelope_v4
            .fingerprint()
            .to_owned(),
        reviewed_local_operator_cooperative_custody_profile_v1_fingerprint: context
            .reviewed_local_operator_cooperative_custody_profile_v1
            .fingerprint()
            .to_owned(),
        reviewed_l1_credential_derivation_proof_policy_v1_fingerprint: reviewed.fingerprint.clone(),
        exact_v1_config_pin_structurally_valid: true,
        exact_online_policy_v2_pin_structurally_valid: true,
        exact_online_authorization_v2_pin_structurally_valid: true,
        exact_reviewed_production_destination_v1_pin_structurally_valid: true,
        exact_fresh_credential_delivery_binding_v1_pin_structurally_valid: true,
        exact_reviewed_signer_proxy_account_identity_v1_pin_structurally_valid: true,
        exact_reviewed_remote_credential_proof_policy_v1_pin_structurally_valid: true,
        exact_reviewed_static_online_authorization_v3_pin_structurally_valid: true,
        exact_reviewed_phase_a_eligibility_envelope_v4_pin_structurally_valid: true,
        exact_reviewed_local_operator_cooperative_custody_profile_v1_pin_structurally_valid: true,
        exact_official_manifest_pin_structurally_valid: true,
        exact_api_authentication_source_entry_pin_structurally_valid: true,
        closed_endpoint_and_route_correlation_grammar_structurally_valid: true,
        closed_request_grammar_structurally_valid: true,
        closed_eip712_timestamp_and_nonce_grammar_structurally_valid: true,
        source_owned_time_requirement_structurally_valid: true,
        monotonic_freshness_envelope_structurally_valid: true,
        one_request_transport_grammar_structurally_valid: true,
        strict_response_grammar_structurally_valid: true,
        same_loaded_l2_holder_tuple_association_requirement_structurally_valid: true,
        review_time_envelope_nested_within_online_authorization_v2: true,
        official_source_manifest_bytes_loaded: false,
        official_source_manifest_hash_verified: false,
        api_authentication_source_bytes_loaded: false,
        api_authentication_source_hash_verified: false,
        official_source_publisher_authorship_attested: false,
        policy_reviewer_authorship_attested: false,
        response_no_cache_semantics_attested: false,
        response_nonshared_semantics_attested: false,
        authentication_before_handler_attested: false,
        credential_derivation_response_evidence_established: false,
        request_constructed: false,
        eip712_signature_created: false,
        request_signed: false,
        request_dispatched: false,
        request_received_by_provider: false,
        response_received: false,
        fixed_socket_created: false,
        tls_handshake_observed: false,
        tls_server_identity_observed: false,
        fixed_reviewed_peer_observed: false,
        fixed_local_egress_observed: false,
        response_status_observed: false,
        response_mime_observed: false,
        response_content_type_observed: false,
        response_content_encoding_observed: false,
        response_body_observed: false,
        response_parser_executed: false,
        exact_three_field_response_object_observed: false,
        returned_tuple_matched_same_loaded_l2_holder: false,
        signer_private_key_control_attested: false,
        signer_control_current: false,
        remote_api_key_owner_attested: false,
        later_l2_remote_acceptance_verified: false,
        l2_credential_current: false,
        l2_credential_unrevoked: false,
        same_holder_across_derivation_proof_and_place_attested: false,
        credential_provider_origin_attested: false,
        credential_provider_delivery_attested: false,
        credential_delivery_lease_verified: false,
        credential_delivery_lease_current_and_unrevoked: false,
        credential_provider_generation_attested: false,
        signer_proxy_relationship_attested: false,
        signer_proxy_relationship_current: false,
        signer_proxy_relationship_unrevoked: false,
        selected_actor_bound: false,
        source_owned_runtime_attempt_bound: false,
        source_clock_owner_bound: false,
        source_owned_current_time_observed: false,
        source_owned_time_proof_verified: false,
        timestamp_creation_observed: false,
        monotonic_timestamp_creation_to_send_checked: false,
        monotonic_timestamp_creation_to_receive_checked: false,
        durable_preparation_record_created: false,
        atomic_consumption_claim_created: false,
        authorization_burn_performed: false,
        durable_no_resend_established: false,
        online_authorization_v2_reverse_pin_established: false,
        static_online_authorization_v3_reverse_pin_established: false,
        phase_a_eligibility_envelope_v4_reverse_pin_established: false,
        online_authorization_v2_consumed: false,
        static_online_authorization_v3_consumed: false,
        phase_a_eligibility_envelope_v4_consumed: false,
        online_authorization_v2_current: false,
        reviewed_static_online_authorization_v3_current: false,
        reviewed_phase_a_eligibility_envelope_v4_current: false,
        reviewed_local_operator_cooperative_custody_profile_v1_current: false,
        reviewed_l1_credential_derivation_proof_policy_v1_current: false,
        place_request_constructed: false,
        cancel_request_constructed: false,
        hmac_constructed: false,
        place_dispatch_owner_or_grant_minted: false,
        network_mutation_performed: false,
        credential_mutation_authority_attested: false,
        credential_derivation_dispatch_allowance: 0,
        cancel_dispatch_allowance: 0,
        authorization: OfflineAuthorizationState::DENIED,
    })
}

#[derive(Debug, Error)]
pub enum PmReviewedL1CredentialDerivationProofPolicyV1Error {
    #[error("controlled-trial reviewed L1 credential-derivation proof policy V1 is invalid: {0}")]
    Invalid(&'static str),
}

fn verify_denied_context(
    context: &ReviewedL1CredentialDerivationProofPolicyContextV1<'_>,
) -> Result<(), PmReviewedL1CredentialDerivationProofPolicyV1Error> {
    let phase_context = copy_phase_context(&context.phase_a_eligibility_context);
    let v4 = verify_reviewed_phase_a_eligibility_envelope_v4(
        &phase_context,
        context.reviewed_phase_a_eligibility_envelope_v4,
    )
    .map_err(|_| invalid("reviewed L1 credential-derivation bound V4 is invalid"))?;
    let custody_context = ReviewedLocalOperatorCooperativeCustodyProfileContextV1 {
        phase_a_eligibility_context: copy_phase_context(&context.phase_a_eligibility_context),
        reviewed_phase_a_eligibility_envelope_v4: context.reviewed_phase_a_eligibility_envelope_v4,
    };
    let custody = verify_reviewed_local_operator_cooperative_custody_profile_v1(
        &custody_context,
        context.reviewed_local_operator_cooperative_custody_profile_v1,
    )
    .map_err(|_| invalid("reviewed L1 credential-derivation bound local custody V1 is invalid"))?;
    if v4.authorization != OfflineAuthorizationState::DENIED
        || v4.offline_phase_a_eligibility_established
        || v4.credential_mutation_authority_attested
        || v4.place_dispatch_owner_or_grant_minted
        || v4.network_dispatch_performed
        || custody.authorization != OfflineAuthorizationState::DENIED
        || custody.loaded_linux_objects_match_delivery_binding
        || custody.private_key_and_l2_credentials_loaded_and_bound
        || custody.same_holder_live_remote_acceptance_proof_verified
        || custody.signer_proxy_control_proof_verified
        || custody.fixed_egress_single_dispatch_owner_minted
        || custody.network_dispatch_performed
        || custody.credential_mutation_authority_attested
    {
        return Err(invalid(
            "reviewed L1 credential-derivation prerequisites are not denied structural evidence",
        ));
    }
    Ok(())
}

fn copy_phase_context<'a>(
    context: &ReviewedPhaseAEligibilityEnvelopeContextV4<'a>,
) -> ReviewedPhaseAEligibilityEnvelopeContextV4<'a> {
    ReviewedPhaseAEligibilityEnvelopeContextV4 {
        v1_config: context.v1_config,
        v1_authorization: context.v1_authorization,
        online_policy_v2: context.online_policy_v2,
        online_authorization_v2: context.online_authorization_v2,
        reviewed_production_destination_v1: context.reviewed_production_destination_v1,
        reviewed_fresh_credential_slot_locator_v1: context
            .reviewed_fresh_credential_slot_locator_v1,
        fresh_credential_delivery_binding_v1: context.fresh_credential_delivery_binding_v1,
        reviewed_signer_proxy_account_identity_v1: context
            .reviewed_signer_proxy_account_identity_v1,
        reviewed_remote_credential_proof_policy_v1: context
            .reviewed_remote_credential_proof_policy_v1,
        reviewed_static_online_authorization_v3: context.reviewed_static_online_authorization_v3,
    }
}

fn validate_context_time_envelope(
    context: &ReviewedL1CredentialDerivationProofPolicyContextV1<'_>,
    record: &ReviewedL1CredentialDerivationProofPolicyV1,
    times: &ValidatedReviewedL1CredentialDerivationTimesV1,
) -> Result<(), PmReviewedL1CredentialDerivationProofPolicyV1Error> {
    let online = context
        .phase_a_eligibility_context
        .online_authorization_v2
        .value();
    if context.phase_a_eligibility_context.v1_config.value().phase != TrialPhase::APlaceCancel
        || online.phase != TrialPhase::APlaceCancel
        || record.reviewer_label != online.issuing_reviewer
        || record.reviewed_at_utc != online.reviewed_at_utc
        || record.not_before_utc != online.not_before_utc
        || record.expires_at_utc != online.expires_at_utc
        || record.cleanup_not_after_utc != online.cleanup_not_after_utc
        || times.reviewed_at != parse_utc(&online.reviewed_at_utc)?
        || times.not_before != parse_utc(&online.not_before_utc)?
        || times.expires_at != parse_utc(&online.expires_at_utc)?
        || times.cleanup_not_after != parse_utc(&online.cleanup_not_after_utc)?
    {
        return Err(invalid(
            "reviewed L1 credential-derivation scope or time envelope differs from online authorization V2",
        ));
    }
    Ok(())
}

fn exact_official_sources() -> ReviewedL1CredentialDerivationOfficialSourcesV1 {
    ReviewedL1CredentialDerivationOfficialSourcesV1 {
        manifest: ReviewedOfficialSourceManifestPinsV1 {
            schema_family: PM_T2_OFFICIAL_SOURCE_MANIFEST_SCHEMA_FAMILY_V1.to_owned(),
            schema_version: PM_T2_OFFICIAL_SOURCE_MANIFEST_SCHEMA_VERSION_V1,
            retrieved_at_utc: PM_T2_OFFICIAL_SOURCE_MANIFEST_RETRIEVED_AT_UTC_V1.to_owned(),
            byte_length: PM_T2_OFFICIAL_SOURCE_MANIFEST_BYTE_LENGTH_V1,
            sha256: PM_T2_OFFICIAL_SOURCE_MANIFEST_SHA256_V1.to_owned(),
        },
        api_authentication: ReviewedL1CredentialDerivationSourceEntryPinsV1 {
            id: API_AUTHENTICATION_SOURCE_ID_V1.to_owned(),
            requested_url: API_AUTHENTICATION_SOURCE_URL_V1.to_owned(),
            final_url: API_AUTHENTICATION_SOURCE_URL_V1.to_owned(),
            retrieved_at_utc: PM_T2_OFFICIAL_SOURCE_MANIFEST_RETRIEVED_AT_UTC_V1.to_owned(),
            content_type: OFFICIAL_SOURCE_CONTENT_TYPE_V1.to_owned(),
            byte_length: API_AUTHENTICATION_SOURCE_BYTE_LENGTH_V1,
            sha256: API_AUTHENTICATION_SOURCE_SHA256_V1.to_owned(),
        },
    }
}

fn unavailable_official_source_contracts()
-> ReviewedL1CredentialDerivationOfficialSourceLimitationsV1 {
    let unavailable =
        ReviewedL1CredentialDerivationUnavailableSourceContractV1::NotEstablishedByFrozenOfficialSourcePinsV1;
    ReviewedL1CredentialDerivationOfficialSourceLimitationsV1 {
        no_cache_response_semantics: unavailable,
        nonshared_response_semantics: unavailable,
        authentication_before_handler_semantics: unavailable,
        credential_current_and_unrevoked_semantics: unavailable,
    }
}

fn validate_exact_pin(
    pin: ExactPinViewV1<'_>,
    message: &'static str,
) -> Result<(), PmReviewedL1CredentialDerivationProofPolicyV1Error> {
    if pin.schema_version != pin.expected_schema_version || pin.canonical_length == 0 {
        return Err(invalid(message));
    }
    validate_sha256(pin.canonical_sha256).map_err(|_| invalid(message))?;
    validate_sha256(pin.fingerprint).map_err(|_| invalid(message))
}

fn parse_exact_canonical<T: DeserializeOwned + Serialize>(
    bytes: &[u8],
) -> Result<T, PmReviewedL1CredentialDerivationProofPolicyV1Error> {
    let value: T = serde_json::from_slice(bytes).map_err(|_| {
        invalid("reviewed L1 credential-derivation proof policy is not valid exact JSON")
    })?;
    let canonical = serde_json::to_vec(&value).map_err(|_| {
        invalid("reviewed L1 credential-derivation proof policy cannot be canonicalized")
    })?;
    if canonical != bytes {
        return Err(invalid(
            "reviewed L1 credential-derivation proof policy bytes are not exact compact canonical JSON",
        ));
    }
    Ok(value)
}

fn parse_utc(
    value: &str,
) -> Result<DateTime<Utc>, PmReviewedL1CredentialDerivationProofPolicyV1Error> {
    let parsed = DateTime::parse_from_rfc3339(value)
        .map_err(|_| invalid("reviewed L1 credential-derivation timestamp is invalid"))?;
    if parsed.to_rfc3339_opts(chrono::SecondsFormat::Secs, true) != value {
        return Err(invalid(
            "reviewed L1 credential-derivation timestamp is not canonical UTC seconds",
        ));
    }
    Ok(parsed.with_timezone(&Utc))
}

fn validate_sha256(value: &str) -> Result<(), PmReviewedL1CredentialDerivationProofPolicyV1Error> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(invalid(
            "reviewed L1 credential-derivation SHA-256 label is invalid",
        ));
    }
    Ok(())
}

fn validate_token(
    value: &str,
    maximum_length: usize,
    message: &'static str,
) -> Result<(), PmReviewedL1CredentialDerivationProofPolicyV1Error> {
    if value.is_empty()
        || value.len() > maximum_length
        || !value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b':' | b'/')
        })
    {
        return Err(invalid(message));
    }
    Ok(())
}

fn validate_reference(
    value: &str,
    message: &'static str,
) -> Result<(), PmReviewedL1CredentialDerivationProofPolicyV1Error> {
    if value.is_empty()
        || value.len() > 512
        || value.chars().any(char::is_control)
        || value.trim() != value
    {
        return Err(invalid(message));
    }
    Ok(())
}

const fn same_address_family(left: IpAddr, right: IpAddr) -> bool {
    matches!(
        (left, right),
        (IpAddr::V4(_), IpAddr::V4(_)) | (IpAddr::V6(_), IpAddr::V6(_))
    )
}

fn hash_bytes(domain: &[u8], bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(domain);
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

fn invalid(message: &'static str) -> PmReviewedL1CredentialDerivationProofPolicyV1Error {
    PmReviewedL1CredentialDerivationProofPolicyV1Error::Invalid(message)
}
