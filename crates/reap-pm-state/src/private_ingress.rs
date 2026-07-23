#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PmPrivateExternalIngressLane {
    Reconnect,
    PrivateLifecycle,
    AccountSnapshot,
    OpenOrders,
    OrderDetail,
    Reconciliation,
}

impl PmPrivateExternalIngressLane {
    const fn index(self) -> usize {
        match self {
            Self::Reconnect => 0,
            Self::PrivateLifecycle => 1,
            Self::AccountSnapshot => 2,
            Self::OpenOrders => 3,
            Self::OrderDetail => 4,
            Self::Reconciliation => 5,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PmPrivateExternalIngressFailure {
    Normalization,
    Service,
    Scope,
    Contract,
}

impl PmPrivateExternalIngressFailure {
    const fn index(self) -> usize {
        match self {
            Self::Normalization => 0,
            Self::Service => 1,
            Self::Scope => 2,
            Self::Contract => 3,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PmPrivateExternalIngressFault {
    lane: PmPrivateExternalIngressLane,
    failure: PmPrivateExternalIngressFailure,
}

impl PmPrivateExternalIngressFault {
    #[must_use]
    pub const fn new(
        lane: PmPrivateExternalIngressLane,
        failure: PmPrivateExternalIngressFailure,
    ) -> Self {
        Self { lane, failure }
    }

    #[must_use]
    pub const fn lane(self) -> PmPrivateExternalIngressLane {
        self.lane
    }

    #[must_use]
    pub const fn failure(self) -> PmPrivateExternalIngressFailure {
        self.failure
    }
}

/// Fixed-cardinality metrics for rejected data at the monitor/state boundary.
///
/// Counters saturate instead of wrapping, and their label sets are closed
/// enums rather than dynamically registered strings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PmPrivateExternalIngressCounters {
    total: u64,
    by_lane: [u64; 6],
    by_failure: [u64; 4],
}

impl PmPrivateExternalIngressCounters {
    pub(crate) fn record(&mut self, fault: PmPrivateExternalIngressFault) {
        self.total = self.total.saturating_add(1);
        let lane = fault.lane().index();
        self.by_lane[lane] = self.by_lane[lane].saturating_add(1);
        let failure = fault.failure().index();
        self.by_failure[failure] = self.by_failure[failure].saturating_add(1);
    }

    #[must_use]
    pub const fn total(self) -> u64 {
        self.total
    }

    #[must_use]
    pub const fn for_lane(self, lane: PmPrivateExternalIngressLane) -> u64 {
        self.by_lane[lane.index()]
    }

    #[must_use]
    pub const fn for_failure(self, failure: PmPrivateExternalIngressFailure) -> u64 {
        self.by_failure[failure.index()]
    }
}
