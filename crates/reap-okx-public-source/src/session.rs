use std::time::Duration;

use reap_core::{Channel, ConnId, RawEnvelope, Venue};
use reap_transport::{
    ConnectionStatusKind, ImmutableDelivery, RawDelivery, ReconnectPolicy, SupervisionChannels,
    SupervisorState, supervision_channels,
};
use serde_json::Value;
use thiserror::Error;

use crate::public_wire::{DecodedPublicFrame, PublicWireError, decode_public_frame};
use crate::reference::{
    OkxIndexTickerReference, OkxIndexTickerReferenceError, configured_reference_from_wire,
};
use crate::subscription::{
    OKX_INDEX_TICKERS_CHANNEL, OkxIndexTickerSubscription, OkxIndexTickerSubscriptionError,
};

/// One event classified by a configured OKX public session.
///
/// Connection identity and epoch remain attached to the immutable delivery so
/// downstream product adapters can prove the exact configured route instead
/// of trusting a caller-stamped source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OkxPublicSessionDelivery {
    delivery: ImmutableDelivery<OkxPublicSessionEvent>,
    connection_id: ConnId,
    connection_epoch: u64,
    local_ingress_sequence: u64,
}

impl OkxPublicSessionDelivery {
    #[must_use]
    pub const fn payload(&self) -> &OkxPublicSessionEvent {
        self.delivery.payload()
    }

    #[must_use]
    pub const fn monotonic_receive_ns(&self) -> u64 {
        self.delivery.monotonic_receive_ns()
    }

    #[must_use]
    pub fn connection_id(&self) -> &str {
        self.connection_id.0.as_str()
    }

    #[must_use]
    pub const fn connection_epoch(&self) -> u64 {
        self.connection_epoch
    }

    #[must_use]
    pub const fn local_ingress_sequence(&self) -> u64 {
        self.local_ingress_sequence
    }

    #[must_use]
    pub fn into_payload(self) -> OkxPublicSessionEvent {
        self.delivery.into_payload()
    }
}

pub type OkxPublicSessionChannels =
    SupervisionChannels<OkxPublicSessionDelivery, ConnectionStatusKind>;

pub const MAX_OKX_PUBLIC_CONNECTION_ID_BYTES: usize = 128;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OkxPublicSessionEvent {
    SubscriptionAcknowledged(OkxPublicEventEvidence),
    Heartbeat(OkxPublicEventEvidence),
    Control(OkxPublicControlEvidence),
    Reference(OkxIndexTickerReference),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OkxPublicSessionFault {
    Disconnect,
    Overflow,
    Stale,
    InvalidTransition,
}

/// One session-sequenced OKX connection-unavailable occurrence.
///
/// The occurrence is intentionally non-cloneable. Its receive timestamps and
/// local ingress sequence are allocated by the session at invalidation, and a
/// connection fault never carries a venue timestamp.
#[derive(Debug, PartialEq, Eq)]
pub struct OkxPublicUnavailableOccurrence {
    connection_id: ConnId,
    connection_epoch: u64,
    wall_receive_ts_ns: u64,
    monotonic_receive_ns: u64,
    local_ingress_sequence: u64,
    fault: OkxPublicSessionFault,
}

impl OkxPublicUnavailableOccurrence {
    #[must_use]
    pub fn connection_id(&self) -> &str {
        self.connection_id.0.as_str()
    }

    #[must_use]
    pub const fn connection_epoch(&self) -> u64 {
        self.connection_epoch
    }

    #[must_use]
    pub const fn wall_receive_ts_ns(&self) -> u64 {
        self.wall_receive_ts_ns
    }

    #[must_use]
    pub const fn monotonic_receive_ns(&self) -> u64 {
        self.monotonic_receive_ns
    }

    #[must_use]
    pub const fn local_ingress_sequence(&self) -> u64 {
        self.local_ingress_sequence
    }

    #[must_use]
    pub const fn fault(&self) -> OkxPublicSessionFault {
        self.fault
    }
}

/// Replay identity common to non-data protocol events.
///
/// The enclosing [`OkxPublicSessionDelivery`] carries the checked monotonic
/// receive time; this value retains the independent wall receive time, raw
/// identity, and connection epoch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OkxPublicEventEvidence {
    wall_receive_ts_ns: u64,
    connection_epoch: u64,
    raw_hash: u64,
}

impl OkxPublicEventEvidence {
    #[must_use]
    pub const fn wall_receive_ts_ns(&self) -> u64 {
        self.wall_receive_ts_ns
    }

    #[must_use]
    pub const fn connection_epoch(&self) -> u64 {
        self.connection_epoch
    }

    #[must_use]
    pub const fn raw_hash(&self) -> u64 {
        self.raw_hash
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OkxPublicControlEvidence {
    source: OkxPublicEventEvidence,
    connection_count: u64,
    connection_id: String,
}

impl OkxPublicControlEvidence {
    #[must_use]
    pub const fn source(&self) -> &OkxPublicEventEvidence {
        &self.source
    }

    #[must_use]
    pub const fn connection_count(&self) -> u64 {
        self.connection_count
    }

    #[must_use]
    pub fn connection_id(&self) -> &str {
        &self.connection_id
    }
}

#[derive(Debug)]
pub struct OkxPublicSession {
    subscription: OkxIndexTickerSubscription,
    connection_id: ConnId,
    connection_epoch: u64,
    acknowledged: bool,
    acknowledged_this_attempt: bool,
    requires_reconnect: bool,
    last_fault: Option<OkxPublicSessionFault>,
    local_ingress_sequence: u64,
    last_monotonic_receive_ns: Option<u64>,
    pending_unavailable: Option<OkxPublicUnavailableOccurrence>,
    unavailable_required: bool,
    supervisor: SupervisorState,
}

impl OkxPublicSession {
    pub fn new(
        instrument: impl AsRef<str>,
        connection_id: ConnId,
        connection_epoch: u64,
        reconnect_policy: ReconnectPolicy,
    ) -> Result<Self, OkxPublicSessionError> {
        validate_connection_id(&connection_id)?;
        if connection_epoch == 0 {
            return Err(OkxPublicSessionError::ZeroConnectionEpoch);
        }
        Ok(Self {
            subscription: OkxIndexTickerSubscription::new(instrument)?,
            connection_id,
            connection_epoch,
            acknowledged: false,
            acknowledged_this_attempt: false,
            requires_reconnect: false,
            last_fault: None,
            local_ingress_sequence: 0,
            last_monotonic_receive_ns: None,
            pending_unavailable: None,
            unavailable_required: false,
            supervisor: SupervisorState::new(reconnect_policy),
        })
    }

    /// Constructs the narrow configured public-reference session used by
    /// public capture/replay callers without exposing the legacy raw
    /// envelope connection-id type across the PM product boundary.
    pub fn new_configured_capture(
        instrument: impl AsRef<str>,
        connection_id: &str,
        connection_epoch: u64,
        reconnect_policy: ReconnectPolicy,
    ) -> Result<Self, OkxPublicSessionError> {
        Self::new(
            instrument,
            ConnId::new(connection_id),
            connection_epoch,
            reconnect_policy,
        )
    }

    #[must_use]
    pub fn subscription(&self) -> &OkxIndexTickerSubscription {
        &self.subscription
    }

    #[must_use]
    pub fn subscription_bytes(&self) -> &[u8] {
        self.subscription.wire_bytes()
    }

    #[must_use]
    pub const fn connection_id(&self) -> &ConnId {
        &self.connection_id
    }

    #[must_use]
    pub const fn connection_epoch(&self) -> u64 {
        self.connection_epoch
    }

    #[must_use]
    pub const fn health(&self) -> ConnectionStatusKind {
        self.supervisor.health()
    }

    #[must_use]
    pub const fn subscription_ready(&self) -> bool {
        self.acknowledged
    }

    #[must_use]
    pub const fn requires_reconnect(&self) -> bool {
        self.requires_reconnect
    }

    #[must_use]
    pub const fn last_fault(&self) -> Option<OkxPublicSessionFault> {
        self.last_fault
    }

    /// Consumes the exact next unavailable occurrence, if this attempt
    /// produced one with receive evidence.
    pub fn take_unavailable(&mut self) -> Option<OkxPublicUnavailableOccurrence> {
        let occurrence = self.pending_unavailable.take();
        if occurrence.is_some() {
            self.unavailable_required = false;
        }
        occurrence
    }

    #[must_use]
    pub fn bounded_channels(requested_capacity: usize) -> OkxPublicSessionChannels {
        supervision_channels(requested_capacity)
    }

    pub fn classify(
        &mut self,
        delivery: RawDelivery,
    ) -> Result<OkxPublicSessionDelivery, OkxPublicSessionError> {
        if self.health() == ConnectionStatusKind::Fatal {
            return Err(OkxPublicSessionError::SessionFatal);
        }
        if self.requires_reconnect {
            return Err(OkxPublicSessionError::ReconnectRequired);
        }
        let wall_receive_ts_ns = delivery.payload().recv_ts_ns;
        let monotonic_receive_ns = delivery.monotonic_receive_ns();
        if self
            .last_monotonic_receive_ns
            .is_some_and(|previous| monotonic_receive_ns < previous)
        {
            self.invalidate_with_receive_evidence_or_fallback(
                OkxPublicSessionFault::InvalidTransition,
                wall_receive_ts_ns,
                monotonic_receive_ns,
            );
            return Err(OkxPublicSessionError::MonotonicClockRegression);
        }
        let Some(local_ingress_sequence) = self.local_ingress_sequence.checked_add(1) else {
            self.invalidate(OkxPublicSessionFault::Overflow);
            return Err(OkxPublicSessionError::IngressSequenceOverflow);
        };
        let connection_id = self.connection_id.clone();
        let connection_epoch = self.connection_epoch;
        let result = delivery.try_map(|envelope| self.classify_envelope(envelope));
        if result.is_err() {
            self.invalidate_with_receive_evidence_or_fallback(
                OkxPublicSessionFault::InvalidTransition,
                wall_receive_ts_ns,
                monotonic_receive_ns,
            );
        } else {
            self.local_ingress_sequence = local_ingress_sequence;
            self.last_monotonic_receive_ns = Some(monotonic_receive_ns);
        }
        result.map(|delivery| OkxPublicSessionDelivery {
            delivery,
            connection_id,
            connection_epoch,
            local_ingress_sequence,
        })
    }

    /// Classifies one exact captured public text payload against this
    /// session's already checked route.
    ///
    /// Capture/replay callers provide only immutable receive evidence; venue,
    /// connection, channel, and instrument identity are reconstructed from
    /// the configured session and cannot be forged through a broad raw
    /// envelope constructor.
    pub fn classify_captured_payload(
        &mut self,
        payload: &str,
        wall_receive_ts_ns: u64,
        monotonic_receive_ns: u64,
        raw_hash: u64,
    ) -> Result<OkxPublicSessionDelivery, OkxPublicSessionError> {
        let envelope = RawEnvelope {
            venue: Venue::Okx,
            conn_id: self.connection_id.clone(),
            channel: Channel::Custom(OKX_INDEX_TICKERS_CHANNEL.to_string()),
            symbol: Some(self.subscription.instrument().to_string()),
            recv_ts_ns: wall_receive_ts_ns,
            raw_hash,
            payload: payload.to_string(),
        };
        let delivery = match RawDelivery::new(envelope, monotonic_receive_ns) {
            Ok(delivery) => delivery,
            Err(error) => {
                self.invalidate_with_receive_evidence_or_fallback(
                    OkxPublicSessionFault::InvalidTransition,
                    wall_receive_ts_ns,
                    monotonic_receive_ns,
                );
                return Err(OkxPublicSessionError::Wire(error.to_string()));
            }
        };
        self.classify(delivery)
    }

    fn classify_envelope(
        &mut self,
        envelope: RawEnvelope,
    ) -> Result<OkxPublicSessionEvent, OkxPublicSessionError> {
        self.validate_envelope(&envelope)?;
        match decode_public_frame(&envelope).map_err(OkxPublicSessionError::from_wire)? {
            DecodedPublicFrame::Heartbeat => {
                // A transport heartbeat is liveness evidence, not subscription
                // readiness. Preserve the pre-ack disconnected state, and
                // preserve ready only after the exact configured ACK.
                Ok(OkxPublicSessionEvent::Heartbeat(
                    self.event_evidence(&envelope),
                ))
            }
            DecodedPublicFrame::Acknowledgement { code, arg } => {
                if self.acknowledged {
                    return Err(OkxPublicSessionError::UnexpectedAcknowledgement);
                }
                if !successful_code(code.as_ref()) {
                    return Err(OkxPublicSessionError::InvalidAcknowledgement);
                }
                let Some(arg) = arg else {
                    return Err(OkxPublicSessionError::InvalidAcknowledgement);
                };
                if arg.channel != OKX_INDEX_TICKERS_CHANNEL
                    || arg.instrument.as_deref() != Some(self.subscription.instrument())
                {
                    return Err(OkxPublicSessionError::InvalidAcknowledgement);
                }
                self.acknowledged = true;
                self.acknowledged_this_attempt = true;
                self.supervisor.mark_ready();
                Ok(OkxPublicSessionEvent::SubscriptionAcknowledged(
                    self.event_evidence(&envelope),
                ))
            }
            DecodedPublicFrame::ConnectionCount {
                channel,
                connection_count,
                connection_id,
            } => {
                if channel != OKX_INDEX_TICKERS_CHANNEL {
                    return Err(OkxPublicSessionError::WrongControlChannel);
                }
                Ok(OkxPublicSessionEvent::Control(OkxPublicControlEvidence {
                    source: self.event_evidence(&envelope),
                    connection_count,
                    connection_id,
                }))
            }
            DecodedPublicFrame::RejectedControl {
                event,
                code,
                message,
            } => Err(OkxPublicSessionError::ServerControl {
                event,
                code: value_text(code.as_ref()),
                message,
            }),
            DecodedPublicFrame::StateChangingControl { event } => {
                Err(OkxPublicSessionError::StateChangingControl { event })
            }
            DecodedPublicFrame::Data { arg, values } => {
                if !self.acknowledged {
                    return Err(OkxPublicSessionError::DataBeforeAcknowledgement);
                }
                let reference = configured_reference_from_wire(
                    &envelope,
                    self.subscription.instrument(),
                    self.connection_epoch,
                    arg,
                    values,
                )?;
                self.supervisor.mark_ready();
                Ok(OkxPublicSessionEvent::Reference(reference))
            }
        }
    }

    pub fn after_failure(&mut self) -> Result<Duration, OkxPublicSessionError> {
        if self.health() == ConnectionStatusKind::Fatal {
            return Err(OkxPublicSessionError::SessionFatal);
        }
        if self.requires_reconnect && self.unavailable_required {
            return if self.pending_unavailable.is_some() {
                Err(OkxPublicSessionError::UnavailableOccurrencePending)
            } else {
                Err(OkxPublicSessionError::UnavailableOccurrenceMissing)
            };
        }
        let Some(next_epoch) = self.connection_epoch.checked_add(1) else {
            self.last_fault = Some(OkxPublicSessionFault::Overflow);
            self.requires_reconnect = true;
            self.mark_fatal();
            return Err(OkxPublicSessionError::ConnectionEpochOverflow);
        };
        let reached_ready = self.acknowledged_this_attempt;
        self.acknowledged = false;
        self.acknowledged_this_attempt = false;
        self.requires_reconnect = false;
        self.last_fault = None;
        self.local_ingress_sequence = 0;
        self.last_monotonic_receive_ns = None;
        self.pending_unavailable = None;
        self.unavailable_required = false;
        self.connection_epoch = next_epoch;
        Ok(self.supervisor.after_failure(reached_ready))
    }

    pub fn preview_after_failure(&self) -> Result<(u64, Duration), OkxPublicSessionError> {
        if self.health() == ConnectionStatusKind::Fatal {
            return Err(OkxPublicSessionError::SessionFatal);
        }
        if self.requires_reconnect && self.unavailable_required {
            return if self.pending_unavailable.is_some() {
                Err(OkxPublicSessionError::UnavailableOccurrencePending)
            } else {
                Err(OkxPublicSessionError::UnavailableOccurrenceMissing)
            };
        }
        let next_epoch = self
            .connection_epoch
            .checked_add(1)
            .ok_or(OkxPublicSessionError::ConnectionEpochOverflow)?;
        Ok((
            next_epoch,
            self.supervisor
                .preview_after_failure(self.acknowledged_this_attempt),
        ))
    }

    pub fn mark_fatal(&mut self) {
        self.acknowledged = false;
        self.acknowledged_this_attempt = false;
        self.supervisor.mark_fatal();
    }

    pub fn invalidate(&mut self, fault: OkxPublicSessionFault) {
        if !self.requires_reconnect {
            self.unavailable_required = true;
        }
        self.acknowledged = false;
        self.requires_reconnect = true;
        self.last_fault = Some(fault);
        if self.health() != ConnectionStatusKind::Fatal {
            self.supervisor.mark_disconnected();
        }
    }

    /// Invalidates the attempt and records one session-sequenced unavailable
    /// occurrence. Connection faults never claim a venue timestamp.
    pub fn invalidate_with_receive_evidence(
        &mut self,
        fault: OkxPublicSessionFault,
        wall_receive_ts_ns: u64,
        monotonic_receive_ns: u64,
    ) -> Result<(), OkxPublicSessionError> {
        if wall_receive_ts_ns == 0 {
            return Err(OkxPublicSessionError::ZeroWallReceiveTimestamp);
        }
        if monotonic_receive_ns == 0 {
            return Err(OkxPublicSessionError::ZeroMonotonicReceiveTimestamp);
        }
        if self
            .last_monotonic_receive_ns
            .is_some_and(|previous| monotonic_receive_ns < previous)
        {
            return Err(OkxPublicSessionError::MonotonicClockRegression);
        }
        if self.pending_unavailable.is_none() {
            let local_ingress_sequence = self
                .local_ingress_sequence
                .checked_add(1)
                .ok_or(OkxPublicSessionError::IngressSequenceOverflow)?;
            self.local_ingress_sequence = local_ingress_sequence;
            self.last_monotonic_receive_ns = Some(monotonic_receive_ns);
            self.pending_unavailable = Some(OkxPublicUnavailableOccurrence {
                connection_id: self.connection_id.clone(),
                connection_epoch: self.connection_epoch,
                wall_receive_ts_ns,
                monotonic_receive_ns,
                local_ingress_sequence,
                fault,
            });
        }
        self.invalidate(fault);
        Ok(())
    }

    pub fn preflight_invalidate_with_receive_evidence(
        &self,
        wall_receive_ts_ns: u64,
        monotonic_receive_ns: u64,
    ) -> Result<(), OkxPublicSessionError> {
        if wall_receive_ts_ns == 0 {
            return Err(OkxPublicSessionError::ZeroWallReceiveTimestamp);
        }
        if monotonic_receive_ns == 0 {
            return Err(OkxPublicSessionError::ZeroMonotonicReceiveTimestamp);
        }
        if self
            .last_monotonic_receive_ns
            .is_some_and(|previous| monotonic_receive_ns < previous)
        {
            return Err(OkxPublicSessionError::MonotonicClockRegression);
        }
        if self.pending_unavailable.is_none() {
            self.local_ingress_sequence
                .checked_add(1)
                .ok_or(OkxPublicSessionError::IngressSequenceOverflow)?;
        }
        Ok(())
    }

    fn invalidate_with_receive_evidence_or_fallback(
        &mut self,
        fault: OkxPublicSessionFault,
        wall_receive_ts_ns: u64,
        monotonic_receive_ns: u64,
    ) {
        if self
            .invalidate_with_receive_evidence(fault, wall_receive_ts_ns, monotonic_receive_ns)
            .is_err()
        {
            self.invalidate(fault);
        }
    }

    fn validate_envelope(&self, envelope: &RawEnvelope) -> Result<(), OkxPublicSessionError> {
        if envelope.venue != Venue::Okx {
            return Err(OkxPublicSessionError::WrongEnvelopeVenue);
        }
        if envelope.conn_id != self.connection_id {
            return Err(OkxPublicSessionError::WrongEnvelopeConnectionId);
        }
        if !matches!(
            &envelope.channel,
            Channel::Custom(channel) if channel == OKX_INDEX_TICKERS_CHANNEL
        ) {
            return Err(OkxPublicSessionError::WrongEnvelopeChannel);
        }
        if envelope.symbol.as_deref() != Some(self.subscription.instrument()) {
            return Err(OkxPublicSessionError::WrongEnvelopeInstrument);
        }
        if envelope.recv_ts_ns == 0 {
            return Err(OkxPublicSessionError::ZeroWallReceiveTimestamp);
        }
        Ok(())
    }

    const fn event_evidence(&self, envelope: &RawEnvelope) -> OkxPublicEventEvidence {
        OkxPublicEventEvidence {
            wall_receive_ts_ns: envelope.recv_ts_ns,
            connection_epoch: self.connection_epoch,
            raw_hash: envelope.raw_hash,
        }
    }
}

fn validate_connection_id(connection_id: &ConnId) -> Result<(), OkxPublicSessionError> {
    let value = connection_id.0.as_str();
    if value.is_empty() {
        return Err(OkxPublicSessionError::EmptyConnectionId);
    }
    if value.len() > MAX_OKX_PUBLIC_CONNECTION_ID_BYTES {
        return Err(OkxPublicSessionError::ConnectionIdTooLong);
    }
    if !value
        .bytes()
        .all(|byte| byte.is_ascii_graphic() && !matches!(byte, b'\\' | b'"'))
    {
        return Err(OkxPublicSessionError::InvalidConnectionId);
    }
    Ok(())
}

fn successful_code(code: Option<&Value>) -> bool {
    match code {
        None => true,
        Some(Value::String(value)) => value == "0",
        Some(_) => false,
    }
}

fn value_text(value: Option<&Value>) -> String {
    match value {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(value)) => value.clone(),
        Some(value) => value.to_string(),
    }
}

#[derive(Debug, Error)]
pub enum OkxPublicSessionError {
    #[error("configured connection id is empty")]
    EmptyConnectionId,
    #[error("configured connection id exceeds its fixed byte bound")]
    ConnectionIdTooLong,
    #[error("configured connection id must use non-quoted printable ASCII")]
    InvalidConnectionId,
    #[error("connection epoch must be positive")]
    ZeroConnectionEpoch,
    #[error("connection epoch cannot advance beyond u64::MAX")]
    ConnectionEpochOverflow,
    #[error("public session is terminal and cannot accept another attempt")]
    SessionFatal,
    #[error("the public session requires reconnect before further protocol input")]
    ReconnectRequired,
    #[error("the pending unavailable occurrence must be consumed before reconnect")]
    UnavailableOccurrencePending,
    #[error("the reconnect-required attempt lacks receive evidence for its unavailable occurrence")]
    UnavailableOccurrenceMissing,
    #[error("monotonic receive timestamp must be positive")]
    ZeroMonotonicReceiveTimestamp,
    #[error("monotonic receive clock regressed")]
    MonotonicClockRegression,
    #[error("local ingress sequence overflowed")]
    IngressSequenceOverflow,
    #[error("envelope venue is not OKX")]
    WrongEnvelopeVenue,
    #[error("envelope connection id does not match the configured session")]
    WrongEnvelopeConnectionId,
    #[error("envelope channel is not configured index-tickers")]
    WrongEnvelopeChannel,
    #[error("envelope instrument is not the configured index instrument")]
    WrongEnvelopeInstrument,
    #[error("envelope wall receive timestamp must be positive")]
    ZeroWallReceiveTimestamp,
    #[error("data arrived before the exact subscription acknowledgement")]
    DataBeforeAcknowledgement,
    #[error("subscription acknowledgement is invalid")]
    InvalidAcknowledgement,
    #[error("subscription acknowledgement was not expected")]
    UnexpectedAcknowledgement,
    #[error("control frame names a different channel")]
    WrongControlChannel,
    #[error("state-changing server control event {event:?} invalidated the configured source")]
    StateChangingControl { event: &'static str },
    #[error("server control event {event:?} code={code:?} message={message:?}")]
    ServerControl {
        event: &'static str,
        code: String,
        message: String,
    },
    #[error("invalid public frame: {0}")]
    Wire(String),
    #[error(transparent)]
    Subscription(#[from] OkxIndexTickerSubscriptionError),
    #[error(transparent)]
    Reference(#[from] OkxIndexTickerReferenceError),
}

impl OkxPublicSessionError {
    fn from_wire(error: PublicWireError) -> Self {
        Self::Wire(error.to_string())
    }
}
