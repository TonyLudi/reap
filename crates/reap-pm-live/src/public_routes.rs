use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};

use reap_okx_public_source::{
    OkxPublicSession, OkxPublicSessionDelivery, OkxPublicSessionEvent, OkxPublicSessionFault,
    OkxPublicUnavailableOccurrence,
};
use reap_pm_core::{
    ConnectionEpoch, EnvelopeError, EventOrdering, IngressSequence, OkxReferenceEvent,
    OkxReferenceEventError, OkxReferenceHandle, OkxReferenceInstrument, OkxReferencePrice,
    OkxReferencePriceError, PmBookEvent, PmConnectionId, PmMarketEvent, PmProductSource,
    PmSourceBound, ReceivedEventClock, ReceivedEventEnvelope,
};
use reap_pm_live_contracts::PmPublicConnectivityConfig;
use reap_polymarket_adapter::{
    PmAuthoritativeMetadata, PmPublicBookDelivery as AdapterPmPublicBookDelivery, PmPublicRole,
    PmPublicSession, PmPublicSessionError, PmPublicSessionFault, PmPublicUnavailableOccurrence,
};
use thiserror::Error;

/// The exact configured public-route authority for one PM product.
///
/// This value is constructed from the already validated capability plan and
/// the two venue-owned public roles. It issues only capability-specific
/// deliveries; it is not itself accepted by a lane and is not a generic route
/// token.
#[derive(Debug)]
pub(crate) struct PmPublicRoutes {
    authority_id: PmPublicRouteAuthorityId,
    pm_role: PmPublicRole,
    pm_authority: PmAuthoritativeMetadata,
    pm_metadata_issued: bool,
    okx_reference: OkxReferenceHandle,
    okx_instrument: OkxReferenceInstrument,
    okx_source: PmProductSource,
    okx_connection: PmConnectionId,
}

/// Process-unique, non-forgeable identity for one live route authority.
///
/// The numeric value is never part of captured/replayed product evidence. It
/// exists only to prevent same-config concurrent roots from exchanging
/// capability deliveries or snapshot-flow proofs inside one process.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct PmPublicRouteAuthorityId(u64);

impl std::fmt::Debug for PmPublicRouteAuthorityId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("PmPublicRouteAuthorityId(<opaque>)")
    }
}

impl PmPublicRouteAuthorityId {
    fn allocate() -> Result<Self, PmPublicRouteError> {
        static NEXT_AUTHORITY_ID: AtomicU64 = AtomicU64::new(1);
        NEXT_AUTHORITY_ID
            .fetch_update(
                AtomicOrdering::Relaxed,
                AtomicOrdering::Relaxed,
                |current| current.checked_add(1),
            )
            .map(Self)
            .map_err(|_| PmPublicRouteError::AuthorityIdExhausted)
    }

    #[cfg(test)]
    pub(crate) const fn for_test(value: u64) -> Self {
        Self(value)
    }
}

impl PmPublicRoutes {
    pub(crate) fn new(
        config: &PmPublicConnectivityConfig,
        pm_session: &PmPublicSession,
        okx_session: &OkxPublicSession,
    ) -> Result<Self, PmPublicRouteError> {
        let pm_role = pm_session.role();
        if pm_role.observation_grant() != config.observation_grant()
            || pm_role.instrument() != config.instrument()
            || pm_role.source() != config.polymarket_route().source()
            || pm_role.connection() != config.polymarket_route().connection()
            || pm_role.wire_scope().condition() != config.expected_metadata().condition()
            || pm_role.wire_scope().market() != config.expected_metadata().market()
            || pm_role.wire_scope().token() != config.expected_metadata().outcome().token()
            || pm_role.parser_config().tick() != config.expected_metadata().tick()
            || pm_role.parser_config().minimum_order_size()
                != config.expected_metadata().minimum_order_size()
            || pm_role.parser_config().negative_risk() != config.expected_metadata().negative_risk()
        {
            return Err(PmPublicRouteError::PmRoleMismatch);
        }
        let pm_authority = pm_session.authoritative_metadata();
        let metadata_event = pm_authority.event();
        if metadata_event.source() != pm_role.source()
            || metadata_event.instrument() != pm_role.instrument()
            || metadata_event.metadata() != config.expected_metadata()
            || metadata_event.metadata_revision() != pm_session.metadata_revision()
        {
            return Err(PmPublicRouteError::PmSessionMismatch);
        }
        if okx_session.subscription().instrument()
            != config.okx_reference_instrument().instrument_id().as_str()
        {
            return Err(PmPublicRouteError::OkxInstrumentMismatch);
        }
        if okx_session.connection_id().0.as_str() != config.okx_route().connection().as_str() {
            return Err(PmPublicRouteError::OkxConnectionMismatch);
        }
        if okx_session.connection_epoch() == 0 {
            return Err(PmPublicRouteError::ZeroConnectionEpoch);
        }
        Ok(Self {
            authority_id: PmPublicRouteAuthorityId::allocate()?,
            pm_role,
            pm_authority,
            pm_metadata_issued: false,
            okx_reference: config.okx_reference(),
            okx_instrument: config.okx_reference_instrument(),
            okx_source: config.okx_route().source(),
            okx_connection: config.okx_route().connection(),
        })
    }

    #[must_use]
    pub(crate) const fn authority_id(&self) -> PmPublicRouteAuthorityId {
        self.authority_id
    }

    /// Issues the exact configured metadata delivery from the PM session's
    /// already checked atomic authority.
    pub(crate) fn pm_metadata(
        &mut self,
        session: &mut PmPublicSession,
        local_wall_receive_ns: u64,
    ) -> Result<PmPublicMetadataDelivery, PmPublicRouteError> {
        self.validate_pm_session(session)?;
        if session.requires_reconnect() {
            return Err(PmPublicRouteError::PmSessionUnavailable);
        }
        if self.pm_metadata_issued {
            return Err(PmPublicRouteError::PmMetadataAlreadyIssued);
        }
        let occurrence = session.issue_metadata_occurrence(local_wall_receive_ns)?;
        if occurrence.source() != self.pm_role.source()
            || occurrence.connection_id() != self.pm_role.connection()
            || occurrence.ordering().connection_epoch() != session.connection_epoch()
        {
            return Err(PmPublicRouteError::PmSessionMismatch);
        }
        let authority = self.pm_authority;
        let event = authority.event();
        let clock = occurrence.received_clock();
        if clock.venue_event_timestamp_ns().is_some()
            || clock.monotonic_receive_ns() != authority.monotonic_receive_ns()
        {
            return Err(PmPublicRouteError::PmSessionMismatch);
        }
        let envelope = ReceivedEventEnvelope::new(
            self.pm_role.source().venue(),
            self.pm_role.source(),
            self.pm_role.connection(),
            clock,
            occurrence.ordering(),
            event,
        )?;
        self.pm_metadata_issued = true;
        Ok(PmPublicMetadataDelivery {
            authority_id: self.authority_id,
            envelope,
        })
    }

    /// Binds the PM adapter's session-issued book proof to this product's exact
    /// configured role before the delivery can enter a public lane.
    pub(crate) fn pm_book(
        &self,
        session: &PmPublicSession,
        delivery: AdapterPmPublicBookDelivery,
    ) -> Result<PmPublicBookDelivery, PmPublicRouteError> {
        self.validate_pm_session(session)?;
        let envelope = delivery.envelope();
        if envelope.source() != self.pm_role.source()
            || envelope.connection_id() != self.pm_role.connection()
            || envelope.payload().source() != self.pm_role.source()
            || envelope.payload().instrument() != self.pm_role.instrument()
            || envelope.payload().metadata_revision()
                != self.pm_authority.event().metadata_revision()
        {
            return Err(PmPublicRouteError::PmDeliveryScopeMismatch);
        }
        if envelope.ordering().connection_epoch() != delivery.attempt_connection_epoch()
            || delivery.attempt_connection_epoch() != session.connection_epoch()
        {
            return Err(PmPublicRouteError::ConnectionEpochMismatch);
        }
        if envelope.received_clock().monotonic_receive_ns()
            < self.pm_authority.monotonic_receive_ns()
        {
            return Err(PmPublicRouteError::ReceiveClockRegression);
        }
        let tick_size_change = matches!(
            envelope.payload().update(),
            reap_pm_core::PmBookUpdate::TickSizeChanged { .. }
        );
        if delivery.is_terminal_tick_size_change() != tick_size_change {
            return Err(PmPublicRouteError::PmDeliveryDispositionMismatch);
        }
        if session.requires_reconnect() {
            if !delivery.is_terminal_tick_size_change()
                || session.last_fault() != Some(PmPublicSessionFault::TickSizeChanged)
            {
                return Err(PmPublicRouteError::PmSessionUnavailable);
            }
        } else if delivery.is_terminal_tick_size_change() {
            return Err(PmPublicRouteError::PmDeliveryDispositionMismatch);
        }
        Ok(PmPublicBookDelivery {
            authority_id: self.authority_id,
            envelope: delivery.into_envelope(),
        })
    }

    /// Converts one session-issued OKX reference to the compact PM product
    /// event while retaining the exact configured connection evidence.
    pub(crate) fn okx_reference(
        &self,
        session: &OkxPublicSession,
        delivery: OkxPublicSessionDelivery,
    ) -> Result<OkxPublicReferenceDelivery, PmPublicRouteError> {
        self.validate_okx_session(session)?;
        if delivery.connection_id() != self.okx_connection.as_str()
            || delivery.connection_epoch() != session.connection_epoch()
        {
            return Err(PmPublicRouteError::OkxDeliveryRouteMismatch);
        }
        if session.requires_reconnect() || !session.subscription_ready() {
            return Err(PmPublicRouteError::OkxSessionUnavailable);
        }
        let monotonic_receive_ns = delivery.monotonic_receive_ns();
        let connection_epoch = delivery.connection_epoch();
        let local_ingress_sequence = delivery.local_ingress_sequence();
        let OkxPublicSessionEvent::Reference(reference) = delivery.into_payload() else {
            return Err(PmPublicRouteError::NotOkxReference);
        };
        if reference.instrument() != self.okx_instrument.instrument_id().as_str()
            || reference.connection_epoch() != connection_epoch
        {
            return Err(PmPublicRouteError::OkxDeliveryRouteMismatch);
        }
        let venue_event_timestamp_ns = reference
            .venue_ts_ms()
            .checked_mul(1_000_000)
            .ok_or(PmPublicRouteError::VenueTimestampOverflow)?;
        let event = OkxReferenceEvent::new(
            self.okx_source,
            self.okx_reference,
            OkxReferencePrice::parse_decimal(reference.index_price_lexeme())?,
        )?;
        let clock = ReceivedEventClock::new(
            Some(venue_event_timestamp_ns),
            reference.wall_receive_ts_ns(),
            monotonic_receive_ns,
        )?;
        let ordering = EventOrdering::new(
            ConnectionEpoch::new(connection_epoch),
            None,
            None,
            None,
            IngressSequence::new(local_ingress_sequence),
        )?;
        let envelope = ReceivedEventEnvelope::new(
            self.okx_source.venue(),
            self.okx_source,
            self.okx_connection,
            clock,
            ordering,
            event,
        )?;
        Ok(OkxPublicReferenceDelivery {
            authority_id: self.authority_id,
            envelope,
        })
    }

    /// Issues an unavailable event only after the exact configured PM session
    /// has recorded a fail-closed fault.
    pub(crate) fn pm_unavailable(
        &self,
        session: &PmPublicSession,
        occurrence: PmPublicUnavailableOccurrence,
    ) -> Result<PmPublicUnavailableDelivery, PmPublicRouteError> {
        self.validate_pm_session(session)?;
        if occurrence.source() != self.pm_role.source()
            || occurrence.connection_id() != self.pm_role.connection()
        {
            return Err(PmPublicRouteError::PmDeliveryScopeMismatch);
        }
        let clock = occurrence.received_clock();
        if clock.venue_event_timestamp_ns().is_some() {
            return Err(PmPublicRouteError::UnavailableHasVenueTimestamp);
        }
        let ordering = occurrence.ordering();
        if ordering.connection_epoch() != session.connection_epoch()
            || !session.requires_reconnect()
            || session.last_fault() != Some(occurrence.fault())
        {
            return Err(PmPublicRouteError::PmSessionUnavailable);
        }
        let event = PmPublicUnavailable {
            source: self.pm_role.source(),
            fault: occurrence.fault(),
        };
        let envelope = ReceivedEventEnvelope::new(
            self.pm_role.source().venue(),
            self.pm_role.source(),
            self.pm_role.connection(),
            clock,
            ordering,
            event,
        )?;
        Ok(PmPublicUnavailableDelivery {
            authority_id: self.authority_id,
            envelope,
        })
    }

    /// Issues an unavailable event only after the exact configured OKX session
    /// has recorded a fail-closed fault.
    pub(crate) fn okx_unavailable(
        &self,
        session: &OkxPublicSession,
        occurrence: OkxPublicUnavailableOccurrence,
    ) -> Result<OkxPublicUnavailableDelivery, PmPublicRouteError> {
        self.validate_okx_session(session)?;
        if occurrence.connection_id() != self.okx_connection.as_str()
            || occurrence.connection_epoch() != session.connection_epoch()
            || !session.requires_reconnect()
            || session.last_fault() != Some(occurrence.fault())
        {
            return Err(PmPublicRouteError::OkxDeliveryRouteMismatch);
        }
        let clock = ReceivedEventClock::new(
            None,
            occurrence.wall_receive_ts_ns(),
            occurrence.monotonic_receive_ns(),
        )?;
        let ordering = EventOrdering::new(
            ConnectionEpoch::new(occurrence.connection_epoch()),
            None,
            None,
            None,
            IngressSequence::new(occurrence.local_ingress_sequence()),
        )?;
        let event = OkxPublicUnavailable {
            source: self.okx_source,
            fault: occurrence.fault(),
        };
        let envelope = ReceivedEventEnvelope::new(
            self.okx_source.venue(),
            self.okx_source,
            self.okx_connection,
            clock,
            ordering,
            event,
        )?;
        Ok(OkxPublicUnavailableDelivery {
            authority_id: self.authority_id,
            envelope,
        })
    }

    fn validate_pm_session(&self, session: &PmPublicSession) -> Result<(), PmPublicRouteError> {
        let authority = session.authoritative_metadata();
        let event = authority.event();
        if session.role() != self.pm_role
            || event.source() != self.pm_role.source()
            || event.instrument() != self.pm_role.instrument()
            || event.metadata() != self.expected_metadata()
            || event.metadata_revision() != self.pm_authority.event().metadata_revision()
        {
            return Err(PmPublicRouteError::PmSessionMismatch);
        }
        Ok(())
    }

    fn expected_metadata(&self) -> reap_pm_core::PmMarketMetadata {
        self.pm_authority.event().metadata()
    }

    fn validate_okx_session(&self, session: &OkxPublicSession) -> Result<(), PmPublicRouteError> {
        if session.connection_id().0.as_str() != self.okx_connection.as_str() {
            return Err(PmPublicRouteError::OkxConnectionMismatch);
        }
        if session.subscription().instrument() != self.okx_instrument.instrument_id().as_str() {
            return Err(PmPublicRouteError::OkxInstrumentMismatch);
        }
        if session.connection_epoch() == 0 {
            return Err(PmPublicRouteError::ZeroConnectionEpoch);
        }
        Ok(())
    }
}

#[derive(Debug, Error)]
pub enum PmPublicRouteError {
    #[error("PM public role does not match the configured product route")]
    PmRoleMismatch,
    #[error("PM public session does not match the configured product route")]
    PmSessionMismatch,
    #[error("PM public session is unavailable and requires reconnect")]
    PmSessionUnavailable,
    #[error("PM authoritative metadata was already issued for this capture run")]
    PmMetadataAlreadyIssued,
    #[error("PM public session rejected metadata occurrence issuance: {0}")]
    PmSession(#[from] PmPublicSessionError),
    #[error("PM public delivery does not match the configured product scope")]
    PmDeliveryScopeMismatch,
    #[error("PM public delivery terminal disposition does not match its event")]
    PmDeliveryDispositionMismatch,
    #[error("OKX public connection does not match the configured product route")]
    OkxConnectionMismatch,
    #[error("OKX public instrument does not match the configured reference")]
    OkxInstrumentMismatch,
    #[error("OKX session delivery does not match the configured connection and epoch")]
    OkxDeliveryRouteMismatch,
    #[error("OKX public session is unavailable or lacks its exact subscription acknowledgement")]
    OkxSessionUnavailable,
    #[error("OKX session delivery is not a reference observation")]
    NotOkxReference,
    #[error("connection epoch must be nonzero")]
    ZeroConnectionEpoch,
    #[error("public route authority identity space is exhausted")]
    AuthorityIdExhausted,
    #[error("delivery connection epoch does not match the configured live session")]
    ConnectionEpochMismatch,
    #[error("delivery receive time precedes authoritative metadata")]
    ReceiveClockRegression,
    #[error("a connection-unavailable occurrence cannot claim a venue timestamp")]
    UnavailableHasVenueTimestamp,
    #[error("OKX venue timestamp overflows nanoseconds")]
    VenueTimestampOverflow,
    #[error(transparent)]
    Envelope(#[from] EnvelopeError),
    #[error(transparent)]
    ReferencePrice(#[from] OkxReferencePriceError),
    #[error(transparent)]
    ReferenceEvent(#[from] OkxReferenceEventError),
}

macro_rules! opaque_delivery {
    ($name:ident, $payload:ty) => {
        #[derive(Debug, PartialEq, Eq)]
        #[must_use = "route-issued deliveries must be consumed by their authorized Run operation"]
        pub struct $name {
            authority_id: PmPublicRouteAuthorityId,
            envelope: ReceivedEventEnvelope<$payload>,
        }

        impl $name {
            #[must_use]
            pub(crate) const fn authority_id(&self) -> PmPublicRouteAuthorityId {
                self.authority_id
            }

            #[must_use]
            pub const fn envelope(&self) -> &ReceivedEventEnvelope<$payload> {
                &self.envelope
            }

            pub(crate) fn into_parts(
                self,
            ) -> (PmPublicRouteAuthorityId, ReceivedEventEnvelope<$payload>) {
                (self.authority_id, self.envelope)
            }

            pub(crate) fn from_parts(
                authority_id: PmPublicRouteAuthorityId,
                envelope: ReceivedEventEnvelope<$payload>,
            ) -> Self {
                Self {
                    authority_id,
                    envelope,
                }
            }
        }
    };
}

opaque_delivery!(PmPublicMetadataDelivery, PmMarketEvent);
opaque_delivery!(PmPublicBookDelivery, PmBookEvent);
opaque_delivery!(OkxPublicReferenceDelivery, OkxReferenceEvent);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PmPublicUnavailable {
    source: PmProductSource,
    fault: PmPublicSessionFault,
}

impl PmPublicUnavailable {
    #[must_use]
    pub const fn source(self) -> PmProductSource {
        self.source
    }

    #[must_use]
    pub const fn fault(self) -> PmPublicSessionFault {
        self.fault
    }
}

impl PmSourceBound for PmPublicUnavailable {
    fn source(&self) -> PmProductSource {
        self.source
    }
}

opaque_delivery!(PmPublicUnavailableDelivery, PmPublicUnavailable);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OkxPublicUnavailable {
    source: PmProductSource,
    fault: OkxPublicSessionFault,
}

impl OkxPublicUnavailable {
    #[must_use]
    pub const fn source(self) -> PmProductSource {
        self.source
    }

    #[must_use]
    pub const fn fault(self) -> OkxPublicSessionFault {
        self.fault
    }
}

impl PmSourceBound for OkxPublicUnavailable {
    fn source(&self) -> PmProductSource {
        self.source
    }
}

opaque_delivery!(OkxPublicUnavailableDelivery, OkxPublicUnavailable);

#[cfg(test)]
mod tests {
    use reap_pm_core::{
        ConnectionEpoch, IngressSequence, OkxReferenceHandle, PmConnectionId, PmProductSource,
        PmSourceHandle, PmTokenHandle, ReceivedEventClock,
    };

    use super::*;
    use crate::lanes::{
        PmLaneKind, PmLanePolicy, PmPublicLaneService, PmPublicLaneState, SaturationAction,
        ServicedLaneItem,
    };

    #[derive(Default)]
    struct UnavailableOrder {
        observed: Vec<(&'static str, u8, u16)>,
    }

    impl PmPublicLaneService for UnavailableOrder {
        fn on_pm_public_unavailable(&mut self, item: ServicedLaneItem<PmPublicUnavailable>) {
            self.observed.push((
                "pm",
                item.key().source_kind_rank(),
                item.key().source_scope_ordinal(),
            ));
        }

        fn on_okx_public_unavailable(&mut self, item: ServicedLaneItem<OkxPublicUnavailable>) {
            self.observed.push((
                "okx",
                item.key().source_kind_rank(),
                item.key().source_scope_ordinal(),
            ));
        }

        fn on_market(&mut self, _item: ServicedLaneItem<PmMarketEvent>) {}

        fn on_book(&mut self, _item: ServicedLaneItem<PmBookEvent>) {}

        fn on_reference(&mut self, _item: ServicedLaneItem<OkxReferenceEvent>) {}
    }

    fn received_clock(monotonic_receive_ns: u64) -> ReceivedEventClock {
        ReceivedEventClock::new(
            None,
            1_700_000_000_000_000_000 + monotonic_receive_ns,
            monotonic_receive_ns,
        )
        .expect("clock")
    }

    fn ordering(local_ingress_sequence: u64) -> EventOrdering {
        EventOrdering::new(
            ConnectionEpoch::new(1),
            None,
            None,
            None,
            IngressSequence::new(local_ingress_sequence),
        )
        .expect("ordering")
    }

    const fn authority_id() -> PmPublicRouteAuthorityId {
        PmPublicRouteAuthorityId(1)
    }

    fn pm_unavailable_delivery(
        monotonic_receive_ns: u64,
        local_ingress_sequence: u64,
    ) -> PmPublicUnavailableDelivery {
        let source = PmProductSource::polymarket_market(
            PmSourceHandle::from_ordinal(0),
            PmTokenHandle::from_ordinal(0),
        );
        PmPublicUnavailableDelivery {
            authority_id: authority_id(),
            envelope: ReceivedEventEnvelope::new(
                source.venue(),
                source,
                PmConnectionId::new("pm-public").expect("connection"),
                received_clock(monotonic_receive_ns),
                ordering(local_ingress_sequence),
                PmPublicUnavailable {
                    source,
                    fault: PmPublicSessionFault::Disconnect,
                },
            )
            .expect("PM unavailable envelope"),
        }
    }

    fn okx_unavailable_delivery(
        monotonic_receive_ns: u64,
        local_ingress_sequence: u64,
    ) -> OkxPublicUnavailableDelivery {
        let source = PmProductSource::okx_reference(
            PmSourceHandle::from_ordinal(0),
            OkxReferenceHandle::from_ordinal(0),
        );
        OkxPublicUnavailableDelivery {
            authority_id: authority_id(),
            envelope: ReceivedEventEnvelope::new(
                source.venue(),
                source,
                PmConnectionId::new("okx-public").expect("connection"),
                received_clock(monotonic_receive_ns),
                ordering(local_ingress_sequence),
                OkxPublicUnavailable {
                    source,
                    fault: OkxPublicSessionFault::Disconnect,
                },
            )
            .expect("OKX unavailable envelope"),
        }
    }

    #[test]
    fn full_source_discriminator_prevents_same_ordinal_public_key_collision() {
        let source_handle = PmSourceHandle::from_ordinal(0);
        let pm_source =
            PmProductSource::polymarket_market(source_handle, PmTokenHandle::from_ordinal(0));
        let okx_source =
            PmProductSource::okx_reference(source_handle, OkxReferenceHandle::from_ordinal(0));
        let clock = ReceivedEventClock::new(None, 1_700_000_000_000_000_100, 100).expect("clock");
        let ordering = EventOrdering::new(
            ConnectionEpoch::new(1),
            None,
            None,
            None,
            IngressSequence::new(1),
        )
        .expect("ordering");

        let pm = PmPublicUnavailableDelivery {
            authority_id: authority_id(),
            envelope: ReceivedEventEnvelope::new(
                pm_source.venue(),
                pm_source,
                PmConnectionId::new("pm-public").expect("connection"),
                clock,
                ordering,
                PmPublicUnavailable {
                    source: pm_source,
                    fault: PmPublicSessionFault::Disconnect,
                },
            )
            .expect("PM unavailable envelope"),
        };
        let okx = OkxPublicUnavailableDelivery {
            authority_id: authority_id(),
            envelope: ReceivedEventEnvelope::new(
                okx_source.venue(),
                okx_source,
                PmConnectionId::new("okx-public").expect("connection"),
                clock,
                ordering,
                OkxPublicUnavailable {
                    source: okx_source,
                    fault: OkxPublicSessionFault::Disconnect,
                },
            )
            .expect("OKX unavailable envelope"),
        };

        let mut public_lane = PmPublicLaneState::new();
        public_lane
            .enqueue_pm_unavailable(pm)
            .expect("PM admission");
        public_lane
            .enqueue_okx_unavailable(okx)
            .expect("OKX admission");
        assert_eq!(public_lane.metrics().depth(), 2);

        let mut recorder = UnavailableOrder::default();
        assert_eq!(
            public_lane
                .service_turn(101, &mut recorder)
                .expect("service"),
            1
        );
        assert_eq!(
            public_lane
                .service_turn(101, &mut recorder)
                .expect("service"),
            1
        );
        assert_eq!(recorder.observed, vec![("okx", 0, 0), ("pm", 1, 0)]);
    }

    #[test]
    fn public_saturation_returns_the_exact_move_only_pm_and_okx_deliveries() {
        let mut public_lane = PmPublicLaneState::new();
        let capacity = PmLanePolicy::for_lane(PmLaneKind::Public).capacity();
        for ingress in 1..=capacity {
            let ingress = u64::try_from(ingress).expect("bounded capacity");
            public_lane
                .enqueue_pm_unavailable(pm_unavailable_delivery(ingress, ingress))
                .expect("fill public lane");
        }

        let pm = pm_unavailable_delivery(9_000, 9_000);
        let error = public_lane
            .enqueue_pm_unavailable(pm)
            .expect_err("full PM admission must return the exact delivery");
        assert!(error.is_full());
        assert_eq!(
            error.action(),
            Some(SaturationAction::InvalidateStreamAndResync)
        );
        let delivery = error.into_delivery();
        assert_eq!(delivery.authority_id(), authority_id());
        assert_eq!(delivery.envelope().connection_id().as_str(), "pm-public");
        assert_eq!(
            delivery
                .envelope()
                .ordering()
                .local_ingress_sequence()
                .value(),
            9_000
        );

        let okx = okx_unavailable_delivery(9_001, 9_001);
        let error = public_lane
            .enqueue_okx_unavailable(okx)
            .expect_err("full OKX admission must return the exact delivery");
        assert!(error.is_full());
        assert_eq!(
            error.action(),
            Some(SaturationAction::InvalidateStreamAndResync)
        );
        let delivery = error.into_delivery();
        assert_eq!(delivery.authority_id(), authority_id());
        assert_eq!(delivery.envelope().connection_id().as_str(), "okx-public");
        assert_eq!(
            delivery
                .envelope()
                .ordering()
                .local_ingress_sequence()
                .value(),
            9_001
        );
    }

    #[test]
    fn unavailable_controls_are_non_expiring_and_service_exactly_once() {
        for (delivery, expected_observed) in [
            (
                EitherUnavailable::Pm(pm_unavailable_delivery(1, 1)),
                ("pm", 1, 0),
            ),
            (
                EitherUnavailable::Okx(okx_unavailable_delivery(1, 1)),
                ("okx", 0, 0),
            ),
        ] {
            let mut public_lane = PmPublicLaneState::new();
            match delivery {
                EitherUnavailable::Pm(delivery) => {
                    public_lane
                        .enqueue_pm_unavailable(delivery)
                        .expect("PM admission");
                }
                EitherUnavailable::Okx(delivery) => {
                    public_lane
                        .enqueue_okx_unavailable(delivery)
                        .expect("OKX admission");
                }
            }
            let mut recorder = UnavailableOrder::default();
            assert_eq!(
                public_lane
                    .service_turn(500_000_002, &mut recorder)
                    .expect("must-deliver controls never expire"),
                1
            );
            assert_eq!(recorder.observed, vec![expected_observed]);
            assert_eq!(public_lane.metrics().depth(), 0);
            assert_eq!(public_lane.metrics().invalidated_purged(), 0);
        }
    }

    #[test]
    fn public_purge_is_exactly_scoped_by_authority_source_and_epoch() {
        let pm_source = PmProductSource::polymarket_market(
            PmSourceHandle::from_ordinal(0),
            PmTokenHandle::from_ordinal(0),
        );
        let mut target = pm_unavailable_delivery(1, 1);
        target.authority_id = PmPublicRouteAuthorityId(1);
        let mut sibling_root = pm_unavailable_delivery(2, 2);
        sibling_root.authority_id = PmPublicRouteAuthorityId(2);
        let okx = okx_unavailable_delivery(3, 3);
        let sibling_connection = {
            let (authority_id, envelope) = pm_unavailable_delivery(4, 4).into_parts();
            let venue = envelope.venue();
            let source = envelope.source();
            let clock = envelope.received_clock();
            let ordering = envelope.ordering();
            let payload = envelope.into_payload();
            PmPublicUnavailableDelivery::from_parts(
                authority_id,
                ReceivedEventEnvelope::new(
                    venue,
                    source,
                    PmConnectionId::new("pm-public-sibling").expect("connection"),
                    clock,
                    ordering,
                    payload,
                )
                .expect("rebound test envelope"),
            )
        };

        let mut public_lane = PmPublicLaneState::new();
        public_lane.enqueue_pm_unavailable(target).expect("target");
        public_lane
            .enqueue_pm_unavailable(sibling_root)
            .expect("sibling");
        public_lane.enqueue_okx_unavailable(okx).expect("OKX");
        public_lane
            .enqueue_pm_unavailable(sibling_connection)
            .expect("sibling connection");

        assert_eq!(
            public_lane.purge_public_route(
                PmPublicRouteAuthorityId(1),
                pm_source,
                PmConnectionId::new("pm-public").expect("connection"),
                ConnectionEpoch::new(1)
            ),
            1
        );
        assert_eq!(public_lane.metrics().depth(), 3);
        assert_eq!(public_lane.metrics().invalidated_purged(), 1);
    }

    enum EitherUnavailable {
        Pm(PmPublicUnavailableDelivery),
        Okx(OkxPublicUnavailableDelivery),
    }
}
