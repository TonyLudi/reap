//! Bounded wire contract for the credential-wide CLOB order heartbeat.

use std::fmt;

use serde::Deserialize;
use thiserror::Error;
use zeroize::Zeroizing;

const MAX_HEARTBEAT_RESPONSE_BYTES: usize = 4 * 1_024;
const MAX_HEARTBEAT_ID_BYTES: usize = 256;

/// One opaque heartbeat identifier returned by Polymarket.
///
/// The identifier is admitted only from a bounded response and deliberately
/// redacted from `Debug`; it is not a credential, but logging it provides no
/// operational value.
pub struct PmOrderHeartbeatId(Zeroizing<String>);

impl PmOrderHeartbeatId {
    #[must_use]
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

impl PartialEq for PmOrderHeartbeatId {
    fn eq(&self, other: &Self) -> bool {
        self.0.as_str() == other.0.as_str()
    }
}

impl Eq for PmOrderHeartbeatId {}

impl fmt::Debug for PmOrderHeartbeatId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PmOrderHeartbeatId([REDACTED])")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum PmOrderHeartbeatWireError {
    #[error("Polymarket order-heartbeat response exceeds its byte bound")]
    PayloadTooLarge,
    #[error("Polymarket order-heartbeat response is malformed")]
    MalformedJson,
    #[error("Polymarket order-heartbeat response has an invalid heartbeat_id")]
    InvalidHeartbeatId,
}

#[derive(Deserialize)]
struct RawHeartbeatResponse {
    heartbeat_id: String,
}

/// Parse either a successful heartbeat response or the current-ID response
/// returned with HTTP 400 when a caller presents a stale identifier.
pub fn parse_order_heartbeat_response(
    raw: &[u8],
) -> Result<PmOrderHeartbeatId, PmOrderHeartbeatWireError> {
    if raw.len() > MAX_HEARTBEAT_RESPONSE_BYTES {
        return Err(PmOrderHeartbeatWireError::PayloadTooLarge);
    }
    let wire = serde_json::from_slice::<RawHeartbeatResponse>(raw)
        .map_err(|_| PmOrderHeartbeatWireError::MalformedJson)?;
    if wire.heartbeat_id.is_empty()
        || wire.heartbeat_id.len() > MAX_HEARTBEAT_ID_BYTES
        || !wire.heartbeat_id.is_ascii()
        || wire
            .heartbeat_id
            .bytes()
            .any(|byte| byte.is_ascii_control())
    {
        return Err(PmOrderHeartbeatWireError::InvalidHeartbeatId);
    }
    Ok(PmOrderHeartbeatId(Zeroizing::new(wire.heartbeat_id)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_bounded_opaque_identifier_and_redacts_debug() {
        let id = parse_order_heartbeat_response(br#"{"heartbeat_id":"hb-123"}"#).unwrap();
        assert_eq!(id.as_str(), "hb-123");
        assert_eq!(format!("{id:?}"), "PmOrderHeartbeatId([REDACTED])");
    }

    #[test]
    fn rejects_empty_control_and_oversized_identifiers() {
        assert!(parse_order_heartbeat_response(br#"{"heartbeat_id":""}"#).is_err());
        assert!(parse_order_heartbeat_response(b"{\"heartbeat_id\":\"a\\n\"}").is_err());
        let oversized = format!(r#"{{"heartbeat_id":"{}"}}"#, "a".repeat(257));
        assert!(parse_order_heartbeat_response(oversized.as_bytes()).is_err());
    }
}
