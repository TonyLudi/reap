use std::fmt;

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
