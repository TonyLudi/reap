use std::fmt;

use base64::{Engine as _, engine::general_purpose::URL_SAFE};
use hmac::{Hmac, Mac};
use reap_pm_core::{PmConditionId, U256};
use serde::Serialize;
use sha2::Sha256;
use zeroize::Zeroizing;

use crate::secret::SecretText;
use crate::{
    FixedOrderId, L2Credentials, OwnedCancelSemanticRequestCommitment,
    PlaceSemanticRequestCommitment, PmAuthError, RuntimeExactBodyCommitment,
    SerializedOwnedCancelRequest, SerializedPlaceRequest,
};

type HmacSha256 = Hmac<Sha256>;
const MIN_L2_TIMESTAMP: u64 = 1_000_000_000;
const MAX_L2_TIMESTAMP: u64 = 9_999_999_999;
const MAX_USER_SUBSCRIPTION_BYTES: usize = 1_024;

/// A canonical ten-digit Unix-seconds value used once in an L2 preimage and
/// the matching `POLY_TIMESTAMP` header.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct L2Timestamp(u64);

impl L2Timestamp {
    pub fn from_unix_seconds(value: u64) -> Result<Self, PmAuthError> {
        if !(MIN_L2_TIMESTAMP..=MAX_L2_TIMESTAMP).contains(&value) {
            return Err(PmAuthError::InvalidL2Timestamp);
        }
        Ok(Self(value))
    }

    #[must_use]
    pub const fn unix_seconds(self) -> u64 {
        self.0
    }
}

impl fmt::Display for L2Timestamp {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.0)
    }
}

/// A consume-once set of the five exact Polymarket L2 headers for a fixed
/// authenticated read operation.
pub struct AuthenticatedL2Headers(L2Headers);

impl AuthenticatedL2Headers {
    pub fn apply_to<S: L2HeaderSink>(self, sink: &mut S) -> Result<(), S::Error> {
        self.0.apply_to(sink)
    }
}

impl fmt::Debug for AuthenticatedL2Headers {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("AuthenticatedL2Headers([REDACTED])")
    }
}

/// A sink for one fixed read operation's five application auth headers.
pub trait L2HeaderSink {
    type Error;

    fn set_polymarket_l2_headers(
        &mut self,
        poly_address: &str,
        poly_signature: &str,
        poly_timestamp: &str,
        poly_api_key: &str,
        poly_passphrase: &str,
    ) -> Result<(), Self::Error>;
}

/// One L2-authenticated, once-serialized fixed GTC/post-only place request.
pub struct AuthenticatedPlaceRequest {
    headers: L2Headers,
    body: Zeroizing<Vec<u8>>,
    expected_order_id: crate::ExpectedOrderId,
    expected_making_amount: U256,
    expected_taking_amount: U256,
    runtime_exact_body_commitment: RuntimeExactBodyCommitment,
    semantic_request_commitment: PlaceSemanticRequestCommitment,
}

impl AuthenticatedPlaceRequest {
    #[must_use]
    pub const fn expected_order_id(&self) -> crate::ExpectedOrderId {
        self.expected_order_id
    }

    #[must_use]
    pub const fn expected_making_amount(&self) -> U256 {
        self.expected_making_amount
    }

    #[must_use]
    pub const fn expected_taking_amount(&self) -> U256 {
        self.expected_taking_amount
    }

    #[must_use]
    pub const fn runtime_exact_body_commitment(&self) -> RuntimeExactBodyCommitment {
        self.runtime_exact_body_commitment
    }

    /// Secret-free fixed-place identity suitable for an upper durable request
    /// commitment. The exact body correlation remains runtime-only.
    #[must_use]
    pub const fn semantic_request_commitment(&self) -> PlaceSemanticRequestCommitment {
        self.semantic_request_commitment
    }

    /// Consume the authority and expose the exact authenticated body only to
    /// the fixed-place transport operation.
    pub fn dispatch<S: FixedPlaceRequestSink>(self, sink: &mut S) -> Result<S::Output, S::Error> {
        let address = self.headers.address.to_string();
        let timestamp = self.headers.timestamp.to_string();
        sink.send_gtc_post_only(
            &address,
            self.headers.signature.as_str(),
            &timestamp,
            self.headers.api_key.as_str(),
            self.headers.passphrase.as_str(),
            self.expected_making_amount,
            self.expected_taking_amount,
            self.body.as_slice(),
        )
    }
}

impl fmt::Debug for AuthenticatedPlaceRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("AuthenticatedPlaceRequest([REDACTED])")
    }
}

pub trait FixedPlaceRequestSink {
    type Output;
    type Error;

    #[allow(
        clippy::too_many_arguments,
        reason = "the fixed sink receives five exact auth headers, two signed integer amounts, and the once-serialized body without a generic request carrier"
    )]
    fn send_gtc_post_only(
        &mut self,
        poly_address: &str,
        poly_signature: &str,
        poly_timestamp: &str,
        poly_api_key: &str,
        poly_passphrase: &str,
        expected_making_amount: U256,
        expected_taking_amount: U256,
        exact_body: &[u8],
    ) -> Result<Self::Output, Self::Error>;
}

/// One L2-authenticated, once-serialized exact-owned cancel request.
pub struct AuthenticatedOwnedCancelRequest {
    headers: L2Headers,
    body: Zeroizing<Vec<u8>>,
    order_id: FixedOrderId,
    runtime_exact_body_commitment: RuntimeExactBodyCommitment,
    semantic_request_commitment: OwnedCancelSemanticRequestCommitment,
}

impl AuthenticatedOwnedCancelRequest {
    #[must_use]
    pub const fn order_id(&self) -> FixedOrderId {
        self.order_id
    }

    #[must_use]
    pub const fn runtime_exact_body_commitment(&self) -> RuntimeExactBodyCommitment {
        self.runtime_exact_body_commitment
    }

    /// Secret-free fixed-cancel identity suitable for an upper durable
    /// request commitment.
    #[must_use]
    pub const fn semantic_request_commitment(&self) -> OwnedCancelSemanticRequestCommitment {
        self.semantic_request_commitment
    }

    /// Consume the authority and expose the exact authenticated body only to
    /// the exact-owned cancel transport operation.
    pub fn dispatch<S: FixedOwnedCancelRequestSink>(
        self,
        sink: &mut S,
    ) -> Result<S::Output, S::Error> {
        let address = self.headers.address.to_string();
        let timestamp = self.headers.timestamp.to_string();
        sink.send_exact_owned_cancel(
            &address,
            self.headers.signature.as_str(),
            &timestamp,
            self.headers.api_key.as_str(),
            self.headers.passphrase.as_str(),
            self.body.as_slice(),
        )
    }
}

impl fmt::Debug for AuthenticatedOwnedCancelRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("AuthenticatedOwnedCancelRequest([REDACTED])")
    }
}

pub trait FixedOwnedCancelRequestSink {
    type Output;
    type Error;

    fn send_exact_owned_cancel(
        &mut self,
        poly_address: &str,
        poly_signature: &str,
        poly_timestamp: &str,
        poly_api_key: &str,
        poly_passphrase: &str,
        exact_body: &[u8],
    ) -> Result<Self::Output, Self::Error>;
}

/// One exact, market-bound authenticated user-WebSocket subscription frame.
pub struct AuthenticatedUserSubscription(Zeroizing<Vec<u8>>);

impl AuthenticatedUserSubscription {
    pub fn dispatch<S: AuthenticatedUserSubscriptionSink>(
        self,
        sink: &mut S,
    ) -> Result<S::Output, S::Error> {
        sink.send_user_subscription(self.0.as_slice())
    }
}

impl fmt::Debug for AuthenticatedUserSubscription {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("AuthenticatedUserSubscription([REDACTED])")
    }
}

pub trait AuthenticatedUserSubscriptionSink {
    type Output;
    type Error;

    fn send_user_subscription(&mut self, exact_frame: &[u8]) -> Result<Self::Output, Self::Error>;
}

impl L2Credentials {
    /// Headers for `GET /data/orders`; query bytes are intentionally absent
    /// from the signed route.
    pub fn authenticate_open_orders(
        &self,
        timestamp: L2Timestamp,
    ) -> Result<AuthenticatedL2Headers, PmAuthError> {
        self.read_headers(timestamp, b"/data/orders")
    }

    /// Headers for `GET /data/trades`; query bytes are intentionally absent
    /// from the signed route.
    pub fn authenticate_trades(
        &self,
        timestamp: L2Timestamp,
    ) -> Result<AuthenticatedL2Headers, PmAuthError> {
        self.read_headers(timestamp, b"/data/trades")
    }

    /// Headers for either fixed `GET /balance-allowance` account projection;
    /// its allowlisted query is intentionally absent from the signed route.
    pub fn authenticate_balance_allowance(
        &self,
        timestamp: L2Timestamp,
    ) -> Result<AuthenticatedL2Headers, PmAuthError> {
        self.read_headers(timestamp, b"/balance-allowance")
    }

    /// Headers for one exact `GET /data/order/{orderID}` operation.
    pub fn authenticate_order_detail(
        &self,
        timestamp: L2Timestamp,
        order_id: FixedOrderId,
    ) -> Result<AuthenticatedL2Headers, PmAuthError> {
        let route = Zeroizing::new(format!("/data/order/{order_id}"));
        self.read_headers(timestamp, route.as_bytes())
    }

    pub fn authenticate_place(
        &self,
        timestamp: L2Timestamp,
        request: SerializedPlaceRequest,
    ) -> Result<AuthenticatedPlaceRequest, PmAuthError> {
        let headers = self.headers(timestamp, b"POST", b"/order", Some(&request.body))?;
        let expected_order_id = request.expected_order_id();
        let expected_making_amount = request.expected_making_amount();
        let expected_taking_amount = request.expected_taking_amount();
        let runtime_exact_body_commitment = request.runtime_exact_body_commitment();
        let semantic_request_commitment = request.semantic_request_commitment();
        Ok(AuthenticatedPlaceRequest {
            headers,
            body: request.body,
            expected_order_id,
            expected_making_amount,
            expected_taking_amount,
            runtime_exact_body_commitment,
            semantic_request_commitment,
        })
    }

    pub fn authenticate_owned_cancel(
        &self,
        timestamp: L2Timestamp,
        request: SerializedOwnedCancelRequest,
    ) -> Result<AuthenticatedOwnedCancelRequest, PmAuthError> {
        let headers = self.headers(timestamp, b"DELETE", b"/order", Some(&request.body))?;
        let order_id = request.order_id();
        let runtime_exact_body_commitment = request.runtime_exact_body_commitment();
        let semantic_request_commitment = request.semantic_request_commitment();
        Ok(AuthenticatedOwnedCancelRequest {
            headers,
            body: request.body,
            order_id,
            runtime_exact_body_commitment,
            semantic_request_commitment,
        })
    }

    /// Serialize the exact one-market initial authenticated user subscription.
    /// The resulting credential frame is consume-once and has no byte getter.
    pub fn user_subscription(
        &self,
        market: PmConditionId,
    ) -> Result<AuthenticatedUserSubscription, PmAuthError> {
        let encoded_secret = Zeroizing::new(URL_SAFE.encode(self.hmac_key()));
        let frame = UserSubscription {
            auth: UserAuth {
                api_key: self.api_key(),
                secret: encoded_secret.as_str(),
                passphrase: self.passphrase(),
            },
            markets: [market],
            kind: "user",
        };
        let bytes = Zeroizing::new(
            serde_json::to_vec(&frame).map_err(|_| PmAuthError::SerializationFailure)?,
        );
        if bytes.len() > MAX_USER_SUBSCRIPTION_BYTES {
            return Err(PmAuthError::RequestBodyTooLong);
        }
        Ok(AuthenticatedUserSubscription(bytes))
    }

    fn read_headers(
        &self,
        timestamp: L2Timestamp,
        route: &[u8],
    ) -> Result<AuthenticatedL2Headers, PmAuthError> {
        self.headers(timestamp, b"GET", route, None)
            .map(AuthenticatedL2Headers)
    }

    fn headers(
        &self,
        timestamp: L2Timestamp,
        method: &[u8],
        route: &[u8],
        body: Option<&[u8]>,
    ) -> Result<L2Headers, PmAuthError> {
        let timestamp_text = timestamp.unix_seconds().to_string();
        let mut mac = HmacSha256::new_from_slice(self.hmac_key())
            .map_err(|_| PmAuthError::CryptographicFailure)?;
        mac.update(timestamp_text.as_bytes());
        mac.update(method);
        mac.update(route);
        if let Some(body) = body {
            mac.update(body);
        }
        let signature = Zeroizing::new(URL_SAFE.encode(mac.finalize().into_bytes()));
        if signature.len() != 44 || !signature.ends_with('=') {
            return Err(PmAuthError::CryptographicFailure);
        }

        Ok(L2Headers {
            address: self.address(),
            signature: SecretText::copy_from(signature.as_bytes())
                .ok_or(PmAuthError::CryptographicFailure)?,
            timestamp,
            api_key: SecretText::copy_from(self.api_key().as_bytes())
                .ok_or(PmAuthError::CryptographicFailure)?,
            passphrase: SecretText::copy_from(self.passphrase().as_bytes())
                .ok_or(PmAuthError::CryptographicFailure)?,
        })
    }
}

struct L2Headers {
    address: crate::EoaAddress,
    signature: SecretText<44>,
    timestamp: L2Timestamp,
    api_key: SecretText<36>,
    passphrase: SecretText<128>,
}

impl L2Headers {
    fn apply_to<S: L2HeaderSink>(self, sink: &mut S) -> Result<(), S::Error> {
        let address = self.address.to_string();
        let timestamp = self.timestamp.to_string();
        sink.set_polymarket_l2_headers(
            &address,
            self.signature.as_str(),
            &timestamp,
            self.api_key.as_str(),
            self.passphrase.as_str(),
        )
    }
}

#[derive(Serialize)]
struct UserSubscription<'a> {
    auth: UserAuth<'a>,
    markets: [PmConditionId; 1],
    #[serde(rename = "type")]
    kind: &'static str,
}

#[derive(Serialize)]
struct UserAuth<'a> {
    #[serde(rename = "apiKey")]
    api_key: &'a str,
    secret: &'a str,
    passphrase: &'a str,
}

#[cfg(test)]
mod tests {
    use std::convert::Infallible;

    use reap_pm_core::PmConditionId;

    use crate::{
        AuthenticatedUserSubscriptionSink, EoaPrivateKeyInput, FixedEoaSigner, L2CredentialInput,
        L2Credentials, L2Timestamp,
    };

    const ADDRESS: &str = "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266";

    fn credentials(secret: &str) -> L2Credentials {
        credentials_with_passphrase(secret, "synthetic-passphrase")
    }

    fn credentials_with_passphrase(secret: &str, passphrase: &str) -> L2Credentials {
        L2Credentials::bind(
            ADDRESS,
            L2CredentialInput::new(
                "00000000-0000-4000-8000-000000000001".into(),
                secret.into(),
                passphrase.into(),
            ),
        )
        .unwrap()
    }

    #[test]
    fn errors_and_secret_debug_are_redacted() {
        let canary = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";
        let credentials = credentials(canary);
        let signer = FixedEoaSigner::bind(
            EoaPrivateKeyInput::new(
                "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80".into(),
            ),
            ADDRESS,
        )
        .unwrap();
        for rendered in [format!("{credentials:?}"), format!("{signer:?}")] {
            assert!(rendered.contains("REDACTED"));
            assert!(!rendered.contains(canary));
            assert!(!rendered.contains("ac0974"));
        }
    }

    #[test]
    fn timestamp_is_exactly_ten_digits() {
        assert!(L2Timestamp::from_unix_seconds(999_999_999).is_err());
        assert!(L2Timestamp::from_unix_seconds(1_000_000_000).is_ok());
        assert!(L2Timestamp::from_unix_seconds(9_999_999_999).is_ok());
        assert!(L2Timestamp::from_unix_seconds(10_000_000_000).is_err());
    }

    #[test]
    fn changing_one_serialized_body_byte_changes_l2_authentication() {
        let credentials = credentials("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=");
        let timestamp = L2Timestamp::from_unix_seconds(1_780_449_126).unwrap();
        let original = credentials
            .headers(timestamp, b"POST", b"/order", Some(&[0_u8]))
            .unwrap();
        let changed = credentials
            .headers(timestamp, b"POST", b"/order", Some(&[1_u8]))
            .unwrap();

        assert_ne!(original.signature.as_str(), changed.signature.as_str());
    }

    #[test]
    fn opaque_visible_ascii_passphrase_is_json_escaped_without_rewriting() {
        struct Capture(Vec<u8>);
        impl AuthenticatedUserSubscriptionSink for Capture {
            type Output = ();
            type Error = Infallible;

            fn send_user_subscription(
                &mut self,
                exact_frame: &[u8],
            ) -> Result<Self::Output, Self::Error> {
                self.0.extend_from_slice(exact_frame);
                Ok(())
            }
        }

        let passphrase = "opaque+/=\\\"passphrase";
        let credentials =
            credentials_with_passphrase("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=", passphrase);
        let condition = PmConditionId::parse(
            "0x1111111111111111111111111111111111111111111111111111111111111111",
        )
        .unwrap();
        let mut capture = Capture(Vec::new());
        credentials
            .user_subscription(condition)
            .unwrap()
            .dispatch(&mut capture)
            .unwrap();
        let decoded: serde_json::Value = serde_json::from_slice(&capture.0).unwrap();
        assert_eq!(decoded["auth"]["passphrase"], passphrase);
        let encoded = std::str::from_utf8(&capture.0).unwrap();
        assert!(encoded.contains(r#""passphrase":"opaque+/=\\\"passphrase""#));
    }
}
