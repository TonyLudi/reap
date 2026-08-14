//! Bounded parsing and one-way local equality join for an L1 credential-
//! derivation response.
//!
//! [`L1CredentialDerivationResponseInput`] contains caller-supplied bytes. Its
//! public construction is not response-source evidence: any caller can supply
//! retained, replayed, fabricated, or misrouted bytes. A successful join only
//! establishes that the three canonical values in this local byte document
//! equal the same three values in the consumed, staged [`L2Credentials`]. It
//! does not prove a server call, the TLS peer or selected local egress,
//! response currentness or uniqueness, the response MIME type, later L2 acceptance,
//! provider delivery, proxy mapping or control, or mutation authorization.
//!
//! Production integration must seal construction behind a future private sink
//! for the fixed `https://clob.polymarket.com` host. That sink must establish
//! the selected TLS peer and local egress and validate the response MIME type
//! before constructing this input. This module contains no transport or remote
//! authority.
//!
//! Reap-owned input bytes and parsed field strings use [`Zeroizing`]. The JSON
//! parser can use implementation-owned scratch or allocator copies that this
//! module cannot zeroize, so this is not a claim that every parser-internal or
//! historical copy is securely erased.

use std::fmt;

use serde::de::Visitor;
use serde::{Deserialize, Deserializer};
use zeroize::Zeroizing;

use crate::{
    FixedClosedOnlyRequestSink, L2CredentialInput, L2Credentials, L2Timestamp, PmAuthError,
    l2::AuthenticatedClosedOnlyRequest,
};

/// Maximum accepted size of one caller-supplied credential-derivation response
/// document.
pub const MAX_L1_CREDENTIAL_DERIVATION_RESPONSE_BYTES: usize = 1024;

/// Move-only, bounded, zeroizing caller input for one credential-derivation
/// response document.
///
/// This type does not attest where the bytes came from. Its only useful public
/// path is consumption together with an existing [`L2Credentials`] holder.
pub struct L1CredentialDerivationResponseInput {
    bytes: Zeroizing<Vec<u8>>,
}

impl L1CredentialDerivationResponseInput {
    /// Wrap caller-supplied bytes after enforcing the fixed local size bound.
    ///
    /// The bytes are placed under Reap-owned [`Zeroizing`] custody before the
    /// bound is checked, including on the error path.
    pub fn new(bytes: Vec<u8>) -> Result<Self, PmAuthError> {
        let bytes = Zeroizing::new(bytes);
        if bytes.len() > MAX_L1_CREDENTIAL_DERIVATION_RESPONSE_BYTES {
            return Err(PmAuthError::L1CredentialDerivationResponseTooLong);
        }
        Ok(Self { bytes })
    }
}

impl fmt::Debug for L1CredentialDerivationResponseInput {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(
            "L1CredentialDerivationResponseInput([REDACTED; CALLER_SUPPLIED_BYTES; NO_SOURCE_PROOF])",
        )
    }
}

/// Move-only result of joining one exact response document to one staged L2
/// credential holder.
///
/// The holder is opaque and deliberately has no recovery, splitting,
/// serialization, credential getter, hash, or length-projection API. It keeps
/// the original consumed [`L2Credentials`] rather than replacing it with
/// values reconstructed from the response. Its existence represents local
/// equality only and carries none of the remote or mutation authority excluded
/// in this module's documentation.
#[must_use]
pub struct L1CredentialDerivationMatchedL2Credentials {
    original_l2_credentials: L2Credentials,
}

impl fmt::Debug for L1CredentialDerivationMatchedL2Credentials {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(
            "L1CredentialDerivationMatchedL2Credentials([REDACTED; LOCAL_EQUALITY_ONLY; NO_REMOTE_OR_MUTATION_AUTHORITY])",
        )
    }
}

/// Move-only exact closed-only request derived from the original L2 holder in
/// one [`L1CredentialDerivationMatchedL2Credentials`] local-equality join.
///
/// Before dispatch, the exact `GET /auth/ban-status/closed-only` HMAC headers
/// and the original L2 holder remain inseparable in this type state. A trusted
/// sink can copy the supplied headers during dispatch. This is HMAC
/// construction for one local holder only. The caller-supplied timestamp has
/// no source or freshness proof, and the request grants no transport,
/// remote-acceptance, currentness, provider, proxy-control, or mutation
/// authority.
#[must_use]
pub struct L1CredentialDerivationMatchedClosedOnlyRequest {
    original_l2_credentials: L2Credentials,
    request: AuthenticatedClosedOnlyRequest,
}

impl fmt::Debug for L1CredentialDerivationMatchedClosedOnlyRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(
            "L1CredentialDerivationMatchedClosedOnlyRequest([REDACTED; LOCAL_EQUALITY_AND_HMAC_CONSTRUCTION_ONLY; NO_REMOTE_OR_MUTATION_AUTHORITY])",
        )
    }
}

/// Move-only custody of the original matched L2 holder and one trusted sink's
/// synchronous output.
///
/// Construction establishes only that the original holder remained owned
/// through the sink call that produced `Output`. Neither value has a public
/// projection. The sink can retain or replay headers, reroute the operation,
/// or fabricate its output, so this holder proves no request, response, remote
/// acceptance, credential currentness, provider origin, proxy control, or
/// mutation authorization.
#[must_use]
pub struct L1CredentialDerivationMatchedClosedOnlyDispatch<Output> {
    _original_l2_credentials: L2Credentials,
    _output: Output,
}

impl<Output> fmt::Debug for L1CredentialDerivationMatchedClosedOnlyDispatch<Output> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(
            "L1CredentialDerivationMatchedClosedOnlyDispatch([REDACTED; TRUSTED_SINK_OUTPUT_RETAINED; NO_REMOTE_OR_MUTATION_AUTHORITY])",
        )
    }
}

impl L1CredentialDerivationMatchedL2Credentials {
    /// Consume this local-equality holder into one exact closed-only HMAC
    /// request while retaining the original loaded L2 holder.
    ///
    /// `timestamp` is caller supplied and carries no source or freshness
    /// evidence. The result cannot be decomposed and can only be consumed at a
    /// [`FixedClosedOnlyRequestSink`].
    pub fn consume_into_authenticated_closed_only(
        self,
        timestamp: L2Timestamp,
    ) -> Result<L1CredentialDerivationMatchedClosedOnlyRequest, PmAuthError> {
        let Self {
            original_l2_credentials,
        } = self;
        let request = original_l2_credentials.authenticate_exact_closed_only_request(timestamp)?;
        Ok(L1CredentialDerivationMatchedClosedOnlyRequest {
            original_l2_credentials,
            request,
        })
    }
}

impl L1CredentialDerivationMatchedClosedOnlyRequest {
    /// Consume the pre-dispatch same-holder request at one trusted exact-route
    /// sink and, on success, retain both the original L2 holder and synchronous
    /// sink output.
    ///
    /// The sink remains trusted and may retain, replay, or reroute the supplied
    /// headers or fabricate `Output` or `Error`. On `Err`, the exact header
    /// carrier and original L2 holder are dropped before only the sink error is
    /// returned; there is no retry or recovery authority. This cannot revoke
    /// copies already retained by the sink. Either result remains a local
    /// ownership transition and does not attest that any network operation or
    /// remote response occurred.
    pub fn dispatch<S: FixedClosedOnlyRequestSink>(
        self,
        sink: &mut S,
    ) -> Result<L1CredentialDerivationMatchedClosedOnlyDispatch<S::Output>, S::Error> {
        let Self {
            original_l2_credentials,
            request,
        } = self;
        let output = match request.dispatch(sink) {
            Ok(output) => output,
            Err(error) => {
                // The request/header carrier was consumed and dropped inside
                // its dispatch. Burn the remaining same-holder custody before
                // returning only the trusted sink's error.
                drop(original_l2_credentials);
                return Err(error);
            }
        };
        Ok(L1CredentialDerivationMatchedClosedOnlyDispatch {
            _original_l2_credentials: original_l2_credentials,
            _output: output,
        })
    }
}

impl L2Credentials {
    /// Consume this staged holder and one caller-supplied response into a
    /// single opaque local-equality holder.
    ///
    /// The response must be one JSON object with exactly `apiKey`, `secret`,
    /// and `passphrase` string fields. Missing, duplicate, unknown, null, or
    /// wrongly typed fields and trailing non-whitespace input are rejected.
    /// Standard JSON surrounding whitespace remains part of the accepted JSON
    /// document grammar. Each value must also obey the same canonical grammar
    /// used when constructing [`L2Credentials`]. Every canonical component is
    /// compared before one generic mismatch result is selected.
    pub fn consume_with_l1_credential_derivation_response(
        self,
        response: L1CredentialDerivationResponseInput,
    ) -> Result<L1CredentialDerivationMatchedL2Credentials, PmAuthError> {
        let candidate_input = response.parse_exact_l2_credential_input()?;
        let candidate = Self::bind_to_address(self.address(), candidate_input)
            .map_err(|_| PmAuthError::InvalidL1CredentialDerivationResponse)?;

        if !self.matches_exact_l2_bundle(&candidate) {
            return Err(PmAuthError::L1CredentialDerivationCredentialMismatch);
        }

        Ok(L1CredentialDerivationMatchedL2Credentials {
            original_l2_credentials: self,
        })
    }
}

impl L1CredentialDerivationResponseInput {
    fn parse_exact_l2_credential_input(self) -> Result<L2CredentialInput, PmAuthError> {
        let parsed: ExactL1CredentialDerivationResponse =
            serde_json::from_slice(self.bytes.as_slice())
                .map_err(|_| PmAuthError::InvalidL1CredentialDerivationResponse)?;
        let ExactL1CredentialDerivationResponse {
            api_key: ZeroizingJsonString(api_key),
            secret: ZeroizingJsonString(secret),
            passphrase: ZeroizingJsonString(passphrase),
        } = parsed;
        Ok(L2CredentialInput::from_zeroizing(
            api_key, secret, passphrase,
        ))
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ExactL1CredentialDerivationResponse {
    #[serde(rename = "apiKey")]
    api_key: ZeroizingJsonString,
    secret: ZeroizingJsonString,
    passphrase: ZeroizingJsonString,
}

struct ZeroizingJsonString(Zeroizing<String>);

impl<'de> Deserialize<'de> for ZeroizingJsonString {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct ZeroizingJsonStringVisitor;

        impl<'de> Visitor<'de> for ZeroizingJsonStringVisitor {
            type Value = ZeroizingJsonString;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a credential string")
            }

            fn visit_string<E>(self, value: String) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                Ok(ZeroizingJsonString(Zeroizing::new(value)))
            }

            fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                let mut owned = Zeroizing::new(String::with_capacity(value.len()));
                owned.push_str(value);
                Ok(ZeroizingJsonString(owned))
            }

            fn visit_borrowed_str<E>(self, value: &'de str) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                self.visit_str(value)
            }
        }

        deserializer.deserialize_string(ZeroizingJsonStringVisitor)
    }
}
