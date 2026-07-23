use reap_pm_live_contracts::PmCapabilityLane;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PmLaneKind {
    Critical,
    Persistence,
    Private,
    Scheduled,
    Public,
    Reconciliation,
    Telemetry,
    ReconciliationRequest,
    Capture,
    Journal,
    FakeEffect,
}

/// Frozen input-lane service order for the future complete scheduler.
///
/// Phase 3 materializes only `Public`; later phases must introduce the other
/// typed producers and the complete scheduler atomically in this order.
pub const PM_INPUT_SERVICE_PRIORITY: [PmLaneKind; 7] = [
    PmLaneKind::Critical,
    PmLaneKind::Persistence,
    PmLaneKind::Private,
    PmLaneKind::Scheduled,
    PmLaneKind::Public,
    PmLaneKind::Reconciliation,
    PmLaneKind::Telemetry,
];

impl PmLaneKind {
    #[must_use]
    pub const fn service_priority_rank(self) -> Option<u8> {
        match self {
            Self::Critical => Some(0),
            Self::Persistence => Some(1),
            Self::Private => Some(2),
            Self::Scheduled => Some(3),
            Self::Public => Some(4),
            Self::Reconciliation => Some(5),
            Self::Telemetry => Some(6),
            Self::ReconciliationRequest | Self::Capture | Self::Journal | Self::FakeEffect => None,
        }
    }
}

impl From<PmCapabilityLane> for PmLaneKind {
    fn from(lane: PmCapabilityLane) -> Self {
        match lane {
            PmCapabilityLane::Critical => Self::Critical,
            PmCapabilityLane::Persistence => Self::Persistence,
            PmCapabilityLane::Private => Self::Private,
            PmCapabilityLane::Scheduled => Self::Scheduled,
            PmCapabilityLane::Public => Self::Public,
            PmCapabilityLane::Reconciliation => Self::Reconciliation,
            PmCapabilityLane::Telemetry => Self::Telemetry,
            PmCapabilityLane::ReconciliationRequest => Self::ReconciliationRequest,
            PmCapabilityLane::Capture => Self::Capture,
            PmCapabilityLane::Journal => Self::Journal,
            PmCapabilityLane::FakeEffect => Self::FakeEffect,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SaturationAction {
    GlobalStop,
    HaltAccountAndRequireReconciliation,
    InvalidateStreamAndResync,
    KeepUnreadyAndRetry,
    RetainPendingRefresh,
    InvalidateCaptureAndResync,
    SuppressDispatchAndHaltQuotes,
    RejectEffectAndHaltQuotes,
    SuppressQuoteAndCancelOwned,
    CoalesceTelemetry,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PmLanePolicy {
    capacity: usize,
    nominal_high_water: usize,
    maximum_age_ns: Option<u64>,
    saturation_action: SaturationAction,
    service_burst: Option<usize>,
}

impl PmLanePolicy {
    #[must_use]
    pub const fn for_lane(lane: PmLaneKind) -> Self {
        match lane {
            PmLaneKind::Critical => Self::new(
                512,
                32,
                Some(250_000_000),
                SaturationAction::GlobalStop,
                Some(512),
            ),
            PmLaneKind::Persistence => Self::new(
                512,
                32,
                Some(250_000_000),
                SaturationAction::GlobalStop,
                Some(512),
            ),
            PmLaneKind::Private => Self::new(
                4_096,
                64,
                Some(250_000_000),
                SaturationAction::HaltAccountAndRequireReconciliation,
                Some(64),
            ),
            PmLaneKind::Scheduled => Self::new(
                4_096,
                64,
                Some(100_000_000),
                SaturationAction::SuppressQuoteAndCancelOwned,
                Some(16),
            ),
            PmLaneKind::Public => Self::new(
                8_192,
                256,
                Some(500_000_000),
                SaturationAction::InvalidateStreamAndResync,
                Some(256),
            ),
            PmLaneKind::Reconciliation => Self::new(
                128,
                16,
                Some(5_000_000_000),
                SaturationAction::KeepUnreadyAndRetry,
                Some(8),
            ),
            PmLaneKind::Telemetry => {
                Self::new(128, 32, None, SaturationAction::CoalesceTelemetry, Some(1))
            }
            PmLaneKind::ReconciliationRequest => Self::new(
                128,
                16,
                Some(1_000_000_000),
                SaturationAction::RetainPendingRefresh,
                None,
            ),
            PmLaneKind::Capture => Self::new(
                8_192,
                256,
                Some(500_000_000),
                SaturationAction::InvalidateCaptureAndResync,
                None,
            ),
            PmLaneKind::Journal => Self::new(
                1_024,
                128,
                Some(1_000_000_000),
                SaturationAction::SuppressDispatchAndHaltQuotes,
                None,
            ),
            PmLaneKind::FakeEffect => Self::new(
                256,
                32,
                Some(250_000_000),
                SaturationAction::RejectEffectAndHaltQuotes,
                None,
            ),
        }
    }

    const fn new(
        capacity: usize,
        nominal_high_water: usize,
        maximum_age_ns: Option<u64>,
        saturation_action: SaturationAction,
        service_burst: Option<usize>,
    ) -> Self {
        Self {
            capacity,
            nominal_high_water,
            maximum_age_ns,
            saturation_action,
            service_burst,
        }
    }

    #[must_use]
    pub const fn capacity(self) -> usize {
        self.capacity
    }

    #[must_use]
    pub const fn nominal_high_water(self) -> usize {
        self.nominal_high_water
    }

    #[must_use]
    pub const fn maximum_age_ns(self) -> Option<u64> {
        self.maximum_age_ns
    }

    #[must_use]
    pub const fn saturation_action(self) -> SaturationAction {
        self.saturation_action
    }

    #[must_use]
    pub const fn service_burst(self) -> Option<usize> {
        self.service_burst
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PmLaneMetrics {
    depth: usize,
    high_water: usize,
    rejected_full: u64,
    coalesced: u64,
    invalidated_purged: u64,
}

impl PmLaneMetrics {
    pub(super) const fn new(
        depth: usize,
        high_water: usize,
        rejected_full: u64,
        coalesced: u64,
        invalidated_purged: u64,
    ) -> Self {
        Self {
            depth,
            high_water,
            rejected_full,
            coalesced,
            invalidated_purged,
        }
    }

    #[must_use]
    pub const fn depth(self) -> usize {
        self.depth
    }

    #[must_use]
    pub const fn high_water(self) -> usize {
        self.high_water
    }

    #[must_use]
    pub const fn rejected_full(self) -> u64 {
        self.rejected_full
    }

    #[must_use]
    pub const fn coalesced(self) -> u64 {
        self.coalesced
    }

    #[must_use]
    pub const fn invalidated_purged(self) -> u64 {
        self.invalidated_purged
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frozen_input_priority_is_total_and_output_lanes_are_unranked() {
        for (expected, lane) in PM_INPUT_SERVICE_PRIORITY.into_iter().enumerate() {
            assert_eq!(
                lane.service_priority_rank(),
                Some(u8::try_from(expected).expect("seven ranks fit u8"))
            );
        }
        for lane in [
            PmLaneKind::ReconciliationRequest,
            PmLaneKind::Capture,
            PmLaneKind::Journal,
            PmLaneKind::FakeEffect,
        ] {
            assert_eq!(lane.service_priority_rank(), None);
        }
    }
}
