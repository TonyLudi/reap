//! Deferred, same-domain mutation-time construction for a selected actor.
//!
//! The staged owner releases the existing observation-only role bundle and
//! one opaque clock capsule derived from the same product-clock domain. It
//! constructs no place/cancel HTTP role. The capsule retains the exact staged
//! CLOB HTTPS configuration and scope; the later promotion owner consumes it
//! without accepting replacement routing inputs and constructs exactly one
//! place-time owner and one cancel-time owner.
//!
//! Every value here remains non-authoritative. Fixed peer/local selections do
//! not prove a connected socket, route, namespace, NAT identity, actor
//! generation, authorization window, or permission to enter an order. Exact
//! permit custody and the decision to call promotion belong to the private
//! runner actor. The compatible full connectivity owner remains available to
//! existing callers and is intentionally not changed by this additive path.

use std::{fmt, marker::PhantomData, rc::Rc, time::Duration};

use reap_polymarket_egress_binding::{PmFixedTlsPeerSelection, PmLocalEgressSelection};
use reap_polymarket_wire::{PmBookParserConfig, PmWireScope};

use crate::{
    PmCancelMutationTimeOwner, PmLiveAdapterError, PmPlaceMutationTimeOwner, PmProductClockOwner,
    PmPublicHttpConfig, PmPublicWsConfig,
    product_clock::PmDeferredMutationClockDomain,
    public_connectivity::{
        PmPublicObservationConnectivityOwner, PmPublicObservationConnectivityRoles,
    },
};

// BEGIN SELECTED_DEFERRED_MUTATION_CLOCK_CAPSULE
/// Opaque, move-only custody of one deferred product-clock domain and the
/// exact selected CLOB HTTP configuration and scope already admitted by the
/// observation staging owner.
///
/// The capsule has no public decomposition, conversion, sampling, proof,
/// finalizer, HTTP, credential, request, or mutation API. Its private `Rc`
/// marker keeps it on the selected actor thread. It remains non-authoritative:
/// runner-private permit and actor-generation custody must independently gate
/// its one consuming promotion.
pub struct PmDeferredMutationClockCapsule {
    clock_domain: PmDeferredMutationClockDomain,
    http_config: PmPublicHttpConfig,
    scope: PmWireScope,
    _actor_local: PhantomData<Rc<()>>,
}

impl fmt::Debug for PmDeferredMutationClockCapsule {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(
            "PmDeferredMutationClockCapsule(<non-authoritative; selected-route-scope-and-domain redacted>)",
        )
    }
}
// END SELECTED_DEFERRED_MUTATION_CLOCK_CAPSULE

// BEGIN DEFERRED_MUTATION_OBSERVATION_STAGING
/// Cold selected-observation construction plus opaque future clock custody.
///
/// Splitting this owner releases the exact existing observation role bundle;
/// the companion capsule has no sampling, proof, finalizer, HTTP, credential,
/// request, or mutation API.
pub struct PmPublicObservationWithDeferredMutationClockOwner {
    observation: PmPublicObservationConnectivityOwner,
    deferred_mutation_clock: PmDeferredMutationClockCapsule,
}

impl PmPublicObservationWithDeferredMutationClockOwner {
    /// Construct only selected production observation roles and retain the
    /// same product-clock domain for a later actor-private promotion.
    #[allow(clippy::too_many_arguments)]
    pub fn production_on_fixed_tls_peer_and_selected_local_egress(
        connect_timeout: Duration,
        request_timeout: Duration,
        parser_config: PmBookParserConfig,
        public_ws_config: PmPublicWsConfig,
        fixed_clob_tls_peer: PmFixedTlsPeerSelection,
        selected_local_egress: PmLocalEgressSelection,
        clock_owner: PmProductClockOwner,
    ) -> Result<Self, PmLiveAdapterError> {
        if !public_ws_config.is_production() {
            return Err(PmLiveAdapterError::InvalidConfiguration(
                "deferred selected production observation connectivity requires a production public WebSocket configuration",
            ));
        }
        if parser_config.scope() != public_ws_config.scope() {
            return Err(PmLiveAdapterError::InvalidConfiguration(
                "public HTTP and WebSocket roles must bind one exact wire scope",
            ));
        }
        let http_config =
            PmPublicHttpConfig::production_on_fixed_tls_peer_and_selected_local_egress(
                connect_timeout,
                request_timeout,
                fixed_clob_tls_peer,
                selected_local_egress,
            )?;
        let deferred_http_config = http_config.clone();
        let configured_scope = parser_config.scope();
        let deferred_clock_views = clock_owner.split_observation_with_deferred_mutation();
        let (clock_views, clock_domain) = deferred_clock_views.into_parts();
        let observation = PmPublicObservationConnectivityOwner::from_observation_clock_views(
            http_config,
            parser_config,
            public_ws_config,
            clock_views,
        )?;
        let deferred_mutation_clock = PmDeferredMutationClockCapsule {
            clock_domain,
            http_config: deferred_http_config,
            scope: configured_scope,
            _actor_local: PhantomData,
        };
        Ok(Self {
            observation,
            deferred_mutation_clock,
        })
    }

    #[must_use]
    pub const fn configured_scope(&self) -> PmWireScope {
        self.observation.configured_scope()
    }

    /// Release observation roles immediately and retain only the sealed clock
    /// capsule for later private actor custody.
    #[must_use]
    pub fn into_parts(
        self,
    ) -> (
        PmPublicObservationConnectivityRoles,
        PmDeferredMutationClockCapsule,
    ) {
        (self.observation.into_roles(), self.deferred_mutation_clock)
    }

    /// Deferred clock custody and selected observation never authorize order
    /// entry.
    #[must_use]
    pub const fn production_order_entry_authorized(&self) -> bool {
        false
    }
}

impl fmt::Debug for PmPublicObservationWithDeferredMutationClockOwner {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(
            "PmPublicObservationWithDeferredMutationClockOwner(<non-authoritative; selected-observation-and-opaque-clock>)",
        )
    }
}
// END DEFERRED_MUTATION_OBSERVATION_STAGING

// BEGIN DEFERRED_MUTATION_SELECTED_PROMOTION
/// Cold, move-only result of consuming one deferred clock capsule into exact
/// fixed-peer place and cancel server-time owners.
///
/// This owner has no request-authentication or mutation transport API. The
/// caller must keep it private until an exact live permit has independently
/// authorized a future purpose-specific consuming bridge. It deliberately has
/// no public split into ordinary movable time owners, so its retained scope
/// and actor-local confinement cannot be silently erased.
pub struct PmProductionSelectedPlaceCancelTimeOwner {
    #[allow(
        dead_code,
        reason = "sealed until a future purpose-specific runner-gated bridge consumes these owners"
    )]
    place: PmPlaceMutationTimeOwner,
    #[allow(
        dead_code,
        reason = "sealed until a future purpose-specific runner-gated bridge consumes these owners"
    )]
    cancel: PmCancelMutationTimeOwner,
    scope: PmWireScope,
    _actor_local: PhantomData<Rc<()>>,
}

impl PmProductionSelectedPlaceCancelTimeOwner {
    /// Consume one deferred clock domain and its already-validated exact
    /// production CLOB HTTPS configuration. Construction is synchronous,
    /// accepts no replacement routing input, and performs no source request.
    pub fn from_deferred_clock(
        deferred_clock: PmDeferredMutationClockCapsule,
    ) -> Result<Self, PmLiveAdapterError> {
        let PmDeferredMutationClockCapsule {
            clock_domain,
            http_config,
            scope,
            _actor_local: _,
        } = deferred_clock;
        let (
            place_server_time_clock,
            place_mutation_time_finalizer,
            cancel_server_time_clock,
            cancel_mutation_time_finalizer,
        ) = clock_domain.into_purpose_closed_views().into_views();
        let place = PmPlaceMutationTimeOwner::with_product_clock(
            http_config.clone(),
            place_server_time_clock,
            place_mutation_time_finalizer,
        )?;
        let cancel = PmCancelMutationTimeOwner::with_product_clock(
            http_config,
            cancel_server_time_clock,
            cancel_mutation_time_finalizer,
        )?;
        Ok(Self {
            place,
            cancel,
            scope,
            _actor_local: PhantomData,
        })
    }

    #[must_use]
    pub const fn configured_scope(&self) -> PmWireScope {
        self.scope
    }

    /// Test-only inspection of atomic purpose-owner construction. Production
    /// code has no split until a future sealed actor bridge is introduced.
    #[cfg(test)]
    #[must_use]
    pub(crate) fn into_purpose_owners(
        self,
    ) -> (PmPlaceMutationTimeOwner, PmCancelMutationTimeOwner) {
        (self.place, self.cancel)
    }

    /// Mutation-time HTTP sources alone never authorize production order
    /// entry.
    #[must_use]
    pub const fn production_order_entry_authorized(&self) -> bool {
        false
    }
}

impl fmt::Debug for PmProductionSelectedPlaceCancelTimeOwner {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(
            "PmProductionSelectedPlaceCancelTimeOwner(<non-authoritative; fixed-place-cancel-time>)",
        )
    }
}
// END DEFERRED_MUTATION_SELECTED_PROMOTION

#[cfg(test)]
mod tests {
    use reap_pm_core::{
        ConnectionEpoch, PmConditionId, PmMarketId, PmQuantity, PmTick, PmTokenId, U256,
    };

    use super::*;
    use crate::PmPublicWsBounds;

    fn scope(seed: u8) -> PmWireScope {
        PmWireScope::new(
            PmConditionId::from_bytes([seed; 32]).unwrap(),
            PmMarketId::from_bytes([seed.wrapping_add(1); 32]).unwrap(),
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

    fn production_ws(scope: PmWireScope) -> PmPublicWsConfig {
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

    fn clob_peer() -> PmFixedTlsPeerSelection {
        PmFixedTlsPeerSelection::production("clob.polymarket.com", "8.8.8.8").unwrap()
    }

    fn selected_local() -> PmLocalEgressSelection {
        PmLocalEgressSelection::production("pm-tunnel0", "192.0.2.10".parse().unwrap()).unwrap()
    }

    #[test]
    fn selected_staging_releases_only_observation_roles_and_one_opaque_capsule() {
        let exact_scope = scope(0x11);
        let staged = PmPublicObservationWithDeferredMutationClockOwner::
            production_on_fixed_tls_peer_and_selected_local_egress(
                Duration::from_secs(1),
                Duration::from_secs(1),
                parser(exact_scope),
                production_ws(exact_scope),
                clob_peer(),
                selected_local(),
                PmProductClockOwner::test_support_scripted(&[(1_000, 10)]).unwrap(),
            )
            .unwrap();
        assert_eq!(staged.configured_scope(), exact_scope);
        assert!(!staged.production_order_entry_authorized());
        assert_eq!(
            format!("{staged:?}"),
            "PmPublicObservationWithDeferredMutationClockOwner(<non-authoritative; selected-observation-and-opaque-clock>)"
        );

        let (observation, deferred) = staged.into_parts();
        let (metadata, book, _, _, public_ws, _, _, _) = observation.into_roles();
        assert_eq!(metadata.configured_scope(), exact_scope);
        assert_eq!(book.parser_config().scope(), exact_scope);
        assert_eq!(public_ws.scope(), exact_scope);
        assert_eq!(
            format!("{deferred:?}"),
            "PmDeferredMutationClockCapsule(<non-authoritative; selected-route-scope-and-domain redacted>)"
        );

        let promoted =
            PmProductionSelectedPlaceCancelTimeOwner::from_deferred_clock(deferred).unwrap();
        assert!(!promoted.production_order_entry_authorized());
        assert_eq!(promoted.configured_scope(), exact_scope);
        assert_eq!(
            format!("{promoted:?}"),
            "PmProductionSelectedPlaceCancelTimeOwner(<non-authoritative; fixed-place-cancel-time>)"
        );
        let (place, cancel) = promoted.into_purpose_owners();
        assert!(format!("{place:?}").contains("fixed-place-time"));
        assert!(format!("{cancel:?}").contains("fixed-cancel-time"));
    }

    #[test]
    fn selected_staging_validates_ws_and_scope_before_fixed_http_construction() {
        let exact_scope = scope(0x11);
        let wrong_host =
            PmFixedTlsPeerSelection::production("status.polymarket.com", "8.8.8.8").unwrap();
        assert!(matches!(
            PmPublicObservationWithDeferredMutationClockOwner::
                production_on_fixed_tls_peer_and_selected_local_egress(
                    Duration::from_secs(1),
                    Duration::from_secs(1),
                    parser(exact_scope),
                    loopback_ws(exact_scope),
                    wrong_host.clone(),
                    selected_local(),
                    PmProductClockOwner::system(),
                ),
            Err(PmLiveAdapterError::InvalidConfiguration(
                "deferred selected production observation connectivity requires a production public WebSocket configuration"
            ))
        ));
        assert!(matches!(
            PmPublicObservationWithDeferredMutationClockOwner::
                production_on_fixed_tls_peer_and_selected_local_egress(
                    Duration::from_secs(1),
                    Duration::from_secs(1),
                    parser(exact_scope),
                    production_ws(scope(0x22)),
                    wrong_host.clone(),
                    selected_local(),
                    PmProductClockOwner::system(),
                ),
            Err(PmLiveAdapterError::InvalidConfiguration(
                "public HTTP and WebSocket roles must bind one exact wire scope"
            ))
        ));
        assert!(matches!(
            PmPublicObservationWithDeferredMutationClockOwner::
                production_on_fixed_tls_peer_and_selected_local_egress(
                    Duration::from_secs(1),
                    Duration::from_secs(1),
                    parser(exact_scope),
                    production_ws(exact_scope),
                    wrong_host,
                    selected_local(),
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
            PmPublicObservationWithDeferredMutationClockOwner::
                production_on_fixed_tls_peer_and_selected_local_egress(
                    Duration::from_secs(1),
                    Duration::from_secs(1),
                    parser(exact_scope),
                    production_ws(exact_scope),
                    ipv6_peer,
                    selected_local(),
                    PmProductClockOwner::system(),
                ),
            Err(PmLiveAdapterError::InvalidConfiguration(
                "fixed TLS peer and selected local egress require one address family"
            ))
        ));
    }
}
