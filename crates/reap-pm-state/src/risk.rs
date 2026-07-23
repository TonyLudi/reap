use reap_pm_core::{
    PmAccountHandle, PmInstrumentHandle, PmOrderSide, PmQuantity, PmSign, PmSignedUnits, U256,
};
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PmOrderRiskLimits {
    max_quantity: PmQuantity,
    max_collateral: U256,
}

impl PmOrderRiskLimits {
    pub fn new(max_quantity: PmQuantity, max_collateral: U256) -> Result<Self, PmRiskLimitsError> {
        require_nonzero(max_collateral)?;
        Ok(Self {
            max_quantity,
            max_collateral,
        })
    }

    #[must_use]
    pub const fn max_quantity(self) -> PmQuantity {
        self.max_quantity
    }

    #[must_use]
    pub const fn max_collateral(self) -> U256 {
        self.max_collateral
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PmExposureRiskLimits {
    max_market_inventory: U256,
    max_account_collateral_at_risk: U256,
    max_reserved_collateral: U256,
    max_reserved_token: U256,
}

impl PmExposureRiskLimits {
    pub fn new(
        max_market_inventory: U256,
        max_account_collateral_at_risk: U256,
        max_reserved_collateral: U256,
        max_reserved_token: U256,
    ) -> Result<Self, PmRiskLimitsError> {
        for limit in [
            max_market_inventory,
            max_account_collateral_at_risk,
            max_reserved_collateral,
            max_reserved_token,
        ] {
            require_nonzero(limit)?;
        }
        Ok(Self {
            max_market_inventory,
            max_account_collateral_at_risk,
            max_reserved_collateral,
            max_reserved_token,
        })
    }

    #[must_use]
    pub const fn max_market_inventory(self) -> U256 {
        self.max_market_inventory
    }

    #[must_use]
    pub const fn max_account_collateral_at_risk(self) -> U256 {
        self.max_account_collateral_at_risk
    }

    #[must_use]
    pub const fn max_reserved_collateral(self) -> U256 {
        self.max_reserved_collateral
    }

    #[must_use]
    pub const fn max_reserved_token(self) -> U256 {
        self.max_reserved_token
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PmCardinalityRiskLimits {
    max_live_orders: u16,
    max_unresolved_orders: u16,
    max_unresolved_fills: u16,
}

impl PmCardinalityRiskLimits {
    pub fn new(
        max_live_orders: u16,
        max_unresolved_orders: u16,
        max_unresolved_fills: u16,
    ) -> Result<Self, PmRiskLimitsError> {
        if max_live_orders == 0 || max_unresolved_orders == 0 || max_unresolved_fills == 0 {
            return Err(PmRiskLimitsError::ZeroCardinality);
        }
        Ok(Self {
            max_live_orders,
            max_unresolved_orders,
            max_unresolved_fills,
        })
    }

    #[must_use]
    pub const fn max_live_orders(self) -> u16 {
        self.max_live_orders
    }

    #[must_use]
    pub const fn max_unresolved_orders(self) -> u16 {
        self.max_unresolved_orders
    }

    #[must_use]
    pub const fn max_unresolved_fills(self) -> u16 {
        self.max_unresolved_fills
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PmFreshnessRiskLimits {
    reference_ns: u64,
    book_ns: u64,
    private_ns: u64,
    account_ns: u64,
    order_ns: u64,
    reconciliation_ns: u64,
}

impl PmFreshnessRiskLimits {
    pub fn new(
        reference_ns: u64,
        book_ns: u64,
        private_ns: u64,
        account_ns: u64,
        order_ns: u64,
        reconciliation_ns: u64,
    ) -> Result<Self, PmRiskLimitsError> {
        if [
            reference_ns,
            book_ns,
            private_ns,
            account_ns,
            order_ns,
            reconciliation_ns,
        ]
        .contains(&0)
        {
            return Err(PmRiskLimitsError::ZeroFreshness);
        }
        Ok(Self {
            reference_ns,
            book_ns,
            private_ns,
            account_ns,
            order_ns,
            reconciliation_ns,
        })
    }

    #[must_use]
    pub const fn reference_ns(self) -> u64 {
        self.reference_ns
    }

    #[must_use]
    pub const fn book_ns(self) -> u64 {
        self.book_ns
    }

    #[must_use]
    pub const fn private_ns(self) -> u64 {
        self.private_ns
    }

    #[must_use]
    pub const fn account_ns(self) -> u64 {
        self.account_ns
    }

    #[must_use]
    pub const fn order_ns(self) -> u64 {
        self.order_ns
    }

    #[must_use]
    pub const fn reconciliation_ns(self) -> u64 {
        self.reconciliation_ns
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PmRiskLimits {
    order: PmOrderRiskLimits,
    exposure: PmExposureRiskLimits,
    cardinality: PmCardinalityRiskLimits,
    freshness: PmFreshnessRiskLimits,
}

impl PmRiskLimits {
    #[must_use]
    pub const fn new(
        order: PmOrderRiskLimits,
        exposure: PmExposureRiskLimits,
        cardinality: PmCardinalityRiskLimits,
        freshness: PmFreshnessRiskLimits,
    ) -> Self {
        Self {
            order,
            exposure,
            cardinality,
            freshness,
        }
    }

    #[must_use]
    pub const fn order(self) -> PmOrderRiskLimits {
        self.order
    }

    #[must_use]
    pub const fn exposure(self) -> PmExposureRiskLimits {
        self.exposure
    }

    #[must_use]
    pub const fn cardinality(self) -> PmCardinalityRiskLimits {
        self.cardinality
    }

    #[must_use]
    pub const fn freshness(self) -> PmFreshnessRiskLimits {
        self.freshness
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum PmRiskLimitsError {
    #[error("exact PM risk limits must be positive")]
    ZeroExactLimit,
    #[error("PM risk cardinality limits must be positive")]
    ZeroCardinality,
    #[error("PM risk freshness limits must be positive")]
    ZeroFreshness,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PmRiskDependency {
    available: bool,
    observed_monotonic_ns: Option<u64>,
}

impl PmRiskDependency {
    #[must_use]
    pub const fn available(observed_monotonic_ns: u64) -> Self {
        Self {
            available: true,
            observed_monotonic_ns: Some(observed_monotonic_ns),
        }
    }

    #[must_use]
    pub const fn unavailable() -> Self {
        Self {
            available: false,
            observed_monotonic_ns: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PmRiskDependencies {
    reference: PmRiskDependency,
    book: PmRiskDependency,
    private: PmRiskDependency,
    account: PmRiskDependency,
    orders: PmRiskDependency,
    reconciliation: PmRiskDependency,
}

impl PmRiskDependencies {
    #[must_use]
    pub const fn new(
        reference: PmRiskDependency,
        book: PmRiskDependency,
        private: PmRiskDependency,
        account: PmRiskDependency,
        orders: PmRiskDependency,
        reconciliation: PmRiskDependency,
    ) -> Self {
        Self {
            reference,
            book,
            private,
            account,
            orders,
            reconciliation,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PmRiskCandidate {
    account: PmAccountHandle,
    instrument: PmInstrumentHandle,
    side: PmOrderSide,
    quantity: PmQuantity,
    collateral_at_risk: U256,
    token_at_risk: U256,
}

impl PmRiskCandidate {
    #[must_use]
    pub const fn new(
        account: PmAccountHandle,
        instrument: PmInstrumentHandle,
        side: PmOrderSide,
        quantity: PmQuantity,
        collateral_at_risk: U256,
        token_at_risk: U256,
    ) -> Self {
        Self {
            account,
            instrument,
            side,
            quantity,
            collateral_at_risk,
            token_at_risk,
        }
    }

    #[must_use]
    pub const fn account(self) -> PmAccountHandle {
        self.account
    }

    #[must_use]
    pub const fn instrument(self) -> PmInstrumentHandle {
        self.instrument
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PmRiskExposure {
    market_inventory: PmSignedUnits,
    account_collateral_at_risk: U256,
    reserved_collateral: U256,
    reserved_token: U256,
    live_orders: u16,
    unresolved_orders: u16,
    unresolved_fills: u16,
}

impl PmRiskExposure {
    #[allow(clippy::too_many_arguments)]
    #[must_use]
    pub const fn new(
        market_inventory: PmSignedUnits,
        account_collateral_at_risk: U256,
        reserved_collateral: U256,
        reserved_token: U256,
        live_orders: u16,
        unresolved_orders: u16,
        unresolved_fills: u16,
    ) -> Self {
        Self {
            market_inventory,
            account_collateral_at_risk,
            reserved_collateral,
            reserved_token,
            live_orders,
            unresolved_orders,
            unresolved_fills,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PmRiskInput {
    monotonic_now_ns: u64,
    candidate: PmRiskCandidate,
    exposure: PmRiskExposure,
    dependencies: PmRiskDependencies,
}

impl PmRiskInput {
    #[must_use]
    pub const fn new(
        monotonic_now_ns: u64,
        candidate: PmRiskCandidate,
        exposure: PmRiskExposure,
        dependencies: PmRiskDependencies,
    ) -> Self {
        Self {
            monotonic_now_ns,
            candidate,
            exposure,
            dependencies,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PmRiskHaltScope {
    None,
    Market,
    Account,
    Global,
}

impl PmRiskHaltScope {
    #[must_use]
    pub const fn cancel_owned_required(self) -> bool {
        !matches!(self, Self::None)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PmRiskDependencyKind {
    Reference,
    Book,
    PrivateLifecycle,
    AccountSnapshot,
    OrderLifecycle,
    Reconciliation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PmRiskReason {
    GlobalHalt,
    AccountHalt,
    MarketHalt,
    DependencyUnavailable(PmRiskDependencyKind),
    DependencyStale {
        dependency: PmRiskDependencyKind,
        age_ns: u64,
        limit_ns: u64,
    },
    MonotonicClockRegression(PmRiskDependencyKind),
    OrderQuantity {
        observed: U256,
        limit: U256,
    },
    OrderCollateral {
        observed: U256,
        limit: U256,
    },
    MarketInventory {
        observed: U256,
        limit: U256,
    },
    AccountCollateralAtRisk {
        observed: U256,
        limit: U256,
    },
    ReservedCollateral {
        observed: U256,
        limit: U256,
    },
    ReservedToken {
        observed: U256,
        limit: U256,
    },
    LiveOrderCount {
        observed: u16,
        limit: u16,
    },
    UnresolvedOrderCount {
        observed: u16,
        limit: u16,
    },
    UnresolvedFillCount {
        observed: u16,
        limit: u16,
    },
    ArithmeticOverflow,
    ScopeMismatch,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PmRiskDecision {
    Approved {
        prospective_market_inventory: U256,
        prospective_account_collateral_at_risk: U256,
        prospective_reserved_collateral: U256,
        prospective_reserved_token: U256,
        prospective_live_orders: u16,
    },
    Rejected {
        reason: PmRiskReason,
        halt: PmRiskHaltScope,
    },
}

impl PmRiskDecision {
    #[must_use]
    pub const fn reason(self) -> Option<PmRiskReason> {
        match self {
            Self::Approved { .. } => None,
            Self::Rejected { reason, .. } => Some(reason),
        }
    }

    #[must_use]
    pub const fn halt_scope(self) -> PmRiskHaltScope {
        match self {
            Self::Approved { .. } => PmRiskHaltScope::None,
            Self::Rejected { halt, .. } => halt,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PmRiskCounters {
    evaluations: u64,
    approvals: u64,
    rejections: u64,
    market_halts: u64,
    account_halts: u64,
    global_halts: u64,
}

impl PmRiskCounters {
    #[must_use]
    pub const fn evaluations(self) -> u64 {
        self.evaluations
    }

    #[must_use]
    pub const fn approvals(self) -> u64 {
        self.approvals
    }

    #[must_use]
    pub const fn rejections(self) -> u64 {
        self.rejections
    }

    #[must_use]
    pub const fn market_halts(self) -> u64 {
        self.market_halts
    }

    #[must_use]
    pub const fn account_halts(self) -> u64 {
        self.account_halts
    }

    #[must_use]
    pub const fn global_halts(self) -> u64 {
        self.global_halts
    }
}

pub(crate) struct PmRiskState {
    account: PmAccountHandle,
    instrument: PmInstrumentHandle,
    limits: PmRiskLimits,
    market_halt: Option<PmRiskReason>,
    account_halt: Option<PmRiskReason>,
    global_halt: Option<PmRiskReason>,
    counters: PmRiskCounters,
}

impl PmRiskState {
    pub(crate) const fn new(
        account: PmAccountHandle,
        instrument: PmInstrumentHandle,
        limits: PmRiskLimits,
    ) -> Self {
        Self {
            account,
            instrument,
            limits,
            market_halt: None,
            account_halt: None,
            global_halt: None,
            counters: PmRiskCounters {
                evaluations: 0,
                approvals: 0,
                rejections: 0,
                market_halts: 0,
                account_halts: 0,
                global_halts: 0,
            },
        }
    }

    pub(crate) fn evaluate(&mut self, input: PmRiskInput) -> PmRiskDecision {
        self.counters.evaluations = self.counters.evaluations.saturating_add(1);
        let decision = self.check(input);
        match decision {
            PmRiskDecision::Approved { .. } => {
                self.counters.approvals = self.counters.approvals.saturating_add(1);
            }
            PmRiskDecision::Rejected { reason, halt } => {
                self.counters.rejections = self.counters.rejections.saturating_add(1);
                self.latch(halt, reason);
            }
        }
        decision
    }

    pub(crate) const fn counters(&self) -> PmRiskCounters {
        self.counters
    }

    pub(crate) const fn market_halt(&self) -> Option<PmRiskReason> {
        self.market_halt
    }

    pub(crate) const fn account_halt(&self) -> Option<PmRiskReason> {
        self.account_halt
    }

    pub(crate) const fn global_halt(&self) -> Option<PmRiskReason> {
        self.global_halt
    }

    pub(crate) fn recover_after_complete_reconciliation(&mut self) {
        self.market_halt = None;
        self.account_halt = None;
    }

    fn check(&self, input: PmRiskInput) -> PmRiskDecision {
        if input.candidate.account != self.account || input.candidate.instrument != self.instrument
        {
            return rejected(PmRiskReason::ScopeMismatch, PmRiskHaltScope::Global);
        }
        if let Some(decision) = check_dependencies(
            input.monotonic_now_ns,
            input.dependencies,
            self.limits.freshness,
        ) {
            return decision;
        }
        if self.global_halt.is_some() {
            return rejected(PmRiskReason::GlobalHalt, PmRiskHaltScope::Global);
        }
        if self.account_halt.is_some() {
            return rejected(PmRiskReason::AccountHalt, PmRiskHaltScope::Account);
        }
        if self.market_halt.is_some() {
            return rejected(PmRiskReason::MarketHalt, PmRiskHaltScope::Market);
        }
        if let Some(decision) = check_current_exposure(input.exposure, self.limits) {
            return decision;
        }

        let candidate_units = input.candidate.quantity.protocol_units();
        if candidate_units > self.limits.order.max_quantity.protocol_units() {
            return rejected(
                PmRiskReason::OrderQuantity {
                    observed: candidate_units,
                    limit: self.limits.order.max_quantity.protocol_units(),
                },
                PmRiskHaltScope::None,
            );
        }
        if input.candidate.collateral_at_risk > self.limits.order.max_collateral {
            return rejected(
                PmRiskReason::OrderCollateral {
                    observed: input.candidate.collateral_at_risk,
                    limit: self.limits.order.max_collateral,
                },
                PmRiskHaltScope::None,
            );
        }

        let candidate_delta = match input.candidate.side {
            PmOrderSide::Buy => PmSignedUnits::from_parts(PmSign::Positive, candidate_units)
                .expect("positive nonzero candidate quantity"),
            PmOrderSide::Sell => PmSignedUnits::from_parts(PmSign::Negative, candidate_units)
                .expect("negative nonzero candidate quantity"),
        };
        let Ok(market_inventory) = add_signed(input.exposure.market_inventory, candidate_delta)
        else {
            return rejected(PmRiskReason::ArithmeticOverflow, PmRiskHaltScope::Global);
        };
        let Ok(account_collateral) = input
            .exposure
            .account_collateral_at_risk
            .checked_add(input.candidate.collateral_at_risk)
        else {
            return rejected(PmRiskReason::ArithmeticOverflow, PmRiskHaltScope::Global);
        };
        let Ok(reserved_collateral) = input
            .exposure
            .reserved_collateral
            .checked_add(input.candidate.collateral_at_risk)
        else {
            return rejected(PmRiskReason::ArithmeticOverflow, PmRiskHaltScope::Global);
        };
        let Ok(reserved_token) = input
            .exposure
            .reserved_token
            .checked_add(input.candidate.token_at_risk)
        else {
            return rejected(PmRiskReason::ArithmeticOverflow, PmRiskHaltScope::Global);
        };
        let Some(live_orders) = input.exposure.live_orders.checked_add(1) else {
            return rejected(PmRiskReason::ArithmeticOverflow, PmRiskHaltScope::Global);
        };

        if market_inventory.magnitude() > self.limits.exposure.max_market_inventory {
            return rejected(
                PmRiskReason::MarketInventory {
                    observed: market_inventory.magnitude(),
                    limit: self.limits.exposure.max_market_inventory,
                },
                PmRiskHaltScope::Market,
            );
        }
        if account_collateral > self.limits.exposure.max_account_collateral_at_risk {
            return rejected(
                PmRiskReason::AccountCollateralAtRisk {
                    observed: account_collateral,
                    limit: self.limits.exposure.max_account_collateral_at_risk,
                },
                PmRiskHaltScope::Account,
            );
        }
        if reserved_collateral > self.limits.exposure.max_reserved_collateral {
            return rejected(
                PmRiskReason::ReservedCollateral {
                    observed: reserved_collateral,
                    limit: self.limits.exposure.max_reserved_collateral,
                },
                PmRiskHaltScope::Account,
            );
        }
        if reserved_token > self.limits.exposure.max_reserved_token {
            return rejected(
                PmRiskReason::ReservedToken {
                    observed: reserved_token,
                    limit: self.limits.exposure.max_reserved_token,
                },
                PmRiskHaltScope::Market,
            );
        }
        if live_orders > self.limits.cardinality.max_live_orders {
            return rejected(
                PmRiskReason::LiveOrderCount {
                    observed: live_orders,
                    limit: self.limits.cardinality.max_live_orders,
                },
                PmRiskHaltScope::Account,
            );
        }
        if input.exposure.unresolved_orders > self.limits.cardinality.max_unresolved_orders {
            return rejected(
                PmRiskReason::UnresolvedOrderCount {
                    observed: input.exposure.unresolved_orders,
                    limit: self.limits.cardinality.max_unresolved_orders,
                },
                PmRiskHaltScope::Account,
            );
        }
        if input.exposure.unresolved_fills > self.limits.cardinality.max_unresolved_fills {
            return rejected(
                PmRiskReason::UnresolvedFillCount {
                    observed: input.exposure.unresolved_fills,
                    limit: self.limits.cardinality.max_unresolved_fills,
                },
                PmRiskHaltScope::Account,
            );
        }

        PmRiskDecision::Approved {
            prospective_market_inventory: market_inventory.magnitude(),
            prospective_account_collateral_at_risk: account_collateral,
            prospective_reserved_collateral: reserved_collateral,
            prospective_reserved_token: reserved_token,
            prospective_live_orders: live_orders,
        }
    }

    fn latch(&mut self, halt: PmRiskHaltScope, reason: PmRiskReason) {
        match halt {
            PmRiskHaltScope::None => {}
            PmRiskHaltScope::Market => {
                if self.market_halt.is_none() {
                    self.market_halt = Some(reason);
                    self.counters.market_halts = self.counters.market_halts.saturating_add(1);
                }
            }
            PmRiskHaltScope::Account => {
                if self.account_halt.is_none() {
                    self.account_halt = Some(reason);
                    self.counters.account_halts = self.counters.account_halts.saturating_add(1);
                }
            }
            PmRiskHaltScope::Global => {
                if self.global_halt.is_none() {
                    self.global_halt = Some(reason);
                    self.counters.global_halts = self.counters.global_halts.saturating_add(1);
                }
            }
        }
    }
}

fn check_current_exposure(
    exposure: PmRiskExposure,
    limits: PmRiskLimits,
) -> Option<PmRiskDecision> {
    if exposure.market_inventory.magnitude() > limits.exposure.max_market_inventory {
        return Some(rejected(
            PmRiskReason::MarketInventory {
                observed: exposure.market_inventory.magnitude(),
                limit: limits.exposure.max_market_inventory,
            },
            PmRiskHaltScope::Market,
        ));
    }
    if exposure.account_collateral_at_risk > limits.exposure.max_account_collateral_at_risk {
        return Some(rejected(
            PmRiskReason::AccountCollateralAtRisk {
                observed: exposure.account_collateral_at_risk,
                limit: limits.exposure.max_account_collateral_at_risk,
            },
            PmRiskHaltScope::Account,
        ));
    }
    if exposure.reserved_collateral > limits.exposure.max_reserved_collateral {
        return Some(rejected(
            PmRiskReason::ReservedCollateral {
                observed: exposure.reserved_collateral,
                limit: limits.exposure.max_reserved_collateral,
            },
            PmRiskHaltScope::Account,
        ));
    }
    if exposure.reserved_token > limits.exposure.max_reserved_token {
        return Some(rejected(
            PmRiskReason::ReservedToken {
                observed: exposure.reserved_token,
                limit: limits.exposure.max_reserved_token,
            },
            PmRiskHaltScope::Market,
        ));
    }
    if exposure.live_orders > limits.cardinality.max_live_orders {
        return Some(rejected(
            PmRiskReason::LiveOrderCount {
                observed: exposure.live_orders,
                limit: limits.cardinality.max_live_orders,
            },
            PmRiskHaltScope::Account,
        ));
    }
    if exposure.unresolved_orders > limits.cardinality.max_unresolved_orders {
        return Some(rejected(
            PmRiskReason::UnresolvedOrderCount {
                observed: exposure.unresolved_orders,
                limit: limits.cardinality.max_unresolved_orders,
            },
            PmRiskHaltScope::Account,
        ));
    }
    if exposure.unresolved_fills > limits.cardinality.max_unresolved_fills {
        return Some(rejected(
            PmRiskReason::UnresolvedFillCount {
                observed: exposure.unresolved_fills,
                limit: limits.cardinality.max_unresolved_fills,
            },
            PmRiskHaltScope::Account,
        ));
    }
    None
}

fn check_dependencies(
    now_ns: u64,
    dependencies: PmRiskDependencies,
    limits: PmFreshnessRiskLimits,
) -> Option<PmRiskDecision> {
    for (kind, dependency, limit_ns, halt) in [
        (
            PmRiskDependencyKind::Reference,
            dependencies.reference,
            limits.reference_ns,
            PmRiskHaltScope::Market,
        ),
        (
            PmRiskDependencyKind::Book,
            dependencies.book,
            limits.book_ns,
            PmRiskHaltScope::Market,
        ),
        (
            PmRiskDependencyKind::PrivateLifecycle,
            dependencies.private,
            limits.private_ns,
            PmRiskHaltScope::Account,
        ),
        (
            PmRiskDependencyKind::AccountSnapshot,
            dependencies.account,
            limits.account_ns,
            PmRiskHaltScope::Account,
        ),
        (
            PmRiskDependencyKind::OrderLifecycle,
            dependencies.orders,
            limits.order_ns,
            PmRiskHaltScope::Account,
        ),
        (
            PmRiskDependencyKind::Reconciliation,
            dependencies.reconciliation,
            limits.reconciliation_ns,
            PmRiskHaltScope::Account,
        ),
    ] {
        if !dependency.available {
            return Some(rejected(PmRiskReason::DependencyUnavailable(kind), halt));
        }
        let Some(observed_ns) = dependency.observed_monotonic_ns else {
            return Some(rejected(PmRiskReason::DependencyUnavailable(kind), halt));
        };
        let Some(age_ns) = now_ns.checked_sub(observed_ns) else {
            return Some(rejected(
                PmRiskReason::MonotonicClockRegression(kind),
                PmRiskHaltScope::Global,
            ));
        };
        if age_ns > limit_ns {
            return Some(rejected(
                PmRiskReason::DependencyStale {
                    dependency: kind,
                    age_ns,
                    limit_ns,
                },
                halt,
            ));
        }
    }
    None
}

const fn rejected(reason: PmRiskReason, halt: PmRiskHaltScope) -> PmRiskDecision {
    PmRiskDecision::Rejected { reason, halt }
}

fn require_nonzero(limit: U256) -> Result<(), PmRiskLimitsError> {
    if limit.is_zero() {
        Err(PmRiskLimitsError::ZeroExactLimit)
    } else {
        Ok(())
    }
}

fn add_signed(left: PmSignedUnits, right: PmSignedUnits) -> Result<PmSignedUnits, ()> {
    if left.sign() == right.sign() {
        return PmSignedUnits::from_parts(
            left.sign(),
            left.magnitude()
                .checked_add(right.magnitude())
                .map_err(|_| ())?,
        )
        .map_err(|_| ());
    }
    match left.magnitude().cmp(&right.magnitude()) {
        std::cmp::Ordering::Greater => PmSignedUnits::from_parts(
            left.sign(),
            left.magnitude()
                .checked_sub(right.magnitude())
                .map_err(|_| ())?,
        )
        .map_err(|_| ()),
        std::cmp::Ordering::Less => PmSignedUnits::from_parts(
            right.sign(),
            right
                .magnitude()
                .checked_sub(left.magnitude())
                .map_err(|_| ())?,
        )
        .map_err(|_| ()),
        std::cmp::Ordering::Equal => Ok(PmSignedUnits::ZERO),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use reap_pm_core::{PmMarketHandle, PmTokenHandle};

    fn account() -> PmAccountHandle {
        PmAccountHandle::from_ordinal(1)
    }

    fn instrument() -> PmInstrumentHandle {
        PmInstrumentHandle::new(
            PmMarketHandle::from_ordinal(2),
            PmTokenHandle::from_ordinal(3),
        )
    }

    fn units(value: u64) -> U256 {
        U256::from_u64(value)
    }

    fn quantity(value: u64) -> PmQuantity {
        PmQuantity::from_protocol_units(units(value)).unwrap()
    }

    fn signed(sign: PmSign, value: u64) -> PmSignedUnits {
        PmSignedUnits::from_parts(sign, units(value)).unwrap()
    }

    fn limits() -> PmRiskLimits {
        PmRiskLimits::new(
            PmOrderRiskLimits::new(quantity(20), units(20)).unwrap(),
            PmExposureRiskLimits::new(units(100), units(100), units(80), units(80)).unwrap(),
            PmCardinalityRiskLimits::new(4, 3, 3).unwrap(),
            PmFreshnessRiskLimits::new(10, 10, 10, 10, 10, 10).unwrap(),
        )
    }

    fn fresh() -> PmRiskDependencies {
        let dependency = PmRiskDependency::available(100);
        PmRiskDependencies::new(
            dependency, dependency, dependency, dependency, dependency, dependency,
        )
    }

    fn baseline_input(side: PmOrderSide) -> PmRiskInput {
        let token_at_risk = match side {
            PmOrderSide::Buy => U256::ZERO,
            PmOrderSide::Sell => units(10),
        };
        PmRiskInput::new(
            110,
            PmRiskCandidate::new(
                account(),
                instrument(),
                side,
                quantity(10),
                units(4),
                token_at_risk,
            ),
            PmRiskExposure::new(
                signed(PmSign::Positive, 5),
                units(7),
                units(8),
                units(9),
                1,
                0,
                0,
            ),
            fresh(),
        )
    }

    #[test]
    fn every_limit_is_explicit_and_positive() {
        assert_eq!(
            PmOrderRiskLimits::new(quantity(1), U256::ZERO),
            Err(PmRiskLimitsError::ZeroExactLimit)
        );
        assert_eq!(
            PmExposureRiskLimits::new(units(1), units(1), U256::ZERO, units(1)),
            Err(PmRiskLimitsError::ZeroExactLimit)
        );
        assert_eq!(
            PmCardinalityRiskLimits::new(1, 0, 1),
            Err(PmRiskLimitsError::ZeroCardinality)
        );
        assert_eq!(
            PmFreshnessRiskLimits::new(1, 1, 0, 1, 1, 1),
            Err(PmRiskLimitsError::ZeroFreshness)
        );
    }

    #[test]
    fn prospective_buy_and_sell_reservations_are_checked_before_approval() {
        let mut state = PmRiskState::new(account(), instrument(), limits());
        assert_eq!(
            state.evaluate(baseline_input(PmOrderSide::Buy)),
            PmRiskDecision::Approved {
                prospective_market_inventory: units(15),
                prospective_account_collateral_at_risk: units(11),
                prospective_reserved_collateral: units(12),
                prospective_reserved_token: units(9),
                prospective_live_orders: 2,
            }
        );

        let mut state = PmRiskState::new(account(), instrument(), limits());
        assert_eq!(
            state.evaluate(baseline_input(PmOrderSide::Sell)),
            PmRiskDecision::Approved {
                prospective_market_inventory: units(5),
                prospective_account_collateral_at_risk: units(11),
                prospective_reserved_collateral: units(12),
                prospective_reserved_token: units(19),
                prospective_live_orders: 2,
            }
        );
    }

    #[test]
    fn candidate_only_limits_reject_without_cancelling_existing_orders() {
        let mut state = PmRiskState::new(account(), instrument(), limits());
        let input = PmRiskInput::new(
            110,
            PmRiskCandidate::new(
                account(),
                instrument(),
                PmOrderSide::Buy,
                quantity(21),
                units(4),
                U256::ZERO,
            ),
            PmRiskExposure::new(
                PmSignedUnits::ZERO,
                U256::ZERO,
                U256::ZERO,
                U256::ZERO,
                0,
                0,
                0,
            ),
            fresh(),
        );
        let decision = state.evaluate(input);
        assert_eq!(
            decision.reason(),
            Some(PmRiskReason::OrderQuantity {
                observed: units(21),
                limit: units(20),
            })
        );
        assert_eq!(decision.halt_scope(), PmRiskHaltScope::None);
        assert_eq!(state.market_halt(), None);
        assert_eq!(state.account_halt(), None);
    }

    #[test]
    fn market_and_account_limit_breaches_latch_the_exact_scope() {
        let mut state = PmRiskState::new(account(), instrument(), limits());
        let market = PmRiskInput::new(
            110,
            PmRiskCandidate::new(
                account(),
                instrument(),
                PmOrderSide::Sell,
                quantity(10),
                units(4),
                units(10),
            ),
            PmRiskExposure::new(
                signed(PmSign::Negative, 95),
                U256::ZERO,
                U256::ZERO,
                U256::ZERO,
                0,
                0,
                0,
            ),
            fresh(),
        );
        assert_eq!(state.evaluate(market).halt_scope(), PmRiskHaltScope::Market);
        assert!(matches!(
            state.market_halt(),
            Some(PmRiskReason::MarketInventory { .. })
        ));
        assert_eq!(state.counters().market_halts(), 1);

        let mut state = PmRiskState::new(account(), instrument(), limits());
        let account_limit = PmRiskInput::new(
            110,
            PmRiskCandidate::new(
                account(),
                instrument(),
                PmOrderSide::Buy,
                quantity(1),
                units(4),
                U256::ZERO,
            ),
            PmRiskExposure::new(
                PmSignedUnits::ZERO,
                units(98),
                U256::ZERO,
                U256::ZERO,
                0,
                0,
                0,
            ),
            fresh(),
        );
        assert_eq!(
            state.evaluate(account_limit).halt_scope(),
            PmRiskHaltScope::Account
        );
        assert!(matches!(
            state.account_halt(),
            Some(PmRiskReason::AccountCollateralAtRisk { .. })
        ));
        assert_eq!(state.counters().account_halts(), 1);
    }

    #[test]
    fn opposite_candidate_cannot_mask_already_breached_current_inventory() {
        let mut state = PmRiskState::new(account(), instrument(), limits());
        let input = PmRiskInput {
            candidate: PmRiskCandidate::new(
                account(),
                instrument(),
                PmOrderSide::Sell,
                quantity(10),
                units(4),
                units(10),
            ),
            exposure: PmRiskExposure::new(
                signed(PmSign::Positive, 101),
                U256::ZERO,
                U256::ZERO,
                U256::ZERO,
                0,
                0,
                0,
            ),
            ..baseline_input(PmOrderSide::Sell)
        };
        assert_eq!(
            state.evaluate(input),
            rejected(
                PmRiskReason::MarketInventory {
                    observed: units(101),
                    limit: units(100),
                },
                PmRiskHaltScope::Market,
            )
        );
    }

    #[test]
    fn dependency_fault_can_escalate_a_previously_latched_market_halt_to_global() {
        let unavailable = PmRiskDependencies::new(
            PmRiskDependency::unavailable(),
            PmRiskDependency::available(100),
            PmRiskDependency::available(100),
            PmRiskDependency::available(100),
            PmRiskDependency::available(100),
            PmRiskDependency::available(100),
        );
        let mut state = PmRiskState::new(account(), instrument(), limits());
        assert_eq!(
            state
                .evaluate(PmRiskInput {
                    dependencies: unavailable,
                    ..baseline_input(PmOrderSide::Buy)
                })
                .halt_scope(),
            PmRiskHaltScope::Market
        );
        let regressed = PmRiskDependencies::new(
            PmRiskDependency::available(111),
            PmRiskDependency::available(100),
            PmRiskDependency::available(100),
            PmRiskDependency::available(100),
            PmRiskDependency::available(100),
            PmRiskDependency::available(100),
        );
        assert_eq!(
            state
                .evaluate(PmRiskInput {
                    dependencies: regressed,
                    ..baseline_input(PmOrderSide::Buy)
                })
                .halt_scope(),
            PmRiskHaltScope::Global
        );
        assert!(matches!(
            state.global_halt(),
            Some(PmRiskReason::MonotonicClockRegression(
                PmRiskDependencyKind::Reference
            ))
        ));
    }

    #[test]
    fn stale_is_inclusive_at_limit_and_unavailable_is_typed() {
        let mut state = PmRiskState::new(account(), instrument(), limits());
        assert!(matches!(
            state.evaluate(baseline_input(PmOrderSide::Buy)),
            PmRiskDecision::Approved { .. }
        ));

        let unavailable = PmRiskDependencies::new(
            PmRiskDependency::unavailable(),
            PmRiskDependency::available(100),
            PmRiskDependency::available(100),
            PmRiskDependency::available(100),
            PmRiskDependency::available(100),
            PmRiskDependency::available(100),
        );
        let mut state = PmRiskState::new(account(), instrument(), limits());
        let input = PmRiskInput {
            dependencies: unavailable,
            ..baseline_input(PmOrderSide::Buy)
        };
        assert_eq!(
            state.evaluate(input),
            rejected(
                PmRiskReason::DependencyUnavailable(PmRiskDependencyKind::Reference),
                PmRiskHaltScope::Market,
            )
        );

        let stale = PmRiskDependencies::new(
            PmRiskDependency::available(99),
            PmRiskDependency::available(100),
            PmRiskDependency::available(100),
            PmRiskDependency::available(100),
            PmRiskDependency::available(100),
            PmRiskDependency::available(100),
        );
        let mut state = PmRiskState::new(account(), instrument(), limits());
        let input = PmRiskInput {
            dependencies: stale,
            ..baseline_input(PmOrderSide::Buy)
        };
        assert_eq!(
            state.evaluate(input).reason(),
            Some(PmRiskReason::DependencyStale {
                dependency: PmRiskDependencyKind::Reference,
                age_ns: 11,
                limit_ns: 10,
            })
        );
    }

    #[test]
    fn clock_regression_and_scope_mismatch_latch_global_halt() {
        let regressed = PmRiskDependencies::new(
            PmRiskDependency::available(111),
            PmRiskDependency::available(100),
            PmRiskDependency::available(100),
            PmRiskDependency::available(100),
            PmRiskDependency::available(100),
            PmRiskDependency::available(100),
        );
        let mut state = PmRiskState::new(account(), instrument(), limits());
        let input = PmRiskInput {
            dependencies: regressed,
            ..baseline_input(PmOrderSide::Buy)
        };
        assert_eq!(state.evaluate(input).halt_scope(), PmRiskHaltScope::Global);
        assert!(matches!(
            state.global_halt(),
            Some(PmRiskReason::MonotonicClockRegression(
                PmRiskDependencyKind::Reference
            ))
        ));

        let mut state = PmRiskState::new(account(), instrument(), limits());
        let input = PmRiskInput {
            candidate: PmRiskCandidate::new(
                PmAccountHandle::from_ordinal(99),
                instrument(),
                PmOrderSide::Buy,
                quantity(1),
                units(1),
                U256::ZERO,
            ),
            ..baseline_input(PmOrderSide::Buy)
        };
        assert_eq!(
            state.evaluate(input),
            rejected(PmRiskReason::ScopeMismatch, PmRiskHaltScope::Global)
        );
    }

    #[test]
    fn cardinality_limits_are_checked_without_wrapping() {
        let mut state = PmRiskState::new(account(), instrument(), limits());
        let input = PmRiskInput {
            exposure: PmRiskExposure::new(
                PmSignedUnits::ZERO,
                U256::ZERO,
                U256::ZERO,
                U256::ZERO,
                4,
                0,
                0,
            ),
            ..baseline_input(PmOrderSide::Buy)
        };
        assert_eq!(
            state.evaluate(input).reason(),
            Some(PmRiskReason::LiveOrderCount {
                observed: 5,
                limit: 4,
            })
        );

        let mut state = PmRiskState::new(account(), instrument(), limits());
        let input = PmRiskInput {
            exposure: PmRiskExposure::new(
                PmSignedUnits::ZERO,
                U256::ZERO,
                U256::ZERO,
                U256::ZERO,
                u16::MAX,
                0,
                0,
            ),
            ..baseline_input(PmOrderSide::Buy)
        };
        assert_eq!(
            state.evaluate(input),
            rejected(
                PmRiskReason::LiveOrderCount {
                    observed: u16::MAX,
                    limit: 4,
                },
                PmRiskHaltScope::Account,
            )
        );
    }

    #[test]
    fn risk_decision_contains_no_order_or_cancel_authority() {
        assert!(
            PmRiskHaltScope::Account.cancel_owned_required(),
            "risk requests deterministic owned cancellation without naming or minting it"
        );
        assert!(!PmRiskHaltScope::None.cancel_owned_required());
    }
}
