use std::fmt;

use reap_pm_core::U256;
use reap_polymarket_auth::{
    AuthenticatedOwnedCancelRequest, AuthenticatedPlaceRequest, ExpectedOrderId, FixedOrderId,
    FixedOwnedCancelRequestSink, FixedPlaceRequestSink, OwnedCancelSemanticRequestCommitment,
    PlaceSemanticRequestCommitment, RuntimeExactBodyCommitment,
};
use zeroize::Zeroizing;

use super::PmMutationEdgeError;

pub(super) const MAX_RETAINED_PLACE_BODY_BYTES: usize = 1_024;
pub(super) const MAX_RETAINED_CANCEL_BODY_BYTES: usize = 128;

const ADDRESS_BYTES: usize = 42;
const SIGNATURE_BYTES: usize = 44;
const TIMESTAMP_BYTES: usize = 10;
const API_KEY_BYTES: usize = 36;
const MAX_PASSPHRASE_BYTES: usize = 128;

pub(super) struct RetainedL2Headers {
    pub(super) address: Zeroizing<String>,
    pub(super) signature: Zeroizing<String>,
    pub(super) timestamp: Zeroizing<String>,
    pub(super) api_key: Zeroizing<String>,
    pub(super) passphrase: Zeroizing<String>,
}

impl RetainedL2Headers {
    fn copy_validated(
        address: &str,
        signature: &str,
        timestamp: &str,
        api_key: &str,
        passphrase: &str,
    ) -> Result<Self, PmMutationEdgeError> {
        if address.len() != ADDRESS_BYTES
            || !address.starts_with("0x")
            || address[2..].bytes().any(|byte| !byte.is_ascii_hexdigit())
            || signature.len() != SIGNATURE_BYTES
            || !signature.ends_with('=')
            || signature[..SIGNATURE_BYTES - 1]
                .bytes()
                .any(|byte| !(byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_')))
            || timestamp.len() != TIMESTAMP_BYTES
            || timestamp.bytes().any(|byte| !byte.is_ascii_digit())
            || !valid_api_key(api_key.as_bytes())
            || passphrase.is_empty()
            || passphrase.len() > MAX_PASSPHRASE_BYTES
            || passphrase
                .bytes()
                .any(|byte| !(0x21..=0x7e).contains(&byte))
        {
            return Err(PmMutationEdgeError::InvalidAuthenticatedRequest);
        }

        Ok(Self {
            address: Zeroizing::new(address.to_owned()),
            signature: Zeroizing::new(signature.to_owned()),
            timestamp: Zeroizing::new(timestamp.to_owned()),
            api_key: Zeroizing::new(api_key.to_owned()),
            passphrase: Zeroizing::new(passphrase.to_owned()),
        })
    }

    pub(super) fn remains_valid(&self) -> bool {
        Self::copy_validated(
            self.address.as_str(),
            self.signature.as_str(),
            self.timestamp.as_str(),
            self.api_key.as_str(),
            self.passphrase.as_str(),
        )
        .is_ok()
    }
}

impl fmt::Debug for RetainedL2Headers {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("RetainedL2Headers([REDACTED])")
    }
}

/// A move-only retained fixed-place request. It retains the authenticated
/// request byte-for-byte, its runtime-only exact-body correlation, and the
/// separate secret-free semantic identity.
///
/// This runtime carrier is intentionally not serializable. In particular,
/// `runtime_exact_body_commitment` is secret-derived and must never enter a
/// journal, capture, log, metric, or other durable artifact.
pub struct PmRetainedPlaceRequest {
    pub(super) headers: RetainedL2Headers,
    pub(super) body: Zeroizing<Vec<u8>>,
    expected_order_id: ExpectedOrderId,
    expected_making_amount: U256,
    expected_taking_amount: U256,
    runtime_exact_body_commitment: RuntimeExactBodyCommitment,
    semantic_request_commitment: PlaceSemanticRequestCommitment,
    l2_timestamp_seconds: u64,
}

impl PmRetainedPlaceRequest {
    pub fn retain(request: AuthenticatedPlaceRequest) -> Result<Self, PmMutationEdgeError> {
        let mut sink = PlaceRetentionSink {
            expected_order_id: request.expected_order_id(),
            expected_making_amount: request.expected_making_amount(),
            expected_taking_amount: request.expected_taking_amount(),
            runtime_exact_body_commitment: request.runtime_exact_body_commitment(),
            semantic_request_commitment: request.semantic_request_commitment(),
        };
        request.dispatch(&mut sink)
    }

    #[must_use]
    pub const fn expected_order_id(&self) -> ExpectedOrderId {
        self.expected_order_id
    }

    #[must_use]
    pub const fn runtime_exact_body_commitment(&self) -> RuntimeExactBodyCommitment {
        self.runtime_exact_body_commitment
    }

    /// Secret-free fixed-place identity. This remains distinct from the exact
    /// serialized-body correlation above.
    #[must_use]
    pub const fn semantic_request_commitment(&self) -> PlaceSemanticRequestCommitment {
        self.semantic_request_commitment
    }

    #[must_use]
    pub const fn expected_making_amount(&self) -> U256 {
        self.expected_making_amount
    }

    #[must_use]
    pub const fn expected_taking_amount(&self) -> U256 {
        self.expected_taking_amount
    }

    /// Exact non-secret L2 timestamp authenticated into this retained request.
    #[must_use]
    pub const fn l2_timestamp_seconds(&self) -> u64 {
        self.l2_timestamp_seconds
    }

    pub(super) fn remains_valid(&self) -> bool {
        self.headers.remains_valid()
            && !self.body.is_empty()
            && self.body.len() <= MAX_RETAINED_PLACE_BODY_BYTES
            && self.headers.timestamp.parse::<u64>().ok() == Some(self.l2_timestamp_seconds)
    }
}

impl fmt::Debug for PmRetainedPlaceRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PmRetainedPlaceRequest")
            .field("headers", &"[REDACTED]")
            .field("body", &"[REDACTED]")
            .field("expected_order_id", &self.expected_order_id)
            .field("expected_making_amount", &self.expected_making_amount)
            .field("expected_taking_amount", &self.expected_taking_amount)
            .field("runtime_exact_body_commitment", &"[REDACTED; NON_DURABLE]")
            .field(
                "semantic_request_commitment",
                &self.semantic_request_commitment,
            )
            .field("l2_timestamp_seconds", &self.l2_timestamp_seconds)
            .finish()
    }
}

/// A move-only retained exact-owned cancel request. It retains the
/// authenticated request byte-for-byte, its runtime-only exact-body
/// correlation, and the separate secret-free semantic identity.
///
/// This runtime carrier is intentionally not serializable. In particular,
/// `runtime_exact_body_commitment` must never enter a journal, capture, log,
/// metric, or other durable artifact.
pub struct PmRetainedOwnedCancelRequest {
    pub(super) headers: RetainedL2Headers,
    pub(super) body: Zeroizing<Vec<u8>>,
    order_id: FixedOrderId,
    runtime_exact_body_commitment: RuntimeExactBodyCommitment,
    semantic_request_commitment: OwnedCancelSemanticRequestCommitment,
    l2_timestamp_seconds: u64,
}

impl PmRetainedOwnedCancelRequest {
    pub fn retain(request: AuthenticatedOwnedCancelRequest) -> Result<Self, PmMutationEdgeError> {
        let mut sink = CancelRetentionSink {
            order_id: request.order_id(),
            runtime_exact_body_commitment: request.runtime_exact_body_commitment(),
            semantic_request_commitment: request.semantic_request_commitment(),
        };
        request.dispatch(&mut sink)
    }

    #[must_use]
    pub const fn order_id(&self) -> FixedOrderId {
        self.order_id
    }

    #[must_use]
    pub const fn runtime_exact_body_commitment(&self) -> RuntimeExactBodyCommitment {
        self.runtime_exact_body_commitment
    }

    /// Secret-free exact-owned-cancel identity. This remains distinct from the
    /// exact serialized-body correlation above.
    #[must_use]
    pub const fn semantic_request_commitment(&self) -> OwnedCancelSemanticRequestCommitment {
        self.semantic_request_commitment
    }

    /// Exact non-secret L2 timestamp authenticated into this retained request.
    #[must_use]
    pub const fn l2_timestamp_seconds(&self) -> u64 {
        self.l2_timestamp_seconds
    }

    pub(super) fn remains_valid(&self) -> bool {
        self.headers.remains_valid()
            && !self.body.is_empty()
            && self.body.len() <= MAX_RETAINED_CANCEL_BODY_BYTES
            && self.headers.timestamp.parse::<u64>().ok() == Some(self.l2_timestamp_seconds)
    }
}

impl fmt::Debug for PmRetainedOwnedCancelRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PmRetainedOwnedCancelRequest")
            .field("headers", &"[REDACTED]")
            .field("body", &"[REDACTED]")
            .field("order_id", &self.order_id)
            .field("runtime_exact_body_commitment", &"[REDACTED; NON_DURABLE]")
            .field(
                "semantic_request_commitment",
                &self.semantic_request_commitment,
            )
            .field("l2_timestamp_seconds", &self.l2_timestamp_seconds)
            .finish()
    }
}

struct PlaceRetentionSink {
    expected_order_id: ExpectedOrderId,
    expected_making_amount: U256,
    expected_taking_amount: U256,
    runtime_exact_body_commitment: RuntimeExactBodyCommitment,
    semantic_request_commitment: PlaceSemanticRequestCommitment,
}

impl FixedPlaceRequestSink for PlaceRetentionSink {
    type Output = PmRetainedPlaceRequest;
    type Error = PmMutationEdgeError;

    #[allow(
        clippy::too_many_arguments,
        reason = "the retention sink validates the complete fixed-purpose auth and amount boundary"
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
    ) -> Result<Self::Output, Self::Error> {
        if exact_body.is_empty()
            || exact_body.len() > MAX_RETAINED_PLACE_BODY_BYTES
            || expected_making_amount != self.expected_making_amount
            || expected_taking_amount != self.expected_taking_amount
        {
            return Err(PmMutationEdgeError::InvalidAuthenticatedRequest);
        }
        let l2_timestamp_seconds = poly_timestamp
            .parse()
            .map_err(|_| PmMutationEdgeError::InvalidAuthenticatedRequest)?;
        Ok(PmRetainedPlaceRequest {
            headers: RetainedL2Headers::copy_validated(
                poly_address,
                poly_signature,
                poly_timestamp,
                poly_api_key,
                poly_passphrase,
            )?,
            body: Zeroizing::new(exact_body.to_vec()),
            expected_order_id: self.expected_order_id,
            expected_making_amount,
            expected_taking_amount,
            runtime_exact_body_commitment: self.runtime_exact_body_commitment,
            semantic_request_commitment: self.semantic_request_commitment,
            l2_timestamp_seconds,
        })
    }
}

struct CancelRetentionSink {
    order_id: FixedOrderId,
    runtime_exact_body_commitment: RuntimeExactBodyCommitment,
    semantic_request_commitment: OwnedCancelSemanticRequestCommitment,
}

impl FixedOwnedCancelRequestSink for CancelRetentionSink {
    type Output = PmRetainedOwnedCancelRequest;
    type Error = PmMutationEdgeError;

    fn send_exact_owned_cancel(
        &mut self,
        poly_address: &str,
        poly_signature: &str,
        poly_timestamp: &str,
        poly_api_key: &str,
        poly_passphrase: &str,
        exact_body: &[u8],
    ) -> Result<Self::Output, Self::Error> {
        if exact_body.is_empty() || exact_body.len() > MAX_RETAINED_CANCEL_BODY_BYTES {
            return Err(PmMutationEdgeError::InvalidAuthenticatedRequest);
        }
        let l2_timestamp_seconds = poly_timestamp
            .parse()
            .map_err(|_| PmMutationEdgeError::InvalidAuthenticatedRequest)?;
        Ok(PmRetainedOwnedCancelRequest {
            headers: RetainedL2Headers::copy_validated(
                poly_address,
                poly_signature,
                poly_timestamp,
                poly_api_key,
                poly_passphrase,
            )?,
            body: Zeroizing::new(exact_body.to_vec()),
            order_id: self.order_id,
            runtime_exact_body_commitment: self.runtime_exact_body_commitment,
            semantic_request_commitment: self.semantic_request_commitment,
            l2_timestamp_seconds,
        })
    }
}

fn valid_api_key(value: &[u8]) -> bool {
    value.len() == API_KEY_BYTES
        && value.iter().enumerate().all(|(index, byte)| {
            if matches!(index, 8 | 13 | 18 | 23) {
                *byte == b'-'
            } else {
                byte.is_ascii_digit() || (b'a'..=b'f').contains(byte)
            }
        })
}
