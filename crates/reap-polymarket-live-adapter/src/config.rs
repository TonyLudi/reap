use std::time::Duration;

#[cfg(any(test, feature = "loopback-evidence", feature = "read-only-evidence"))]
use std::net::IpAddr;

use reap_polymarket_egress_binding::{PmFixedTlsPeerSelection, PmLocalEgressSelection};
use reap_polymarket_wire::PmWireScope;
use url::Url;

use crate::PmLiveAdapterError;

pub const PM_CLOB_PRODUCTION_ORIGIN: &str = "https://clob.polymarket.com";
pub const PM_GEOBLOCK_PRODUCTION_ORIGIN: &str = "https://polymarket.com";
pub const PM_STATUS_PRODUCTION_ORIGIN: &str = "https://status.polymarket.com";
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
    selected_local_egress: Option<PmLocalEgressSelection>,
    fixed_tls_peer: Option<PmFixedTlsPeerSelection>,
}

/// Fixed public-safety endpoint configuration. Production construction has no
/// caller-supplied origin and can reach only `https://polymarket.com`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PmGeoblockHttpConfig {
    origin: Url,
    connect_timeout: Duration,
    request_timeout: Duration,
    mode: OriginMode,
    selected_local_egress: Option<PmLocalEgressSelection>,
    fixed_tls_peer: Option<PmFixedTlsPeerSelection>,
}

/// Crate-private configuration for the fixed Polymarket status-page source.
/// Production construction has no caller-selected origin.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PmStatusHttpConfig {
    origin: Url,
    connect_timeout: Duration,
    request_timeout: Duration,
    mode: OriginMode,
    selected_local_egress: Option<PmLocalEgressSelection>,
    fixed_tls_peer: Option<PmFixedTlsPeerSelection>,
}

impl PmStatusHttpConfig {
    pub(crate) fn production(
        connect_timeout: Duration,
        request_timeout: Duration,
    ) -> Result<Self, PmLiveAdapterError> {
        validate_timeouts(connect_timeout, request_timeout)?;
        let origin =
            validate_exact_production_origin(PM_STATUS_PRODUCTION_ORIGIN, "status.polymarket.com")?;
        Ok(Self {
            origin,
            connect_timeout,
            request_timeout,
            mode: OriginMode::Production,
            selected_local_egress: None,
            fixed_tls_peer: None,
        })
    }

    pub(crate) fn production_on_selected_local_egress(
        connect_timeout: Duration,
        request_timeout: Duration,
        selected_local_egress: PmLocalEgressSelection,
    ) -> Result<Self, PmLiveAdapterError> {
        require_production_local_egress(&selected_local_egress)?;
        let mut config = Self::production(connect_timeout, request_timeout)?;
        config.selected_local_egress = Some(selected_local_egress);
        Ok(config)
    }

    pub(crate) fn production_on_fixed_tls_peer_and_selected_local_egress(
        connect_timeout: Duration,
        request_timeout: Duration,
        fixed_tls_peer: PmFixedTlsPeerSelection,
        selected_local_egress: PmLocalEgressSelection,
    ) -> Result<Self, PmLiveAdapterError> {
        require_production_fixed_tls_peer(&fixed_tls_peer, "status.polymarket.com")?;
        require_production_local_egress(&selected_local_egress)?;
        require_same_address_family(&fixed_tls_peer, &selected_local_egress)?;
        let mut config = Self::production(connect_timeout, request_timeout)?;
        config.fixed_tls_peer = Some(fixed_tls_peer);
        config.selected_local_egress = Some(selected_local_egress);
        Ok(config)
    }

    #[cfg(any(test, feature = "read-only-evidence"))]
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
            selected_local_egress: None,
            fixed_tls_peer: None,
        })
    }

    #[cfg(test)]
    pub(crate) fn local_evidence_on_selected_local_egress(
        origin: &str,
        connect_timeout: Duration,
        request_timeout: Duration,
        selected_local_egress: PmLocalEgressSelection,
    ) -> Result<Self, PmLiveAdapterError> {
        require_loopback_local_egress(&selected_local_egress)?;
        let mut config = Self::local_evidence(origin, connect_timeout, request_timeout)?;
        config.selected_local_egress = Some(selected_local_egress);
        Ok(config)
    }

    #[cfg(test)]
    pub(crate) fn loopback_evidence_on_fixed_tls_peer_and_selected_local_egress(
        connect_timeout: Duration,
        request_timeout: Duration,
        fixed_tls_peer: PmFixedTlsPeerSelection,
        selected_local_egress: PmLocalEgressSelection,
    ) -> Result<Self, PmLiveAdapterError> {
        require_loopback_fixed_tls_peer(&fixed_tls_peer)?;
        require_loopback_local_egress(&selected_local_egress)?;
        require_same_address_family(&fixed_tls_peer, &selected_local_egress)?;
        validate_timeouts(connect_timeout, request_timeout)?;
        Ok(Self {
            origin: fixed_tls_peer_loopback_origin(&fixed_tls_peer)?,
            connect_timeout,
            request_timeout,
            mode: OriginMode::LocalEvidence,
            selected_local_egress: Some(selected_local_egress),
            fixed_tls_peer: Some(fixed_tls_peer),
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

    pub(crate) const fn selected_local_egress(&self) -> Option<&PmLocalEgressSelection> {
        self.selected_local_egress.as_ref()
    }

    pub(crate) const fn fixed_tls_peer(&self) -> Option<&PmFixedTlsPeerSelection> {
        self.fixed_tls_peer.as_ref()
    }
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
            selected_local_egress: None,
            fixed_tls_peer: None,
        })
    }

    pub fn production_on_selected_local_egress(
        connect_timeout: Duration,
        request_timeout: Duration,
        selected_local_egress: PmLocalEgressSelection,
    ) -> Result<Self, PmLiveAdapterError> {
        require_production_local_egress(&selected_local_egress)?;
        let mut config = Self::production(connect_timeout, request_timeout)?;
        config.selected_local_egress = Some(selected_local_egress);
        Ok(config)
    }

    pub fn production_on_fixed_tls_peer_and_selected_local_egress(
        connect_timeout: Duration,
        request_timeout: Duration,
        fixed_tls_peer: PmFixedTlsPeerSelection,
        selected_local_egress: PmLocalEgressSelection,
    ) -> Result<Self, PmLiveAdapterError> {
        require_production_fixed_tls_peer(&fixed_tls_peer, "polymarket.com")?;
        require_production_local_egress(&selected_local_egress)?;
        require_same_address_family(&fixed_tls_peer, &selected_local_egress)?;
        let mut config = Self::production(connect_timeout, request_timeout)?;
        config.fixed_tls_peer = Some(fixed_tls_peer);
        config.selected_local_egress = Some(selected_local_egress);
        Ok(config)
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
            selected_local_egress: None,
            fixed_tls_peer: None,
        })
    }

    #[cfg(test)]
    pub(crate) fn local_evidence_on_selected_local_egress(
        origin: &str,
        connect_timeout: Duration,
        request_timeout: Duration,
        selected_local_egress: PmLocalEgressSelection,
    ) -> Result<Self, PmLiveAdapterError> {
        require_loopback_local_egress(&selected_local_egress)?;
        let mut config = Self::local_evidence(origin, connect_timeout, request_timeout)?;
        config.selected_local_egress = Some(selected_local_egress);
        Ok(config)
    }

    #[cfg(test)]
    pub(crate) fn loopback_evidence_on_fixed_tls_peer_and_selected_local_egress(
        connect_timeout: Duration,
        request_timeout: Duration,
        fixed_tls_peer: PmFixedTlsPeerSelection,
        selected_local_egress: PmLocalEgressSelection,
    ) -> Result<Self, PmLiveAdapterError> {
        require_loopback_fixed_tls_peer(&fixed_tls_peer)?;
        require_loopback_local_egress(&selected_local_egress)?;
        require_same_address_family(&fixed_tls_peer, &selected_local_egress)?;
        validate_timeouts(connect_timeout, request_timeout)?;
        Ok(Self {
            origin: fixed_tls_peer_loopback_origin(&fixed_tls_peer)?,
            connect_timeout,
            request_timeout,
            mode: OriginMode::LocalEvidence,
            selected_local_egress: Some(selected_local_egress),
            fixed_tls_peer: Some(fixed_tls_peer),
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

    pub(crate) const fn selected_local_egress(&self) -> Option<&PmLocalEgressSelection> {
        self.selected_local_egress.as_ref()
    }

    pub(crate) const fn fixed_tls_peer(&self) -> Option<&PmFixedTlsPeerSelection> {
        self.fixed_tls_peer.as_ref()
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
    selected_local_egress: Option<PmLocalEgressSelection>,
    fixed_tls_peer: Option<PmFixedTlsPeerSelection>,
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
            selected_local_egress: None,
            fixed_tls_peer: None,
        })
    }

    pub fn production_on_selected_local_egress(
        connect_timeout: Duration,
        request_timeout: Duration,
        exact_order_scope: PmWireScope,
        selected_local_egress: PmLocalEgressSelection,
    ) -> Result<Self, PmLiveAdapterError> {
        require_production_local_egress(&selected_local_egress)?;
        let mut config = Self::production(connect_timeout, request_timeout, exact_order_scope)?;
        config.selected_local_egress = Some(selected_local_egress);
        Ok(config)
    }

    pub fn production_on_fixed_tls_peer_and_selected_local_egress(
        connect_timeout: Duration,
        request_timeout: Duration,
        exact_order_scope: PmWireScope,
        fixed_tls_peer: PmFixedTlsPeerSelection,
        selected_local_egress: PmLocalEgressSelection,
    ) -> Result<Self, PmLiveAdapterError> {
        require_production_fixed_tls_peer(&fixed_tls_peer, "clob.polymarket.com")?;
        require_production_local_egress(&selected_local_egress)?;
        require_same_address_family(&fixed_tls_peer, &selected_local_egress)?;
        let mut config = Self::production(connect_timeout, request_timeout, exact_order_scope)?;
        config.fixed_tls_peer = Some(fixed_tls_peer);
        config.selected_local_egress = Some(selected_local_egress);
        Ok(config)
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
            selected_local_egress: None,
            fixed_tls_peer: None,
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

    #[cfg(test)]
    pub(crate) fn local_evidence_on_selected_local_egress(
        origin: &str,
        connect_timeout: Duration,
        request_timeout: Duration,
        exact_order_scope: PmWireScope,
        selected_local_egress: PmLocalEgressSelection,
    ) -> Result<Self, PmLiveAdapterError> {
        require_loopback_local_egress(&selected_local_egress)?;
        let mut config =
            Self::local_evidence(origin, connect_timeout, request_timeout, exact_order_scope)?;
        config.selected_local_egress = Some(selected_local_egress);
        Ok(config)
    }

    #[cfg(test)]
    pub(crate) fn loopback_evidence_on_fixed_tls_peer_and_selected_local_egress(
        connect_timeout: Duration,
        request_timeout: Duration,
        exact_order_scope: PmWireScope,
        fixed_tls_peer: PmFixedTlsPeerSelection,
        selected_local_egress: PmLocalEgressSelection,
    ) -> Result<Self, PmLiveAdapterError> {
        require_loopback_fixed_tls_peer(&fixed_tls_peer)?;
        require_loopback_local_egress(&selected_local_egress)?;
        require_same_address_family(&fixed_tls_peer, &selected_local_egress)?;
        validate_timeouts(connect_timeout, request_timeout)?;
        Ok(Self {
            origin: fixed_tls_peer_loopback_origin(&fixed_tls_peer)?,
            connect_timeout,
            request_timeout,
            exact_order_scope,
            mode: OriginMode::LocalEvidence,
            selected_local_egress: Some(selected_local_egress),
            fixed_tls_peer: Some(fixed_tls_peer),
        })
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

    pub(crate) const fn selected_local_egress(&self) -> Option<&PmLocalEgressSelection> {
        self.selected_local_egress.as_ref()
    }

    pub(crate) const fn fixed_tls_peer(&self) -> Option<&PmFixedTlsPeerSelection> {
        self.fixed_tls_peer.as_ref()
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
            selected_local_egress: None,
            fixed_tls_peer: None,
        })
    }

    pub(crate) fn production_on_selected_local_egress(
        origin: &str,
        connect_timeout: Duration,
        request_timeout: Duration,
        selected_local_egress: PmLocalEgressSelection,
    ) -> Result<Self, PmLiveAdapterError> {
        require_production_local_egress(&selected_local_egress)?;
        let mut config = Self::production(origin, connect_timeout, request_timeout)?;
        config.selected_local_egress = Some(selected_local_egress);
        Ok(config)
    }

    pub(crate) fn production_on_fixed_tls_peer_and_selected_local_egress(
        connect_timeout: Duration,
        request_timeout: Duration,
        fixed_tls_peer: PmFixedTlsPeerSelection,
        selected_local_egress: PmLocalEgressSelection,
    ) -> Result<Self, PmLiveAdapterError> {
        require_production_fixed_tls_peer(&fixed_tls_peer, "clob.polymarket.com")?;
        require_production_local_egress(&selected_local_egress)?;
        require_same_address_family(&fixed_tls_peer, &selected_local_egress)?;
        let mut config =
            Self::production(PM_CLOB_PRODUCTION_ORIGIN, connect_timeout, request_timeout)?;
        config.fixed_tls_peer = Some(fixed_tls_peer);
        config.selected_local_egress = Some(selected_local_egress);
        Ok(config)
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
            selected_local_egress: None,
            fixed_tls_peer: None,
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

    #[cfg(test)]
    pub(crate) fn local_evidence_on_selected_local_egress(
        origin: &str,
        connect_timeout: Duration,
        request_timeout: Duration,
        selected_local_egress: PmLocalEgressSelection,
    ) -> Result<Self, PmLiveAdapterError> {
        require_loopback_local_egress(&selected_local_egress)?;
        let mut config = Self::local_evidence(origin, connect_timeout, request_timeout)?;
        config.selected_local_egress = Some(selected_local_egress);
        Ok(config)
    }

    #[cfg(test)]
    pub(crate) fn loopback_evidence_on_fixed_tls_peer_and_selected_local_egress(
        connect_timeout: Duration,
        request_timeout: Duration,
        fixed_tls_peer: PmFixedTlsPeerSelection,
        selected_local_egress: PmLocalEgressSelection,
    ) -> Result<Self, PmLiveAdapterError> {
        require_loopback_fixed_tls_peer(&fixed_tls_peer)?;
        require_loopback_local_egress(&selected_local_egress)?;
        require_same_address_family(&fixed_tls_peer, &selected_local_egress)?;
        validate_timeouts(connect_timeout, request_timeout)?;
        Ok(Self {
            origin: fixed_tls_peer_loopback_origin(&fixed_tls_peer)?,
            connect_timeout,
            request_timeout,
            mode: OriginMode::LocalEvidence,
            selected_local_egress: Some(selected_local_egress),
            fixed_tls_peer: Some(fixed_tls_peer),
        })
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

    pub(crate) const fn selected_local_egress(&self) -> Option<&PmLocalEgressSelection> {
        self.selected_local_egress.as_ref()
    }

    pub(crate) const fn fixed_tls_peer(&self) -> Option<&PmFixedTlsPeerSelection> {
        self.fixed_tls_peer.as_ref()
    }
}

fn require_production_fixed_tls_peer(
    fixed_tls_peer: &PmFixedTlsPeerSelection,
    exact_dns_name: &'static str,
) -> Result<(), PmLiveAdapterError> {
    fixed_tls_peer.require_production().map_err(|_| {
        PmLiveAdapterError::InvalidConfiguration(
            "production HTTP requires a production fixed TLS peer",
        )
    })?;
    if fixed_tls_peer.dns_name() != exact_dns_name || fixed_tls_peer.peer_addr().port() != 443 {
        return Err(PmLiveAdapterError::InvalidConfiguration(
            "fixed TLS peer DNS identity does not match the fixed production role",
        ));
    }
    Ok(())
}

fn require_same_address_family(
    fixed_tls_peer: &PmFixedTlsPeerSelection,
    selected_local_egress: &PmLocalEgressSelection,
) -> Result<(), PmLiveAdapterError> {
    fixed_tls_peer
        .require_same_address_family(selected_local_egress)
        .map_err(|_| {
            PmLiveAdapterError::InvalidConfiguration(
                "fixed TLS peer and selected local egress require one address family",
            )
        })
}

#[cfg(test)]
fn require_loopback_fixed_tls_peer(
    fixed_tls_peer: &PmFixedTlsPeerSelection,
) -> Result<(), PmLiveAdapterError> {
    fixed_tls_peer.require_loopback_evidence().map_err(|_| {
        PmLiveAdapterError::InvalidConfiguration(
            "loopback HTTP evidence requires a loopback fixed TLS peer",
        )
    })
}

#[cfg(test)]
fn fixed_tls_peer_loopback_origin(
    fixed_tls_peer: &PmFixedTlsPeerSelection,
) -> Result<Url, PmLiveAdapterError> {
    let origin = format!(
        "http://{}:{}",
        fixed_tls_peer.dns_name(),
        fixed_tls_peer.peer_addr().port()
    );
    let url = validate_base_origin(&origin)?;
    if url.scheme() != "http"
        || url.host_str() != Some(fixed_tls_peer.dns_name())
        || url.port() != Some(fixed_tls_peer.peer_addr().port())
    {
        return Err(PmLiveAdapterError::InvalidConfiguration(
            "loopback fixed-peer origin does not match its DNS identity and TCP port",
        ));
    }
    Ok(url)
}

fn require_production_local_egress(
    selected_local_egress: &PmLocalEgressSelection,
) -> Result<(), PmLiveAdapterError> {
    selected_local_egress.require_production().map_err(|_| {
        PmLiveAdapterError::InvalidConfiguration(
            "production HTTP requires a production local-egress selection",
        )
    })
}

#[cfg(test)]
fn require_loopback_local_egress(
    selected_local_egress: &PmLocalEgressSelection,
) -> Result<(), PmLiveAdapterError> {
    selected_local_egress
        .require_loopback_evidence()
        .map_err(|_| {
            PmLiveAdapterError::InvalidConfiguration(
                "loopback HTTP evidence requires a loopback local-egress selection",
            )
        })
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
    fn status_production_origin_is_fixed_and_not_caller_selectable() {
        let config =
            PmStatusHttpConfig::production(CONNECT, REQUEST).expect("official status origin");
        assert_eq!(config.origin().as_str(), "https://status.polymarket.com/");
        assert_eq!(config.mode(), OriginMode::Production);
    }

    #[test]
    fn selected_production_configs_retain_only_the_validated_local_selection() {
        let selection = PmLocalEgressSelection::production(
            "pm-tunnel0",
            "192.0.2.10".parse().expect("test IP"),
        )
        .expect("valid local selection");
        let public = PmPublicHttpConfig::production_on_selected_local_egress(
            PM_CLOB_PRODUCTION_ORIGIN,
            CONNECT,
            REQUEST,
            selection.clone(),
        )
        .expect("selected public config");
        let geoblock = PmGeoblockHttpConfig::production_on_selected_local_egress(
            CONNECT,
            REQUEST,
            selection.clone(),
        )
        .expect("selected geoblock config");
        let status = PmStatusHttpConfig::production_on_selected_local_egress(
            CONNECT,
            REQUEST,
            selection.clone(),
        )
        .expect("selected status config");

        for retained in [
            public.selected_local_egress(),
            geoblock.selected_local_egress(),
            status.selected_local_egress(),
        ] {
            let retained = retained.expect("selected config retains binding");
            assert_eq!(retained.interface_name(), "pm-tunnel0");
            assert_eq!(retained.local_source_ip(), selection.local_source_ip());
            assert!(!retained.production_order_entry_authorized());
        }
        assert!(
            PmPublicHttpConfig::production(PM_CLOB_PRODUCTION_ORIGIN, CONNECT, REQUEST)
                .unwrap()
                .selected_local_egress()
                .is_none()
        );

        let loopback = PmLocalEgressSelection::loopback_evidence(
            "lo",
            "127.0.0.2".parse().expect("test loopback IP"),
        )
        .expect("loopback selection");
        let local_geoblock = PmGeoblockHttpConfig::local_evidence_on_selected_local_egress(
            "http://127.0.0.1:18080",
            CONNECT,
            REQUEST,
            loopback.clone(),
        )
        .expect("selected loopback geoblock config");
        let local_status = PmStatusHttpConfig::local_evidence_on_selected_local_egress(
            "http://127.0.0.1:18080",
            CONNECT,
            REQUEST,
            loopback.clone(),
        )
        .expect("selected loopback status config");
        assert_eq!(
            local_geoblock
                .selected_local_egress()
                .unwrap()
                .local_source_ip(),
            "127.0.0.2".parse::<IpAddr>().unwrap()
        );
        assert_eq!(
            local_status
                .selected_local_egress()
                .unwrap()
                .local_source_ip(),
            "127.0.0.2".parse::<IpAddr>().unwrap()
        );
        assert!(
            PmPublicHttpConfig::production_on_selected_local_egress(
                PM_CLOB_PRODUCTION_ORIGIN,
                CONNECT,
                REQUEST,
                loopback.clone(),
            )
            .is_err()
        );
        assert!(
            PmGeoblockHttpConfig::production_on_selected_local_egress(
                CONNECT,
                REQUEST,
                loopback.clone(),
            )
            .is_err()
        );
        assert!(
            PmStatusHttpConfig::production_on_selected_local_egress(CONNECT, REQUEST, loopback,)
                .is_err()
        );
        assert!(
            PmPublicHttpConfig::local_evidence_on_selected_local_egress(
                "http://127.0.0.1:18080",
                CONNECT,
                REQUEST,
                selection,
            )
            .is_err()
        );
    }

    #[test]
    fn fixed_tls_peer_configs_bind_exact_role_host_and_reject_mode_crossing() {
        let production_local =
            PmLocalEgressSelection::production("pm-tunnel0", "192.0.2.10".parse().unwrap())
                .unwrap();
        let clob_peer =
            PmFixedTlsPeerSelection::production("clob.polymarket.com", "8.8.8.8").unwrap();
        let geoblock_peer =
            PmFixedTlsPeerSelection::production("polymarket.com", "8.8.4.4").unwrap();
        let status_peer =
            PmFixedTlsPeerSelection::production("status.polymarket.com", "1.1.1.1").unwrap();

        let public = PmPublicHttpConfig::production_on_fixed_tls_peer_and_selected_local_egress(
            CONNECT,
            REQUEST,
            clob_peer.clone(),
            production_local.clone(),
        )
        .unwrap();
        let geoblock =
            PmGeoblockHttpConfig::production_on_fixed_tls_peer_and_selected_local_egress(
                CONNECT,
                REQUEST,
                geoblock_peer.clone(),
                production_local.clone(),
            )
            .unwrap();
        let status = PmStatusHttpConfig::production_on_fixed_tls_peer_and_selected_local_egress(
            CONNECT,
            REQUEST,
            status_peer.clone(),
            production_local.clone(),
        )
        .unwrap();
        for (config_peer, expected_name) in [
            (public.fixed_tls_peer(), "clob.polymarket.com"),
            (geoblock.fixed_tls_peer(), "polymarket.com"),
            (status.fixed_tls_peer(), "status.polymarket.com"),
        ] {
            let config_peer = config_peer.unwrap();
            assert_eq!(config_peer.dns_name(), expected_name);
            assert_eq!(config_peer.peer_addr().port(), 443);
            assert!(!config_peer.production_order_entry_authorized());
        }
        assert!(
            PmPublicHttpConfig::production_on_fixed_tls_peer_and_selected_local_egress(
                CONNECT,
                REQUEST,
                geoblock_peer,
                production_local.clone(),
            )
            .is_err()
        );

        let loopback_local =
            PmLocalEgressSelection::loopback_evidence("lo", "127.0.0.2".parse().unwrap()).unwrap();
        let loopback_peer = PmFixedTlsPeerSelection::loopback_evidence(
            "clob.polymarket.test",
            "127.0.0.1:18080".parse().unwrap(),
        )
        .unwrap();
        let loopback =
            PmPublicHttpConfig::loopback_evidence_on_fixed_tls_peer_and_selected_local_egress(
                CONNECT,
                REQUEST,
                loopback_peer.clone(),
                loopback_local.clone(),
            )
            .unwrap();
        assert_eq!(
            loopback.origin().as_str(),
            "http://clob.polymarket.test:18080/"
        );
        assert_eq!(
            loopback.fixed_tls_peer().unwrap().peer_addr(),
            "127.0.0.1:18080".parse().unwrap()
        );
        let loopback_geoblock =
            PmGeoblockHttpConfig::loopback_evidence_on_fixed_tls_peer_and_selected_local_egress(
                CONNECT,
                REQUEST,
                PmFixedTlsPeerSelection::loopback_evidence(
                    "geoblock-source.test",
                    "127.0.0.1:18081".parse().unwrap(),
                )
                .unwrap(),
                loopback_local.clone(),
            )
            .unwrap();
        let loopback_status =
            PmStatusHttpConfig::loopback_evidence_on_fixed_tls_peer_and_selected_local_egress(
                CONNECT,
                REQUEST,
                PmFixedTlsPeerSelection::loopback_evidence(
                    "status-source.test",
                    "127.0.0.1:18082".parse().unwrap(),
                )
                .unwrap(),
                loopback_local.clone(),
            )
            .unwrap();
        assert_eq!(
            loopback_geoblock.origin().as_str(),
            "http://geoblock-source.test:18081/"
        );
        assert_eq!(
            loopback_status.origin().as_str(),
            "http://status-source.test:18082/"
        );
        assert!(
            PmPublicHttpConfig::production_on_fixed_tls_peer_and_selected_local_egress(
                CONNECT,
                REQUEST,
                loopback_peer,
                production_local.clone(),
            )
            .is_err()
        );
        assert!(
            PmPublicHttpConfig::loopback_evidence_on_fixed_tls_peer_and_selected_local_egress(
                CONNECT,
                REQUEST,
                clob_peer.clone(),
                loopback_local.clone(),
            )
            .is_err()
        );
        assert!(
            PmPublicHttpConfig::loopback_evidence_on_fixed_tls_peer_and_selected_local_egress(
                CONNECT,
                REQUEST,
                PmFixedTlsPeerSelection::loopback_evidence(
                    "clob.polymarket.test",
                    "127.0.0.1:18080".parse().unwrap(),
                )
                .unwrap(),
                production_local,
            )
            .is_err()
        );
        assert!(
            PmPublicHttpConfig::loopback_evidence_on_fixed_tls_peer_and_selected_local_egress(
                CONNECT,
                REQUEST,
                PmFixedTlsPeerSelection::loopback_evidence(
                    "clob.polymarket.test",
                    "[::1]:18080".parse().unwrap(),
                )
                .unwrap(),
                loopback_local,
            )
            .is_err()
        );
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
        assert!(
            PmStatusHttpConfig::local_evidence("http://127.0.0.1:18080", CONNECT, REQUEST,).is_ok()
        );
        for invalid in [
            "http://localhost:18080",
            "http://192.0.2.1:18080",
            "https://127.0.0.1:18080",
            "http://127.0.0.1",
        ] {
            assert!(PmGeoblockHttpConfig::read_only_evidence(invalid, CONNECT, REQUEST).is_err());
            assert!(PmStatusHttpConfig::local_evidence(invalid, CONNECT, REQUEST).is_err());
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
