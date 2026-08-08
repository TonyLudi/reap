use std::time::Duration;

#[cfg(any(test, feature = "loopback-evidence"))]
use std::net::IpAddr;

use reap_pm_core::{ConnectionEpoch, PmConditionId};
use reap_polymarket_wire::MAX_PM_LIVE_BODY_BYTES;
use url::Url;

use crate::PmLiveAdapterError;

/// Current official authenticated user-channel endpoint.
///
/// Protocol authority revalidated 2026-08-08:
/// <https://docs.polymarket.com/trading/realtime-order-updates> documents the
/// fixed-market initial `{auth,markets,type}` form and ten-second text
/// heartbeat; <https://docs.polymarket.com/api-reference/wss/user> documents
/// the equivalent unfiltered initial frame plus a later market update. This
/// fixed one-condition profile sends exactly the former supported form, never
/// both forms.
///
/// Pinned Predarb `8222273a9c72033b760e1d2fec813bc77144556d`
/// corroborates host/path and reconnect subscription, but its five-second
/// heartbeat plus `initial_dump`/`operation` fields are stale negative
/// evidence and are intentionally not copied.
pub const PM_USER_WS_ENDPOINT: &str = "wss://ws-subscriptions-clob.polymarket.com/ws/user";
pub const PM_USER_WS_HEARTBEAT_INTERVAL: Duration = Duration::from_secs(10);

const MAX_USER_WS_TIMEOUT: Duration = Duration::from_secs(60);
const MAX_USER_WS_RECONNECT_ATTEMPTS: u8 = 16;
const MAX_USER_WS_CHANNEL_CAPACITY: usize = 1_024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PmUserWsBounds {
    connect_timeout: Duration,
    idle_timeout: Duration,
    pong_timeout: Duration,
    max_frame_bytes: usize,
    max_reconnect_attempts: u8,
    reconnect_backoff: Duration,
    event_channel_capacity: usize,
    initial_connection_epoch: ConnectionEpoch,
}

impl PmUserWsBounds {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        connect_timeout: Duration,
        idle_timeout: Duration,
        pong_timeout: Duration,
        max_frame_bytes: usize,
        max_reconnect_attempts: u8,
        reconnect_backoff: Duration,
        event_channel_capacity: usize,
        initial_connection_epoch: ConnectionEpoch,
    ) -> Result<Self, PmLiveAdapterError> {
        validate_bounds(
            connect_timeout,
            idle_timeout,
            PM_USER_WS_HEARTBEAT_INTERVAL,
            pong_timeout,
            max_frame_bytes,
            max_reconnect_attempts,
            reconnect_backoff,
            event_channel_capacity,
            initial_connection_epoch,
        )?;
        Ok(Self {
            connect_timeout,
            idle_timeout,
            pong_timeout,
            max_frame_bytes,
            max_reconnect_attempts,
            reconnect_backoff,
            event_channel_capacity,
            initial_connection_epoch,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EndpointMode {
    Production,
    #[cfg(any(test, feature = "loopback-evidence"))]
    LoopbackEvidence,
}

/// Exact one-condition authenticated user-channel configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PmUserWsConfig {
    endpoint: Url,
    condition: PmConditionId,
    bounds: PmUserWsBounds,
    heartbeat_interval: Duration,
    mode: EndpointMode,
}

impl PmUserWsConfig {
    pub fn production(
        condition: PmConditionId,
        bounds: PmUserWsBounds,
    ) -> Result<Self, PmLiveAdapterError> {
        Ok(Self {
            endpoint: validate_production_endpoint(PM_USER_WS_ENDPOINT)?,
            condition,
            bounds,
            heartbeat_interval: PM_USER_WS_HEARTBEAT_INTERVAL,
            mode: EndpointMode::Production,
        })
    }

    #[cfg(any(test, feature = "loopback-evidence"))]
    #[allow(clippy::too_many_arguments)]
    pub fn loopback_evidence(
        endpoint: &str,
        condition: PmConditionId,
        connect_timeout: Duration,
        idle_timeout: Duration,
        heartbeat_interval: Duration,
        pong_timeout: Duration,
        max_frame_bytes: usize,
        max_reconnect_attempts: u8,
        reconnect_backoff: Duration,
        event_channel_capacity: usize,
        initial_connection_epoch: ConnectionEpoch,
    ) -> Result<Self, PmLiveAdapterError> {
        validate_bounds(
            connect_timeout,
            idle_timeout,
            heartbeat_interval,
            pong_timeout,
            max_frame_bytes,
            max_reconnect_attempts,
            reconnect_backoff,
            event_channel_capacity,
            initial_connection_epoch,
        )?;
        Ok(Self {
            endpoint: validate_loopback_endpoint(endpoint)?,
            condition,
            bounds: PmUserWsBounds {
                connect_timeout,
                idle_timeout,
                pong_timeout,
                max_frame_bytes,
                max_reconnect_attempts,
                reconnect_backoff,
                event_channel_capacity,
                initial_connection_epoch,
            },
            heartbeat_interval,
            mode: EndpointMode::LoopbackEvidence,
        })
    }

    #[must_use]
    pub const fn condition(&self) -> PmConditionId {
        self.condition
    }

    #[must_use]
    pub const fn production_order_entry_authorized(&self) -> bool {
        false
    }

    pub(crate) fn endpoint(&self) -> &Url {
        &self.endpoint
    }

    pub(crate) const fn connect_timeout(&self) -> Duration {
        self.bounds.connect_timeout
    }

    pub(crate) const fn idle_timeout(&self) -> Duration {
        self.bounds.idle_timeout
    }

    pub(crate) const fn heartbeat_interval(&self) -> Duration {
        self.heartbeat_interval
    }

    pub(crate) const fn pong_timeout(&self) -> Duration {
        self.bounds.pong_timeout
    }

    pub(crate) const fn max_frame_bytes(&self) -> usize {
        self.bounds.max_frame_bytes
    }

    pub(crate) const fn max_reconnect_attempts(&self) -> u8 {
        self.bounds.max_reconnect_attempts
    }

    pub(crate) const fn reconnect_backoff(&self) -> Duration {
        self.bounds.reconnect_backoff
    }

    pub(crate) const fn event_channel_capacity(&self) -> usize {
        self.bounds.event_channel_capacity
    }

    pub(crate) const fn initial_connection_epoch(&self) -> ConnectionEpoch {
        self.bounds.initial_connection_epoch
    }

    #[allow(dead_code)]
    pub(crate) const fn is_production(&self) -> bool {
        matches!(self.mode, EndpointMode::Production)
    }
}

#[allow(clippy::too_many_arguments)]
fn validate_bounds(
    connect_timeout: Duration,
    idle_timeout: Duration,
    heartbeat_interval: Duration,
    pong_timeout: Duration,
    max_frame_bytes: usize,
    max_reconnect_attempts: u8,
    reconnect_backoff: Duration,
    event_channel_capacity: usize,
    initial_connection_epoch: ConnectionEpoch,
) -> Result<(), PmLiveAdapterError> {
    if [
        connect_timeout,
        idle_timeout,
        heartbeat_interval,
        pong_timeout,
    ]
    .into_iter()
    .any(|value| value.is_zero())
        || [
            connect_timeout,
            idle_timeout,
            heartbeat_interval,
            pong_timeout,
        ]
        .into_iter()
        .any(|value| value > MAX_USER_WS_TIMEOUT)
        || idle_timeout <= heartbeat_interval
        || pong_timeout >= heartbeat_interval
        || pong_timeout >= idle_timeout
    {
        return Err(PmLiveAdapterError::InvalidConfiguration(
            "user WebSocket timing bounds are invalid",
        ));
    }
    if max_frame_bytes == 0 || max_frame_bytes > MAX_PM_LIVE_BODY_BYTES {
        return Err(PmLiveAdapterError::InvalidConfiguration(
            "user WebSocket frame bound is invalid",
        ));
    }
    if max_reconnect_attempts > MAX_USER_WS_RECONNECT_ATTEMPTS
        || reconnect_backoff.is_zero()
        || reconnect_backoff > MAX_USER_WS_TIMEOUT
    {
        return Err(PmLiveAdapterError::InvalidConfiguration(
            "user WebSocket reconnect bounds are invalid",
        ));
    }
    if event_channel_capacity == 0
        || event_channel_capacity > MAX_USER_WS_CHANNEL_CAPACITY
        || initial_connection_epoch.value() == 0
    {
        return Err(PmLiveAdapterError::InvalidConfiguration(
            "user WebSocket channel or epoch bound is invalid",
        ));
    }
    Ok(())
}

fn validate_production_endpoint(value: &str) -> Result<Url, PmLiveAdapterError> {
    let endpoint = validate_endpoint(value)?;
    if endpoint.scheme() != "wss"
        || endpoint.host_str() != Some("ws-subscriptions-clob.polymarket.com")
        || endpoint.port_or_known_default() != Some(443)
    {
        return Err(PmLiveAdapterError::InvalidConfiguration(
            "production user WebSocket must use the exact documented WSS host",
        ));
    }
    Ok(endpoint)
}

#[cfg(any(test, feature = "loopback-evidence"))]
fn validate_loopback_endpoint(value: &str) -> Result<Url, PmLiveAdapterError> {
    let endpoint = validate_endpoint(value)?;
    let host = endpoint
        .host_str()
        .ok_or(PmLiveAdapterError::InvalidConfiguration(
            "user WebSocket endpoint must contain a host",
        ))?;
    if endpoint.scheme() != "ws"
        || !host
            .trim_matches(['[', ']'])
            .parse::<IpAddr>()
            .is_ok_and(|address| address.is_loopback())
        || endpoint.port().is_none()
    {
        return Err(PmLiveAdapterError::InvalidConfiguration(
            "user WebSocket evidence must use literal-loopback WS",
        ));
    }
    Ok(endpoint)
}

fn validate_endpoint(value: &str) -> Result<Url, PmLiveAdapterError> {
    let endpoint = Url::parse(value).map_err(|_| {
        PmLiveAdapterError::InvalidConfiguration("user WebSocket endpoint URL is malformed")
    })?;
    if endpoint.path() != "/ws/user"
        || endpoint.query().is_some()
        || endpoint.fragment().is_some()
        || !endpoint.username().is_empty()
        || endpoint.password().is_some()
        || endpoint.host_str().is_none()
    {
        return Err(PmLiveAdapterError::InvalidConfiguration(
            "user WebSocket endpoint must use exact /ws/user without URL extras",
        ));
    }
    Ok(endpoint)
}

#[cfg(test)]
mod tests {
    use reap_pm_core::PmConditionId;

    use super::*;

    fn condition() -> PmConditionId {
        PmConditionId::from_bytes([0x11; 32]).unwrap()
    }

    fn bounds() -> PmUserWsBounds {
        PmUserWsBounds::new(
            Duration::from_secs(2),
            Duration::from_secs(30),
            Duration::from_secs(5),
            64 * 1_024,
            3,
            Duration::from_millis(100),
            32,
            ConnectionEpoch::new(1),
        )
        .unwrap()
    }

    #[test]
    fn production_and_loopback_endpoints_are_exactly_separated() {
        let production = PmUserWsConfig::production(condition(), bounds()).unwrap();
        assert_eq!(production.endpoint().as_str(), PM_USER_WS_ENDPOINT);
        assert!(!production.production_order_entry_authorized());
        for invalid in [
            "ws://ws-subscriptions-clob.polymarket.com/ws/user",
            "wss://ws-subscriptions-clob.polymarket.com/ws/market",
            "wss://ws-subscriptions-clob.polymarket.com.evil/ws/user",
            "wss://ws-subscriptions-clob.polymarket.com/ws/user?all=true",
        ] {
            assert!(validate_production_endpoint(invalid).is_err());
        }
        assert!(
            PmUserWsConfig::loopback_evidence(
                "ws://127.0.0.1:18080/ws/user",
                condition(),
                Duration::from_millis(100),
                Duration::from_millis(500),
                Duration::from_millis(100),
                Duration::from_millis(50),
                1_024,
                1,
                Duration::from_millis(10),
                4,
                ConnectionEpoch::new(1),
            )
            .is_ok()
        );
        for invalid in [
            "ws://localhost:18080/ws/user",
            "ws://192.0.2.1:18080/ws/user",
            "ws://127.0.0.1:18080/ws/market",
        ] {
            assert!(validate_loopback_endpoint(invalid).is_err());
        }
    }
}
