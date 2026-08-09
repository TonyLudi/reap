use std::time::Duration;

#[cfg(any(test, feature = "loopback-evidence", feature = "read-only-evidence"))]
use std::net::IpAddr;

use reap_polymarket_wire::PmWireScope;
use url::Url;

use crate::PmLiveAdapterError;

pub const PM_CLOB_PRODUCTION_ORIGIN: &str = "https://clob.polymarket.com";
pub const PM_GEOBLOCK_PRODUCTION_ORIGIN: &str = "https://polymarket.com";
const MAX_HTTP_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OriginMode {
    Production,
    #[cfg(any(test, feature = "loopback-evidence", feature = "read-only-evidence"))]
    LocalEvidence,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PmPublicHttpConfig {
    origin: Url,
    connect_timeout: Duration,
    request_timeout: Duration,
    mode: OriginMode,
}

/// Fixed public-safety endpoint configuration. Production construction has no
/// caller-supplied origin and can reach only `https://polymarket.com`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PmGeoblockHttpConfig {
    origin: Url,
    connect_timeout: Duration,
    request_timeout: Duration,
    mode: OriginMode,
}

impl PmGeoblockHttpConfig {
    pub fn production(
        connect_timeout: Duration,
        request_timeout: Duration,
    ) -> Result<Self, PmLiveAdapterError> {
        validate_timeouts(connect_timeout, request_timeout)?;
        let origin =
            validate_exact_production_origin(PM_GEOBLOCK_PRODUCTION_ORIGIN, "polymarket.com")?;
        Ok(Self {
            origin,
            connect_timeout,
            request_timeout,
            mode: OriginMode::Production,
        })
    }

    #[cfg(any(test, feature = "read-only-evidence"))]
    pub fn read_only_evidence(
        origin: &str,
        connect_timeout: Duration,
        request_timeout: Duration,
    ) -> Result<Self, PmLiveAdapterError> {
        Self::local_evidence(origin, connect_timeout, request_timeout)
    }

    #[cfg(any(test, feature = "loopback-evidence", feature = "read-only-evidence"))]
    pub(crate) fn local_evidence(
        origin: &str,
        connect_timeout: Duration,
        request_timeout: Duration,
    ) -> Result<Self, PmLiveAdapterError> {
        validate_timeouts(connect_timeout, request_timeout)?;
        Ok(Self {
            origin: validate_local_evidence_origin(origin)?,
            connect_timeout,
            request_timeout,
            mode: OriginMode::LocalEvidence,
        })
    }

    pub(crate) fn origin(&self) -> &Url {
        &self.origin
    }

    pub(crate) const fn connect_timeout(&self) -> Duration {
        self.connect_timeout
    }

    pub(crate) const fn request_timeout(&self) -> Duration {
        self.request_timeout
    }

    pub(crate) const fn mode(&self) -> OriginMode {
        self.mode
    }

    #[must_use]
    pub const fn production_order_entry_authorized(&self) -> bool {
        false
    }
}

/// Fixed private-read configuration for one PM-T1 instrument.
///
/// Account-wide order/trade cuts deliberately retain rows for every market.
/// The scope is used only for the configured conditional-token balance query
/// and strict validation of a journal-known exact order detail.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PmPrivateHttpConfig {
    origin: Url,
    connect_timeout: Duration,
    request_timeout: Duration,
    exact_order_scope: PmWireScope,
    mode: OriginMode,
}

impl PmPrivateHttpConfig {
    pub fn production(
        connect_timeout: Duration,
        request_timeout: Duration,
        exact_order_scope: PmWireScope,
    ) -> Result<Self, PmLiveAdapterError> {
        validate_timeouts(connect_timeout, request_timeout)?;
        let origin = validate_production_origin(PM_CLOB_PRODUCTION_ORIGIN)?;
        Ok(Self {
            origin,
            connect_timeout,
            request_timeout,
            exact_order_scope,
            mode: OriginMode::Production,
        })
    }

    #[cfg(any(test, feature = "loopback-evidence"))]
    pub fn loopback_evidence(
        origin: &str,
        connect_timeout: Duration,
        request_timeout: Duration,
        exact_order_scope: PmWireScope,
    ) -> Result<Self, PmLiveAdapterError> {
        Self::local_read_config(origin, connect_timeout, request_timeout, exact_order_scope)
    }

    #[cfg(any(test, feature = "read-only-evidence"))]
    pub fn read_only_evidence(
        origin: &str,
        connect_timeout: Duration,
        request_timeout: Duration,
        exact_order_scope: PmWireScope,
    ) -> Result<Self, PmLiveAdapterError> {
        Self::local_read_config(origin, connect_timeout, request_timeout, exact_order_scope)
    }

    #[cfg(any(test, feature = "loopback-evidence", feature = "read-only-evidence"))]
    fn local_read_config(
        origin: &str,
        connect_timeout: Duration,
        request_timeout: Duration,
        exact_order_scope: PmWireScope,
    ) -> Result<Self, PmLiveAdapterError> {
        validate_timeouts(connect_timeout, request_timeout)?;
        let origin = validate_local_evidence_origin(origin)?;
        Ok(Self {
            origin,
            connect_timeout,
            request_timeout,
            exact_order_scope,
            mode: OriginMode::LocalEvidence,
        })
    }

    #[cfg(test)]
    pub(crate) fn local_evidence(
        origin: &str,
        connect_timeout: Duration,
        request_timeout: Duration,
        exact_order_scope: PmWireScope,
    ) -> Result<Self, PmLiveAdapterError> {
        Self::local_read_config(origin, connect_timeout, request_timeout, exact_order_scope)
    }

    #[must_use]
    pub const fn exact_order_scope(&self) -> PmWireScope {
        self.exact_order_scope
    }

    #[must_use]
    pub const fn production_order_entry_authorized(&self) -> bool {
        false
    }

    pub(crate) fn origin(&self) -> &Url {
        &self.origin
    }

    pub(crate) const fn connect_timeout(&self) -> Duration {
        self.connect_timeout
    }

    pub(crate) const fn request_timeout(&self) -> Duration {
        self.request_timeout
    }

    pub(crate) const fn mode(&self) -> OriginMode {
        self.mode
    }
}

impl PmPublicHttpConfig {
    pub fn production(
        origin: &str,
        connect_timeout: Duration,
        request_timeout: Duration,
    ) -> Result<Self, PmLiveAdapterError> {
        validate_timeouts(connect_timeout, request_timeout)?;
        let origin = validate_production_origin(origin)?;
        Ok(Self {
            origin,
            connect_timeout,
            request_timeout,
            mode: OriginMode::Production,
        })
    }

    #[cfg(any(test, feature = "loopback-evidence"))]
    pub fn loopback_evidence(
        origin: &str,
        connect_timeout: Duration,
        request_timeout: Duration,
    ) -> Result<Self, PmLiveAdapterError> {
        Self::local_read_config(origin, connect_timeout, request_timeout)
    }

    #[cfg(any(test, feature = "read-only-evidence"))]
    pub fn read_only_evidence(
        origin: &str,
        connect_timeout: Duration,
        request_timeout: Duration,
    ) -> Result<Self, PmLiveAdapterError> {
        Self::local_read_config(origin, connect_timeout, request_timeout)
    }

    #[cfg(any(test, feature = "loopback-evidence", feature = "read-only-evidence"))]
    fn local_read_config(
        origin: &str,
        connect_timeout: Duration,
        request_timeout: Duration,
    ) -> Result<Self, PmLiveAdapterError> {
        validate_timeouts(connect_timeout, request_timeout)?;
        let origin = validate_local_evidence_origin(origin)?;
        Ok(Self {
            origin,
            connect_timeout,
            request_timeout,
            mode: OriginMode::LocalEvidence,
        })
    }

    #[cfg(any(test, feature = "loopback-evidence", feature = "read-only-evidence"))]
    pub(crate) fn local_evidence(
        origin: &str,
        connect_timeout: Duration,
        request_timeout: Duration,
    ) -> Result<Self, PmLiveAdapterError> {
        Self::local_read_config(origin, connect_timeout, request_timeout)
    }

    #[must_use]
    pub const fn connect_timeout(&self) -> Duration {
        self.connect_timeout
    }

    #[must_use]
    pub const fn request_timeout(&self) -> Duration {
        self.request_timeout
    }

    #[must_use]
    pub const fn production_order_entry_authorized(&self) -> bool {
        false
    }

    pub(crate) fn origin(&self) -> &Url {
        &self.origin
    }

    pub(crate) const fn mode(&self) -> OriginMode {
        self.mode
    }
}

fn validate_timeouts(
    connect_timeout: Duration,
    request_timeout: Duration,
) -> Result<(), PmLiveAdapterError> {
    if connect_timeout.is_zero() || request_timeout.is_zero() {
        return Err(PmLiveAdapterError::InvalidConfiguration(
            "connect and request timeouts must be positive",
        ));
    }
    if connect_timeout > MAX_HTTP_TIMEOUT || request_timeout > MAX_HTTP_TIMEOUT {
        return Err(PmLiveAdapterError::InvalidConfiguration(
            "connect and request timeouts must not exceed 60 seconds",
        ));
    }
    Ok(())
}

fn validate_production_origin(origin: &str) -> Result<Url, PmLiveAdapterError> {
    validate_exact_production_origin(origin, "clob.polymarket.com")
}

fn validate_exact_production_origin(
    origin: &str,
    exact_host: &'static str,
) -> Result<Url, PmLiveAdapterError> {
    let url = validate_base_origin(origin)?;
    if url.scheme() != "https" {
        return Err(PmLiveAdapterError::InvalidConfiguration(
            "production origin must use HTTPS",
        ));
    }
    if url.host_str() != Some(exact_host) {
        return Err(PmLiveAdapterError::InvalidConfiguration(
            "production origin must be the exact documented host",
        ));
    }
    if url.port_or_known_default() != Some(443) {
        return Err(PmLiveAdapterError::InvalidConfiguration(
            "production origin must use HTTPS port 443",
        ));
    }
    Ok(url)
}

#[cfg(any(test, feature = "loopback-evidence", feature = "read-only-evidence"))]
fn validate_local_evidence_origin(origin: &str) -> Result<Url, PmLiveAdapterError> {
    let url = validate_base_origin(origin)?;
    if url.scheme() != "http" {
        return Err(PmLiveAdapterError::InvalidConfiguration(
            "local evidence origin must use loopback HTTP",
        ));
    }
    let host = url
        .host_str()
        .ok_or(PmLiveAdapterError::InvalidConfiguration(
            "origin must contain a host",
        ))?;
    if !host
        .trim_matches(['[', ']'])
        .parse::<IpAddr>()
        .is_ok_and(|address| address.is_loopback())
    {
        return Err(PmLiveAdapterError::InvalidConfiguration(
            "local evidence origin must use a literal loopback address",
        ));
    }
    if url.port().is_none() {
        return Err(PmLiveAdapterError::InvalidConfiguration(
            "local evidence origin must use an explicit port",
        ));
    }
    Ok(url)
}

fn validate_base_origin(origin: &str) -> Result<Url, PmLiveAdapterError> {
    let url = Url::parse(origin)
        .map_err(|_| PmLiveAdapterError::InvalidConfiguration("origin URL is malformed"))?;
    if !url.username().is_empty() || url.password().is_some() {
        return Err(PmLiveAdapterError::InvalidConfiguration(
            "origin must not contain user information",
        ));
    }
    if url.path() != "/" || url.query().is_some() || url.fragment().is_some() {
        return Err(PmLiveAdapterError::InvalidConfiguration(
            "origin must use exact root path without query or fragment",
        ));
    }
    if url.host_str().is_none() {
        return Err(PmLiveAdapterError::InvalidConfiguration(
            "origin must contain a host",
        ));
    }
    Ok(url)
}

#[cfg(test)]
mod tests {
    use super::*;

    const CONNECT: Duration = Duration::from_millis(100);
    const REQUEST: Duration = Duration::from_millis(200);

    #[test]
    fn production_accepts_only_the_exact_https_clob_origin() {
        let config = PmPublicHttpConfig::production(PM_CLOB_PRODUCTION_ORIGIN, CONNECT, REQUEST)
            .expect("official CLOB origin");
        assert_eq!(config.origin().as_str(), "https://clob.polymarket.com/");
        assert!(!config.production_order_entry_authorized());

        for invalid in [
            "http://clob.polymarket.com",
            "https://clob.polymarket.com.evil.example",
            "https://127.0.0.1",
            "https://clob.polymarket.com:8443",
            "https://user:secret@clob.polymarket.com",
            "https://clob.polymarket.com/book",
            "https://clob.polymarket.com/?next=/book",
            "https://clob.polymarket.com/#fragment",
        ] {
            assert!(PmPublicHttpConfig::production(invalid, CONNECT, REQUEST).is_err());
        }
    }

    #[test]
    fn geoblock_production_origin_is_fixed_and_not_caller_selectable() {
        let config =
            PmGeoblockHttpConfig::production(CONNECT, REQUEST).expect("official geoblock origin");
        assert_eq!(config.origin().as_str(), "https://polymarket.com/");
        assert_eq!(config.mode(), OriginMode::Production);
        assert!(!config.production_order_entry_authorized());
    }

    #[test]
    fn local_evidence_is_a_separate_literal_loopback_only_mode() {
        assert!(
            PmPublicHttpConfig::local_evidence("http://127.0.0.1:18080", CONNECT, REQUEST).is_ok()
        );
        assert!(PmPublicHttpConfig::local_evidence("http://[::1]:18080", CONNECT, REQUEST).is_ok());
        for invalid in [
            "http://localhost:18080",
            "http://192.0.2.1:18080",
            "https://127.0.0.1:18080",
            "http://127.0.0.1",
        ] {
            assert!(PmPublicHttpConfig::local_evidence(invalid, CONNECT, REQUEST).is_err());
        }
        assert!(
            PmPublicHttpConfig::production("http://127.0.0.1:18080", CONNECT, REQUEST).is_err()
        );

        assert!(
            PmGeoblockHttpConfig::read_only_evidence("http://127.0.0.1:18080", CONNECT, REQUEST,)
                .is_ok()
        );
        for invalid in [
            "http://localhost:18080",
            "http://192.0.2.1:18080",
            "https://127.0.0.1:18080",
            "http://127.0.0.1",
        ] {
            assert!(PmGeoblockHttpConfig::read_only_evidence(invalid, CONNECT, REQUEST).is_err());
        }
    }

    #[test]
    fn timeouts_are_positive_and_bounded() {
        assert!(
            PmPublicHttpConfig::production(PM_CLOB_PRODUCTION_ORIGIN, Duration::ZERO, REQUEST,)
                .is_err()
        );
        assert!(
            PmPublicHttpConfig::production(
                PM_CLOB_PRODUCTION_ORIGIN,
                CONNECT,
                Duration::from_secs(61),
            )
            .is_err()
        );
    }
}
