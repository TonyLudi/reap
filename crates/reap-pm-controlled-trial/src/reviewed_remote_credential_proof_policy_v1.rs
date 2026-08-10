//! Exact, offline-only reviewed remote credential-proof policy V1.
//!
//! This additive controlled-trial sidecar pins one closed-only authenticated
//! read policy to the exact canonical config, online policy and authorization,
//! reviewed production destinations, reviewed fresh credential locator,
//! unsigned delivery binding, and reviewed signer/proxy identity. Verification
//! borrows those canonical holders and consumes none of them. It constructs no
//! request, authentication header, HMAC, proof token, network client, or live
//! adapter capability.
//!
//! This is a pre-bind offline policy conjunction only. Borrowing the raw
//! canonical locator and unsigned delivery binding does not join retained
//! delivery evidence, its one load token, a loaded credential holder, or a
//! selected actor/generation. A future V3 must join the whole retained
//! evidence and load token to the selected actor generation before any
//! path-bearing or credential-bearing projection.
//!
//! The source pins identify the locally frozen raw official-source manifest
//! and its exact `api_authentication` and `manage_orders` entry labels. No
//! source bytes are loaded here, and neither the manifest nor those entries is
//! publisher-signed. Most importantly, the frozen sources do not supply an
//! authoritative authentication-acceptance contract proving that every API
//! key, L2 secret, passphrase, address, or timestamp mismatch is rejected
//! before HTTP 200, that authentication precedes the closed-only handler, that
//! a response cannot be shared or cache-derived, or that strict HTTP 200 means
//! the live tuple was accepted. The schema records that contract as closed and
//! unavailable; it accepts no caller-supplied substitute digest or signature.
//!
//! The exact protocol values below are requirements for a future actor, not
//! evidence about the current adapter. In particular, explicit
//! `Accept-Encoding: identity`, response Content-Type/Content-Encoding checks,
//! a one-dispatch no-retry disposition, and durable ambiguous-outcome burn all
//! remain unchecked. Every live, ownership, freshness, consumption, and
//! mutation-authority fact returned by this module is false and authorization
//! is permanently DENIED.
//!
//! The record contains only public policy grammar, non-secret canonical pins,
//! reviewer labels, and public selected IP labels. It defines no credential
//! value, private key, API key, L2 secret, passphrase, cryptographic signature
//! bytes, authentication-header bytes, HMAC value, or request body. Arbitrary
//! labels cannot be proven secret-free, so callers must never place secrets or
//! secret-derived material in them.

use std::{fmt, net::IpAddr, path::Path, str::FromStr as _};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use sha2::{Digest as _, Sha256};
use thiserror::Error;

use crate::{
    CanonicalFreshCredentialDeliveryBindingV1, CanonicalOnlineAuthorizationV2,
    CanonicalOnlinePolicyV2, CanonicalReviewedFreshCredentialSlotLocatorV1,
    CanonicalReviewedProductionDestinationProfileV1, CanonicalReviewedSignerProxyAccountIdentityV1,
    CanonicalTrialConfig, FRESH_CREDENTIAL_DELIVERY_BINDING_V1_SCHEMA_VERSION,
    OfflineAuthorizationState, OnlinePolicyPinsV2, PM_T2_OFFICIAL_SOURCE_MANIFEST_BYTE_LENGTH_V1,
    PM_T2_OFFICIAL_SOURCE_MANIFEST_RETRIEVED_AT_UTC_V1,
    PM_T2_OFFICIAL_SOURCE_MANIFEST_SCHEMA_FAMILY_V1,
    PM_T2_OFFICIAL_SOURCE_MANIFEST_SCHEMA_VERSION_V1, PM_T2_OFFICIAL_SOURCE_MANIFEST_SHA256_V1,
    REVIEWED_FRESH_CREDENTIAL_SLOT_LOCATOR_V1_SCHEMA_VERSION,
    REVIEWED_PRODUCTION_DESTINATION_PROFILE_V1_SCHEMA_VERSION,
    REVIEWED_SIGNER_PROXY_ACCOUNT_IDENTITY_V1_SCHEMA_VERSION, ReviewedOfficialSourceManifestPinsV1,
    ReviewedOnlineAuthorizationPinsV1, V1ConfigPinsV2,
    online_policy_v2::validate_online_authorization_contract_v2,
    protected_file::{ProtectedFileKind, read_one},
    verify_fresh_credential_delivery_binding_v1, verify_reviewed_fresh_credential_slot_locator_v1,
    verify_reviewed_production_destination_profile_v1,
    verify_reviewed_signer_proxy_account_identity_v1,
};

pub const REVIEWED_REMOTE_CREDENTIAL_PROOF_POLICY_V1_SCHEMA_VERSION: u32 = 1;
pub const PM_T2_REVIEWED_REMOTE_CREDENTIAL_PROOF_POLICY_FILE_V1: &str =
    "pm-t2-reviewed-remote-credential-proof-policy-v1.json";

const MAX_CANONICAL_REVIEWED_REMOTE_CREDENTIAL_PROOF_POLICY_BYTES_V1: usize = 128 * 1024;
const REVIEWED_REMOTE_CREDENTIAL_PROOF_POLICY_V1_FINGERPRINT_DOMAIN: &[u8] =
    b"reap.pm-t2.controlled-trial.reviewed-remote-credential-proof-policy.v1\0";

const API_AUTHENTICATION_SOURCE_ID_V1: &str = "api_authentication";
const API_AUTHENTICATION_SOURCE_URL_V1: &str = "https://docs.polymarket.com/getting-started/api.md";
const API_AUTHENTICATION_SOURCE_BYTE_LENGTH_V1: u64 = 9_391;
const API_AUTHENTICATION_SOURCE_SHA256_V1: &str =
    "6c397c66109852220b3f5d8033ea274061b3fc44b426edc9faa60673ecbef8fc";
const MANAGE_ORDERS_SOURCE_ID_V1: &str = "manage_orders";
const MANAGE_ORDERS_SOURCE_URL_V1: &str = "https://docs.polymarket.com/trading/manage-orders.md";
const MANAGE_ORDERS_SOURCE_BYTE_LENGTH_V1: u64 = 52_401;
const MANAGE_ORDERS_SOURCE_SHA256_V1: &str =
    "e4a0238db31d5137b4d0da0d4333b1fb90be8f7c7b47d92968edfd993c8c4482";
const OFFICIAL_SOURCE_CONTENT_TYPE_V1: &str = "text/markdown; charset=utf-8";

const PROTOCOL_SCHEME_V1: &str = "https";
const CLOB_DNS_NAME_V1: &str = "clob.polymarket.com";
const CLOB_TCP_PORT_V1: u16 = 443;
const SERVER_TIME_PATH_V1: &str = "/time";
const CLOSED_ONLY_METHOD_V1: &str = "GET";
const CLOSED_ONLY_PATH_V1: &str = "/auth/ban-status/closed-only";
const ABSENT_COMPONENT_V1: &str = "absent";
const ACCEPT_APPLICATION_JSON_V1: &str = "application/json";
const ACCEPT_ENCODING_IDENTITY_V1: &str = "identity";
const HMAC_TIMESTAMP_COMPONENT_V1: &str = "fresh_clob_server_unix_seconds_decimal_ascii";
const HMAC_ALGORITHM_V1: &str = "hmac_sha256";
const HMAC_KEY_SOURCE_V1: &str =
    "same_loaded_l2_credential_holder_decoded_url_safe_base64_secret_bytes";
const L2_SECRET_INPUT_ENCODING_V1: &str = "rfc4648_url_safe_base64_with_padding_canonical";
const MAXIMUM_L2_SECRET_ENCODED_BYTES_V1: u16 = 172;
const MINIMUM_HMAC_KEY_BYTES_V1: u16 = 1;
const MAXIMUM_HMAC_KEY_BYTES_V1: u16 = 128;
const HMAC_TIMESTAMP_DECIMAL_DIGITS_V1: u8 = 10;
const HMAC_MINIMUM_TIMESTAMP_UNIX_SECONDS_V1: u64 = 1_000_000_000;
const HMAC_MAXIMUM_TIMESTAMP_UNIX_SECONDS_V1: u64 = 9_999_999_999;
const HMAC_SEPARATOR_V1: &str = "none";
const HMAC_EXCLUDED_HEADER_ONLY_V1: &str = "excluded_from_preimage_header_only";
const HMAC_SECRET_ROLE_V1: &str = "hmac_key_only_not_preimage";
const HMAC_SIGNATURE_ENCODING_V1: &str = "rfc4648_url_safe_base64_with_padding";
const HMAC_SIGNATURE_DECODED_LENGTH_V1: u8 = 32;
const HMAC_SIGNATURE_ENCODED_LENGTH_V1: u8 = 44;
const HMAC_SIGNATURE_TERMINAL_PADDING_V1: &str = "=";
const POLY_ADDRESS_SOURCE_V1: &str = "canonical_config_signer_eip55";
const POLY_TIMESTAMP_SOURCE_V1: &str = "same_canonical_l2_timestamp_as_hmac_preimage";
const POLY_SIGNATURE_SOURCE_V1: &str = "exact_hmac_output";
const POLY_L2_HOLDER_SOURCE_V1: &str = "same_loaded_l2_credential_holder";
const MAXIMUM_SERVER_TIME_SAMPLE_TO_DISPATCH_AGE_MS_V1: u64 = 5_000;
const MAXIMUM_PROOF_OBSERVATION_AGE_MS_V1: u64 = 5_000;
const CONNECT_TIMEOUT_MS_V1: u64 = 3_000;
const REQUEST_TIMEOUT_MS_V1: u64 = 5_000;
const REQUIRED_CONTENT_TYPE_ESSENCE_V1: &str = "application/json";
const ALLOWED_CONTENT_TYPE_CHARSET_V1: &str = "none_or_single_charset_utf8_ascii_case_insensitive";
const ALLOWED_CONTENT_ENCODING_V1: &str = "absent_or_single_identity_ascii_case_insensitive";
const CLOSED_ONLY_FIELD_NAME_V1: &str = "closed_only";
const CLOSED_ONLY_FIELD_TYPE_V1: &str = "boolean";
const CLOSED_ONLY_ALLOWED_VALUES_V1: &str = "true_or_false_shape_only";
const CLOSED_ONLY_FALSE_SEMANTICS_V1: &str = "placement_candidate_evaluated_separately";
const CLOSED_ONLY_TRUE_SEMANTICS_V1: &str = "hard_block";
const CLOSED_ONLY_AUTHENTICATION_SEMANTICS_V1: &str =
    "neither_value_proves_authentication_acceptance_or_failure";

/// Exact canonical-byte pins for the reviewed production destination profile.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewedRemoteCredentialProofDestinationPinsV1 {
    pub schema_version: u32,
    pub profile_id: String,
    pub canonical_sha256: String,
    pub canonical_length: u64,
    pub fingerprint: String,
}

impl ReviewedRemoteCredentialProofDestinationPinsV1 {
    fn validate(&self) -> Result<(), PmReviewedRemoteCredentialProofPolicyV1Error> {
        validate_artifact_pins(
            self.schema_version,
            REVIEWED_PRODUCTION_DESTINATION_PROFILE_V1_SCHEMA_VERSION,
            &self.profile_id,
            &self.canonical_sha256,
            self.canonical_length,
            &self.fingerprint,
            "reviewed remote credential-proof destination pins are invalid",
        )
    }
}

/// Exact canonical-byte pins for the reviewed fresh credential locator.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewedRemoteCredentialProofLocatorPinsV1 {
    pub schema_version: u32,
    pub locator_id: String,
    pub canonical_sha256: String,
    pub canonical_length: u64,
    pub fingerprint: String,
}

impl ReviewedRemoteCredentialProofLocatorPinsV1 {
    fn validate(&self) -> Result<(), PmReviewedRemoteCredentialProofPolicyV1Error> {
        validate_artifact_pins(
            self.schema_version,
            REVIEWED_FRESH_CREDENTIAL_SLOT_LOCATOR_V1_SCHEMA_VERSION,
            &self.locator_id,
            &self.canonical_sha256,
            self.canonical_length,
            &self.fingerprint,
            "reviewed remote credential-proof locator pins are invalid",
        )
    }
}

/// Exact canonical-byte pins for the unsigned fresh credential delivery binding.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewedRemoteCredentialProofDeliveryPinsV1 {
    pub schema_version: u32,
    pub binding_id: String,
    pub canonical_sha256: String,
    pub canonical_length: u64,
    pub fingerprint: String,
}

impl ReviewedRemoteCredentialProofDeliveryPinsV1 {
    fn validate(&self) -> Result<(), PmReviewedRemoteCredentialProofPolicyV1Error> {
        validate_artifact_pins(
            self.schema_version,
            FRESH_CREDENTIAL_DELIVERY_BINDING_V1_SCHEMA_VERSION,
            &self.binding_id,
            &self.canonical_sha256,
            self.canonical_length,
            &self.fingerprint,
            "reviewed remote credential-proof delivery pins are invalid",
        )
    }
}

/// Exact canonical-byte pins for the reviewed signer/proxy account identity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewedRemoteCredentialProofAccountIdentityPinsV1 {
    pub schema_version: u32,
    pub identity_id: String,
    pub canonical_sha256: String,
    pub canonical_length: u64,
    pub fingerprint: String,
}

impl ReviewedRemoteCredentialProofAccountIdentityPinsV1 {
    fn validate(&self) -> Result<(), PmReviewedRemoteCredentialProofPolicyV1Error> {
        validate_artifact_pins(
            self.schema_version,
            REVIEWED_SIGNER_PROXY_ACCOUNT_IDENTITY_V1_SCHEMA_VERSION,
            &self.identity_id,
            &self.canonical_sha256,
            self.canonical_length,
            &self.fingerprint,
            "reviewed remote credential-proof account-identity pins are invalid",
        )
    }
}

/// One exact raw entry label from the locally frozen official-source manifest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewedRemoteCredentialProofSourceEntryPinsV1 {
    pub id: String,
    pub requested_url: String,
    pub final_url: String,
    pub retrieved_at_utc: String,
    pub content_type: String,
    pub byte_length: u64,
    pub sha256: String,
}

struct ExpectedSourceEntryV1 {
    id: &'static str,
    url: &'static str,
    byte_length: u64,
    sha256: &'static str,
}

impl ReviewedRemoteCredentialProofSourceEntryPinsV1 {
    fn validate_exact(
        &self,
        expected: ExpectedSourceEntryV1,
    ) -> Result<DateTime<Utc>, PmReviewedRemoteCredentialProofPolicyV1Error> {
        if self.id != expected.id
            || self.requested_url != expected.url
            || self.final_url != expected.url
            || self.retrieved_at_utc != PM_T2_OFFICIAL_SOURCE_MANIFEST_RETRIEVED_AT_UTC_V1
            || self.content_type != OFFICIAL_SOURCE_CONTENT_TYPE_V1
            || self.byte_length != expected.byte_length
            || self.sha256 != expected.sha256
        {
            return Err(invalid(
                "reviewed remote credential-proof official-source entry differs from the exact frozen V1 pin",
            ));
        }
        parse_utc(&self.retrieved_at_utc)
    }
}

/// Closed raw-manifest and two relevant source-entry pins.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewedRemoteCredentialProofOfficialSourcesV1 {
    pub manifest: ReviewedOfficialSourceManifestPinsV1,
    pub api_authentication: ReviewedRemoteCredentialProofSourceEntryPinsV1,
    pub manage_orders: ReviewedRemoteCredentialProofSourceEntryPinsV1,
}

impl ReviewedRemoteCredentialProofOfficialSourcesV1 {
    fn validate(&self) -> Result<[DateTime<Utc>; 3], PmReviewedRemoteCredentialProofPolicyV1Error> {
        if self.manifest.schema_family != PM_T2_OFFICIAL_SOURCE_MANIFEST_SCHEMA_FAMILY_V1
            || self.manifest.schema_version != PM_T2_OFFICIAL_SOURCE_MANIFEST_SCHEMA_VERSION_V1
            || self.manifest.retrieved_at_utc != PM_T2_OFFICIAL_SOURCE_MANIFEST_RETRIEVED_AT_UTC_V1
            || self.manifest.byte_length != PM_T2_OFFICIAL_SOURCE_MANIFEST_BYTE_LENGTH_V1
            || self.manifest.sha256 != PM_T2_OFFICIAL_SOURCE_MANIFEST_SHA256_V1
        {
            return Err(invalid(
                "reviewed remote credential-proof manifest differs from the exact raw V1 pin",
            ));
        }
        Ok([
            parse_utc(&self.manifest.retrieved_at_utc)?,
            self.api_authentication
                .validate_exact(ExpectedSourceEntryV1 {
                    id: API_AUTHENTICATION_SOURCE_ID_V1,
                    url: API_AUTHENTICATION_SOURCE_URL_V1,
                    byte_length: API_AUTHENTICATION_SOURCE_BYTE_LENGTH_V1,
                    sha256: API_AUTHENTICATION_SOURCE_SHA256_V1,
                })?,
            self.manage_orders.validate_exact(ExpectedSourceEntryV1 {
                id: MANAGE_ORDERS_SOURCE_ID_V1,
                url: MANAGE_ORDERS_SOURCE_URL_V1,
                byte_length: MANAGE_ORDERS_SOURCE_BYTE_LENGTH_V1,
                sha256: MANAGE_ORDERS_SOURCE_SHA256_V1,
            })?,
        ])
    }
}

/// Closed status for the missing authoritative remote acceptance contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReviewedRemoteCredentialAuthenticationAcceptanceContractStatusV1 {
    #[serde(rename = "unavailable_in_frozen_sources_v1")]
    UnavailableInFrozenSourcesV1,
}

/// Names only: no header value or serialized authentication material exists.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewedRemoteCredentialProofSensitiveHeaderNamesV1 {
    pub poly_address: String,
    pub poly_signature: String,
    pub poly_timestamp: String,
    pub poly_api_key: String,
    pub poly_passphrase: String,
}

impl ReviewedRemoteCredentialProofSensitiveHeaderNamesV1 {
    fn validate(&self) -> Result<(), PmReviewedRemoteCredentialProofPolicyV1Error> {
        if self.poly_address != "POLY_ADDRESS"
            || self.poly_signature != "POLY_SIGNATURE"
            || self.poly_timestamp != "POLY_TIMESTAMP"
            || self.poly_api_key != "POLY_API_KEY"
            || self.poly_passphrase != "POLY_PASSPHRASE"
        {
            return Err(invalid(
                "reviewed remote credential-proof sensitive header names differ from the exact five-name grammar",
            ));
        }
        Ok(())
    }
}

/// Semantic HMAC-preimage grammar only; contains no timestamp or secret value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewedRemoteCredentialProofHmacPreimageGrammarV1 {
    pub hmac_algorithm: String,
    pub hmac_key_source: String,
    pub l2_secret_input_encoding: String,
    pub maximum_l2_secret_encoded_bytes: u16,
    pub minimum_hmac_key_bytes: u16,
    pub maximum_hmac_key_bytes: u16,
    pub ordered_variant: ReviewedRemoteCredentialProofHmacPreimageOrderedVariantV1,
    pub timestamp_component: String,
    pub timestamp_decimal_digits: u8,
    pub minimum_timestamp_unix_seconds: u64,
    pub maximum_timestamp_unix_seconds: u64,
    pub method_component: String,
    pub path_component: String,
    pub separator: String,
    pub query_component: String,
    pub body_component: String,
    pub poly_address_component: String,
    pub poly_api_key_component: String,
    pub poly_passphrase_component: String,
    pub l2_secret_role: String,
    pub signature_encoding: String,
    pub signature_decoded_length: u8,
    pub signature_encoded_length: u8,
    pub signature_terminal_padding: String,
}

/// The only HMAC-preimage ordering admitted by V1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReviewedRemoteCredentialProofHmacPreimageOrderedVariantV1 {
    #[serde(rename = "decimal_timestamp_then_uppercase_get_then_exact_path_no_separators_v1")]
    DecimalTimestampThenUppercaseGetThenExactPathNoSeparatorsV1,
}

impl ReviewedRemoteCredentialProofHmacPreimageGrammarV1 {
    fn validate(&self) -> Result<(), PmReviewedRemoteCredentialProofPolicyV1Error> {
        if self.hmac_algorithm != HMAC_ALGORITHM_V1
            || self.hmac_key_source != HMAC_KEY_SOURCE_V1
            || self.l2_secret_input_encoding != L2_SECRET_INPUT_ENCODING_V1
            || self.maximum_l2_secret_encoded_bytes != MAXIMUM_L2_SECRET_ENCODED_BYTES_V1
            || self.minimum_hmac_key_bytes != MINIMUM_HMAC_KEY_BYTES_V1
            || self.maximum_hmac_key_bytes != MAXIMUM_HMAC_KEY_BYTES_V1
            || self.timestamp_component != HMAC_TIMESTAMP_COMPONENT_V1
            || self.timestamp_decimal_digits != HMAC_TIMESTAMP_DECIMAL_DIGITS_V1
            || self.minimum_timestamp_unix_seconds != HMAC_MINIMUM_TIMESTAMP_UNIX_SECONDS_V1
            || self.maximum_timestamp_unix_seconds != HMAC_MAXIMUM_TIMESTAMP_UNIX_SECONDS_V1
            || self.method_component != CLOSED_ONLY_METHOD_V1
            || self.path_component != CLOSED_ONLY_PATH_V1
            || self.separator != HMAC_SEPARATOR_V1
            || self.query_component != ABSENT_COMPONENT_V1
            || self.body_component != ABSENT_COMPONENT_V1
            || self.poly_address_component != HMAC_EXCLUDED_HEADER_ONLY_V1
            || self.poly_api_key_component != HMAC_EXCLUDED_HEADER_ONLY_V1
            || self.poly_passphrase_component != HMAC_EXCLUDED_HEADER_ONLY_V1
            || self.l2_secret_role != HMAC_SECRET_ROLE_V1
            || self.signature_encoding != HMAC_SIGNATURE_ENCODING_V1
            || self.signature_decoded_length != HMAC_SIGNATURE_DECODED_LENGTH_V1
            || self.signature_encoded_length != HMAC_SIGNATURE_ENCODED_LENGTH_V1
            || self.signature_terminal_padding != HMAC_SIGNATURE_TERMINAL_PADDING_V1
        {
            return Err(invalid(
                "reviewed remote credential-proof HMAC preimage grammar drifted",
            ));
        }
        Ok(())
    }
}

/// Exact HTTPS origin, selected reviewed peer, and authorized local egress.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewedRemoteCredentialProofEndpointPolicyV1 {
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
}

impl ReviewedRemoteCredentialProofEndpointPolicyV1 {
    fn validate(&self) -> Result<(), PmReviewedRemoteCredentialProofPolicyV1Error> {
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
                "reviewed remote credential-proof HTTPS endpoint grammar drifted",
            ));
        }
        let peer = IpAddr::from_str(&self.selected_peer_ip)
            .map_err(|_| invalid("reviewed remote credential-proof selected peer IP is invalid"))?;
        let egress = IpAddr::from_str(&self.local_egress_ip)
            .map_err(|_| invalid("reviewed remote credential-proof local egress IP is invalid"))?;
        validate_token(
            &self.interface_name,
            64,
            "reviewed remote credential-proof interface name is invalid",
        )?;
        validate_reference(
            &self.dedicated_tunnel_or_gateway_profile_reference,
            "reviewed remote credential-proof tunnel or gateway profile reference is invalid",
        )?;
        validate_sha256(&self.dedicated_tunnel_or_gateway_profile_sha256)?;
        if !same_address_family(peer, egress) {
            return Err(invalid(
                "reviewed remote credential-proof selected peer and local egress families differ",
            ));
        }
        Ok(())
    }
}

/// Exact fixed authenticated GET grammar.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewedRemoteCredentialProofRequestPolicyV1 {
    pub method: String,
    pub path: String,
    pub query: String,
    pub body: String,
    pub content_type: String,
    pub accept: String,
    pub accept_encoding: String,
    pub sensitive_header_names: ReviewedRemoteCredentialProofSensitiveHeaderNamesV1,
    pub hmac_preimage: ReviewedRemoteCredentialProofHmacPreimageGrammarV1,
    pub poly_address_source: String,
    pub poly_timestamp_source: String,
    pub poly_signature_source: String,
    pub poly_api_key_source: String,
    pub poly_passphrase_source: String,
}

impl ReviewedRemoteCredentialProofRequestPolicyV1 {
    fn validate(&self) -> Result<(), PmReviewedRemoteCredentialProofPolicyV1Error> {
        if self.method != CLOSED_ONLY_METHOD_V1
            || self.path != CLOSED_ONLY_PATH_V1
            || self.query != ABSENT_COMPONENT_V1
            || self.body != ABSENT_COMPONENT_V1
            || self.content_type != ABSENT_COMPONENT_V1
            || self.accept != ACCEPT_APPLICATION_JSON_V1
            || self.accept_encoding != ACCEPT_ENCODING_IDENTITY_V1
            || self.poly_address_source != POLY_ADDRESS_SOURCE_V1
            || self.poly_timestamp_source != POLY_TIMESTAMP_SOURCE_V1
            || self.poly_signature_source != POLY_SIGNATURE_SOURCE_V1
            || self.poly_api_key_source != POLY_L2_HOLDER_SOURCE_V1
            || self.poly_passphrase_source != POLY_L2_HOLDER_SOURCE_V1
        {
            return Err(invalid(
                "reviewed remote credential-proof authenticated request grammar drifted",
            ));
        }
        self.sensitive_header_names.validate()?;
        self.hmac_preimage.validate()
    }
}

/// Two distinct five-second freshness obligations for a future actor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewedRemoteCredentialProofFreshnessPolicyV1 {
    pub server_time_path: String,
    pub server_time_sample_required: bool,
    pub server_time_sample_must_precede_authenticated_dispatch: bool,
    pub maximum_server_time_sample_to_dispatch_age_ms: u64,
    pub maximum_proof_observation_age_ms: u64,
}

impl ReviewedRemoteCredentialProofFreshnessPolicyV1 {
    fn validate(&self) -> Result<(), PmReviewedRemoteCredentialProofPolicyV1Error> {
        if self.server_time_path != SERVER_TIME_PATH_V1
            || !self.server_time_sample_required
            || !self.server_time_sample_must_precede_authenticated_dispatch
            || self.maximum_server_time_sample_to_dispatch_age_ms
                != MAXIMUM_SERVER_TIME_SAMPLE_TO_DISPATCH_AGE_MS_V1
            || self.maximum_proof_observation_age_ms != MAXIMUM_PROOF_OBSERVATION_AGE_MS_V1
        {
            return Err(invalid(
                "reviewed remote credential-proof freshness policy drifted",
            ));
        }
        Ok(())
    }
}

/// One-dispatch transport requirements; none is runtime evidence here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewedRemoteCredentialProofDispatchPolicyV1 {
    pub maximum_authenticated_dispatch_count: u8,
    pub connect_timeout_ms: u64,
    pub request_timeout_ms: u64,
    pub redirects_allowed: bool,
    pub retries_allowed: bool,
    pub forward_proxy_allowed: bool,
    pub destination_fallback_allowed: bool,
    pub connected_peer_check_before_status_and_body_required: bool,
    pub ambiguous_outcome_requires_durable_burn: bool,
}

impl ReviewedRemoteCredentialProofDispatchPolicyV1 {
    fn validate(&self) -> Result<(), PmReviewedRemoteCredentialProofPolicyV1Error> {
        if self.maximum_authenticated_dispatch_count != 1
            || self.connect_timeout_ms != CONNECT_TIMEOUT_MS_V1
            || self.request_timeout_ms != REQUEST_TIMEOUT_MS_V1
            || self.redirects_allowed
            || self.retries_allowed
            || self.forward_proxy_allowed
            || self.destination_fallback_allowed
            || !self.connected_peer_check_before_status_and_body_required
            || !self.ambiguous_outcome_requires_durable_burn
        {
            return Err(invalid(
                "reviewed remote credential-proof dispatch policy drifted",
            ));
        }
        Ok(())
    }
}

/// Strict response grammar. Either boolean is shape-valid; readiness is later.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewedRemoteCredentialProofResponsePolicyV1 {
    pub required_status_code: u16,
    pub content_type_header_required: bool,
    pub required_content_type_essence: String,
    pub allowed_content_type_charset: String,
    pub allowed_content_encoding: String,
    pub maximum_body_bytes: u64,
    pub required_json_object_field_count: u8,
    pub required_json_field_name: String,
    pub required_json_field_type: String,
    pub allowed_json_boolean_values: String,
    pub closed_only_false_semantics: String,
    pub closed_only_true_semantics: String,
    pub authentication_semantics: String,
}

impl ReviewedRemoteCredentialProofResponsePolicyV1 {
    fn validate(&self) -> Result<(), PmReviewedRemoteCredentialProofPolicyV1Error> {
        if self.required_status_code != 200
            || !self.content_type_header_required
            || self.required_content_type_essence != REQUIRED_CONTENT_TYPE_ESSENCE_V1
            || self.allowed_content_type_charset != ALLOWED_CONTENT_TYPE_CHARSET_V1
            || self.allowed_content_encoding != ALLOWED_CONTENT_ENCODING_V1
            || self.maximum_body_bytes != 64
            || self.required_json_object_field_count != 1
            || self.required_json_field_name != CLOSED_ONLY_FIELD_NAME_V1
            || self.required_json_field_type != CLOSED_ONLY_FIELD_TYPE_V1
            || self.allowed_json_boolean_values != CLOSED_ONLY_ALLOWED_VALUES_V1
            || self.closed_only_false_semantics != CLOSED_ONLY_FALSE_SEMANTICS_V1
            || self.closed_only_true_semantics != CLOSED_ONLY_TRUE_SEMANTICS_V1
            || self.authentication_semantics != CLOSED_ONLY_AUTHENTICATION_SEMANTICS_V1
        {
            return Err(invalid(
                "reviewed remote credential-proof response grammar drifted",
            ));
        }
        Ok(())
    }
}

/// Closed protocol conjunction for one future closed-only proof read.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewedRemoteCredentialProofProtocolPolicyV1 {
    pub endpoint: ReviewedRemoteCredentialProofEndpointPolicyV1,
    pub request: ReviewedRemoteCredentialProofRequestPolicyV1,
    pub freshness: ReviewedRemoteCredentialProofFreshnessPolicyV1,
    pub dispatch: ReviewedRemoteCredentialProofDispatchPolicyV1,
    pub response: ReviewedRemoteCredentialProofResponsePolicyV1,
}

impl ReviewedRemoteCredentialProofProtocolPolicyV1 {
    fn validate(&self) -> Result<(), PmReviewedRemoteCredentialProofPolicyV1Error> {
        self.endpoint.validate()?;
        self.request.validate()?;
        self.freshness.validate()?;
        self.dispatch.validate()?;
        self.response.validate()
    }
}

/// Reviewer-labeled, non-authoritative policy sidecar for a future live proof.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewedRemoteCredentialProofPolicyV1 {
    pub schema_version: u32,
    pub policy_id: String,
    pub reviewer_label: String,
    pub reviewed_at_utc: String,
    pub valid_not_before_utc: String,
    pub valid_not_after_utc: String,
    pub v1_config: V1ConfigPinsV2,
    pub online_policy: OnlinePolicyPinsV2,
    pub online_authorization: ReviewedOnlineAuthorizationPinsV1,
    pub reviewed_destination: ReviewedRemoteCredentialProofDestinationPinsV1,
    pub reviewed_fresh_credential_locator: ReviewedRemoteCredentialProofLocatorPinsV1,
    pub fresh_credential_delivery: ReviewedRemoteCredentialProofDeliveryPinsV1,
    pub reviewed_signer_proxy_identity: ReviewedRemoteCredentialProofAccountIdentityPinsV1,
    pub official_sources: ReviewedRemoteCredentialProofOfficialSourcesV1,
    pub authentication_acceptance_contract_status:
        ReviewedRemoteCredentialAuthenticationAcceptanceContractStatusV1,
    pub protocol: ReviewedRemoteCredentialProofProtocolPolicyV1,
}

impl fmt::Debug for ReviewedRemoteCredentialProofPolicyV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(
            "ReviewedRemoteCredentialProofPolicyV1(<reviewer-labeled-protocol-and-source-pins; route-address-and-request-redacted; acceptance-contract-unavailable; denied>)",
        )
    }
}

struct ValidatedReviewedRemoteCredentialProofPolicyTimesV1 {
    reviewed_at: DateTime<Utc>,
    valid_not_before: DateTime<Utc>,
    valid_not_after: DateTime<Utc>,
}

impl ReviewedRemoteCredentialProofPolicyV1 {
    fn validate_intrinsic(
        &self,
    ) -> Result<
        ValidatedReviewedRemoteCredentialProofPolicyTimesV1,
        PmReviewedRemoteCredentialProofPolicyV1Error,
    > {
        if self.schema_version != REVIEWED_REMOTE_CREDENTIAL_PROOF_POLICY_V1_SCHEMA_VERSION {
            return Err(invalid(
                "unsupported reviewed remote credential-proof policy V1 schema",
            ));
        }
        validate_token(
            &self.policy_id,
            128,
            "reviewed remote credential-proof policy ID is invalid",
        )?;
        validate_reference(
            &self.reviewer_label,
            "reviewed remote credential-proof reviewer label is invalid",
        )?;
        validate_v1_config_pins(&self.v1_config)?;
        validate_online_policy_pins(&self.online_policy)?;
        validate_online_authorization_pins(&self.online_authorization)?;
        self.reviewed_destination.validate()?;
        self.reviewed_fresh_credential_locator.validate()?;
        self.fresh_credential_delivery.validate()?;
        self.reviewed_signer_proxy_identity.validate()?;
        let source_retrieval_times = self.official_sources.validate()?;
        self.protocol.validate()?;

        let reviewed_at = parse_utc(&self.reviewed_at_utc)?;
        let valid_not_before = parse_utc(&self.valid_not_before_utc)?;
        let valid_not_after = parse_utc(&self.valid_not_after_utc)?;
        if source_retrieval_times
            .into_iter()
            .any(|retrieved_at| retrieved_at > reviewed_at)
            || reviewed_at > valid_not_before
            || valid_not_before >= valid_not_after
        {
            return Err(invalid(
                "reviewed remote credential-proof policy time envelope is invalid",
            ));
        }
        Ok(ValidatedReviewedRemoteCredentialProofPolicyTimesV1 {
            reviewed_at,
            valid_not_before,
            valid_not_after,
        })
    }
}

/// Move-only, non-serializable holder of exact protected canonical bytes.
pub struct CanonicalReviewedRemoteCredentialProofPolicyV1 {
    value: ReviewedRemoteCredentialProofPolicyV1,
    canonical_bytes: Vec<u8>,
    canonical_sha256: String,
    fingerprint: String,
}

impl CanonicalReviewedRemoteCredentialProofPolicyV1 {
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

impl fmt::Debug for CanonicalReviewedRemoteCredentialProofPolicyV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(
            "CanonicalReviewedRemoteCredentialProofPolicyV1(<exact-protected-canonical-bytes; no-value-route-address-source-or-request-projection; redacted; denied>)",
        )
    }
}

/// Borrowed pre-bind exact-holder conjunction; constructing it grants no
/// capability.
///
/// The locator and delivery fields are raw canonical holders only, not
/// retained delivery evidence or a post-split load-token join. They establish
/// no same-loaded-holder or runtime actor/generation conjunction. A future V3
/// must join the whole retained evidence and load token to the selected actor
/// generation before projection.
pub struct ReviewedRemoteCredentialProofPolicyContextV1<'a> {
    pub config: &'a CanonicalTrialConfig,
    pub online_policy: &'a CanonicalOnlinePolicyV2,
    pub online_authorization: &'a CanonicalOnlineAuthorizationV2,
    pub reviewed_destination: &'a CanonicalReviewedProductionDestinationProfileV1,
    pub reviewed_fresh_credential_locator: &'a CanonicalReviewedFreshCredentialSlotLocatorV1,
    pub fresh_credential_delivery: &'a CanonicalFreshCredentialDeliveryBindingV1,
    pub reviewed_signer_proxy_identity: &'a CanonicalReviewedSignerProxyAccountIdentityV1,
}

/// Offline structural display. Every runtime and authority fact remains false.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ReviewedRemoteCredentialProofPolicyVerificationV1 {
    pub schema_version: u32,
    pub config_fingerprint: String,
    pub online_policy_fingerprint: String,
    pub online_authorization_fingerprint: String,
    pub reviewed_destination_fingerprint: String,
    pub reviewed_fresh_credential_locator_fingerprint: String,
    pub fresh_credential_delivery_fingerprint: String,
    pub reviewed_signer_proxy_identity_fingerprint: String,
    pub reviewed_remote_credential_proof_policy_fingerprint: String,
    pub exact_config_policy_authorization_pins_structurally_valid: bool,
    pub exact_destination_locator_delivery_identity_pins_structurally_valid: bool,
    pub exact_official_source_manifest_and_entries_structurally_valid: bool,
    pub exact_closed_only_protocol_policy_structurally_valid: bool,
    pub validity_envelope_nested_within_online_authorization_v2: bool,
    pub selected_peer_and_local_egress_labels_match_bound_records: bool,
    pub official_source_manifest_bytes_loaded_and_hash_verified: bool,
    pub api_authentication_source_bytes_loaded_and_hash_verified: bool,
    pub manage_orders_source_bytes_loaded_and_hash_verified: bool,
    pub official_source_publisher_authorship_attested: bool,
    pub official_source_manifest_publisher_authorship_attested: bool,
    pub api_authentication_source_publisher_authorship_attested: bool,
    pub manage_orders_source_publisher_authorship_attested: bool,
    pub reviewer_authorship_attested: bool,
    pub remote_api_key_owner_attested: bool,
    pub authoritative_authentication_acceptance_contract_available: bool,
    pub api_key_mismatch_rejected_before_http_200_attested: bool,
    pub l2_secret_mismatch_rejected_before_http_200_attested: bool,
    pub passphrase_mismatch_rejected_before_http_200_attested: bool,
    pub poly_address_mismatch_rejected_before_http_200_attested: bool,
    pub timestamp_mismatch_rejected_before_http_200_attested: bool,
    pub authentication_precedes_closed_only_handler_attested: bool,
    pub response_not_shared_or_cache_derived_attested: bool,
    pub strict_http_200_implies_live_credential_tuple_acceptance_attested: bool,
    pub credential_provider_authorship_attested: bool,
    pub credential_delivery_generation_attested: bool,
    pub same_loaded_credential_holder_attested: bool,
    pub post_load_same_holder_runtime_conjunction_attested: bool,
    pub credential_delivery_and_remote_proof_same_source_generation_attested: bool,
    pub globally_unique_credential_delivery_attested: bool,
    pub rotation_generation_attested: bool,
    pub protected_credential_directory_and_four_objects_checked: bool,
    pub loaded_credentials_match_delivery_binding: bool,
    pub request_l2_tuple_from_same_loaded_credential_holder_checked: bool,
    pub selected_actor_generation_bound: bool,
    pub product_clock_owner_bound: bool,
    pub retained_delivery_evidence_and_load_token_joined_for_selected_actor: bool,
    pub private_key_derived_signer_matches_config_checked: bool,
    pub l2_credentials_match_configured_signer_checked: bool,
    pub signer_controls_proxy_attested: bool,
    pub signer_proxy_relationship_current_and_unrevoked_attested: bool,
    pub server_time_sample_received: bool,
    pub server_time_proof_authenticated_and_fresh: bool,
    pub source_owned_current_time_checked: bool,
    pub server_time_sample_to_dispatch_freshness_checked: bool,
    pub server_time_and_closed_only_same_peer_pairing_checked: bool,
    pub proof_observation_freshness_checked: bool,
    pub response_receive_freshness_checked: bool,
    pub poly_address_header_from_configured_signer_produced: bool,
    pub sensitive_request_headers_produced: bool,
    pub request_query_body_and_content_type_absence_enforced: bool,
    pub request_accept_application_json_header_produced: bool,
    pub accept_encoding_identity_header_produced: bool,
    pub hmac_preimage_produced: bool,
    pub hmac_signature_produced: bool,
    pub fixed_local_egress_selected_and_checked: bool,
    pub fixed_reviewed_peer_selected_and_checked: bool,
    pub network_namespace_and_interface_selected_and_checked: bool,
    pub tunnel_or_gateway_profile_checked: bool,
    pub live_dns_answer_checked: bool,
    pub dnssec_checked: bool,
    pub dns_ttl_freshness_checked: bool,
    pub destination_nat_equivalence_checked: bool,
    pub authorized_public_ip_checked: bool,
    pub connect_and_request_timeouts_enforced: bool,
    pub authenticated_dispatch_performed_once: bool,
    pub redirect_retry_proxy_and_fallback_absence_enforced: bool,
    pub response_received: bool,
    pub connected_peer_checked_before_status_and_body: bool,
    pub tls_server_identity_verified: bool,
    pub http_status_200_checked: bool,
    pub response_content_type_checked: bool,
    pub response_content_encoding_checked: bool,
    pub response_body_length_and_exact_schema_checked: bool,
    pub closed_only_boolean_observed: bool,
    pub closed_only_false_readiness_checked: bool,
    pub closed_only_true_hard_block_checked: bool,
    pub ambiguous_outcome_durable_burn_performed: bool,
    pub live_credential_tuple_accepted_by_provider: bool,
    pub credential_tuple_current_and_unrevoked_attested: bool,
    pub online_authorization_v2_reverse_pins_remote_policy: bool,
    pub reviewed_destination_reverse_pins_remote_policy: bool,
    pub reviewed_locator_reverse_pins_remote_policy: bool,
    pub fresh_delivery_reverse_pins_remote_policy: bool,
    pub reviewed_identity_reverse_pins_remote_policy: bool,
    pub remote_policy_fingerprint_pinned_by_online_authorization_v2: bool,
    pub remote_policy_fingerprint_pinned_by_v3: bool,
    pub remote_policy_consumption_durably_recorded: bool,
    pub authorization_consumption_checked: bool,
    pub credential_mutation_authority_attested: bool,
    #[serde(flatten)]
    pub authorization: OfflineAuthorizationState,
}

pub fn load_canonical_reviewed_remote_credential_proof_policy_v1(
    path: &Path,
) -> Result<
    CanonicalReviewedRemoteCredentialProofPolicyV1,
    PmReviewedRemoteCredentialProofPolicyV1Error,
> {
    let bytes = read_one(
        path,
        ProtectedFileKind::ReviewedRemoteCredentialProofPolicyV1,
        MAX_CANONICAL_REVIEWED_REMOTE_CREDENTIAL_PROOF_POLICY_BYTES_V1,
    )
    .map_err(|_| {
        invalid("reviewed remote credential-proof policy protection or stability check failed")
    })?;
    let value: ReviewedRemoteCredentialProofPolicyV1 = parse_exact_canonical(&bytes)?;
    let _ = value.validate_intrinsic()?;
    let canonical_bytes = bytes.to_vec();
    Ok(CanonicalReviewedRemoteCredentialProofPolicyV1 {
        canonical_sha256: hash_bytes(&[], &canonical_bytes),
        fingerprint: hash_bytes(
            REVIEWED_REMOTE_CREDENTIAL_PROOF_POLICY_V1_FINGERPRINT_DOMAIN,
            &canonical_bytes,
        ),
        canonical_bytes,
        value,
    })
}

/// Verify exact borrowed structural bindings without a clock, network, or load.
pub fn verify_reviewed_remote_credential_proof_policy_v1(
    context: &ReviewedRemoteCredentialProofPolicyContextV1<'_>,
    reviewed_policy: &CanonicalReviewedRemoteCredentialProofPolicyV1,
) -> Result<
    ReviewedRemoteCredentialProofPolicyVerificationV1,
    PmReviewedRemoteCredentialProofPolicyV1Error,
> {
    let authorization_times = validate_online_authorization_contract_v2(
        context.config,
        context.online_policy,
        context.online_authorization,
    )
    .map_err(|_| invalid("reviewed remote credential-proof bound online V2 contract is invalid"))?;
    let reviewed_times = reviewed_policy.value.validate_intrinsic()?;
    if !v1_config_pins_match(&reviewed_policy.value.v1_config, context.config)
        || !online_policy_pins_match(&reviewed_policy.value.online_policy, context.online_policy)
        || !online_authorization_pins_match(
            &reviewed_policy.value.online_authorization,
            context.online_authorization,
        )
    {
        return Err(invalid(
            "reviewed remote credential-proof config, policy, or authorization pins mismatched",
        ));
    }
    if reviewed_times.reviewed_at < authorization_times.reviewed_at
        || reviewed_times.reviewed_at > authorization_times.not_before
        || reviewed_times.valid_not_before != authorization_times.not_before
        || reviewed_times.valid_not_after != authorization_times.cleanup_not_after
    {
        return Err(invalid(
            "reviewed remote credential-proof validity differs from the bound authorization envelope",
        ));
    }
    if reviewed_policy.value.official_sources.manifest.sha256
        != context.config.value().source_pin_manifest_sha256
    {
        return Err(invalid(
            "reviewed remote credential-proof manifest SHA label differs from config",
        ));
    }

    let destination_verification = verify_reviewed_production_destination_profile_v1(
        context.config,
        context.online_policy,
        context.online_authorization,
        context.reviewed_destination,
    )
    .map_err(|_| invalid("reviewed remote credential-proof destination profile is invalid"))?;
    let locator_verification = verify_reviewed_fresh_credential_slot_locator_v1(
        context.config,
        context.online_policy,
        context.online_authorization,
        context.reviewed_fresh_credential_locator,
    )
    .map_err(|_| invalid("reviewed remote credential-proof fresh locator is invalid"))?;
    let delivery_verification = verify_fresh_credential_delivery_binding_v1(
        context.config,
        context.online_policy,
        context.online_authorization,
        context.reviewed_fresh_credential_locator,
        context.fresh_credential_delivery,
    )
    .map_err(|_| invalid("reviewed remote credential-proof delivery binding is invalid"))?;
    let identity_verification = verify_reviewed_signer_proxy_account_identity_v1(
        context.config,
        context.online_policy,
        context.online_authorization,
        context.reviewed_signer_proxy_identity,
    )
    .map_err(|_| invalid("reviewed remote credential-proof account identity is invalid"))?;

    if !destination_pins_match(
        &reviewed_policy.value.reviewed_destination,
        destination_verification.schema_version,
        &destination_verification.profile_id,
        context.reviewed_destination.canonical_sha256(),
        context.reviewed_destination.canonical_length(),
        context.reviewed_destination.fingerprint(),
    ) || !locator_pins_match(
        &reviewed_policy.value.reviewed_fresh_credential_locator,
        locator_verification.schema_version,
        &locator_verification.locator_id,
        context.reviewed_fresh_credential_locator.canonical_sha256(),
        context.reviewed_fresh_credential_locator.canonical_length(),
        context.reviewed_fresh_credential_locator.fingerprint(),
    ) || !delivery_pins_match(
        &reviewed_policy.value.fresh_credential_delivery,
        delivery_verification.schema_version,
        &delivery_verification.binding_id,
        context.fresh_credential_delivery.canonical_sha256(),
        context.fresh_credential_delivery.canonical_length(),
        context.fresh_credential_delivery.fingerprint(),
    ) || !identity_pins_match(
        &reviewed_policy.value.reviewed_signer_proxy_identity,
        identity_verification.schema_version,
        &identity_verification.identity_id,
        context.reviewed_signer_proxy_identity.canonical_sha256(),
        context.reviewed_signer_proxy_identity.canonical_length(),
        context.reviewed_signer_proxy_identity.fingerprint(),
    ) {
        return Err(invalid(
            "reviewed remote credential-proof destination, locator, delivery, or identity pins mismatched",
        ));
    }

    let endpoint = &reviewed_policy.value.protocol.endpoint;
    let reviewed_clob = &context.reviewed_destination.value().destinations.clob_https;
    if endpoint.dns_name != reviewed_clob.dns_name
        || endpoint.tls_server_name != reviewed_clob.tls_server_name
        || endpoint.http_host != reviewed_clob.http_host
        || endpoint.tcp_port != reviewed_clob.tcp_port
        || endpoint.selected_peer_ip != reviewed_clob.peer_ip
        || endpoint.network_namespace_device
            != context
                .online_authorization
                .value()
                .host
                .egress
                .network_namespace_device
        || endpoint.network_namespace_inode
            != context
                .online_authorization
                .value()
                .host
                .egress
                .network_namespace_inode
        || endpoint.interface_name
            != context
                .online_authorization
                .value()
                .host
                .egress
                .interface_name
        || endpoint.interface_index
            != context
                .online_authorization
                .value()
                .host
                .egress
                .interface_index
        || endpoint.local_egress_ip
            != context
                .online_authorization
                .value()
                .host
                .egress
                .local_source_ip
        || endpoint.dedicated_tunnel_or_gateway_profile_reference
            != context
                .online_authorization
                .value()
                .host
                .egress
                .dedicated_tunnel_or_gateway_profile_reference
        || endpoint.dedicated_tunnel_or_gateway_profile_sha256
            != context
                .online_authorization
                .value()
                .host
                .egress
                .dedicated_tunnel_or_gateway_profile_sha256
    {
        return Err(invalid(
            "reviewed remote credential-proof selected peer or local egress label mismatched",
        ));
    }
    let freshness = &reviewed_policy.value.protocol.freshness;
    if freshness.maximum_server_time_sample_to_dispatch_age_ms
        != context
            .config
            .value()
            .time_limits
            .maximum_preflight_observation_age_ms
        || freshness.maximum_proof_observation_age_ms
            != context.online_policy.value().maximum_observation_age_ms
    {
        return Err(invalid(
            "reviewed remote credential-proof freshness bounds differ from config or online policy",
        ));
    }

    Ok(ReviewedRemoteCredentialProofPolicyVerificationV1 {
        schema_version: reviewed_policy.value.schema_version,
        config_fingerprint: context.config.fingerprint().to_owned(),
        online_policy_fingerprint: context.online_policy.fingerprint().to_owned(),
        online_authorization_fingerprint: context.online_authorization.fingerprint().to_owned(),
        reviewed_destination_fingerprint: context.reviewed_destination.fingerprint().to_owned(),
        reviewed_fresh_credential_locator_fingerprint: context
            .reviewed_fresh_credential_locator
            .fingerprint()
            .to_owned(),
        fresh_credential_delivery_fingerprint: context
            .fresh_credential_delivery
            .fingerprint()
            .to_owned(),
        reviewed_signer_proxy_identity_fingerprint: context
            .reviewed_signer_proxy_identity
            .fingerprint()
            .to_owned(),
        reviewed_remote_credential_proof_policy_fingerprint: reviewed_policy.fingerprint.clone(),
        exact_config_policy_authorization_pins_structurally_valid: true,
        exact_destination_locator_delivery_identity_pins_structurally_valid: true,
        exact_official_source_manifest_and_entries_structurally_valid: true,
        exact_closed_only_protocol_policy_structurally_valid: true,
        validity_envelope_nested_within_online_authorization_v2: true,
        selected_peer_and_local_egress_labels_match_bound_records: true,
        official_source_manifest_bytes_loaded_and_hash_verified: false,
        api_authentication_source_bytes_loaded_and_hash_verified: false,
        manage_orders_source_bytes_loaded_and_hash_verified: false,
        official_source_publisher_authorship_attested: false,
        official_source_manifest_publisher_authorship_attested: false,
        api_authentication_source_publisher_authorship_attested: false,
        manage_orders_source_publisher_authorship_attested: false,
        reviewer_authorship_attested: false,
        remote_api_key_owner_attested: false,
        authoritative_authentication_acceptance_contract_available: false,
        api_key_mismatch_rejected_before_http_200_attested: false,
        l2_secret_mismatch_rejected_before_http_200_attested: false,
        passphrase_mismatch_rejected_before_http_200_attested: false,
        poly_address_mismatch_rejected_before_http_200_attested: false,
        timestamp_mismatch_rejected_before_http_200_attested: false,
        authentication_precedes_closed_only_handler_attested: false,
        response_not_shared_or_cache_derived_attested: false,
        strict_http_200_implies_live_credential_tuple_acceptance_attested: false,
        credential_provider_authorship_attested: false,
        credential_delivery_generation_attested: false,
        same_loaded_credential_holder_attested: false,
        post_load_same_holder_runtime_conjunction_attested: false,
        credential_delivery_and_remote_proof_same_source_generation_attested: false,
        globally_unique_credential_delivery_attested: false,
        rotation_generation_attested: false,
        protected_credential_directory_and_four_objects_checked: false,
        loaded_credentials_match_delivery_binding: false,
        request_l2_tuple_from_same_loaded_credential_holder_checked: false,
        selected_actor_generation_bound: false,
        product_clock_owner_bound: false,
        retained_delivery_evidence_and_load_token_joined_for_selected_actor: false,
        private_key_derived_signer_matches_config_checked: false,
        l2_credentials_match_configured_signer_checked: false,
        signer_controls_proxy_attested: false,
        signer_proxy_relationship_current_and_unrevoked_attested: false,
        server_time_sample_received: false,
        server_time_proof_authenticated_and_fresh: false,
        source_owned_current_time_checked: false,
        server_time_sample_to_dispatch_freshness_checked: false,
        server_time_and_closed_only_same_peer_pairing_checked: false,
        proof_observation_freshness_checked: false,
        response_receive_freshness_checked: false,
        poly_address_header_from_configured_signer_produced: false,
        sensitive_request_headers_produced: false,
        request_query_body_and_content_type_absence_enforced: false,
        request_accept_application_json_header_produced: false,
        accept_encoding_identity_header_produced: false,
        hmac_preimage_produced: false,
        hmac_signature_produced: false,
        fixed_local_egress_selected_and_checked: false,
        fixed_reviewed_peer_selected_and_checked: false,
        network_namespace_and_interface_selected_and_checked: false,
        tunnel_or_gateway_profile_checked: false,
        live_dns_answer_checked: false,
        dnssec_checked: false,
        dns_ttl_freshness_checked: false,
        destination_nat_equivalence_checked: false,
        authorized_public_ip_checked: false,
        connect_and_request_timeouts_enforced: false,
        authenticated_dispatch_performed_once: false,
        redirect_retry_proxy_and_fallback_absence_enforced: false,
        response_received: false,
        connected_peer_checked_before_status_and_body: false,
        tls_server_identity_verified: false,
        http_status_200_checked: false,
        response_content_type_checked: false,
        response_content_encoding_checked: false,
        response_body_length_and_exact_schema_checked: false,
        closed_only_boolean_observed: false,
        closed_only_false_readiness_checked: false,
        closed_only_true_hard_block_checked: false,
        ambiguous_outcome_durable_burn_performed: false,
        live_credential_tuple_accepted_by_provider: false,
        credential_tuple_current_and_unrevoked_attested: false,
        online_authorization_v2_reverse_pins_remote_policy: false,
        reviewed_destination_reverse_pins_remote_policy: false,
        reviewed_locator_reverse_pins_remote_policy: false,
        fresh_delivery_reverse_pins_remote_policy: false,
        reviewed_identity_reverse_pins_remote_policy: false,
        remote_policy_fingerprint_pinned_by_online_authorization_v2: false,
        remote_policy_fingerprint_pinned_by_v3: false,
        remote_policy_consumption_durably_recorded: false,
        authorization_consumption_checked: false,
        credential_mutation_authority_attested: false,
        authorization: OfflineAuthorizationState::DENIED,
    })
}

#[derive(Debug, Error)]
pub enum PmReviewedRemoteCredentialProofPolicyV1Error {
    #[error("controlled-trial reviewed remote credential-proof policy V1 is invalid: {0}")]
    Invalid(&'static str),
}

fn invalid(message: &'static str) -> PmReviewedRemoteCredentialProofPolicyV1Error {
    PmReviewedRemoteCredentialProofPolicyV1Error::Invalid(message)
}

fn parse_exact_canonical<T: DeserializeOwned + Serialize>(
    bytes: &[u8],
) -> Result<T, PmReviewedRemoteCredentialProofPolicyV1Error> {
    let value: T = serde_json::from_slice(bytes).map_err(|_| {
        invalid(
            "reviewed remote credential-proof JSON is malformed, duplicated, unknown, or trailing",
        )
    })?;
    let canonical = serde_json::to_vec(&value).map_err(|_| {
        invalid("reviewed remote credential-proof policy cannot be serialized canonically")
    })?;
    if canonical != bytes {
        return Err(invalid(
            "reviewed remote credential-proof bytes are not exact canonical compact JSON",
        ));
    }
    Ok(value)
}

fn validate_artifact_pins(
    schema_version: u32,
    expected_schema_version: u32,
    id: &str,
    canonical_sha256: &str,
    canonical_length: u64,
    fingerprint: &str,
    message: &'static str,
) -> Result<(), PmReviewedRemoteCredentialProofPolicyV1Error> {
    if schema_version != expected_schema_version || canonical_length == 0 {
        return Err(invalid(message));
    }
    validate_token(id, 128, message)?;
    validate_sha256(canonical_sha256)?;
    validate_sha256(fingerprint)
}

fn validate_v1_config_pins(
    pins: &V1ConfigPinsV2,
) -> Result<(), PmReviewedRemoteCredentialProofPolicyV1Error> {
    validate_sha256(&pins.canonical_config_sha256)?;
    validate_sha256(&pins.canonical_config_fingerprint)?;
    validate_sha256(&pins.trial_plan_fingerprint)?;
    if pins.canonical_config_length == 0 {
        return Err(invalid(
            "reviewed remote credential-proof config length must be nonzero",
        ));
    }
    Ok(())
}

fn validate_online_policy_pins(
    pins: &OnlinePolicyPinsV2,
) -> Result<(), PmReviewedRemoteCredentialProofPolicyV1Error> {
    validate_sha256(&pins.canonical_sha256)?;
    validate_sha256(&pins.fingerprint)?;
    if pins.canonical_length == 0 {
        return Err(invalid(
            "reviewed remote credential-proof online-policy length must be nonzero",
        ));
    }
    Ok(())
}

fn validate_online_authorization_pins(
    pins: &ReviewedOnlineAuthorizationPinsV1,
) -> Result<(), PmReviewedRemoteCredentialProofPolicyV1Error> {
    validate_token(
        &pins.authorization_id,
        128,
        "reviewed remote credential-proof authorization ID is invalid",
    )?;
    validate_sha256(&pins.canonical_sha256)?;
    validate_sha256(&pins.fingerprint)?;
    if pins.canonical_length == 0 {
        return Err(invalid(
            "reviewed remote credential-proof authorization length must be nonzero",
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

fn destination_pins_match(
    pins: &ReviewedRemoteCredentialProofDestinationPinsV1,
    schema_version: u32,
    profile_id: &str,
    canonical_sha256: &str,
    canonical_length: u64,
    fingerprint: &str,
) -> bool {
    pins.schema_version == schema_version
        && pins.profile_id == profile_id
        && pins.canonical_sha256 == canonical_sha256
        && pins.canonical_length == canonical_length
        && pins.fingerprint == fingerprint
}

fn locator_pins_match(
    pins: &ReviewedRemoteCredentialProofLocatorPinsV1,
    schema_version: u32,
    locator_id: &str,
    canonical_sha256: &str,
    canonical_length: u64,
    fingerprint: &str,
) -> bool {
    pins.schema_version == schema_version
        && pins.locator_id == locator_id
        && pins.canonical_sha256 == canonical_sha256
        && pins.canonical_length == canonical_length
        && pins.fingerprint == fingerprint
}

fn delivery_pins_match(
    pins: &ReviewedRemoteCredentialProofDeliveryPinsV1,
    schema_version: u32,
    binding_id: &str,
    canonical_sha256: &str,
    canonical_length: u64,
    fingerprint: &str,
) -> bool {
    pins.schema_version == schema_version
        && pins.binding_id == binding_id
        && pins.canonical_sha256 == canonical_sha256
        && pins.canonical_length == canonical_length
        && pins.fingerprint == fingerprint
}

fn identity_pins_match(
    pins: &ReviewedRemoteCredentialProofAccountIdentityPinsV1,
    schema_version: u32,
    identity_id: &str,
    canonical_sha256: &str,
    canonical_length: u64,
    fingerprint: &str,
) -> bool {
    pins.schema_version == schema_version
        && pins.identity_id == identity_id
        && pins.canonical_sha256 == canonical_sha256
        && pins.canonical_length == canonical_length
        && pins.fingerprint == fingerprint
}

fn validate_sha256(value: &str) -> Result<(), PmReviewedRemoteCredentialProofPolicyV1Error> {
    if value.len() != 64
        || value
            .bytes()
            .any(|byte| !byte.is_ascii_digit() && !(b'a'..=b'f').contains(&byte))
    {
        return Err(invalid(
            "reviewed remote credential-proof SHA-256 value is invalid",
        ));
    }
    Ok(())
}

fn validate_token(
    value: &str,
    maximum: usize,
    message: &'static str,
) -> Result<(), PmReviewedRemoteCredentialProofPolicyV1Error> {
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
) -> Result<(), PmReviewedRemoteCredentialProofPolicyV1Error> {
    if value.is_empty()
        || value.len() > 512
        || value.bytes().any(|byte| !(0x21..=0x7e).contains(&byte))
    {
        return Err(invalid(message));
    }
    Ok(())
}

fn parse_utc(value: &str) -> Result<DateTime<Utc>, PmReviewedRemoteCredentialProofPolicyV1Error> {
    let parsed = DateTime::parse_from_rfc3339(value)
        .map_err(|_| invalid("reviewed remote credential-proof timestamp is invalid"))?
        .with_timezone(&Utc);
    if parsed.to_rfc3339_opts(chrono::SecondsFormat::Secs, true) != value {
        return Err(invalid(
            "reviewed remote credential-proof timestamp is not canonical UTC seconds",
        ));
    }
    Ok(parsed)
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
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
