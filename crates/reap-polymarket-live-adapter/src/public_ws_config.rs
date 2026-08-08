use std::time::Duration;

#[cfg(any(test, feature = "loopback-evidence"))]
use std::net::IpAddr;

use reap_pm_core::ConnectionEpoch;
use reap_polymarket_wire::{MAX_PUBLIC_WS_FRAME_BYTES, PmWireScope};
use url::Url;

use crate::PmLiveAdapterError;

/// Current official public market-channel endpoint.
///
/// Authority, revalidated for PM-T1 on 2026-08-08:
/// <https://docs.polymarket.com/market-data/websocket/overview>.
/// Pinned Predarb `8222273a9c72033b760e1d2fec813bc77144556d`
/// agrees on host/path but its initial frame and five-second heartbeat are
/// stale; they are intentionally not copied here.
pub const PM_PUBLIC_MARKET_WS_ENDPOINT: &str =
    "wss://ws-subscriptions-clob.polymarket.com/ws/market";
/// Current official text `PING`/`PONG` application heartbeat interval.
///
/// Authority: <https://docs.polymarket.com/market-data/websocket/market-channel>.
pub const PM_PUBLIC_WS_HEARTBEAT_INTERVAL: Duration = Duration::from_secs(10);

const MAX_WS_TIMEOUT: Duration = Duration::from_secs(60);
const MAX_RECONNECT_BACKOFF: Duration = Duration::from_secs(60);
const MAX_RECONNECT_ATTEMPTS: u8 = 16;
const MAX_EVENT_CHANNEL_CAPACITY: usize = 1_024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PmPublicWsBounds {
    connect_timeout: Duration,
    idle_timeout: Duration,
    pong_timeout: Duration,
    max_frame_bytes: usize,
    max_reconnect_attempts: u8,
    reconnect_backoff: Duration,
    event_channel_capacity: usize,
    initial_connection_epoch: ConnectionEpoch,
}

impl PmPublicWsBounds {
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
            PM_PUBLIC_WS_HEARTBEAT_INTERVAL,
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
    LocalEvidence,
}

/// Configuration for exactly one public market channel and outcome token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PmPublicWsConfig {
    endpoint: Url,
    scope: PmWireScope,
    bounds: PmPublicWsBounds,
    heartbeat_interval: Duration,
    mode: EndpointMode,
}

/// Secret-free transport bounds that must be paired with the canonical
/// public-session policy before a live socket can enter composition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PmPublicWsTransportPolicy {
    heartbeat_interval: Duration,
    pong_timeout: Duration,
    initial_connection_epoch: ConnectionEpoch,
    max_reconnect_attempts: u8,
    max_reconnect_backoff: Duration,
}

impl PmPublicWsTransportPolicy {
    #[must_use]
    pub const fn heartbeat_interval(self) -> Duration {
        self.heartbeat_interval
    }

    #[must_use]
    pub const fn pong_timeout(self) -> Duration {
        self.pong_timeout
    }

    #[must_use]
    pub const fn initial_connection_epoch(self) -> ConnectionEpoch {
        self.initial_connection_epoch
    }

    #[must_use]
    pub const fn max_reconnect_attempts(self) -> u8 {
        self.max_reconnect_attempts
    }

    #[must_use]
    pub const fn max_reconnect_backoff(self) -> Duration {
        self.max_reconnect_backoff
    }
}

impl PmPublicWsConfig {
    pub fn production(
        scope: PmWireScope,
        bounds: PmPublicWsBounds,
    ) -> Result<Self, PmLiveAdapterError> {
        let endpoint = validate_production_endpoint(PM_PUBLIC_MARKET_WS_ENDPOINT)?;
        Ok(Self {
            endpoint,
            scope,
            bounds,
            heartbeat_interval: PM_PUBLIC_WS_HEARTBEAT_INTERVAL,
            mode: EndpointMode::Production,
        })
    }

    #[cfg(any(test, feature = "loopback-evidence"))]
    #[allow(clippy::too_many_arguments)]
    pub fn loopback_evidence(
        endpoint: &str,
        scope: PmWireScope,
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
        let endpoint = validate_local_evidence_endpoint(endpoint)?;
        Ok(Self {
            endpoint,
            scope,
            bounds: PmPublicWsBounds {
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
            mode: EndpointMode::LocalEvidence,
        })
    }

    #[cfg(test)]
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn local_evidence(
        endpoint: &str,
        scope: PmWireScope,
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
        Self::loopback_evidence(
            endpoint,
            scope,
            connect_timeout,
            idle_timeout,
            heartbeat_interval,
            pong_timeout,
            max_frame_bytes,
            max_reconnect_attempts,
            reconnect_backoff,
            event_channel_capacity,
            initial_connection_epoch,
        )
    }

    #[must_use]
    pub const fn scope(&self) -> PmWireScope {
        self.scope
    }

    #[must_use]
    pub const fn transport_policy(&self) -> PmPublicWsTransportPolicy {
        PmPublicWsTransportPolicy {
            heartbeat_interval: self.heartbeat_interval,
            pong_timeout: self.bounds.pong_timeout,
            initial_connection_epoch: self.bounds.initial_connection_epoch,
            max_reconnect_attempts: self.bounds.max_reconnect_attempts,
            max_reconnect_backoff: self.bounds.reconnect_backoff,
        }
    }

    #[must_use]
    pub const fn production_order_entry_authorized(&self) -> bool {
        false
    }

    pub(crate) fn endpoint(&self) -> &Url {
        &self.endpoint
    }

    pub(crate) const fn heartbeat_interval(&self) -> Duration {
        self.heartbeat_interval
    }

    pub(crate) const fn connect_timeout(&self) -> Duration {
        self.bounds.connect_timeout
    }

    pub(crate) const fn idle_timeout(&self) -> Duration {
        self.bounds.idle_timeout
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
    if connect_timeout.is_zero()
        || idle_timeout.is_zero()
        || heartbeat_interval.is_zero()
        || pong_timeout.is_zero()
    {
        return Err(PmLiveAdapterError::InvalidConfiguration(
            "public WebSocket timeouts and heartbeat must be positive",
        ));
    }
    if connect_timeout > MAX_WS_TIMEOUT
        || idle_timeout > MAX_WS_TIMEOUT
        || heartbeat_interval > MAX_WS_TIMEOUT
        || pong_timeout > MAX_WS_TIMEOUT
    {
        return Err(PmLiveAdapterError::InvalidConfiguration(
            "public WebSocket timeouts and heartbeat must not exceed 60 seconds",
        ));
    }
    if idle_timeout <= heartbeat_interval
        || pong_timeout >= idle_timeout
        || pong_timeout >= heartbeat_interval
    {
        return Err(PmLiveAdapterError::InvalidConfiguration(
            "public WebSocket idle timeout must exceed heartbeat and pong must precede the next heartbeat",
        ));
    }
    if max_frame_bytes == 0 || max_frame_bytes > MAX_PUBLIC_WS_FRAME_BYTES {
        return Err(PmLiveAdapterError::InvalidConfiguration(
            "public WebSocket frame bound is invalid",
        ));
    }
    if max_reconnect_attempts > MAX_RECONNECT_ATTEMPTS {
        return Err(PmLiveAdapterError::InvalidConfiguration(
            "public WebSocket reconnect-attempt bound exceeds 16",
        ));
    }
    if reconnect_backoff.is_zero() || reconnect_backoff > MAX_RECONNECT_BACKOFF {
        return Err(PmLiveAdapterError::InvalidConfiguration(
            "public WebSocket reconnect backoff must be positive and at most 60 seconds",
        ));
    }
    if event_channel_capacity == 0 || event_channel_capacity > MAX_EVENT_CHANNEL_CAPACITY {
        return Err(PmLiveAdapterError::InvalidConfiguration(
            "public WebSocket event channel capacity must be between 1 and 1024",
        ));
    }
    if initial_connection_epoch.value() == 0 {
        return Err(PmLiveAdapterError::InvalidConfiguration(
            "public WebSocket connection epoch must be nonzero",
        ));
    }
    Ok(())
}

fn validate_production_endpoint(endpoint: &str) -> Result<Url, PmLiveAdapterError> {
    let endpoint = validate_endpoint(endpoint)?;
    if endpoint.scheme() != "wss"
        || endpoint.host_str() != Some("ws-subscriptions-clob.polymarket.com")
        || endpoint.port_or_known_default() != Some(443)
    {
        return Err(PmLiveAdapterError::InvalidConfiguration(
            "production public WebSocket must use the exact documented WSS host",
        ));
    }
    Ok(endpoint)
}

#[cfg(any(test, feature = "loopback-evidence"))]
fn validate_local_evidence_endpoint(endpoint: &str) -> Result<Url, PmLiveAdapterError> {
    let endpoint = validate_endpoint(endpoint)?;
    let host = endpoint
        .host_str()
        .ok_or(PmLiveAdapterError::InvalidConfiguration(
            "public WebSocket endpoint must contain a host",
        ))?;
    if endpoint.scheme() != "ws"
        || !host
            .trim_matches(['[', ']'])
            .parse::<IpAddr>()
            .is_ok_and(|address| address.is_loopback())
        || endpoint.port().is_none()
    {
        return Err(PmLiveAdapterError::InvalidConfiguration(
            "local public WebSocket evidence must use loopback WS with an explicit port",
        ));
    }
    Ok(endpoint)
}

fn validate_endpoint(endpoint: &str) -> Result<Url, PmLiveAdapterError> {
    let endpoint = Url::parse(endpoint).map_err(|_| {
        PmLiveAdapterError::InvalidConfiguration("public WebSocket endpoint URL is malformed")
    })?;
    if endpoint.path() != "/ws/market"
        || endpoint.query().is_some()
        || endpoint.fragment().is_some()
        || !endpoint.username().is_empty()
        || endpoint.password().is_some()
        || endpoint.host_str().is_none()
    {
        return Err(PmLiveAdapterError::InvalidConfiguration(
            "public WebSocket endpoint must use exact /ws/market without query, fragment, or user information",
        ));
    }
    Ok(endpoint)
}

#[cfg(test)]
mod tests {
    use reap_pm_core::{PmConditionId, PmMarketId, PmTokenId, U256};

    use super::*;

    fn scope() -> PmWireScope {
        PmWireScope::new(
            PmConditionId::from_bytes([1; 32]).unwrap(),
            PmMarketId::from_bytes([2; 32]).unwrap(),
            PmTokenId::new(U256::from_u64(3)).unwrap(),
        )
    }

    fn bounds() -> PmPublicWsBounds {
        PmPublicWsBounds::new(
            Duration::from_secs(2),
            Duration::from_secs(30),
            Duration::from_secs(5),
            MAX_PUBLIC_WS_FRAME_BYTES,
            3,
            Duration::from_millis(100),
            32,
            ConnectionEpoch::new(1),
        )
        .expect("valid bounds")
    }

    #[test]
    fn production_is_pinned_to_exact_current_public_market_endpoint() {
        let config = PmPublicWsConfig::production(scope(), bounds()).expect("production config");
        assert_eq!(
            config.endpoint().as_str(),
            "wss://ws-subscriptions-clob.polymarket.com/ws/market"
        );
        assert_eq!(config.heartbeat_interval(), PM_PUBLIC_WS_HEARTBEAT_INTERVAL);
        let policy = config.transport_policy();
        assert_eq!(policy.heartbeat_interval(), PM_PUBLIC_WS_HEARTBEAT_INTERVAL);
        assert_eq!(policy.pong_timeout(), Duration::from_secs(5));
        assert_eq!(policy.initial_connection_epoch(), ConnectionEpoch::new(1));
        assert_eq!(policy.max_reconnect_attempts(), 3);
        assert_eq!(policy.max_reconnect_backoff(), Duration::from_millis(100));
        assert!(!config.production_order_entry_authorized());

        for invalid in [
            "ws://ws-subscriptions-clob.polymarket.com/ws/market",
            "wss://ws-subscriptions-clob.polymarket.com.evil.example/ws/market",
            "wss://ws-subscriptions-clob.polymarket.com:8443/ws/market",
            "wss://ws-subscriptions-clob.polymarket.com/ws/user",
            "wss://user:secret@ws-subscriptions-clob.polymarket.com/ws/market",
            "wss://ws-subscriptions-clob.polymarket.com/ws/market?token=x",
        ] {
            assert!(validate_production_endpoint(invalid).is_err(), "{invalid}");
        }
    }

    #[test]
    fn local_evidence_is_loopback_only_and_all_bounds_fail_closed() {
        let valid = || {
            PmPublicWsConfig::local_evidence(
                "ws://127.0.0.1:18080/ws/market",
                scope(),
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
        };
        assert!(valid().is_ok());
        for invalid in [
            "ws://localhost:18080/ws/market",
            "ws://192.0.2.1:18080/ws/market",
            "wss://127.0.0.1:18080/ws/market",
            "ws://127.0.0.1/ws/market",
            "ws://127.0.0.1:18080/ws/user",
        ] {
            assert!(
                PmPublicWsConfig::local_evidence(
                    invalid,
                    scope(),
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
                .is_err(),
                "{invalid}"
            );
        }

        assert!(
            PmPublicWsBounds::new(
                Duration::ZERO,
                Duration::from_secs(30),
                Duration::from_secs(5),
                1_024,
                1,
                Duration::from_millis(1),
                1,
                ConnectionEpoch::new(1),
            )
            .is_err()
        );
        assert!(
            PmPublicWsBounds::new(
                Duration::from_secs(1),
                Duration::from_secs(30),
                Duration::from_secs(5),
                MAX_PUBLIC_WS_FRAME_BYTES + 1,
                1,
                Duration::from_millis(1),
                1,
                ConnectionEpoch::new(1),
            )
            .is_err()
        );
        assert!(
            PmPublicWsBounds::new(
                Duration::from_secs(1),
                Duration::from_secs(30),
                Duration::from_secs(5),
                1_024,
                MAX_RECONNECT_ATTEMPTS + 1,
                Duration::from_millis(1),
                1,
                ConnectionEpoch::new(1),
            )
            .is_err()
        );
        assert!(
            PmPublicWsBounds::new(
                Duration::from_secs(1),
                Duration::from_secs(30),
                Duration::from_secs(5),
                1_024,
                1,
                Duration::from_millis(1),
                0,
                ConnectionEpoch::new(1),
            )
            .is_err()
        );
        assert!(
            PmPublicWsBounds::new(
                Duration::from_secs(1),
                Duration::from_secs(30),
                Duration::from_secs(5),
                1_024,
                1,
                Duration::from_millis(1),
                1,
                ConnectionEpoch::new(0),
            )
            .is_err()
        );
    }
}
