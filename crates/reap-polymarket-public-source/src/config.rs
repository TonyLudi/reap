#[cfg(test)]
use std::net::IpAddr;
use std::time::Duration;

use reap_pm_core::{EvmAddress, PmConditionId, PmTokenId};
use reqwest::Url;

use crate::PmPublicPositionError;

pub const PM_DATA_API_PRODUCTION_ORIGIN: &str = "https://data-api.polymarket.com";
const MAX_HTTP_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PmDataApiPositionScope {
    proxy_funder: EvmAddress,
    condition: PmConditionId,
    configured_token: PmTokenId,
}

impl PmDataApiPositionScope {
    #[must_use]
    pub const fn new(
        proxy_funder: EvmAddress,
        condition: PmConditionId,
        configured_token: PmTokenId,
    ) -> Self {
        Self {
            proxy_funder,
            condition,
            configured_token,
        }
    }

    #[must_use]
    pub const fn proxy_funder(self) -> EvmAddress {
        self.proxy_funder
    }

    #[must_use]
    pub const fn condition(self) -> PmConditionId {
        self.condition
    }

    #[must_use]
    pub const fn configured_token(self) -> PmTokenId {
        self.configured_token
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OriginMode {
    Production,
    #[cfg(test)]
    NumericLoopback,
}

/// Closed configuration for the public Data API position observation.
///
/// Production construction has no origin, route, query, credential, or HTTP
/// method input. The only test seam is a numeric loopback origin.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PmDataApiPositionConfig {
    origin: Url,
    connect_timeout: Duration,
    request_timeout: Duration,
    scope: PmDataApiPositionScope,
    mode: OriginMode,
}

impl PmDataApiPositionConfig {
    pub fn production(
        scope: PmDataApiPositionScope,
        connect_timeout: Duration,
        request_timeout: Duration,
    ) -> Result<Self, PmPublicPositionError> {
        validate_timeouts(connect_timeout, request_timeout)?;
        let origin = exact_production_origin()?;
        Ok(Self {
            origin,
            connect_timeout,
            request_timeout,
            scope,
            mode: OriginMode::Production,
        })
    }

    #[cfg(test)]
    pub(crate) fn numeric_loopback_evidence(
        origin: &str,
        scope: PmDataApiPositionScope,
        connect_timeout: Duration,
        request_timeout: Duration,
    ) -> Result<Self, PmPublicPositionError> {
        validate_timeouts(connect_timeout, request_timeout)?;
        let origin = validate_numeric_loopback_origin(origin)?;
        Ok(Self {
            origin,
            connect_timeout,
            request_timeout,
            scope,
            mode: OriginMode::NumericLoopback,
        })
    }

    #[must_use]
    pub const fn scope(&self) -> PmDataApiPositionScope {
        self.scope
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

fn validate_timeouts(
    connect_timeout: Duration,
    request_timeout: Duration,
) -> Result<(), PmPublicPositionError> {
    if connect_timeout.is_zero()
        || request_timeout.is_zero()
        || connect_timeout > MAX_HTTP_TIMEOUT
        || request_timeout > MAX_HTTP_TIMEOUT
    {
        return Err(PmPublicPositionError::InvalidConfiguration);
    }
    Ok(())
}

fn exact_production_origin() -> Result<Url, PmPublicPositionError> {
    let origin = Url::parse(PM_DATA_API_PRODUCTION_ORIGIN)
        .map_err(|_| PmPublicPositionError::InvalidConfiguration)?;
    if origin.scheme() != "https"
        || origin.host_str() != Some("data-api.polymarket.com")
        || origin.port_or_known_default() != Some(443)
        || origin.path() != "/"
        || origin.query().is_some()
        || origin.fragment().is_some()
        || !origin.username().is_empty()
        || origin.password().is_some()
    {
        return Err(PmPublicPositionError::InvalidConfiguration);
    }
    Ok(origin)
}

#[cfg(test)]
fn validate_numeric_loopback_origin(origin: &str) -> Result<Url, PmPublicPositionError> {
    let origin = Url::parse(origin).map_err(|_| PmPublicPositionError::InvalidConfiguration)?;
    let numeric_host = origin
        .host_str()
        .and_then(|host| host.parse::<IpAddr>().ok())
        .is_some_and(|address| address.is_loopback());
    if origin.scheme() != "http"
        || !numeric_host
        || origin.port().is_none()
        || origin.path() != "/"
        || origin.query().is_some()
        || origin.fragment().is_some()
        || !origin.username().is_empty()
        || origin.password().is_some()
    {
        return Err(PmPublicPositionError::InvalidConfiguration);
    }
    Ok(origin)
}

#[cfg(test)]
mod tests {
    use reap_pm_core::U256;

    use super::*;

    fn scope() -> PmDataApiPositionScope {
        PmDataApiPositionScope::new(
            EvmAddress::parse("0x1111111111111111111111111111111111111111").unwrap(),
            PmConditionId::parse(
                "0x2222222222222222222222222222222222222222222222222222222222222222",
            )
            .unwrap(),
            PmTokenId::new(U256::from_u64(3)).unwrap(),
        )
    }

    #[test]
    fn production_origin_is_fixed_and_timeouts_are_bounded() {
        let config = PmDataApiPositionConfig::production(
            scope(),
            Duration::from_secs(1),
            Duration::from_secs(2),
        )
        .unwrap();
        assert_eq!(
            config.origin().as_str(),
            PM_DATA_API_PRODUCTION_ORIGIN.to_owned() + "/"
        );
        assert_eq!(config.mode(), OriginMode::Production);
        assert!(!config.production_order_entry_authorized());
        assert_eq!(
            PmDataApiPositionConfig::production(scope(), Duration::ZERO, Duration::from_secs(1)),
            Err(PmPublicPositionError::InvalidConfiguration)
        );
    }

    #[test]
    fn test_seam_requires_numeric_loopback_with_an_explicit_port() {
        assert!(
            PmDataApiPositionConfig::numeric_loopback_evidence(
                "http://127.0.0.1:1234",
                scope(),
                Duration::from_secs(1),
                Duration::from_secs(1),
            )
            .is_ok()
        );
        for invalid in [
            "http://localhost:1234",
            "https://127.0.0.1:1234",
            "http://127.0.0.1",
            "http://127.0.0.1:1234/path",
            "http://127.0.0.1:1234?query=x",
        ] {
            assert_eq!(
                PmDataApiPositionConfig::numeric_loopback_evidence(
                    invalid,
                    scope(),
                    Duration::from_secs(1),
                    Duration::from_secs(1),
                ),
                Err(PmPublicPositionError::InvalidConfiguration),
                "accepted {invalid}"
            );
        }
    }
}
