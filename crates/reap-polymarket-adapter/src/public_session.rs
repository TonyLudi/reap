use std::time::Duration;

use reap_pm_core::{
    ConnectionEpoch, EnvelopeError, EventOrdering, IngressSequence, PmBookDeltaBatch, PmBookEvent,
    PmBookSnapshot as CoreBookSnapshot, PmBookTopCheck, PmBookUpdate, PmConfigurationFingerprint,
    PmEventError, PmVenueChangeHash, ReceivedEventClock, ReceivedEventEnvelope, SnapshotRevision,
    VenueEventHash,
};
use reap_polymarket_wire::{
    MAX_WS_EVENTS_PER_FRAME, PmIgnoredEvent, PmMarketSubscription, PmWireError, PmWsEvent,
    parse_ws_frame,
};
use reap_transport::{ConnectionStatusKind, ReconnectPolicy, SupervisorState};
use thiserror::Error;

use crate::{PmAuthoritativeMetadata, PmPublicRole};

pub const PM_PUBLIC_PING_BYTES: &[u8] = b"PING";
pub const PM_PUBLIC_PONG_BYTES: &[u8] = b"PONG";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PmPublicHeartbeatConfig {
    ping_interval_ns: u64,
    pong_timeout_ns: u64,
}

impl PmPublicHeartbeatConfig {
    pub fn new(ping_interval_ns: u64, pong_timeout_ns: u64) -> Result<Self, PmPublicSessionError> {
        if ping_interval_ns == 0 {
            return Err(PmPublicSessionError::ZeroPingInterval);
        }
        if pong_timeout_ns == 0 {
            return Err(PmPublicSessionError::ZeroPongTimeout);
        }
        Ok(Self {
            ping_interval_ns,
            pong_timeout_ns,
        })
    }

    #[must_use]
    pub const fn ping_interval_ns(self) -> u64 {
        self.ping_interval_ns
    }

    #[must_use]
    pub const fn pong_timeout_ns(self) -> u64 {
        self.pong_timeout_ns
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PmPublicHeartbeatAction {
    Idle,
    SendPing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PmPublicHeartbeatEvidence {
    connection_epoch: ConnectionEpoch,
    local_wall_receive_ns: u64,
    monotonic_receive_ns: u64,
}

impl PmPublicHeartbeatEvidence {
    #[must_use]
    pub const fn connection_epoch(self) -> ConnectionEpoch {
        self.connection_epoch
    }

    #[must_use]
    pub const fn local_wall_receive_ns(self) -> u64 {
        self.local_wall_receive_ns
    }

    #[must_use]
    pub const fn monotonic_receive_ns(self) -> u64 {
        self.monotonic_receive_ns
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PmPublicSessionIgnored {
    PublicTrade,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PmSnapshotFlowToken {
    connection_epoch: ConnectionEpoch,
    snapshot_revision: SnapshotRevision,
    local_ingress_sequence: IngressSequence,
    venue_hash: VenueEventHash,
}

impl PmSnapshotFlowToken {
    #[must_use]
    pub const fn connection_epoch(self) -> ConnectionEpoch {
        self.connection_epoch
    }

    #[must_use]
    pub const fn snapshot_revision(self) -> SnapshotRevision {
        self.snapshot_revision
    }

    #[must_use]
    pub const fn local_ingress_sequence(self) -> IngressSequence {
        self.local_ingress_sequence
    }

    #[must_use]
    pub const fn venue_hash(self) -> VenueEventHash {
        self.venue_hash
    }
}

/// One normalized public-book delivery whose configured route was proven by
/// [`PmPublicSession`].
///
/// The inner envelope has no public wrapping constructor. Consumers may
/// inspect it or consume the proof into the normalized envelope after the
/// capability-bearing PM lane has admitted it.
#[derive(Debug, PartialEq, Eq)]
pub struct PmPublicBookDelivery {
    envelope: ReceivedEventEnvelope<PmBookEvent>,
    attempt_connection_epoch: ConnectionEpoch,
    disposition: PmPublicBookDisposition,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PmPublicBookDisposition {
    Normal,
    TerminalTickSizeChange,
}

impl PmPublicBookDelivery {
    #[must_use]
    pub const fn envelope(&self) -> &ReceivedEventEnvelope<PmBookEvent> {
        &self.envelope
    }

    #[must_use]
    pub const fn source(&self) -> reap_pm_core::PmProductSource {
        self.envelope.source()
    }

    #[must_use]
    pub const fn connection_id(&self) -> reap_pm_core::PmConnectionId {
        self.envelope.connection_id()
    }

    #[must_use]
    pub const fn received_clock(&self) -> ReceivedEventClock {
        self.envelope.received_clock()
    }

    #[must_use]
    pub const fn ordering(&self) -> EventOrdering {
        self.envelope.ordering()
    }

    /// Returns the private attempt epoch stamped by the session that
    /// normalized this delivery.
    ///
    /// The value is inspectable for route validation, but callers cannot
    /// construct or change the proof.
    #[must_use]
    pub const fn attempt_connection_epoch(&self) -> ConnectionEpoch {
        self.attempt_connection_epoch
    }

    /// Whether this is the exact tick-size transition that terminally closed
    /// the attempt after the event was normalized.
    #[must_use]
    pub const fn is_terminal_tick_size_change(&self) -> bool {
        matches!(
            self.disposition,
            PmPublicBookDisposition::TerminalTickSizeChange
        )
    }

    #[must_use]
    pub const fn payload(&self) -> &PmBookEvent {
        self.envelope.payload()
    }

    #[must_use]
    pub fn into_envelope(self) -> ReceivedEventEnvelope<PmBookEvent> {
        self.envelope
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct PmPublicSessionBatch {
    events: Vec<PmPublicBookDelivery>,
    ignored: Vec<PmPublicSessionIgnored>,
    snapshot_flow_token: Option<PmSnapshotFlowToken>,
    heartbeat: Option<PmPublicHeartbeatEvidence>,
}

impl PmPublicSessionBatch {
    fn data(
        events: Vec<PmPublicBookDelivery>,
        ignored: Vec<PmPublicSessionIgnored>,
        snapshot_flow_token: Option<PmSnapshotFlowToken>,
    ) -> Self {
        debug_assert!(events.len().saturating_add(ignored.len()) <= MAX_WS_EVENTS_PER_FRAME);
        Self {
            events,
            ignored,
            snapshot_flow_token,
            heartbeat: None,
        }
    }

    const fn from_heartbeat(evidence: PmPublicHeartbeatEvidence) -> Self {
        Self {
            events: Vec::new(),
            ignored: Vec::new(),
            snapshot_flow_token: None,
            heartbeat: Some(evidence),
        }
    }

    #[must_use]
    pub fn events(&self) -> &[PmPublicBookDelivery] {
        &self.events
    }

    #[must_use]
    pub fn into_events(self) -> Vec<PmPublicBookDelivery> {
        self.events
    }

    #[must_use]
    pub fn ignored(&self) -> &[PmPublicSessionIgnored] {
        &self.ignored
    }

    #[must_use]
    /// Correlates the emitted snapshot with the protocol-flow gate.
    ///
    /// Possessing this token is not product readiness. Product readiness is
    /// owned solely by the PM book/readiness reducer.
    pub const fn snapshot_flow_token(&self) -> Option<PmSnapshotFlowToken> {
        self.snapshot_flow_token
    }

    #[must_use]
    pub const fn heartbeat(&self) -> Option<PmPublicHeartbeatEvidence> {
        self.heartbeat
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PmPublicSessionFault {
    Disconnect,
    Gap,
    Overflow,
    Stale,
    InvalidTransition,
    HashMismatch,
    TickSizeChanged,
    HeartbeatTimeout,
    ReducerRejected,
}

/// One connection-unavailable occurrence emitted by a PM public session.
///
/// Construction and sequencing are session-owned. The value is intentionally
/// non-cloneable so one `take_unavailable` call can authorize at most one
/// downstream unavailable delivery.
#[derive(Debug, PartialEq, Eq)]
pub struct PmPublicUnavailableOccurrence {
    source: reap_pm_core::PmProductSource,
    connection_id: reap_pm_core::PmConnectionId,
    received_clock: ReceivedEventClock,
    ordering: EventOrdering,
    fault: PmPublicSessionFault,
}

impl PmPublicUnavailableOccurrence {
    #[must_use]
    pub const fn source(&self) -> reap_pm_core::PmProductSource {
        self.source
    }

    #[must_use]
    pub const fn connection_id(&self) -> reap_pm_core::PmConnectionId {
        self.connection_id
    }

    #[must_use]
    pub const fn received_clock(&self) -> ReceivedEventClock {
        self.received_clock
    }

    #[must_use]
    pub const fn ordering(&self) -> EventOrdering {
        self.ordering
    }

    #[must_use]
    pub const fn fault(&self) -> PmPublicSessionFault {
        self.fault
    }
}

/// The single authoritative-metadata occurrence minted by a PM public
/// session.
///
/// Its ingress sequence comes from the same counter as normalized websocket
/// events and unavailable occurrences. This makes metadata ordering
/// collision-free without accepting a caller-selected sequence. The
/// occurrence is move-only and cannot be forged outside this module.
#[derive(Debug, PartialEq, Eq)]
pub struct PmPublicMetadataOccurrence {
    source: reap_pm_core::PmProductSource,
    connection_id: reap_pm_core::PmConnectionId,
    received_clock: ReceivedEventClock,
    ordering: EventOrdering,
}

impl PmPublicMetadataOccurrence {
    #[must_use]
    pub const fn source(&self) -> reap_pm_core::PmProductSource {
        self.source
    }

    #[must_use]
    pub const fn connection_id(&self) -> reap_pm_core::PmConnectionId {
        self.connection_id
    }

    #[must_use]
    pub const fn received_clock(&self) -> ReceivedEventClock {
        self.received_clock
    }

    #[must_use]
    pub const fn ordering(&self) -> EventOrdering {
        self.ordering
    }
}

#[derive(Debug, Clone, Copy)]
struct HeartbeatState {
    next_ping_ns: Option<u64>,
    pong_deadline_ns: Option<u64>,
}

impl HeartbeatState {
    const fn disconnected() -> Self {
        Self {
            next_ping_ns: None,
            pong_deadline_ns: None,
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct AttemptState {
    subscription_sent: bool,
    requires_reconnect: bool,
    flow_open: bool,
    reached_flow_open: bool,
    local_ingress_sequence: u64,
    current_snapshot_revision: Option<SnapshotRevision>,
    pending_snapshot_flow: Option<PmSnapshotFlowToken>,
    last_monotonic_ns: Option<u64>,
    heartbeat: HeartbeatState,
}

impl AttemptState {
    const fn new() -> Self {
        Self {
            subscription_sent: false,
            requires_reconnect: false,
            flow_open: false,
            reached_flow_open: false,
            local_ingress_sequence: 0,
            current_snapshot_revision: None,
            pending_snapshot_flow: None,
            last_monotonic_ns: None,
            heartbeat: HeartbeatState::disconnected(),
        }
    }
}

#[derive(Debug)]
pub struct PmPublicSession {
    role: PmPublicRole,
    authoritative_metadata: PmAuthoritativeMetadata,
    subscription: Vec<u8>,
    connection_epoch: ConnectionEpoch,
    last_snapshot_revision: u64,
    heartbeat_config: PmPublicHeartbeatConfig,
    attempt: AttemptState,
    last_fault: Option<PmPublicSessionFault>,
    pending_unavailable: Option<PmPublicUnavailableOccurrence>,
    unavailable_required: bool,
    metadata_occurrence_issued: bool,
    supervisor: SupervisorState,
}

impl PmPublicSession {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        role: PmPublicRole,
        authoritative_metadata: PmAuthoritativeMetadata,
        connection_epoch: ConnectionEpoch,
        last_snapshot_revision: Option<SnapshotRevision>,
        reconnect_policy: ReconnectPolicy,
        heartbeat_config: PmPublicHeartbeatConfig,
    ) -> Result<Self, PmPublicSessionError> {
        if connection_epoch.value() == 0 {
            return Err(PmPublicSessionError::ZeroConnectionEpoch);
        }
        if last_snapshot_revision.is_some_and(|revision| revision.value() == 0) {
            return Err(PmPublicSessionError::ZeroLastSnapshotRevision);
        }
        let metadata_event = authoritative_metadata.event();
        if metadata_event.instrument() != role.instrument() {
            return Err(PmPublicSessionError::MetadataInstrumentMismatch);
        }
        if metadata_event.source() != role.source() {
            return Err(PmPublicSessionError::MetadataSourceMismatch);
        }
        if authoritative_metadata.parser_config() != role.parser_config() {
            return Err(PmPublicSessionError::MetadataParserConfigMismatch);
        }
        let subscription = PmMarketSubscription::new(role.wire_scope().token())
            .to_json()
            .map_err(PmPublicSessionError::SubscriptionSerialization)?
            .into_bytes();
        Ok(Self {
            role,
            authoritative_metadata,
            subscription,
            connection_epoch,
            last_snapshot_revision: last_snapshot_revision.map_or(0, SnapshotRevision::value),
            heartbeat_config,
            attempt: AttemptState::new(),
            last_fault: None,
            pending_unavailable: None,
            unavailable_required: false,
            metadata_occurrence_issued: false,
            supervisor: SupervisorState::new(reconnect_policy),
        })
    }

    #[must_use]
    pub const fn role(&self) -> PmPublicRole {
        self.role
    }

    #[must_use]
    pub const fn authoritative_metadata(&self) -> PmAuthoritativeMetadata {
        self.authoritative_metadata
    }

    #[must_use]
    pub const fn metadata_revision(&self) -> SnapshotRevision {
        self.authoritative_metadata.event().metadata_revision()
    }

    #[must_use]
    pub const fn configuration_fingerprint(&self) -> PmConfigurationFingerprint {
        self.role.observation_grant().configuration_fingerprint()
    }

    #[must_use]
    pub fn subscription_bytes(&self) -> &[u8] {
        &self.subscription
    }

    #[must_use]
    pub const fn ping_bytes(&self) -> &'static [u8] {
        PM_PUBLIC_PING_BYTES
    }

    #[must_use]
    pub const fn connection_epoch(&self) -> ConnectionEpoch {
        self.connection_epoch
    }

    #[must_use]
    pub const fn health(&self) -> ConnectionStatusKind {
        self.supervisor.health()
    }

    /// Whether this venue protocol may accept deltas for the current snapshot.
    ///
    /// This is connection flow state, not quote/product readiness.
    #[must_use]
    pub const fn protocol_flow_open(&self) -> bool {
        self.attempt.flow_open
    }

    #[must_use]
    pub const fn subscription_sent(&self) -> bool {
        self.attempt.subscription_sent
    }

    #[must_use]
    pub const fn requires_reconnect(&self) -> bool {
        self.attempt.requires_reconnect
    }

    #[must_use]
    pub const fn current_snapshot_revision(&self) -> Option<SnapshotRevision> {
        self.attempt.current_snapshot_revision
    }

    #[must_use]
    pub const fn local_ingress_sequence(&self) -> IngressSequence {
        IngressSequence::new(self.attempt.local_ingress_sequence)
    }

    #[must_use]
    pub const fn last_fault(&self) -> Option<PmPublicSessionFault> {
        self.last_fault
    }

    /// Mints the sole ordering occurrence for authoritative metadata.
    ///
    /// Metadata uses the authority's original monotonic observation time, but
    /// shares the attempt's ingress counter with all connection deliveries.
    /// The counter is committed only after the complete occurrence validates.
    pub fn issue_metadata_occurrence(
        &mut self,
        local_wall_receive_ns: u64,
    ) -> Result<PmPublicMetadataOccurrence, PmPublicSessionError> {
        self.ensure_live_attempt()?;
        if self.metadata_occurrence_issued {
            return Err(PmPublicSessionError::MetadataOccurrenceAlreadyIssued);
        }
        let next_ingress = self
            .attempt
            .local_ingress_sequence
            .checked_add(1)
            .ok_or(PmPublicSessionError::IngressSequenceOverflow)?;
        let clock = ReceivedEventClock::new(
            None,
            local_wall_receive_ns,
            self.authoritative_metadata.monotonic_receive_ns(),
        )
        .map_err(PmPublicSessionError::Envelope)?;
        let ordering = EventOrdering::new(
            self.connection_epoch,
            None,
            None,
            None,
            IngressSequence::new(next_ingress),
        )
        .map_err(PmPublicSessionError::Envelope)?;
        let occurrence = PmPublicMetadataOccurrence {
            source: self.role.source(),
            connection_id: self.role.connection(),
            received_clock: clock,
            ordering,
        };
        self.attempt.local_ingress_sequence = next_ingress;
        self.metadata_occurrence_issued = true;
        Ok(occurrence)
    }

    /// Consumes the exact next unavailable occurrence, if this attempt
    /// produced one with receive evidence.
    pub fn take_unavailable(&mut self) -> Option<PmPublicUnavailableOccurrence> {
        let occurrence = self.pending_unavailable.take();
        if occurrence.is_some() {
            self.unavailable_required = false;
        }
        occurrence
    }

    pub fn mark_subscription_sent(
        &mut self,
        monotonic_now_ns: u64,
    ) -> Result<(), PmPublicSessionError> {
        let result = self.try_mark_subscription_sent(monotonic_now_ns);
        if let Err(error) = result {
            self.invalidate_for_error(error);
        }
        result
    }

    pub fn preflight_mark_subscription_sent(
        &self,
        monotonic_now_ns: u64,
    ) -> Result<(), PmPublicSessionError> {
        self.validate_mark_subscription_sent(monotonic_now_ns)
    }

    fn try_mark_subscription_sent(
        &mut self,
        monotonic_now_ns: u64,
    ) -> Result<(), PmPublicSessionError> {
        self.validate_mark_subscription_sent(monotonic_now_ns)?;
        let next_ping_ns = monotonic_now_ns
            .checked_add(self.heartbeat_config.ping_interval_ns())
            .ok_or(PmPublicSessionError::HeartbeatDeadlineOverflow)?;
        self.attempt.subscription_sent = true;
        self.attempt.last_monotonic_ns = Some(monotonic_now_ns);
        self.attempt.heartbeat.next_ping_ns = Some(next_ping_ns);
        Ok(())
    }

    fn validate_mark_subscription_sent(
        &self,
        monotonic_now_ns: u64,
    ) -> Result<(), PmPublicSessionError> {
        self.ensure_live_attempt()?;
        if self.attempt.subscription_sent {
            return Err(PmPublicSessionError::SubscriptionAlreadySent);
        }
        if monotonic_now_ns == 0 {
            return Err(PmPublicSessionError::ZeroMonotonicTimestamp);
        }
        if monotonic_now_ns < self.authoritative_metadata.monotonic_receive_ns() {
            return Err(PmPublicSessionError::MonotonicClockRegression);
        }
        monotonic_now_ns
            .checked_add(self.heartbeat_config.ping_interval_ns())
            .ok_or(PmPublicSessionError::HeartbeatDeadlineOverflow)?;
        Ok(())
    }

    pub fn classify(
        &mut self,
        raw: &[u8],
        local_wall_receive_ns: u64,
        monotonic_receive_ns: u64,
    ) -> Result<PmPublicSessionBatch, PmPublicSessionError> {
        let result = self.classify_inner(raw, local_wall_receive_ns, monotonic_receive_ns);
        if let Err(error) = result {
            self.invalidate_for_error_with_receive_evidence(
                error,
                local_wall_receive_ns,
                monotonic_receive_ns,
            );
        }
        result
    }

    fn classify_inner(
        &mut self,
        raw: &[u8],
        local_wall_receive_ns: u64,
        monotonic_receive_ns: u64,
    ) -> Result<PmPublicSessionBatch, PmPublicSessionError> {
        self.ensure_subscribed()?;
        self.validate_receive_clock(local_wall_receive_ns, monotonic_receive_ns)?;
        if let Some(deadline_ns) = self.attempt.heartbeat.pong_deadline_ns
            && monotonic_receive_ns >= deadline_ns
        {
            return Err(PmPublicSessionError::HeartbeatTimeout { deadline_ns });
        }
        if raw == PM_PUBLIC_PONG_BYTES {
            return self.receive_pong(local_wall_receive_ns, monotonic_receive_ns);
        }

        let frame =
            parse_ws_frame(raw, self.role.parser_config()).map_err(PmPublicSessionError::Wire)?;
        let translated =
            self.translate_frame(frame.events(), local_wall_receive_ns, monotonic_receive_ns)?;
        if !self.attempt.requires_reconnect {
            self.attempt.last_monotonic_ns = Some(monotonic_receive_ns);
        }
        Ok(translated)
    }

    fn translate_frame(
        &mut self,
        wire_events: &[PmWsEvent],
        local_wall_receive_ns: u64,
        monotonic_receive_ns: u64,
    ) -> Result<PmPublicSessionBatch, PmPublicSessionError> {
        let normalized_count = wire_events
            .iter()
            .filter(|event| !matches!(event, PmWsEvent::Ignored(_)))
            .count();
        if normalized_count > 1
            && wire_events
                .iter()
                .any(|event| matches!(event, PmWsEvent::TickSizeChange(_)))
        {
            return Err(PmPublicSessionError::TickSizeChangeMustBeSoleEvent);
        }
        let mut scratch = TranslationState::new(self);
        let mut events = Vec::with_capacity(wire_events.len());
        let mut ignored = Vec::with_capacity(wire_events.len());
        let mut emitted_snapshot = None;
        let mut terminal_tick_change = false;

        for wire_event in wire_events {
            match wire_event {
                PmWsEvent::BookSnapshot(snapshot) => {
                    let token = self.translate_snapshot(
                        &mut scratch,
                        snapshot,
                        local_wall_receive_ns,
                        monotonic_receive_ns,
                        &mut events,
                    )?;
                    emitted_snapshot = Some(token);
                }
                PmWsEvent::PriceChanges(batch) => self.translate_delta(
                    &mut scratch,
                    batch,
                    local_wall_receive_ns,
                    monotonic_receive_ns,
                    &mut events,
                )?,
                PmWsEvent::BestBidAsk(top) => self.translate_top(
                    &mut scratch,
                    *top,
                    local_wall_receive_ns,
                    monotonic_receive_ns,
                    &mut events,
                )?,
                PmWsEvent::TickSizeChange(change) => {
                    self.translate_tick_change(
                        &mut scratch,
                        *change,
                        local_wall_receive_ns,
                        monotonic_receive_ns,
                        &mut events,
                    )?;
                    terminal_tick_change = true;
                }
                PmWsEvent::Ignored(PmIgnoredEvent::PublicTrade) => {
                    ignored.push(PmPublicSessionIgnored::PublicTrade);
                }
            }
        }

        self.last_snapshot_revision = scratch.last_snapshot_revision;
        self.attempt.local_ingress_sequence = scratch.local_ingress_sequence;
        self.attempt.current_snapshot_revision = scratch.current_snapshot_revision;
        self.attempt.pending_snapshot_flow = scratch.pending_snapshot_flow;
        self.attempt.flow_open = scratch.flow_open;
        if emitted_snapshot.is_some() {
            self.supervisor.mark_disconnected();
        }
        let translated = PmPublicSessionBatch::data(events, ignored, emitted_snapshot);
        if terminal_tick_change {
            self.invalidate_with_receive_evidence(
                PmPublicSessionFault::TickSizeChanged,
                local_wall_receive_ns,
                monotonic_receive_ns,
            )?;
        }
        Ok(translated)
    }

    fn translate_snapshot(
        &self,
        scratch: &mut TranslationState,
        snapshot: &reap_polymarket_wire::PmBookSnapshot,
        local_wall_receive_ns: u64,
        monotonic_receive_ns: u64,
        events: &mut Vec<PmPublicBookDelivery>,
    ) -> Result<PmSnapshotFlowToken, PmPublicSessionError> {
        if scratch.pending_snapshot_flow.is_some() {
            return Err(PmPublicSessionError::SnapshotFlowPending);
        }
        if !events.is_empty() {
            return Err(PmPublicSessionError::SnapshotMustBeFirst);
        }
        let next_revision = scratch
            .last_snapshot_revision
            .checked_add(1)
            .ok_or(PmPublicSessionError::SnapshotRevisionOverflow)?;
        let revision = SnapshotRevision::new(next_revision);
        let venue_hash = VenueEventHash::sha1(snapshot.verified_hash().bytes())
            .map_err(PmPublicSessionError::Envelope)?;
        let mut levels = Vec::with_capacity(snapshot.bids().len() + snapshot.asks().len());
        levels.extend(snapshot.bids().iter().map(|level| level.level()));
        levels.extend(snapshot.asks().iter().map(|level| level.level()));
        let update = PmBookUpdate::Snapshot(
            CoreBookSnapshot::new(levels.into_boxed_slice())
                .map_err(PmPublicSessionError::Event)?,
        );
        let (envelope, ingress) = self.normalized_envelope(
            scratch,
            revision,
            Some(venue_hash),
            snapshot.timestamp_millis(),
            local_wall_receive_ns,
            monotonic_receive_ns,
            update,
        )?;
        let token = PmSnapshotFlowToken {
            connection_epoch: self.connection_epoch,
            snapshot_revision: revision,
            local_ingress_sequence: ingress,
            venue_hash,
        };
        scratch.last_snapshot_revision = next_revision;
        scratch.current_snapshot_revision = Some(revision);
        scratch.pending_snapshot_flow = Some(token);
        scratch.flow_open = false;
        events.push(envelope);
        Ok(token)
    }

    fn translate_delta(
        &self,
        scratch: &mut TranslationState,
        batch: &reap_polymarket_wire::PmPriceChangeBatch,
        local_wall_receive_ns: u64,
        monotonic_receive_ns: u64,
        events: &mut Vec<PmPublicBookDelivery>,
    ) -> Result<(), PmPublicSessionError> {
        let revision = scratch.flow_revision()?;
        let changes = batch
            .changes()
            .iter()
            .map(|change| change.level())
            .collect::<Vec<_>>();
        let venue_change_hashes = batch
            .changes()
            .iter()
            .map(|change| {
                change
                    .transaction_hash()
                    .map(PmVenueChangeHash::new)
                    .transpose()
            })
            .collect::<Result<Vec<_>, _>>()
            .map_err(PmPublicSessionError::Event)?;
        let final_prices = batch.final_best_prices();
        let expected_top = PmBookTopCheck::new(Some(final_prices.bid()), Some(final_prices.ask()));
        let update = PmBookUpdate::DeltaBatch(
            PmBookDeltaBatch::new_with_venue_hashes(
                changes.into_boxed_slice(),
                venue_change_hashes.into_boxed_slice(),
                expected_top,
            )
            .map_err(PmPublicSessionError::Event)?,
        );
        let (envelope, _) = self.normalized_envelope(
            scratch,
            revision,
            None,
            batch.timestamp_millis(),
            local_wall_receive_ns,
            monotonic_receive_ns,
            update,
        )?;
        events.push(envelope);
        Ok(())
    }

    fn translate_top(
        &self,
        scratch: &mut TranslationState,
        top: reap_polymarket_wire::PmBestBidAsk,
        local_wall_receive_ns: u64,
        monotonic_receive_ns: u64,
        events: &mut Vec<PmPublicBookDelivery>,
    ) -> Result<(), PmPublicSessionError> {
        let revision = scratch.flow_revision()?;
        let prices = top.prices();
        let update =
            PmBookUpdate::TopCheck(PmBookTopCheck::new(Some(prices.bid()), Some(prices.ask())));
        let (envelope, _) = self.normalized_envelope(
            scratch,
            revision,
            None,
            top.timestamp_millis(),
            local_wall_receive_ns,
            monotonic_receive_ns,
            update,
        )?;
        events.push(envelope);
        Ok(())
    }

    fn translate_tick_change(
        &self,
        scratch: &mut TranslationState,
        change: reap_polymarket_wire::PmTickSizeChange,
        local_wall_receive_ns: u64,
        monotonic_receive_ns: u64,
        events: &mut Vec<PmPublicBookDelivery>,
    ) -> Result<(), PmPublicSessionError> {
        let revision = scratch.flow_revision()?;
        let update = PmBookUpdate::TickSizeChanged {
            old: change.old_tick(),
            new: change.new_tick(),
        };
        let (mut envelope, _) = self.normalized_envelope(
            scratch,
            revision,
            None,
            change.timestamp_millis(),
            local_wall_receive_ns,
            monotonic_receive_ns,
            update,
        )?;
        envelope.disposition = PmPublicBookDisposition::TerminalTickSizeChange;
        scratch.flow_open = false;
        events.push(envelope);
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn normalized_envelope(
        &self,
        scratch: &mut TranslationState,
        snapshot_revision: SnapshotRevision,
        venue_hash: Option<VenueEventHash>,
        timestamp_millis: u64,
        local_wall_receive_ns: u64,
        monotonic_receive_ns: u64,
        update: PmBookUpdate,
    ) -> Result<(PmPublicBookDelivery, IngressSequence), PmPublicSessionError> {
        let next_ingress = scratch
            .local_ingress_sequence
            .checked_add(1)
            .ok_or(PmPublicSessionError::IngressSequenceOverflow)?;
        let ingress = IngressSequence::new(next_ingress);
        let payload = PmBookEvent::new(
            self.role.source(),
            self.role.instrument(),
            self.metadata_revision(),
            update,
        )
        .map_err(PmPublicSessionError::Event)?;
        let clock = ReceivedEventClock::new(
            Some(venue_timestamp_ns(timestamp_millis)?),
            local_wall_receive_ns,
            monotonic_receive_ns,
        )
        .map_err(PmPublicSessionError::Envelope)?;
        let ordering = EventOrdering::new(
            self.connection_epoch,
            Some(snapshot_revision),
            None,
            venue_hash,
            ingress,
        )
        .map_err(PmPublicSessionError::Envelope)?;
        let envelope = ReceivedEventEnvelope::new(
            self.role.source().venue(),
            self.role.source(),
            self.role.connection(),
            clock,
            ordering,
            payload,
        )
        .map_err(PmPublicSessionError::Envelope)?;
        scratch.local_ingress_sequence = next_ingress;
        Ok((
            PmPublicBookDelivery {
                envelope,
                attempt_connection_epoch: self.connection_epoch,
                disposition: PmPublicBookDisposition::Normal,
            },
            ingress,
        ))
    }

    /// Opens only the venue protocol flow after the caller has synchronously
    /// applied the exact correlated snapshot to the PM reducer.
    ///
    /// This method does not establish product readiness. The reducer's typed
    /// readiness remains the sole authority for quote decisions.
    pub fn open_protocol_flow_after_snapshot(
        &mut self,
        token: PmSnapshotFlowToken,
    ) -> Result<(), PmPublicSessionError> {
        let result = self.try_open_protocol_flow_after_snapshot(token);
        if let Err(error) = result {
            self.invalidate_for_error(error);
        }
        result
    }

    fn try_open_protocol_flow_after_snapshot(
        &mut self,
        token: PmSnapshotFlowToken,
    ) -> Result<(), PmPublicSessionError> {
        self.ensure_subscribed()?;
        match self.attempt.pending_snapshot_flow {
            None => return Err(PmPublicSessionError::NoSnapshotFlowPending),
            Some(expected) if expected != token => {
                return Err(PmPublicSessionError::SnapshotFlowTokenMismatch);
            }
            Some(_) => {}
        }
        self.attempt.pending_snapshot_flow = None;
        self.attempt.flow_open = true;
        self.attempt.reached_flow_open = true;
        self.supervisor.mark_ready();
        Ok(())
    }

    pub fn reject_snapshot_flow(
        &mut self,
        token: PmSnapshotFlowToken,
    ) -> Result<(), PmPublicSessionError> {
        if self.attempt.pending_snapshot_flow != Some(token) {
            self.invalidate(PmPublicSessionFault::InvalidTransition);
            return Err(PmPublicSessionError::SnapshotFlowTokenMismatch);
        }
        self.invalidate(PmPublicSessionFault::ReducerRejected);
        Ok(())
    }

    /// Rejects a reducer snapshot with the receive evidence required to emit
    /// one fail-closed unavailable occurrence before reconnect.
    pub fn reject_snapshot_flow_with_receive_evidence(
        &mut self,
        token: PmSnapshotFlowToken,
        local_wall_receive_ns: u64,
        monotonic_receive_ns: u64,
    ) -> Result<(), PmPublicSessionError> {
        let (fault, result) = if self.attempt.pending_snapshot_flow != Some(token) {
            (
                PmPublicSessionFault::InvalidTransition,
                Err(PmPublicSessionError::SnapshotFlowTokenMismatch),
            )
        } else {
            (PmPublicSessionFault::ReducerRejected, Ok(()))
        };
        self.invalidate_with_receive_evidence(fault, local_wall_receive_ns, monotonic_receive_ns)?;
        result
    }

    pub fn poll_heartbeat(
        &mut self,
        monotonic_now_ns: u64,
    ) -> Result<PmPublicHeartbeatAction, PmPublicSessionError> {
        let result = self.poll_heartbeat_inner(monotonic_now_ns);
        if let Err(error) = result {
            self.invalidate_for_error(error);
        }
        result
    }

    /// Polls heartbeat state with a local wall clock so timeout invalidation
    /// can emit one evidence-bearing unavailable occurrence.
    pub fn poll_heartbeat_with_receive_evidence(
        &mut self,
        local_wall_now_ns: u64,
        monotonic_now_ns: u64,
    ) -> Result<PmPublicHeartbeatAction, PmPublicSessionError> {
        let result = self.poll_heartbeat_inner(monotonic_now_ns);
        if let Err(error) = result {
            self.invalidate_for_error_with_receive_evidence(
                error,
                local_wall_now_ns,
                monotonic_now_ns,
            );
        }
        result
    }

    /// Previews the exact heartbeat transition without mutating session state.
    ///
    /// Active composition owners use this to preflight and durably record the
    /// corresponding lifecycle event before enacting the session transition.
    /// In particular, an `Idle` preview does not advance the monotonic clock,
    /// and a timeout preview does not invalidate the session or consume its
    /// one unavailable occurrence.
    pub fn preview_heartbeat(
        &self,
        monotonic_now_ns: u64,
    ) -> Result<PmPublicHeartbeatAction, PmPublicSessionError> {
        self.ensure_subscribed()?;
        self.validate_monotonic(monotonic_now_ns)?;
        if let Some(deadline_ns) = self.attempt.heartbeat.pong_deadline_ns {
            return if monotonic_now_ns >= deadline_ns {
                Err(PmPublicSessionError::HeartbeatTimeout { deadline_ns })
            } else {
                Ok(PmPublicHeartbeatAction::Idle)
            };
        }
        let next_ping_ns = self
            .attempt
            .heartbeat
            .next_ping_ns
            .ok_or(PmPublicSessionError::InvalidHeartbeatState)?;
        if monotonic_now_ns < next_ping_ns {
            return Ok(PmPublicHeartbeatAction::Idle);
        }
        monotonic_now_ns
            .checked_add(self.heartbeat_config.pong_timeout_ns())
            .ok_or(PmPublicSessionError::HeartbeatDeadlineOverflow)?;
        Ok(PmPublicHeartbeatAction::SendPing)
    }

    fn poll_heartbeat_inner(
        &mut self,
        monotonic_now_ns: u64,
    ) -> Result<PmPublicHeartbeatAction, PmPublicSessionError> {
        let action = self.preview_heartbeat(monotonic_now_ns)?;
        self.attempt.last_monotonic_ns = Some(monotonic_now_ns);
        if action == PmPublicHeartbeatAction::Idle {
            return Ok(action);
        }
        let deadline_ns = monotonic_now_ns
            .checked_add(self.heartbeat_config.pong_timeout_ns())
            .ok_or(PmPublicSessionError::HeartbeatDeadlineOverflow)?;
        self.attempt.heartbeat.next_ping_ns = None;
        self.attempt.heartbeat.pong_deadline_ns = Some(deadline_ns);
        Ok(action)
    }

    fn receive_pong(
        &mut self,
        local_wall_receive_ns: u64,
        monotonic_receive_ns: u64,
    ) -> Result<PmPublicSessionBatch, PmPublicSessionError> {
        let deadline_ns = self
            .attempt
            .heartbeat
            .pong_deadline_ns
            .ok_or(PmPublicSessionError::UnexpectedPong)?;
        if monotonic_receive_ns >= deadline_ns {
            return Err(PmPublicSessionError::HeartbeatTimeout { deadline_ns });
        }
        let next_ping_ns = monotonic_receive_ns
            .checked_add(self.heartbeat_config.ping_interval_ns())
            .ok_or(PmPublicSessionError::HeartbeatDeadlineOverflow)?;
        self.attempt.heartbeat.pong_deadline_ns = None;
        self.attempt.heartbeat.next_ping_ns = Some(next_ping_ns);
        self.attempt.last_monotonic_ns = Some(monotonic_receive_ns);
        Ok(PmPublicSessionBatch::from_heartbeat(
            PmPublicHeartbeatEvidence {
                connection_epoch: self.connection_epoch,
                local_wall_receive_ns,
                monotonic_receive_ns,
            },
        ))
    }

    pub fn after_failure(&mut self) -> Result<Duration, PmPublicSessionError> {
        if self.health() == ConnectionStatusKind::Fatal {
            return Err(PmPublicSessionError::SessionFatal);
        }
        if self.attempt.requires_reconnect && self.unavailable_required {
            return if self.pending_unavailable.is_some() {
                Err(PmPublicSessionError::UnavailableOccurrencePending)
            } else {
                Err(PmPublicSessionError::UnavailableOccurrenceMissing)
            };
        }
        let Some(next_epoch) = self.connection_epoch.value().checked_add(1) else {
            self.attempt.subscription_sent = false;
            self.attempt.requires_reconnect = true;
            self.attempt.flow_open = false;
            self.attempt.current_snapshot_revision = None;
            self.attempt.pending_snapshot_flow = None;
            self.attempt.heartbeat = HeartbeatState::disconnected();
            self.last_fault = Some(PmPublicSessionFault::Overflow);
            self.supervisor.mark_fatal();
            return Err(PmPublicSessionError::ConnectionEpochOverflow);
        };
        let reached_flow_open = self.attempt.reached_flow_open;
        self.connection_epoch = ConnectionEpoch::new(next_epoch);
        self.attempt = AttemptState::new();
        self.last_fault = None;
        self.pending_unavailable = None;
        self.unavailable_required = false;
        Ok(self.supervisor.after_failure(reached_flow_open))
    }

    pub fn preview_after_failure(
        &self,
    ) -> Result<(ConnectionEpoch, Duration), PmPublicSessionError> {
        if self.health() == ConnectionStatusKind::Fatal {
            return Err(PmPublicSessionError::SessionFatal);
        }
        if self.attempt.requires_reconnect && self.unavailable_required {
            return if self.pending_unavailable.is_some() {
                Err(PmPublicSessionError::UnavailableOccurrencePending)
            } else {
                Err(PmPublicSessionError::UnavailableOccurrenceMissing)
            };
        }
        let next_epoch = self
            .connection_epoch
            .value()
            .checked_add(1)
            .ok_or(PmPublicSessionError::ConnectionEpochOverflow)?;
        Ok((
            ConnectionEpoch::new(next_epoch),
            self.supervisor
                .preview_after_failure(self.attempt.reached_flow_open),
        ))
    }

    pub fn preflight_invalidate_with_receive_evidence(
        &self,
        local_wall_receive_ns: u64,
        monotonic_receive_ns: u64,
    ) -> Result<(), PmPublicSessionError> {
        if self.pending_unavailable.is_some() {
            return Ok(());
        }
        ReceivedEventClock::new(None, local_wall_receive_ns, monotonic_receive_ns)
            .map_err(PmPublicSessionError::Envelope)?;
        self.validate_monotonic(monotonic_receive_ns)?;
        self.attempt
            .local_ingress_sequence
            .checked_add(1)
            .ok_or(PmPublicSessionError::IngressSequenceOverflow)?;
        Ok(())
    }

    /// Invalidates the attempt and records one session-sequenced unavailable
    /// occurrence. Connection faults never claim a venue timestamp.
    pub fn invalidate_with_receive_evidence(
        &mut self,
        fault: PmPublicSessionFault,
        local_wall_receive_ns: u64,
        monotonic_receive_ns: u64,
    ) -> Result<(), PmPublicSessionError> {
        if self.pending_unavailable.is_none() {
            let clock = ReceivedEventClock::new(None, local_wall_receive_ns, monotonic_receive_ns)
                .map_err(PmPublicSessionError::Envelope)?;
            self.validate_monotonic(monotonic_receive_ns)?;
            let next_ingress = self
                .attempt
                .local_ingress_sequence
                .checked_add(1)
                .ok_or(PmPublicSessionError::IngressSequenceOverflow)?;
            let ordering = EventOrdering::new(
                self.connection_epoch,
                None,
                None,
                None,
                IngressSequence::new(next_ingress),
            )
            .map_err(PmPublicSessionError::Envelope)?;
            self.attempt.local_ingress_sequence = next_ingress;
            self.attempt.last_monotonic_ns = Some(monotonic_receive_ns);
            self.pending_unavailable = Some(PmPublicUnavailableOccurrence {
                source: self.role.source(),
                connection_id: self.role.connection(),
                received_clock: clock,
                ordering,
                fault,
            });
        }
        self.invalidate(fault);
        Ok(())
    }

    pub fn invalidate(&mut self, fault: PmPublicSessionFault) {
        if !self.attempt.requires_reconnect {
            self.unavailable_required = true;
        }
        self.last_fault = Some(fault);
        self.attempt.subscription_sent = false;
        self.attempt.requires_reconnect = true;
        self.attempt.flow_open = false;
        self.attempt.current_snapshot_revision = None;
        self.attempt.pending_snapshot_flow = None;
        self.attempt.heartbeat = HeartbeatState::disconnected();
        if self.health() != ConnectionStatusKind::Fatal {
            self.supervisor.mark_disconnected();
        }
    }

    fn invalidate_for_error(&mut self, error: PmPublicSessionError) {
        if matches!(
            error,
            PmPublicSessionError::SessionFatal | PmPublicSessionError::ReconnectRequired
        ) {
            return;
        }
        self.invalidate(error.fault());
    }

    fn invalidate_for_error_with_receive_evidence(
        &mut self,
        error: PmPublicSessionError,
        local_wall_receive_ns: u64,
        monotonic_receive_ns: u64,
    ) {
        if matches!(
            error,
            PmPublicSessionError::SessionFatal | PmPublicSessionError::ReconnectRequired
        ) {
            return;
        }
        let fault = error.fault();
        if self
            .invalidate_with_receive_evidence(fault, local_wall_receive_ns, monotonic_receive_ns)
            .is_err()
        {
            self.invalidate(fault);
        }
    }

    fn ensure_live_attempt(&self) -> Result<(), PmPublicSessionError> {
        if self.health() == ConnectionStatusKind::Fatal {
            return Err(PmPublicSessionError::SessionFatal);
        }
        if self.attempt.requires_reconnect {
            return Err(PmPublicSessionError::ReconnectRequired);
        }
        Ok(())
    }

    fn ensure_subscribed(&self) -> Result<(), PmPublicSessionError> {
        self.ensure_live_attempt()?;
        if !self.attempt.subscription_sent {
            return Err(PmPublicSessionError::SubscriptionNotSent);
        }
        Ok(())
    }

    fn validate_receive_clock(
        &self,
        local_wall_receive_ns: u64,
        monotonic_receive_ns: u64,
    ) -> Result<(), PmPublicSessionError> {
        if local_wall_receive_ns == 0 {
            return Err(PmPublicSessionError::ZeroWallReceiveTimestamp);
        }
        self.validate_monotonic(monotonic_receive_ns)
    }

    fn validate_monotonic(&self, monotonic_ns: u64) -> Result<(), PmPublicSessionError> {
        if monotonic_ns == 0 {
            return Err(PmPublicSessionError::ZeroMonotonicTimestamp);
        }
        if self
            .attempt
            .last_monotonic_ns
            .is_some_and(|previous| monotonic_ns < previous)
        {
            return Err(PmPublicSessionError::MonotonicClockRegression);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy)]
struct TranslationState {
    last_snapshot_revision: u64,
    local_ingress_sequence: u64,
    current_snapshot_revision: Option<SnapshotRevision>,
    pending_snapshot_flow: Option<PmSnapshotFlowToken>,
    flow_open: bool,
}

impl TranslationState {
    const fn new(session: &PmPublicSession) -> Self {
        Self {
            last_snapshot_revision: session.last_snapshot_revision,
            local_ingress_sequence: session.attempt.local_ingress_sequence,
            current_snapshot_revision: session.attempt.current_snapshot_revision,
            pending_snapshot_flow: session.attempt.pending_snapshot_flow,
            flow_open: session.attempt.flow_open,
        }
    }

    fn flow_revision(self) -> Result<SnapshotRevision, PmPublicSessionError> {
        if !self.flow_open || self.pending_snapshot_flow.is_some() {
            return Err(PmPublicSessionError::DataBeforeSnapshotFlowOpen);
        }
        self.current_snapshot_revision
            .ok_or(PmPublicSessionError::DataBeforeSnapshotFlowOpen)
    }
}

fn venue_timestamp_ns(timestamp_millis: u64) -> Result<u64, PmPublicSessionError> {
    timestamp_millis
        .checked_mul(1_000_000)
        .ok_or(PmPublicSessionError::VenueTimestampOverflow)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum PmPublicSessionError {
    #[error("connection epoch must be nonzero")]
    ZeroConnectionEpoch,
    #[error("restored snapshot revision must be nonzero when present")]
    ZeroLastSnapshotRevision,
    #[error("authoritative metadata instrument does not match the public role")]
    MetadataInstrumentMismatch,
    #[error("authoritative metadata source does not match the public role")]
    MetadataSourceMismatch,
    #[error("authoritative metadata parser contract does not match the public role")]
    MetadataParserConfigMismatch,
    #[error("the authoritative metadata occurrence was already issued")]
    MetadataOccurrenceAlreadyIssued,
    #[error("heartbeat ping interval must be nonzero")]
    ZeroPingInterval,
    #[error("heartbeat pong timeout must be nonzero")]
    ZeroPongTimeout,
    #[error("subscription serialization failed: {0}")]
    SubscriptionSerialization(PmWireError),
    #[error("the one-token subscription was already sent for this attempt")]
    SubscriptionAlreadySent,
    #[error("the one-token subscription has not been sent for this attempt")]
    SubscriptionNotSent,
    #[error("the session requires reconnect before further protocol input")]
    ReconnectRequired,
    #[error("the pending unavailable occurrence must be consumed before reconnect")]
    UnavailableOccurrencePending,
    #[error("the reconnect-required attempt lacks receive evidence for its unavailable occurrence")]
    UnavailableOccurrenceMissing,
    #[error("session is terminal")]
    SessionFatal,
    #[error("public Polymarket wire failure: {0}")]
    Wire(PmWireError),
    #[error("normalized PM event construction failed: {0}")]
    Event(PmEventError),
    #[error("normalized PM envelope construction failed: {0}")]
    Envelope(EnvelopeError),
    #[error("local wall receive timestamp must be nonzero")]
    ZeroWallReceiveTimestamp,
    #[error("monotonic timestamp must be nonzero")]
    ZeroMonotonicTimestamp,
    #[error("monotonic clock regressed")]
    MonotonicClockRegression,
    #[error("venue millisecond timestamp cannot be represented in nanoseconds")]
    VenueTimestampOverflow,
    #[error("local ingress sequence overflowed")]
    IngressSequenceOverflow,
    #[error("local snapshot revision overflowed")]
    SnapshotRevisionOverflow,
    #[error("connection epoch overflowed")]
    ConnectionEpochOverflow,
    #[error("book delta or BBO arrived before the reduced snapshot opened protocol flow")]
    DataBeforeSnapshotFlowOpen,
    #[error("a previous snapshot still awaits protocol-flow acknowledgement after reduction")]
    SnapshotFlowPending,
    #[error("a snapshot must precede normalized data in its frame")]
    SnapshotMustBeFirst,
    #[error("a tick-size change must be the sole normalized event in its frame")]
    TickSizeChangeMustBeSoleEvent,
    #[error("no snapshot protocol-flow token is pending")]
    NoSnapshotFlowPending,
    #[error("snapshot protocol-flow token does not match the exact pending snapshot")]
    SnapshotFlowTokenMismatch,
    #[error("heartbeat deadline arithmetic overflowed")]
    HeartbeatDeadlineOverflow,
    #[error("heartbeat PONG deadline {deadline_ns}ns expired")]
    HeartbeatTimeout { deadline_ns: u64 },
    #[error("PONG arrived without an outstanding venue PING")]
    UnexpectedPong,
    #[error("heartbeat lifecycle state is incomplete")]
    InvalidHeartbeatState,
}

impl PmPublicSessionError {
    const fn fault(self) -> PmPublicSessionFault {
        match self {
            Self::ConnectionEpochOverflow
            | Self::SnapshotRevisionOverflow
            | Self::IngressSequenceOverflow
            | Self::VenueTimestampOverflow
            | Self::HeartbeatDeadlineOverflow => PmPublicSessionFault::Overflow,
            Self::HeartbeatTimeout { .. } => PmPublicSessionFault::HeartbeatTimeout,
            Self::Wire(PmWireError::SnapshotHashMismatch { .. }) => {
                PmPublicSessionFault::HashMismatch
            }
            _ => PmPublicSessionFault::InvalidTransition,
        }
    }
}
