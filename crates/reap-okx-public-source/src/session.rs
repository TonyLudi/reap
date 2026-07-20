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

pub type OkxPublicSessionDelivery = ImmutableDelivery<OkxPublicSessionEvent>;
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
            supervisor: SupervisorState::new(reconnect_policy),
        })
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
        let result = delivery.try_map(|envelope| self.classify_envelope(envelope));
        if result.is_err() {
            self.invalidate_readiness();
        }
        result
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
        let Some(next_epoch) = self.connection_epoch.checked_add(1) else {
            self.mark_fatal();
            return Err(OkxPublicSessionError::ConnectionEpochOverflow);
        };
        let reached_ready = self.acknowledged_this_attempt;
        self.acknowledged = false;
        self.acknowledged_this_attempt = false;
        self.connection_epoch = next_epoch;
        Ok(self.supervisor.after_failure(reached_ready))
    }

    pub fn mark_fatal(&mut self) {
        self.acknowledged = false;
        self.acknowledged_this_attempt = false;
        self.supervisor.mark_fatal();
    }

    fn invalidate_readiness(&mut self) {
        self.acknowledged = false;
        if self.health() != ConnectionStatusKind::Fatal {
            self.supervisor.mark_disconnected();
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
