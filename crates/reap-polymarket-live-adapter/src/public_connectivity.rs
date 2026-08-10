use std::{fmt, time::Duration};

use reap_polymarket_egress_binding::{PmFixedTlsPeerSelection, PmLocalEgressSelection};
use reap_polymarket_wire::PmBookParserConfig;

use crate::{
    PmActorProductClock, PmCancelMutationTimeOwner, PmLiveAdapterError, PmOkxProductClock,
    PmPlaceMutationTimeOwner, PmPrivateReadProductClock, PmProductClockOwner, PmPublicHttpConfig,
    PmPublicHttpRole, PmPublicMarketWsRole, PmPublicMetadataHttpRole, PmPublicWsConfig,
    PmReadServerTimeHttpRole, PmUserWsProductClock,
};

/// One cold, move-only constructor for all public connectivity and every
/// receive-clock view used by an authenticated product.
///
/// The owner accepts one HTTP origin/configuration, one exact condition-bound
/// parser scope, one matching public-WS configuration, and one clock domain.
/// Independent pre-clocked roles therefore cannot enter the authenticated
/// composition root.
pub struct PmPublicConnectivityOwner {
    metadata_http: PmPublicMetadataHttpRole,
    book_http: PmPublicHttpRole,
    read_server_time_http: PmReadServerTimeHttpRole,
    private_read_clock: PmPrivateReadProductClock,
    place_mutation_time: PmPlaceMutationTimeOwner,
    cancel_mutation_time: PmCancelMutationTimeOwner,
    public_ws: PmPublicMarketWsRole,
    user_ws_clock: PmUserWsProductClock,
    actor_clock: PmActorProductClock,
    okx_clock: PmOkxProductClock,
}

impl PmPublicConnectivityOwner {
    pub fn new(
        http_config: PmPublicHttpConfig,
        parser_config: PmBookParserConfig,
        public_ws_config: PmPublicWsConfig,
        clock_owner: PmProductClockOwner,
    ) -> Result<Self, PmLiveAdapterError> {
        if parser_config.scope() != public_ws_config.scope() {
            return Err(PmLiveAdapterError::InvalidConfiguration(
                "public HTTP and WebSocket roles must bind one exact wire scope",
            ));
        }
        let (
            public_ws_clock,
            user_ws_clock,
            public_http_clock,
            read_server_time_clock,
            private_read_clock,
            place_server_time_clock,
            place_mutation_time_finalizer,
            cancel_server_time_clock,
            cancel_mutation_time_finalizer,
            actor_clock,
            okx_clock,
        ) = clock_owner.split().into_views();
        let metadata_http =
            PmPublicMetadataHttpRole::new(http_config.clone(), parser_config.scope())?;
        let book_http = PmPublicHttpRole::with_product_clock(
            http_config.clone(),
            parser_config,
            public_http_clock,
        )?;
        let read_server_time_http = PmReadServerTimeHttpRole::with_product_clock(
            http_config.clone(),
            read_server_time_clock,
        )?;
        let place_mutation_time = PmPlaceMutationTimeOwner::with_product_clock(
            http_config.clone(),
            place_server_time_clock,
            place_mutation_time_finalizer,
        )?;
        let cancel_mutation_time = PmCancelMutationTimeOwner::with_product_clock(
            http_config,
            cancel_server_time_clock,
            cancel_mutation_time_finalizer,
        )?;
        let public_ws = PmPublicMarketWsRole::with_clock_source(public_ws_config, public_ws_clock)?;
        Ok(Self {
            metadata_http,
            book_http,
            read_server_time_http,
            private_read_clock,
            place_mutation_time,
            cancel_mutation_time,
            public_ws,
            user_ws_clock,
            actor_clock,
            okx_clock,
        })
    }

    #[must_use]
    pub const fn configured_scope(&self) -> reap_polymarket_wire::PmWireScope {
        self.book_http.parser_config().scope()
    }

    #[must_use]
    pub fn into_roles(self) -> PmPublicConnectivityRoles {
        PmPublicConnectivityRoles {
            metadata_http: self.metadata_http,
            book_http: self.book_http,
            read_server_time_http: self.read_server_time_http,
            private_read_clock: self.private_read_clock,
            place_mutation_time: self.place_mutation_time,
            cancel_mutation_time: self.cancel_mutation_time,
            public_ws: self.public_ws,
            user_ws_clock: self.user_ws_clock,
            actor_clock: self.actor_clock,
            okx_clock: self.okx_clock,
        }
    }
}

impl fmt::Debug for PmPublicConnectivityOwner {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PmPublicConnectivityOwner(<scope-and-clock-bound>)")
    }
}

pub struct PmPublicConnectivityRoles {
    metadata_http: PmPublicMetadataHttpRole,
    book_http: PmPublicHttpRole,
    read_server_time_http: PmReadServerTimeHttpRole,
    private_read_clock: PmPrivateReadProductClock,
    place_mutation_time: PmPlaceMutationTimeOwner,
    cancel_mutation_time: PmCancelMutationTimeOwner,
    public_ws: PmPublicMarketWsRole,
    user_ws_clock: PmUserWsProductClock,
    actor_clock: PmActorProductClock,
    okx_clock: PmOkxProductClock,
}

impl PmPublicConnectivityRoles {
    #[must_use]
    pub fn into_roles(
        self,
    ) -> (
        PmPublicMetadataHttpRole,
        PmPublicHttpRole,
        PmReadServerTimeHttpRole,
        PmPrivateReadProductClock,
        PmPlaceMutationTimeOwner,
        PmCancelMutationTimeOwner,
        PmPublicMarketWsRole,
        PmUserWsProductClock,
        PmActorProductClock,
        PmOkxProductClock,
    ) {
        (
            self.metadata_http,
            self.book_http,
            self.read_server_time_http,
            self.private_read_clock,
            self.place_mutation_time,
            self.cancel_mutation_time,
            self.public_ws,
            self.user_ws_clock,
            self.actor_clock,
            self.okx_clock,
        )
    }
}

impl fmt::Debug for PmPublicConnectivityRoles {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PmPublicConnectivityRoles(<move-only>)")
    }
}

// BEGIN OBSERVATION_ONLY_PUBLIC_CONNECTIVITY
/// One cold, move-only constructor for the public and authenticated-read
/// observation roles that share an exact product scope and clock domain.
pub struct PmPublicObservationConnectivityOwner {
    metadata_http: PmPublicMetadataHttpRole,
    book_http: PmPublicHttpRole,
    read_server_time_http: PmReadServerTimeHttpRole,
    private_read_clock: PmPrivateReadProductClock,
    public_ws: PmPublicMarketWsRole,
    user_ws_clock: PmUserWsProductClock,
    actor_clock: PmActorProductClock,
    okx_clock: PmOkxProductClock,
}

impl PmPublicObservationConnectivityOwner {
    pub fn new(
        http_config: PmPublicHttpConfig,
        parser_config: PmBookParserConfig,
        public_ws_config: PmPublicWsConfig,
        clock_owner: PmProductClockOwner,
    ) -> Result<Self, PmLiveAdapterError> {
        if parser_config.scope() != public_ws_config.scope() {
            return Err(PmLiveAdapterError::InvalidConfiguration(
                "public HTTP and WebSocket roles must bind one exact wire scope",
            ));
        }
        let (
            public_ws_clock,
            user_ws_clock,
            public_http_clock,
            read_server_time_clock,
            private_read_clock,
            actor_clock,
            okx_clock,
        ) = clock_owner.split_observation_only().into_views();
        let metadata_http =
            PmPublicMetadataHttpRole::new(http_config.clone(), parser_config.scope())?;
        let book_http = PmPublicHttpRole::with_product_clock(
            http_config.clone(),
            parser_config,
            public_http_clock,
        )?;
        let read_server_time_http =
            PmReadServerTimeHttpRole::with_product_clock(http_config, read_server_time_clock)?;
        let public_ws = PmPublicMarketWsRole::with_clock_source(public_ws_config, public_ws_clock)?;
        Ok(Self {
            metadata_http,
            book_http,
            read_server_time_http,
            private_read_clock,
            public_ws,
            user_ws_clock,
            actor_clock,
            okx_clock,
        })
    }

    /// Construct the same observation-only role owner while fixing every
    /// public HTTP role to one caller-provided fixed TLS peer and selected
    /// local egress. Both values remain non-authoritative configuration.
    #[allow(clippy::too_many_arguments)]
    pub fn production_on_fixed_tls_peer_and_selected_local_egress(
        connect_timeout: Duration,
        request_timeout: Duration,
        parser_config: PmBookParserConfig,
        public_ws_config: PmPublicWsConfig,
        fixed_tls_peer: PmFixedTlsPeerSelection,
        selected_local_egress: PmLocalEgressSelection,
        clock_owner: PmProductClockOwner,
    ) -> Result<Self, PmLiveAdapterError> {
        if !public_ws_config.is_production() {
            return Err(PmLiveAdapterError::InvalidConfiguration(
                "selected production observation connectivity requires a production public WebSocket configuration",
            ));
        }
        let http_config =
            PmPublicHttpConfig::production_on_fixed_tls_peer_and_selected_local_egress(
                connect_timeout,
                request_timeout,
                fixed_tls_peer,
                selected_local_egress,
            )?;
        Self::new(http_config, parser_config, public_ws_config, clock_owner)
    }

    #[must_use]
    pub const fn configured_scope(&self) -> reap_polymarket_wire::PmWireScope {
        self.book_http.parser_config().scope()
    }

    #[must_use]
    pub fn into_roles(self) -> PmPublicObservationConnectivityRoles {
        PmPublicObservationConnectivityRoles {
            metadata_http: self.metadata_http,
            book_http: self.book_http,
            read_server_time_http: self.read_server_time_http,
            private_read_clock: self.private_read_clock,
            public_ws: self.public_ws,
            user_ws_clock: self.user_ws_clock,
            actor_clock: self.actor_clock,
            okx_clock: self.okx_clock,
        }
    }
}

impl fmt::Debug for PmPublicObservationConnectivityOwner {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(
            "PmPublicObservationConnectivityOwner(<observation-only; scope-and-clock-bound>)",
        )
    }
}

/// Exact move-only roles released by one observation connectivity owner.
pub struct PmPublicObservationConnectivityRoles {
    metadata_http: PmPublicMetadataHttpRole,
    book_http: PmPublicHttpRole,
    read_server_time_http: PmReadServerTimeHttpRole,
    private_read_clock: PmPrivateReadProductClock,
    public_ws: PmPublicMarketWsRole,
    user_ws_clock: PmUserWsProductClock,
    actor_clock: PmActorProductClock,
    okx_clock: PmOkxProductClock,
}

impl PmPublicObservationConnectivityRoles {
    #[must_use]
    pub fn into_roles(
        self,
    ) -> (
        PmPublicMetadataHttpRole,
        PmPublicHttpRole,
        PmReadServerTimeHttpRole,
        PmPrivateReadProductClock,
        PmPublicMarketWsRole,
        PmUserWsProductClock,
        PmActorProductClock,
        PmOkxProductClock,
    ) {
        (
            self.metadata_http,
            self.book_http,
            self.read_server_time_http,
            self.private_read_clock,
            self.public_ws,
            self.user_ws_clock,
            self.actor_clock,
            self.okx_clock,
        )
    }
}

impl fmt::Debug for PmPublicObservationConnectivityRoles {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PmPublicObservationConnectivityRoles(<observation-only; move-only>)")
    }
}
// END OBSERVATION_ONLY_PUBLIC_CONNECTIVITY

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use reap_pm_core::{
        ConnectionEpoch, PmConditionId, PmMarketId, PmQuantity, PmTick, PmTokenId, U256,
    };
    use reap_polymarket_wire::{PmBookParserConfig, PmWireScope};

    use super::*;
    use crate::PmPublicWsBounds;

    fn scope(seed: char) -> PmWireScope {
        PmWireScope::new(
            PmConditionId::parse(&format!("0x{}", seed.to_string().repeat(64))).unwrap(),
            PmMarketId::parse(&format!("0x{}", seed.to_string().repeat(64))).unwrap(),
            PmTokenId::new(U256::from_u64(7)).unwrap(),
        )
    }

    fn parser(scope: PmWireScope) -> PmBookParserConfig {
        PmBookParserConfig::new_condition_bound(
            scope,
            PmTick::parse_decimal("0.01").unwrap(),
            PmQuantity::parse_decimal("1").unwrap(),
            false,
        )
    }

    fn ws(scope: PmWireScope) -> PmPublicWsConfig {
        let bounds = PmPublicWsBounds::new(
            Duration::from_secs(1),
            Duration::from_secs(20),
            Duration::from_secs(1),
            1_024,
            2,
            Duration::from_millis(1),
            8,
            ConnectionEpoch::new(1),
        )
        .unwrap();
        PmPublicWsConfig::production(scope, bounds).unwrap()
    }

    fn loopback_ws(scope: PmWireScope) -> PmPublicWsConfig {
        PmPublicWsConfig::loopback_evidence(
            "ws://127.0.0.1:18080/ws/market",
            scope,
            Duration::from_secs(1),
            Duration::from_secs(20),
            Duration::from_secs(10),
            Duration::from_secs(1),
            1_024,
            2,
            Duration::from_millis(1),
            8,
            ConnectionEpoch::new(1),
        )
        .unwrap()
    }

    #[test]
    fn owner_rejects_independently_scoped_http_and_websocket_roles() {
        let http = PmPublicHttpConfig::production(
            crate::PM_CLOB_PRODUCTION_ORIGIN,
            Duration::from_secs(1),
            Duration::from_secs(1),
        )
        .unwrap();
        assert!(matches!(
            PmPublicConnectivityOwner::new(
                http,
                parser(scope('a')),
                ws(scope('b')),
                PmProductClockOwner::system(),
            ),
            Err(PmLiveAdapterError::InvalidConfiguration(
                "public HTTP and WebSocket roles must bind one exact wire scope"
            ))
        ));
    }

    #[test]
    fn observation_owner_rejects_independently_scoped_roles() {
        let http = PmPublicHttpConfig::production(
            crate::PM_CLOB_PRODUCTION_ORIGIN,
            Duration::from_secs(1),
            Duration::from_secs(1),
        )
        .unwrap();
        assert!(matches!(
            PmPublicObservationConnectivityOwner::new(
                http,
                parser(scope('a')),
                ws(scope('b')),
                PmProductClockOwner::system(),
            ),
            Err(PmLiveAdapterError::InvalidConfiguration(
                "public HTTP and WebSocket roles must bind one exact wire scope"
            ))
        ));
    }

    #[test]
    fn observation_owner_releases_only_the_exact_read_role_set() {
        let exact_scope = scope('a');
        let http = PmPublicHttpConfig::production(
            crate::PM_CLOB_PRODUCTION_ORIGIN,
            Duration::from_secs(1),
            Duration::from_secs(1),
        )
        .unwrap();
        let owner = PmPublicObservationConnectivityOwner::new(
            http,
            parser(exact_scope),
            ws(exact_scope),
            PmProductClockOwner::test_support_scripted(&[(1_700_000_000_000_000_001, 1)]).unwrap(),
        )
        .unwrap();
        assert_eq!(owner.configured_scope(), exact_scope);
        assert_eq!(
            format!("{owner:?}"),
            "PmPublicObservationConnectivityOwner(<observation-only; scope-and-clock-bound>)"
        );
        let roles = owner.into_roles();
        assert_eq!(
            format!("{roles:?}"),
            "PmPublicObservationConnectivityRoles(<observation-only; move-only>)"
        );
        let (
            metadata,
            book,
            _read_time,
            _private_read_clock,
            public_ws,
            _user_ws_clock,
            _actor_clock,
            _okx_clock,
        ) = roles.into_roles();
        assert_eq!(metadata.configured_scope(), exact_scope);
        assert_eq!(book.parser_config().scope(), exact_scope);
        assert_eq!(public_ws.scope(), exact_scope);
    }

    #[test]
    fn selected_observation_owner_delegates_peer_mode_host_and_family_checks() {
        let exact_scope = scope('a');
        let local = PmLocalEgressSelection::production("pm-tunnel0", "192.0.2.10".parse().unwrap())
            .unwrap();
        let clob_peer =
            PmFixedTlsPeerSelection::production("clob.polymarket.com", "8.8.8.8").unwrap();
        let owner = PmPublicObservationConnectivityOwner::
            production_on_fixed_tls_peer_and_selected_local_egress(
                Duration::from_secs(1),
                Duration::from_secs(1),
                parser(exact_scope),
                ws(exact_scope),
                clob_peer,
                local.clone(),
                PmProductClockOwner::system(),
            )
            .unwrap();
        assert_eq!(owner.configured_scope(), exact_scope);

        let wrong_host =
            PmFixedTlsPeerSelection::production("status.polymarket.com", "8.8.8.8").unwrap();
        assert!(matches!(
            PmPublicObservationConnectivityOwner::
                production_on_fixed_tls_peer_and_selected_local_egress(
                    Duration::from_secs(1),
                    Duration::from_secs(1),
                    parser(exact_scope),
                    ws(exact_scope),
                    wrong_host,
                    local.clone(),
                    PmProductClockOwner::system(),
                ),
            Err(PmLiveAdapterError::InvalidConfiguration(
                "fixed TLS peer DNS identity does not match the fixed production role"
            ))
        ));

        let ipv6_peer =
            PmFixedTlsPeerSelection::production("clob.polymarket.com", "2606:4700:4700::1111")
                .unwrap();
        assert!(matches!(
            PmPublicObservationConnectivityOwner::
                production_on_fixed_tls_peer_and_selected_local_egress(
                    Duration::from_secs(1),
                    Duration::from_secs(1),
                    parser(exact_scope),
                    ws(exact_scope),
                    ipv6_peer,
                    local,
                    PmProductClockOwner::system(),
                ),
            Err(PmLiveAdapterError::InvalidConfiguration(
                "fixed TLS peer and selected local egress require one address family"
            ))
        ));

        let production_local =
            PmLocalEgressSelection::production("pm-tunnel0", "192.0.2.10".parse().unwrap())
                .unwrap();
        let production_peer =
            PmFixedTlsPeerSelection::production("clob.polymarket.com", "8.8.8.8").unwrap();
        assert!(matches!(
            PmPublicObservationConnectivityOwner::
                production_on_fixed_tls_peer_and_selected_local_egress(
                    Duration::from_secs(1),
                    Duration::from_secs(1),
                    parser(exact_scope),
                    loopback_ws(exact_scope),
                    production_peer,
                    production_local,
                    PmProductClockOwner::system(),
                ),
            Err(PmLiveAdapterError::InvalidConfiguration(
                "selected production observation connectivity requires a production public WebSocket configuration"
            ))
        ));

        let loopback_peer = PmFixedTlsPeerSelection::loopback_evidence(
            "clob.polymarket.test",
            "127.0.0.1:443".parse().unwrap(),
        )
        .unwrap();
        let loopback_local =
            PmLocalEgressSelection::loopback_evidence("lo", "127.0.0.2".parse().unwrap()).unwrap();
        assert!(matches!(
            PmPublicObservationConnectivityOwner::
                production_on_fixed_tls_peer_and_selected_local_egress(
                    Duration::from_secs(1),
                    Duration::from_secs(1),
                    parser(exact_scope),
                    ws(exact_scope),
                    loopback_peer,
                    loopback_local,
                    PmProductClockOwner::system(),
                ),
            Err(PmLiveAdapterError::InvalidConfiguration(
                "production HTTP requires a production fixed TLS peer"
            ))
        ));
    }

    #[test]
    fn owner_releases_only_one_scope_bound_clocked_role_bundle() {
        let exact_scope = scope('a');
        let http = PmPublicHttpConfig::production(
            crate::PM_CLOB_PRODUCTION_ORIGIN,
            Duration::from_secs(1),
            Duration::from_secs(1),
        )
        .unwrap();
        let owner = PmPublicConnectivityOwner::new(
            http,
            parser(exact_scope),
            ws(exact_scope),
            PmProductClockOwner::test_support_scripted(&[(1_700_000_000_000_000_001, 1)]).unwrap(),
        )
        .unwrap();
        assert_eq!(owner.configured_scope(), exact_scope);
        let (
            metadata,
            book,
            _read_time,
            _private_read_clock,
            place_time,
            cancel_time,
            public_ws,
            _user_clock,
            _actor,
            _okx,
        ) = owner.into_roles().into_roles();
        assert_eq!(metadata.configured_scope(), exact_scope);
        assert_eq!(book.parser_config().scope(), exact_scope);
        assert_eq!(public_ws.scope(), exact_scope);
        let (place_http, place_finalizer) = place_time.into_roles();
        let (cancel_http, cancel_finalizer) = cancel_time.into_roles();
        assert!(format!("{place_http:?}").contains("fixed-GET-/time"));
        assert!(format!("{place_finalizer:?}").contains("place-authority"));
        assert!(format!("{cancel_http:?}").contains("fixed-GET-/time"));
        assert!(format!("{cancel_finalizer:?}").contains("cancel-authority"));
    }
}
