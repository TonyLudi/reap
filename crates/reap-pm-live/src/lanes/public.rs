use super::*;

/// The bounded public queue and its proof authority for one active capture Run.
///
/// This is the only scheduler state materialized by the Phase-3 capture Run.
/// Later phases must introduce their typed producers and complete scheduler
/// atomically; routed public deliveries can never outlive or be serviced by a
/// sibling capture authority.
#[derive(Debug)]
pub(crate) struct PmPublicLaneState {
    pub(super) authority_id: PmLaneAuthorityId,
    pub(super) queue: BoundedHeap<PmServiceKey, LaneItem<PmPublicInput>>,
    pub(super) consumer_transfer_in_flight: bool,
}

impl Default for PmPublicLaneState {
    fn default() -> Self {
        Self::new()
    }
}

impl PmPublicLaneState {
    pub(crate) fn new() -> Self {
        Self {
            authority_id: PmLaneAuthorityId::allocate(),
            queue: BoundedHeap::new(PmLaneKind::Public),
            consumer_transfer_in_flight: false,
        }
    }

    pub(crate) fn metrics(&self) -> PmLaneMetrics {
        self.queue.metrics()
    }

    pub(crate) const fn consumer_transfer_poisoned(&self) -> bool {
        self.consumer_transfer_in_flight
    }

    pub(crate) fn reserved_capacity_bytes(&self) -> usize {
        self.queue.reserved_capacity_bytes()
    }

    /// Admits only configured, role-issued PM metadata.
    #[allow(
        clippy::result_large_err,
        reason = "fail-closed admission returns the unconsumed inline metadata without heap allocation"
    )]
    pub(crate) fn enqueue_pm_metadata(
        &mut self,
        delivery: PmPublicMetadataDelivery,
    ) -> Result<(), PmPublicLaneEnqueueError<PmPublicMetadataDelivery>> {
        self.enqueue_public_delivery(
            delivery,
            PmPublicAgedHead::PmMetadata,
            PmPublicMetadataDelivery::into_parts,
            PmPublicMetadataDelivery::from_parts,
            PmPublicInput::Market,
        )
    }

    /// Admits only the configured PM public-session book proof.
    pub(crate) fn enqueue_pm_book(
        &mut self,
        delivery: PmPublicBookDelivery,
        projection: PmBookDecisionProjection,
    ) -> Result<(), PmPublicLaneEnqueueError<PmPublicBookDelivery>> {
        let head = match delivery.envelope().payload().update() {
            PmBookUpdate::TickSizeChanged { old, new } => PmPublicAgedHead::PmTickSizeChanged {
                old: *old,
                new: *new,
            },
            _ => PmPublicAgedHead::PmBook,
        };
        self.enqueue_public_delivery(
            delivery,
            head,
            PmPublicBookDelivery::into_parts,
            PmPublicBookDelivery::from_parts,
            move |event| PmPublicInput::Book { event, projection },
        )
    }

    /// Admits only the configured OKX public-session reference proof.
    pub(crate) fn enqueue_okx_reference(
        &mut self,
        delivery: OkxPublicReferenceDelivery,
    ) -> Result<(), PmPublicLaneEnqueueError<OkxPublicReferenceDelivery>> {
        self.enqueue_public_delivery(
            delivery,
            PmPublicAgedHead::OkxReference,
            OkxPublicReferenceDelivery::into_parts,
            OkxPublicReferenceDelivery::from_parts,
            PmPublicInput::Reference,
        )
    }

    /// Admits only a fault emitted by the configured PM public session.
    pub(crate) fn enqueue_pm_unavailable(
        &mut self,
        delivery: PmPublicUnavailableDelivery,
    ) -> Result<(), PmPublicLaneEnqueueError<PmPublicUnavailableDelivery>> {
        let head = PmPublicAgedHead::PmUnavailable(delivery.envelope().payload().fault());
        self.enqueue_public_delivery(
            delivery,
            head,
            PmPublicUnavailableDelivery::into_parts,
            PmPublicUnavailableDelivery::from_parts,
            PmPublicInput::PmUnavailable,
        )
    }

    /// Admits only a fault emitted by the configured OKX public session.
    pub(crate) fn enqueue_okx_unavailable(
        &mut self,
        delivery: OkxPublicUnavailableDelivery,
    ) -> Result<(), PmPublicLaneEnqueueError<OkxPublicUnavailableDelivery>> {
        let head = PmPublicAgedHead::OkxUnavailable(delivery.envelope().payload().fault());
        self.enqueue_public_delivery(
            delivery,
            head,
            OkxPublicUnavailableDelivery::into_parts,
            OkxPublicUnavailableDelivery::from_parts,
            PmPublicInput::OkxUnavailable,
        )
    }

    #[allow(
        clippy::result_large_err,
        reason = "fail-closed admission returns the exact unconsumed move-only route delivery"
    )]
    fn enqueue_public_delivery<E: PmObservedEvent, D>(
        &mut self,
        delivery: D,
        head: PmPublicAgedHead,
        into_parts: impl FnOnce(D) -> (PmPublicRouteAuthorityId, ReceivedEventEnvelope<E>),
        from_parts: impl FnOnce(PmPublicRouteAuthorityId, ReceivedEventEnvelope<E>) -> D,
        wrap: impl FnOnce(E) -> PmPublicInput,
    ) -> Result<(), PmPublicLaneEnqueueError<D>> {
        let (authority_id, envelope) = into_parts(delivery);
        let venue = envelope.venue();
        let source = envelope.source();
        let connection = envelope.connection_id();
        let clock = envelope.received_clock();
        let ordering = envelope.ordering();
        let evidence = PmIngressOrder::from_ordering(connection, ordering);
        let rejected_key =
            PmServiceKey::derived(clock, source, evidence, envelope.payload().variant_rank());
        let lane_generation = self.queue.generation();
        let lane_capacity = self.queue.policy().capacity();
        let public_route = PmPublicRouteLaneEvidence {
            authority_id,
            source,
            head,
        };
        match enqueue_received(
            &mut self.queue,
            rejected_key,
            evidence,
            clock,
            public_route,
            envelope.into_payload(),
            wrap,
        ) {
            Ok(()) => Ok(()),
            Err(LaneEnqueueError::Full { value, action }) => {
                let envelope = ReceivedEventEnvelope::new(
                    venue, source, connection, clock, ordering, value,
                )
                .expect("route-issued envelope remains valid when admission returns its payload");
                Err(PmPublicLaneEnqueueError::full(
                    from_parts(authority_id, envelope),
                    action,
                    self.authority_id,
                    lane_generation,
                    lane_capacity,
                    rejected_key,
                ))
            }
            Err(LaneEnqueueError::DuplicateKey { value }) => {
                let envelope = ReceivedEventEnvelope::new(
                    venue, source, connection, clock, ordering, value,
                )
                .expect("route-issued envelope remains valid when admission returns its payload");
                Err(PmPublicLaneEnqueueError::duplicate_key(
                    from_parts(authority_id, envelope),
                    self.authority_id,
                    lane_generation,
                    lane_capacity,
                    rejected_key,
                ))
            }
        }
    }

    /// Quarantines all queued public work issued by one exact route authority,
    /// product source, connection identity, and connection epoch.
    pub(crate) fn purge_public_route(
        &mut self,
        authority_id: PmPublicRouteAuthorityId,
        source: reap_pm_core::PmProductSource,
        connection: PmConnectionId,
        connection_epoch: ConnectionEpoch,
    ) -> usize {
        self.queue.purge_where(|_, item| {
            let route = item.public_route();
            route.authority_id == authority_id
                && route.source == source
                && item.connection() == connection
                && item.ordering().connection_epoch() == connection_epoch
        })
    }

    pub(crate) fn public_route_depth(
        &self,
        authority_id: PmPublicRouteAuthorityId,
        source: reap_pm_core::PmProductSource,
        connection: PmConnectionId,
        connection_epoch: ConnectionEpoch,
    ) -> usize {
        self.queue.count_where(|_, item| {
            let route = item.public_route();
            route.authority_id == authority_id
                && route.source == source
                && item.connection() == connection
                && item.ordering().connection_epoch() == connection_epoch
        })
    }

    pub(crate) fn authenticate_lane_failure<D>(
        &self,
        failure: PmPublicLaneEnqueueError<D>,
    ) -> Result<PmAuthenticatedPublicLaneFailure<D>, PmPublicLaneEnqueueError<D>> {
        let key_present = self.queue.contains_key(failure.rejected_key());
        let common_matches = failure.lane_authority() == self.authority_id
            && failure.lane_generation() == self.queue.generation()
            && failure.lane_capacity() == self.queue.policy().capacity();
        let state_matches = if failure.is_full() {
            self.queue.len() == self.queue.policy().capacity()
                && !key_present
                && failure.action() == Some(self.queue.policy().saturation_action())
        } else {
            key_present
        };
        if !common_matches || !state_matches {
            return Err(failure);
        }
        Ok(failure.into_authenticated())
    }

    pub(crate) fn authenticate_aged_failure(
        &self,
        failure: PmServiceTurnError,
        monotonic_now_ns: u64,
    ) -> Result<PmAgedDeliveryEvidence, PmServiceTurnError> {
        let Some(aged) = failure.aged_failure() else {
            return Err(failure);
        };
        let evidence = aged.evidence();
        let Some(head) = self.queue.peek() else {
            return Err(failure);
        };
        let age_is_still_proven = head
            .value
            .queue_age_ns(evidence.observed_now_ns())
            .ok()
            .zip(self.queue.policy().maximum_age_ns())
            .is_some_and(|(age, maximum)| age > maximum);
        let exact_head_matches = head.value.key() == evidence.key()
            && head.value.connection() == evidence.connection()
            && head.value.ordering() == evidence.ordering()
            && head.value.received_clock() == evidence.received_clock()
            && head.value.public_route() == evidence.public_route();
        let proof_matches = aged.lane() == PmLaneKind::Public
            && aged.action() == self.queue.policy().saturation_action()
            && evidence.lane_authority() == self.authority_id
            && evidence.lane_generation() == self.queue.generation()
            && monotonic_now_ns >= evidence.observed_now_ns()
            && exact_head_matches
            && age_is_still_proven;
        if !proof_matches {
            return Err(failure);
        }
        Ok(failure
            .into_aged_evidence()
            .expect("authenticated service age failure carries evidence"))
    }
}
