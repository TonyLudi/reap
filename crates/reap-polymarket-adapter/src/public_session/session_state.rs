use reap_pm_core::{PmMarketEvent, SnapshotRevision};
use reap_transport::ConnectionStatusKind;

use crate::{PmAuthoritativeMetadata, PmRecordedMetadataEvidence};

use super::{PmPublicSession, PmPublicSessionError, PmSnapshotFlowToken};

#[derive(Debug, Clone, Copy)]
pub(super) struct HeartbeatState {
    pub(super) next_ping_ns: Option<u64>,
    pub(super) pong_deadline_ns: Option<u64>,
}

impl HeartbeatState {
    pub(super) const fn disconnected() -> Self {
        Self {
            next_ping_ns: None,
            pong_deadline_ns: None,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(super) struct AttemptState {
    pub(super) subscription_sent: bool,
    pub(super) requires_reconnect: bool,
    pub(super) flow_open: bool,
    pub(super) reached_flow_open: bool,
    pub(super) local_ingress_sequence: u64,
    pub(super) current_snapshot_revision: Option<SnapshotRevision>,
    pub(super) pending_snapshot_flow: Option<PmSnapshotFlowToken>,
    pub(super) last_monotonic_ns: Option<u64>,
    pub(super) heartbeat: HeartbeatState,
}

impl AttemptState {
    pub(super) const fn new() -> Self {
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
pub(super) enum PmPublicSessionMetadata {
    Live(PmAuthoritativeMetadata),
    Recorded(PmRecordedMetadataEvidence),
}

impl PmPublicSessionMetadata {
    pub(super) const fn event(&self) -> PmMarketEvent {
        match self {
            Self::Live(metadata) => metadata.event(),
            Self::Recorded(metadata) => metadata.event(),
        }
    }

    pub(super) const fn parser_config(&self) -> reap_polymarket_wire::PmBookParserConfig {
        match self {
            Self::Live(metadata) => metadata.parser_config(),
            Self::Recorded(metadata) => metadata.parser_config(),
        }
    }

    pub(super) const fn monotonic_receive_ns(&self) -> u64 {
        match self {
            Self::Live(metadata) => metadata.monotonic_receive_ns(),
            Self::Recorded(metadata) => metadata.monotonic_receive_ns(),
        }
    }

    pub(super) const fn is_live(&self) -> bool {
        matches!(self, Self::Live(_))
    }
}

#[derive(Debug, Clone, Copy)]
pub(super) struct TranslationState {
    pub(super) last_snapshot_revision: u64,
    pub(super) local_ingress_sequence: u64,
    pub(super) current_snapshot_revision: Option<SnapshotRevision>,
    pub(super) pending_snapshot_flow: Option<PmSnapshotFlowToken>,
    pub(super) flow_open: bool,
}

impl TranslationState {
    pub(super) const fn new(session: &PmPublicSession) -> Self {
        Self {
            last_snapshot_revision: session.last_snapshot_revision,
            local_ingress_sequence: session.attempt.local_ingress_sequence,
            current_snapshot_revision: session.attempt.current_snapshot_revision,
            pending_snapshot_flow: session.attempt.pending_snapshot_flow,
            flow_open: session.attempt.flow_open,
        }
    }

    pub(super) fn flow_revision(self) -> Result<SnapshotRevision, PmPublicSessionError> {
        if !self.flow_open || self.pending_snapshot_flow.is_some() {
            return Err(PmPublicSessionError::DataBeforeSnapshotFlowOpen);
        }
        self.current_snapshot_revision
            .ok_or(PmPublicSessionError::DataBeforeSnapshotFlowOpen)
    }
}

pub(super) fn venue_timestamp_ns(timestamp_millis: u64) -> Result<u64, PmPublicSessionError> {
    timestamp_millis
        .checked_mul(1_000_000)
        .ok_or(PmPublicSessionError::VenueTimestampOverflow)
}

impl PmPublicSession {
    pub(super) fn invalidate_for_error(&mut self, error: PmPublicSessionError) {
        if matches!(
            error,
            PmPublicSessionError::SessionFatal | PmPublicSessionError::ReconnectRequired
        ) {
            return;
        }
        self.invalidate(error.fault());
    }

    pub(super) fn invalidate_for_error_with_receive_evidence(
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

    pub(super) fn ensure_live_attempt(&self) -> Result<(), PmPublicSessionError> {
        if self.health() == ConnectionStatusKind::Fatal {
            return Err(PmPublicSessionError::SessionFatal);
        }
        if self.attempt.requires_reconnect {
            return Err(PmPublicSessionError::ReconnectRequired);
        }
        Ok(())
    }

    pub(super) fn ensure_subscribed(&self) -> Result<(), PmPublicSessionError> {
        self.ensure_live_attempt()?;
        if !self.attempt.subscription_sent {
            return Err(PmPublicSessionError::SubscriptionNotSent);
        }
        Ok(())
    }

    pub(super) fn validate_receive_clock(
        &self,
        local_wall_receive_ns: u64,
        monotonic_receive_ns: u64,
    ) -> Result<(), PmPublicSessionError> {
        if local_wall_receive_ns == 0 {
            return Err(PmPublicSessionError::ZeroWallReceiveTimestamp);
        }
        self.validate_monotonic(monotonic_receive_ns)
    }

    pub(super) fn validate_monotonic(&self, monotonic_ns: u64) -> Result<(), PmPublicSessionError> {
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
