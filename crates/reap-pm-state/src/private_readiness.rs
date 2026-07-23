use reap_pm_core::{
    PmAllowanceValue, PmAssetId, PmFillKey, PmFillSettlementStatus, PmOrderIdentity, PmOrderSide,
    PmPrice, PmQuantity, PmReconciliationRequestBoundary, PmSpenderId, U256,
};

use crate::account::{
    PmAccountState, PmAllowanceKnowledge, PmObservedAmount, PmPositionKnowledge, apply_signed,
};
use crate::fill_state::{PmFillFeeState, PmFillState};
use crate::order_state::{PmExactReservation, PmOrderState, PmReservationTotalsError};
use crate::private_config::PmPrivateStateConfig;
use crate::private_ingress::PmPrivateExternalIngressFault;
use crate::risk::{PmFreshnessRiskLimits, PmRiskReason};
use crate::unresolved_fill::{PmUnresolvedFillKey, PmUnresolvedFillReason, PmUnresolvedFillState};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PmPrivateDependency {
    PrivateLifecycle,
    AccountSnapshot,
    OrderLifecycle,
    Reconciliation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PmPrivateHaltReason {
    OrderCapacity,
    FillCapacity,
    UnresolvedFillCapacity,
    ArithmeticOverflow,
    ContractViolation,
    ExternalIngressFault(PmPrivateExternalIngressFault),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PmPrivateConvergence {
    Uninitialized,
    Divergent {
        uncovered_fills: u16,
    },
    Converged {
        boundary: PmReconciliationRequestBoundary,
        observed_monotonic_ns: u64,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PmPrivateReadinessReason {
    Halted(PmPrivateHaltReason),
    RiskHalted(PmRiskReason),
    FullReconciliationRequired,
    DependencyUnavailable(PmPrivateDependency),
    DependencyStale {
        dependency: PmPrivateDependency,
        age_ns: u64,
        limit_ns: u64,
    },
    MonotonicClockRegression(PmPrivateDependency),
    Divergent {
        uncovered_fills: u16,
    },
    UnknownReservation(PmOrderIdentity),
    UnmanagedOwnershipAmbiguous,
    FillFeeUnknown(PmFillKey),
    FillFeeIncomplete(PmFillKey),
    FillFeeAssetUnmapped {
        fill: PmFillKey,
        asset: PmAssetId,
    },
    FillSettlementRetrying(PmFillKey),
    FillSettlementFailed(PmFillKey),
    UnresolvedFill {
        fill: PmUnresolvedFillKey,
        reason: PmUnresolvedFillReason,
    },
    BalanceUnavailable(PmAssetId),
    PositionResolvedUnredeemed(U256),
    PositionUnavailable,
    PublishedInventoryMismatch {
        balance: U256,
        position: U256,
    },
    ExactAllowanceUnconfigured(PmSpenderId),
    ExactAllowanceUnavailable(PmSpenderId),
    ExactAllowanceAbsent(PmSpenderId),
    ExactAllowanceInsufficient {
        spender: PmSpenderId,
        required: U256,
        available: U256,
    },
    ExactOperatorNotApproved(PmSpenderId),
    InsufficientCollateral {
        available: U256,
        required: U256,
    },
    InsufficientOutcomeInventory {
        available: U256,
        required: U256,
    },
    ArithmeticInvalid,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PmPrivateQuoteRequest {
    monotonic_now_ns: u64,
    side: PmOrderSide,
    price: PmPrice,
    quantity: PmQuantity,
    reservation: PmExactReservation,
}

impl PmPrivateQuoteRequest {
    #[must_use]
    pub const fn new(
        monotonic_now_ns: u64,
        side: PmOrderSide,
        price: PmPrice,
        quantity: PmQuantity,
        reservation: PmExactReservation,
    ) -> Self {
        Self {
            monotonic_now_ns,
            side,
            price,
            quantity,
            reservation,
        }
    }

    #[must_use]
    pub const fn monotonic_now_ns(self) -> u64 {
        self.monotonic_now_ns
    }

    #[must_use]
    pub const fn side(self) -> PmOrderSide {
        self.side
    }

    #[must_use]
    pub const fn price(self) -> PmPrice {
        self.price
    }

    #[must_use]
    pub const fn quantity(self) -> PmQuantity {
        self.quantity
    }

    #[must_use]
    pub const fn reservation(self) -> PmExactReservation {
        self.reservation
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PmPrivateReady {
    effective_collateral: U256,
    effective_outcome: U256,
    reserved_collateral: U256,
    reserved_outcome: U256,
    candidate_collateral: U256,
    candidate_outcome: U256,
}

impl PmPrivateReady {
    #[must_use]
    pub const fn effective_collateral(self) -> U256 {
        self.effective_collateral
    }

    #[must_use]
    pub const fn effective_outcome(self) -> U256 {
        self.effective_outcome
    }

    #[must_use]
    pub const fn reserved_collateral(self) -> U256 {
        self.reserved_collateral
    }

    #[must_use]
    pub const fn reserved_outcome(self) -> U256 {
        self.reserved_outcome
    }

    #[must_use]
    pub const fn candidate_collateral(self) -> U256 {
        self.candidate_collateral
    }

    #[must_use]
    pub const fn candidate_outcome(self) -> U256 {
        self.candidate_outcome
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PmPrivateReadiness {
    Ready(PmPrivateReady),
    Blocked(PmPrivateReadinessReason),
}

pub(crate) struct PmReadinessContext<'a> {
    pub(crate) config: &'a PmPrivateStateConfig,
    pub(crate) account: &'a PmAccountState,
    pub(crate) orders: &'a PmOrderState,
    pub(crate) fills: &'a PmFillState,
    pub(crate) unresolved_fills: &'a PmUnresolvedFillState,
    pub(crate) convergence: PmPrivateConvergence,
    pub(crate) private_observed_ns: Option<u64>,
    pub(crate) private_available: bool,
    pub(crate) halt: Option<PmPrivateHaltReason>,
    pub(crate) risk_halt: Option<PmRiskReason>,
    pub(crate) full_reconcile_required: bool,
    pub(crate) freshness: PmFreshnessRiskLimits,
}

pub(crate) fn check(
    context: PmReadinessContext<'_>,
    request: PmPrivateQuoteRequest,
) -> PmPrivateReadiness {
    if let Some(reason) = precondition_reason(&context, request.monotonic_now_ns) {
        return PmPrivateReadiness::Blocked(reason);
    }
    if let Some(reason) =
        reservation_or_fill_reason(context.orders, context.fills, context.unresolved_fills)
    {
        return PmPrivateReadiness::Blocked(reason);
    }
    if let Some(reason) = convergence_reason(
        context.convergence,
        request.monotonic_now_ns,
        context.freshness.reconciliation_ns(),
    ) {
        return PmPrivateReadiness::Blocked(reason);
    }
    let Some((collateral, outcome)) = effective_resources(context.account, context.fills) else {
        return PmPrivateReadiness::Blocked(account_resource_reason(
            context.account,
            context.config,
        ));
    };
    let Ok((reserved_collateral, reserved_outcome)) = context.orders.reservation_totals() else {
        return PmPrivateReadiness::Blocked(PmPrivateReadinessReason::ArithmeticInvalid);
    };
    let candidate = request.reservation;
    let Some(required_collateral) = reserved_collateral.checked_add(candidate.collateral()).ok()
    else {
        return PmPrivateReadiness::Blocked(PmPrivateReadinessReason::ArithmeticInvalid);
    };
    let Some(required_outcome) = reserved_outcome.checked_add(candidate.outcome()).ok() else {
        return PmPrivateReadiness::Blocked(PmPrivateReadinessReason::ArithmeticInvalid);
    };
    if let Some(reason) = allowance_reason(context, required_collateral) {
        return PmPrivateReadiness::Blocked(reason);
    }
    if collateral < required_collateral {
        return PmPrivateReadiness::Blocked(PmPrivateReadinessReason::InsufficientCollateral {
            available: collateral,
            required: required_collateral,
        });
    }
    if outcome < required_outcome {
        return PmPrivateReadiness::Blocked(
            PmPrivateReadinessReason::InsufficientOutcomeInventory {
                available: outcome,
                required: required_outcome,
            },
        );
    }
    PmPrivateReadiness::Ready(PmPrivateReady {
        effective_collateral: collateral,
        effective_outcome: outcome,
        reserved_collateral,
        reserved_outcome,
        candidate_collateral: candidate.collateral(),
        candidate_outcome: candidate.outcome(),
    })
}

fn precondition_reason(
    context: &PmReadinessContext<'_>,
    now_ns: u64,
) -> Option<PmPrivateReadinessReason> {
    if let Some(halt) = context.halt {
        return Some(PmPrivateReadinessReason::Halted(halt));
    }
    if let Some(halt) = context.risk_halt {
        return Some(PmPrivateReadinessReason::RiskHalted(halt));
    }
    if context.full_reconcile_required {
        return Some(PmPrivateReadinessReason::FullReconciliationRequired);
    }
    if !context.private_available {
        return Some(PmPrivateReadinessReason::DependencyUnavailable(
            PmPrivateDependency::PrivateLifecycle,
        ));
    }
    for (dependency, observed, limit) in [
        (
            PmPrivateDependency::PrivateLifecycle,
            context.private_observed_ns,
            context.freshness.private_ns(),
        ),
        (
            PmPrivateDependency::AccountSnapshot,
            context.account.observed_monotonic_ns(),
            context.freshness.account_ns(),
        ),
        (
            PmPrivateDependency::OrderLifecycle,
            context.orders.observed_monotonic_ns(),
            context.freshness.order_ns(),
        ),
    ] {
        let Some(observed) = observed else {
            return Some(PmPrivateReadinessReason::DependencyUnavailable(dependency));
        };
        let Some(age_ns) = now_ns.checked_sub(observed) else {
            return Some(PmPrivateReadinessReason::MonotonicClockRegression(
                dependency,
            ));
        };
        if age_ns > limit {
            return Some(PmPrivateReadinessReason::DependencyStale {
                dependency,
                age_ns,
                limit_ns: limit,
            });
        }
    }
    None
}

fn reservation_or_fill_reason(
    orders: &PmOrderState,
    fills: &PmFillState,
    unresolved_fills: &PmUnresolvedFillState,
) -> Option<PmPrivateReadinessReason> {
    if orders.has_unmanaged_ambiguity() {
        return Some(PmPrivateReadinessReason::UnmanagedOwnershipAmbiguous);
    }
    if let Some(identity) = orders.first_unknown_reservation() {
        return Some(PmPrivateReadinessReason::UnknownReservation(identity));
    }
    if let Some(fill) = unresolved_fills.first_active() {
        return Some(PmPrivateReadinessReason::UnresolvedFill {
            fill: fill.key(),
            reason: fill.reason(),
        });
    }
    if let Some((fill, fee)) = fills.first_unresolved_fee() {
        return Some(match fee {
            PmFillFeeState::Unknown => PmPrivateReadinessReason::FillFeeUnknown(fill),
            PmFillFeeState::Incomplete => PmPrivateReadinessReason::FillFeeIncomplete(fill),
            PmFillFeeState::UnmappedAsset { asset, .. } => {
                PmPrivateReadinessReason::FillFeeAssetUnmapped { fill, asset }
            }
            PmFillFeeState::Known { .. } => unreachable!("resolved fee was filtered"),
        });
    }
    if let Some((fill, settlement)) = fills.first_unresolved_settlement() {
        return Some(match settlement {
            PmFillSettlementStatus::Retrying => {
                PmPrivateReadinessReason::FillSettlementRetrying(fill)
            }
            PmFillSettlementStatus::Failed => PmPrivateReadinessReason::FillSettlementFailed(fill),
            PmFillSettlementStatus::Matched
            | PmFillSettlementStatus::Mined
            | PmFillSettlementStatus::Confirmed => {
                unreachable!("resolved settlement was filtered")
            }
        });
    }
    match orders.reservation_totals() {
        Err(PmReservationTotalsError::Unknown(identity)) => {
            Some(PmPrivateReadinessReason::UnknownReservation(identity))
        }
        Err(PmReservationTotalsError::Overflow) => {
            Some(PmPrivateReadinessReason::ArithmeticInvalid)
        }
        Ok(_) => None,
    }
}

fn effective_resources(account: &PmAccountState, fills: &PmFillState) -> Option<(U256, U256)> {
    let collateral = account.collateral().value()?;
    let outcome_balance = account.outcome_balance().value()?;
    let position = account.position().tradable_units()?;
    if outcome_balance != position {
        return None;
    }
    let provisional = fills.provisional();
    Some((
        apply_signed(collateral, provisional.collateral())?,
        apply_signed(position, provisional.outcome())?,
    ))
}

fn account_resource_reason(
    account: &PmAccountState,
    config: &PmPrivateStateConfig,
) -> PmPrivateReadinessReason {
    if account.collateral() == PmObservedAmount::Unavailable {
        return PmPrivateReadinessReason::BalanceUnavailable(config.collateral_asset());
    }
    if account.outcome_balance() == PmObservedAmount::Unavailable {
        return PmPrivateReadinessReason::BalanceUnavailable(config.outcome_asset());
    }
    match account.position() {
        PmPositionKnowledge::ResolvedUnredeemed(quantity) => {
            PmPrivateReadinessReason::PositionResolvedUnredeemed(quantity)
        }
        PmPositionKnowledge::Unavailable | PmPositionKnowledge::VenueUnavailable(_) => {
            PmPrivateReadinessReason::PositionUnavailable
        }
        PmPositionKnowledge::ExplicitAbsent | PmPositionKnowledge::Tradable(_) => {
            let balance = account.outcome_balance().value().unwrap_or(U256::ZERO);
            let position = account.position().tradable_units().unwrap_or(U256::ZERO);
            if balance != position {
                PmPrivateReadinessReason::PublishedInventoryMismatch { balance, position }
            } else {
                PmPrivateReadinessReason::ArithmeticInvalid
            }
        }
    }
}

fn allowance_reason(
    context: PmReadinessContext<'_>,
    required_collateral: U256,
) -> Option<PmPrivateReadinessReason> {
    for spender in context.config.required_spenders().iter().copied() {
        let required = if spender.requirement().asset() == context.config.collateral_asset() {
            Some(required_collateral)
        } else {
            None
        };
        match (context.account.allowance(spender), required) {
            (PmAllowanceKnowledge::Unconfigured, _) => {
                return Some(PmPrivateReadinessReason::ExactAllowanceUnconfigured(
                    spender,
                ));
            }
            (PmAllowanceKnowledge::Unavailable, _) => {
                return Some(PmPrivateReadinessReason::ExactAllowanceUnavailable(spender));
            }
            (PmAllowanceKnowledge::ExplicitAbsent, _) => {
                return Some(PmPrivateReadinessReason::ExactAllowanceAbsent(spender));
            }
            (PmAllowanceKnowledge::Present(PmAllowanceValue::Erc20(available)), Some(required))
                if available < required =>
            {
                return Some(PmPrivateReadinessReason::ExactAllowanceInsufficient {
                    spender,
                    required,
                    available,
                });
            }
            (PmAllowanceKnowledge::Present(PmAllowanceValue::Erc1155Operator(approval)), None)
                if !approval.is_approved() =>
            {
                return Some(PmPrivateReadinessReason::ExactOperatorNotApproved(spender));
            }
            (PmAllowanceKnowledge::Present(PmAllowanceValue::Erc20(_)), Some(_))
            | (PmAllowanceKnowledge::Present(PmAllowanceValue::Erc1155Operator(_)), None) => {}
            (PmAllowanceKnowledge::Present(_), _) => {
                return Some(PmPrivateReadinessReason::ExactAllowanceUnavailable(spender));
            }
        }
    }
    None
}

fn convergence_reason(
    convergence: PmPrivateConvergence,
    now_ns: u64,
    limit_ns: u64,
) -> Option<PmPrivateReadinessReason> {
    match convergence {
        PmPrivateConvergence::Converged {
            observed_monotonic_ns,
            ..
        } => match now_ns.checked_sub(observed_monotonic_ns) {
            None => Some(PmPrivateReadinessReason::MonotonicClockRegression(
                PmPrivateDependency::Reconciliation,
            )),
            Some(age_ns) if age_ns > limit_ns => Some(PmPrivateReadinessReason::DependencyStale {
                dependency: PmPrivateDependency::Reconciliation,
                age_ns,
                limit_ns,
            }),
            Some(_) => None,
        },
        PmPrivateConvergence::Uninitialized => Some(
            PmPrivateReadinessReason::DependencyUnavailable(PmPrivateDependency::Reconciliation),
        ),
        PmPrivateConvergence::Divergent { uncovered_fills } => {
            Some(PmPrivateReadinessReason::Divergent { uncovered_fills })
        }
    }
}
