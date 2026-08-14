//! Network-free rollover and settlement lifecycle for BTC Up/Down five-minute
//! markets.
//!
//! This state machine makes the safety joins explicit: new quoting stops
//! before expiry, rollover requires a complete zero-open-order cut, retired
//! markets remain tracked until their Data API positions are flat, and
//! resolved inventory is represented as `ReadyToRedeem`/`RedemptionDispatched`
//! rather than silently treated as tradable. It owns no discovery, order,
//! relayer, signer, RPC, or redemption transport.

use reap_pm_core::{PmConditionId, PmTokenId, U256};
use thiserror::Error;

use crate::{PmBtcFiveMinuteMarket, PmConfiguredTokenPosition};

const FIVE_MINUTE_SECONDS: u64 = 300;
const QUIESCE_BEFORE_END_SECONDS: u64 = 30;
pub const MAX_PENDING_BTC_FIVE_MINUTE_SETTLEMENTS: usize = 24;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum PmBtcFiveMinuteLifecycleError {
    #[error("BTC five-minute rollover occurred before the active window ended")]
    RolloverTooEarly,
    #[error("BTC five-minute rollover requires a complete zero-open-order cut")]
    OpenOrdersRemain,
    #[error("BTC five-minute rollover candidate is not a later aligned window")]
    InvalidRolloverCandidate,
    #[error("BTC five-minute rollover candidate repeated a condition or token identity")]
    RepeatedMarketIdentity,
    #[error("too many BTC five-minute settlements remain pending")]
    PendingSettlementBound,
    #[error("BTC five-minute settlement target is unknown")]
    UnknownSettlement,
    #[error("BTC five-minute position observation does not match the retired token")]
    PositionScopeMismatch,
    #[error("BTC five-minute position quantity is not exactly representable")]
    PositionQuantity,
    #[error("BTC five-minute settlement quantity overflowed")]
    PositionOverflow,
    #[error("BTC five-minute redemption dispatch requires redeemable nonzero inventory")]
    RedemptionNotReady,
    #[error("BTC five-minute redemption result transition is invalid")]
    InvalidRedemptionTransition,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PmBtcFiveMinuteRolloverReadiness {
    Active,
    Quiesce,
    AwaitingZeroOpenOrders { open_orders: usize },
    Ready,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PmBtcFiveMinuteSettlementState {
    AwaitingResolution { inventory: U256 },
    ReadyToRedeem { inventory: U256 },
    RedemptionDispatched { inventory: U256 },
    Complete,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PmBtcFiveMinuteRollover {
    retired_condition: PmConditionId,
    active_condition: PmConditionId,
    skipped_windows: u64,
}

impl PmBtcFiveMinuteRollover {
    #[must_use]
    pub const fn retired_condition(self) -> PmConditionId {
        self.retired_condition
    }

    #[must_use]
    pub const fn active_condition(self) -> PmConditionId {
        self.active_condition
    }

    #[must_use]
    pub const fn skipped_windows(self) -> u64 {
        self.skipped_windows
    }
}

#[derive(Debug)]
struct PendingSettlement {
    condition: PmConditionId,
    up_token: PmTokenId,
    down_token: PmTokenId,
    negative_risk: bool,
    state: PmBtcFiveMinuteSettlementState,
}

/// One active window plus every non-flat retired window still requiring
/// resolution/redemption reconciliation.
#[derive(Debug)]
pub struct PmBtcFiveMinuteLifecycle {
    active: PmBtcFiveMinuteMarket,
    pending: Vec<PendingSettlement>,
}

impl PmBtcFiveMinuteLifecycle {
    #[must_use]
    pub fn new(active: PmBtcFiveMinuteMarket) -> Self {
        Self {
            active,
            pending: Vec::new(),
        }
    }

    #[must_use]
    pub const fn active(&self) -> &PmBtcFiveMinuteMarket {
        &self.active
    }

    #[must_use]
    pub fn pending_settlement_count(&self) -> usize {
        self.pending
            .iter()
            .filter(|settlement| settlement.state != PmBtcFiveMinuteSettlementState::Complete)
            .count()
    }

    /// Decide whether strategy entry must stop and whether the active market
    /// can be replaced. `now_millis` is an observed wall-clock fact; the
    /// caller remains responsible for clock-quality admission.
    #[must_use]
    pub fn rollover_readiness(
        &self,
        now_millis: u64,
        complete_open_order_count: usize,
    ) -> PmBtcFiveMinuteRolloverReadiness {
        let now_seconds = now_millis / 1_000;
        let end = self.active.window_end_epoch();
        if now_seconds < end.saturating_sub(QUIESCE_BEFORE_END_SECONDS) {
            PmBtcFiveMinuteRolloverReadiness::Active
        } else if now_seconds < end {
            PmBtcFiveMinuteRolloverReadiness::Quiesce
        } else if complete_open_order_count != 0 {
            PmBtcFiveMinuteRolloverReadiness::AwaitingZeroOpenOrders {
                open_orders: complete_open_order_count,
            }
        } else {
            PmBtcFiveMinuteRolloverReadiness::Ready
        }
    }

    /// Retire the active window only after its end and a complete zero-order
    /// proof, then admit a later aligned discovered market. Skipped windows
    /// are reported explicitly for restart/recovery telemetry.
    pub fn rollover(
        &mut self,
        candidate: PmBtcFiveMinuteMarket,
        now_millis: u64,
        complete_open_order_count: usize,
    ) -> Result<PmBtcFiveMinuteRollover, PmBtcFiveMinuteLifecycleError> {
        if now_millis / 1_000 < self.active.window_end_epoch() {
            return Err(PmBtcFiveMinuteLifecycleError::RolloverTooEarly);
        }
        if complete_open_order_count != 0 {
            return Err(PmBtcFiveMinuteLifecycleError::OpenOrdersRemain);
        }
        let candidate_start = candidate.window_start_epoch();
        let expected_start = self.active.window_end_epoch();
        if candidate_start < expected_start
            || !candidate_start.is_multiple_of(FIVE_MINUTE_SECONDS)
            || candidate.window_end_epoch() != candidate_start.saturating_add(FIVE_MINUTE_SECONDS)
        {
            return Err(PmBtcFiveMinuteLifecycleError::InvalidRolloverCandidate);
        }
        if candidate.condition() == self.active.condition()
            || candidate.up_token() == self.active.up_token()
            || candidate.up_token() == self.active.down_token()
            || candidate.down_token() == self.active.up_token()
            || candidate.down_token() == self.active.down_token()
            || self.pending.iter().any(|settlement| {
                settlement.condition == candidate.condition()
                    || settlement.up_token == candidate.up_token()
                    || settlement.down_token == candidate.up_token()
                    || settlement.up_token == candidate.down_token()
                    || settlement.down_token == candidate.down_token()
            })
        {
            return Err(PmBtcFiveMinuteLifecycleError::RepeatedMarketIdentity);
        }
        if self.pending_settlement_count() >= MAX_PENDING_BTC_FIVE_MINUTE_SETTLEMENTS {
            return Err(PmBtcFiveMinuteLifecycleError::PendingSettlementBound);
        }
        let skipped_windows = candidate_start.saturating_sub(expected_start) / FIVE_MINUTE_SECONDS;
        let retired_condition = self.active.condition();
        self.pending.push(PendingSettlement {
            condition: retired_condition,
            up_token: self.active.up_token(),
            down_token: self.active.down_token(),
            negative_risk: self.active.negative_risk(),
            state: PmBtcFiveMinuteSettlementState::AwaitingResolution {
                inventory: U256::ZERO,
            },
        });
        let active_condition = candidate.condition();
        self.active = candidate;
        Ok(PmBtcFiveMinuteRollover {
            retired_condition,
            active_condition,
            skipped_windows,
        })
    }

    /// Apply authoritative Data API observations for both retired outcomes.
    /// Any nonzero redeemable row makes the condition a redemption candidate;
    /// both outcomes are still passed to the eventual CTF redemption call.
    pub fn observe_settlement(
        &mut self,
        condition: PmConditionId,
        up: &PmConfiguredTokenPosition,
        down: &PmConfiguredTokenPosition,
    ) -> Result<PmBtcFiveMinuteSettlementState, PmBtcFiveMinuteLifecycleError> {
        let settlement = self
            .pending
            .iter_mut()
            .find(|settlement| settlement.condition == condition)
            .ok_or(PmBtcFiveMinuteLifecycleError::UnknownSettlement)?;
        let up = position_view(up, settlement.up_token)?;
        let down = position_view(down, settlement.down_token)?;
        let inventory = up
            .quantity
            .checked_add(down.quantity)
            .map_err(|_| PmBtcFiveMinuteLifecycleError::PositionOverflow)?;
        settlement.state = if inventory.is_zero() {
            PmBtcFiveMinuteSettlementState::Complete
        } else if matches!(
            settlement.state,
            PmBtcFiveMinuteSettlementState::RedemptionDispatched { .. }
        ) {
            PmBtcFiveMinuteSettlementState::RedemptionDispatched { inventory }
        } else if up.redeemable || down.redeemable {
            PmBtcFiveMinuteSettlementState::ReadyToRedeem { inventory }
        } else {
            PmBtcFiveMinuteSettlementState::AwaitingResolution { inventory }
        };
        Ok(settlement.state)
    }

    /// Record that a separate relayer/proxy redemption plane crossed its
    /// may-have-dispatched boundary. The state remains pending until later
    /// authoritative position observations prove zero.
    pub fn mark_redemption_dispatched(
        &mut self,
        condition: PmConditionId,
    ) -> Result<(), PmBtcFiveMinuteLifecycleError> {
        let settlement = self
            .pending
            .iter_mut()
            .find(|settlement| settlement.condition == condition)
            .ok_or(PmBtcFiveMinuteLifecycleError::UnknownSettlement)?;
        let PmBtcFiveMinuteSettlementState::ReadyToRedeem { inventory } = settlement.state else {
            return Err(PmBtcFiveMinuteLifecycleError::RedemptionNotReady);
        };
        settlement.state = PmBtcFiveMinuteSettlementState::RedemptionDispatched { inventory };
        Ok(())
    }

    /// Only a definitely-not-dispatched relayer result can re-open the
    /// candidate. Unknown outcomes stay `RedemptionDispatched` for polling.
    pub fn mark_redemption_definitely_not_dispatched(
        &mut self,
        condition: PmConditionId,
    ) -> Result<(), PmBtcFiveMinuteLifecycleError> {
        let settlement = self
            .pending
            .iter_mut()
            .find(|settlement| settlement.condition == condition)
            .ok_or(PmBtcFiveMinuteLifecycleError::UnknownSettlement)?;
        let PmBtcFiveMinuteSettlementState::RedemptionDispatched { inventory } = settlement.state
        else {
            return Err(PmBtcFiveMinuteLifecycleError::InvalidRedemptionTransition);
        };
        settlement.state = PmBtcFiveMinuteSettlementState::ReadyToRedeem { inventory };
        Ok(())
    }

    #[must_use]
    pub fn settlement_state(
        &self,
        condition: PmConditionId,
    ) -> Option<PmBtcFiveMinuteSettlementState> {
        self.pending
            .iter()
            .find(|settlement| settlement.condition == condition)
            .map(|settlement| settlement.state)
    }

    /// Exposes whether the retired market needs the distinct negative-risk
    /// redemption contract. A standard-only redemption plane must reject it.
    #[must_use]
    pub fn settlement_negative_risk(&self, condition: PmConditionId) -> Option<bool> {
        self.pending
            .iter()
            .find(|settlement| settlement.condition == condition)
            .map(|settlement| settlement.negative_risk)
    }
}

#[derive(Clone, Copy)]
struct PositionView {
    quantity: U256,
    redeemable: bool,
}

fn position_view(
    position: &PmConfiguredTokenPosition,
    expected_token: PmTokenId,
) -> Result<PositionView, PmBtcFiveMinuteLifecycleError> {
    match position {
        PmConfiguredTokenPosition::Absent => Ok(PositionView {
            quantity: U256::ZERO,
            redeemable: false,
        }),
        PmConfiguredTokenPosition::Present(evidence) => {
            if evidence.asset() != expected_token {
                return Err(PmBtcFiveMinuteLifecycleError::PositionScopeMismatch);
            }
            Ok(PositionView {
                quantity: evidence
                    .size_protocol_units()
                    .map_err(|_| PmBtcFiveMinuteLifecycleError::PositionQuantity)?,
                redeemable: evidence.redeemable(),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn classify_synthetic(
        previous: PmBtcFiveMinuteSettlementState,
        up: PositionView,
        down: PositionView,
    ) -> PmBtcFiveMinuteSettlementState {
        let inventory = up.quantity.checked_add(down.quantity).unwrap();
        if inventory.is_zero() {
            PmBtcFiveMinuteSettlementState::Complete
        } else if matches!(
            previous,
            PmBtcFiveMinuteSettlementState::RedemptionDispatched { .. }
        ) {
            PmBtcFiveMinuteSettlementState::RedemptionDispatched { inventory }
        } else if up.redeemable || down.redeemable {
            PmBtcFiveMinuteSettlementState::ReadyToRedeem { inventory }
        } else {
            PmBtcFiveMinuteSettlementState::AwaitingResolution { inventory }
        }
    }

    #[test]
    fn settlement_never_makes_resolved_inventory_tradable() {
        let quantity = U256::from_u64(5_000_000);
        assert_eq!(
            classify_synthetic(
                PmBtcFiveMinuteSettlementState::AwaitingResolution {
                    inventory: quantity
                },
                PositionView {
                    quantity,
                    redeemable: true
                },
                PositionView {
                    quantity: U256::ZERO,
                    redeemable: false
                }
            ),
            PmBtcFiveMinuteSettlementState::ReadyToRedeem {
                inventory: quantity
            }
        );
        assert_eq!(
            classify_synthetic(
                PmBtcFiveMinuteSettlementState::RedemptionDispatched {
                    inventory: quantity
                },
                PositionView {
                    quantity,
                    redeemable: true
                },
                PositionView {
                    quantity: U256::ZERO,
                    redeemable: false
                }
            ),
            PmBtcFiveMinuteSettlementState::RedemptionDispatched {
                inventory: quantity
            }
        );
    }

    #[test]
    fn zero_authoritative_inventory_completes_even_after_unknown_dispatch() {
        assert_eq!(
            classify_synthetic(
                PmBtcFiveMinuteSettlementState::RedemptionDispatched {
                    inventory: U256::ONE
                },
                PositionView {
                    quantity: U256::ZERO,
                    redeemable: false
                },
                PositionView {
                    quantity: U256::ZERO,
                    redeemable: false
                }
            ),
            PmBtcFiveMinuteSettlementState::Complete
        );
    }
}
