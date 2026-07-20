use std::cell::Cell;

use reap_core::{OrderBook, Price, Quantity, Side, TimeMs};

use crate::execution::ChaosHedgeCommit;

use super::ChaosStrategy;

const OKX_PENDING_HEDGE_EXPIRY_MS: TimeMs = 30;

#[derive(Debug, Clone, Copy)]
struct PublicTrade {
    price: Price,
    qty: Quantity,
}

#[derive(Debug, Clone, Copy)]
struct PendingHedge {
    price: Price,
    qty: Quantity,
    updated_ms: TimeMs,
}

impl PendingHedge {
    const fn empty(side: Side) -> Self {
        Self {
            price: match side {
                Side::Buy => 0.0,
                Side::Sell => f64::INFINITY,
            },
            qty: 0.0,
            updated_ms: 0,
        }
    }

    fn clear(&mut self, side: Side) {
        *self = Self::empty(side);
    }
}

/// The reached `OkEntity` implied-depth state.
///
/// It deliberately remains below `InstrumentState`: public trades, depth
/// replacement, and locally sent hedges are the only transitions that can
/// mutate it.
#[derive(Debug, Clone)]
pub(super) struct ImpliedDepthState {
    last_trade: [Option<PublicTrade>; 2],
    first_valid_level: [Cell<Option<usize>>; 2],
    pending_hedge: [PendingHedge; 2],
}

impl Default for ImpliedDepthState {
    fn default() -> Self {
        Self {
            last_trade: [None, None],
            first_valid_level: [Cell::new(None), Cell::new(None)],
            pending_hedge: [
                PendingHedge::empty(Side::Buy),
                PendingHedge::empty(Side::Sell),
            ],
        }
    }
}

impl ImpliedDepthState {
    pub(super) fn on_depth(&mut self, now_ms: TimeMs) {
        self.last_trade = [None, None];
        self.clear_first_valid_levels();
        for side in [Side::Buy, Side::Sell] {
            let pending = &mut self.pending_hedge[side_index(side)];
            if pending.updated_ms != 0
                && pending
                    .updated_ms
                    .saturating_add(OKX_PENDING_HEDGE_EXPIRY_MS)
                    < now_ms
            {
                pending.clear(side);
            }
        }
    }

    /// Applies `OkEntity.onPublicTrade` and returns the separate
    /// `isDepthUpdatedOnTrade` result used by iarb2 scheduling.
    pub(super) fn on_public_trade(
        &mut self,
        book: Option<&OrderBook>,
        price: Price,
        qty: Quantity,
        taker_side: Side,
    ) -> bool {
        let book_side = taker_side.reverse();
        let index = side_index(book_side);
        self.last_trade[index] = Some(PublicTrade { price, qty });
        self.first_valid_level[index].set(None);

        let crossed = book
            .and_then(|book| book.levels(book_side).first())
            .is_some_and(|raw_best| java_more_aggressive(book_side, raw_best.px, price));
        let opposite = side_index(book_side.reverse());
        if crossed && self.last_trade[opposite].is_some() {
            self.last_trade[opposite] = None;
            self.first_valid_level[opposite].set(None);
        }
        crossed
    }

    pub(super) fn first_valid_level(
        &self,
        book: &OrderBook,
        side: Side,
        ignore_best_level: bool,
    ) -> usize {
        let index = side_index(side);
        if let Some(level) = self.first_valid_level[index].get() {
            return level;
        }

        let levels = book.levels(side);
        let Some(last_trade) = self.last_trade[index] else {
            // Preserve Reap's pre-Phase-2 no-trade fallback for a one-level
            // book. Java returns index 1 here, but changing an existing
            // no-trade decision is outside this phase.
            let first = usize::from(ignore_best_level && levels.len() > 1);
            self.first_valid_level[index].set(Some(first));
            return first;
        };

        let pending_price = self.pending_hedge[side_index(side.reverse())].price;
        for (level_index, level) in levels.iter().enumerate() {
            if ignore_best_level && level_index == 0 {
                continue;
            }
            let valid_after_trade = java_more_passive(side, level.px, last_trade.price)
                || (level.px == last_trade.price && last_trade.qty < level.qty / 2.0);
            if valid_after_trade && java_equal_or_more_passive(side, level.px, pending_price) {
                self.first_valid_level[index].set(Some(level_index));
                return level_index;
            }
        }

        let final_level = levels.len().saturating_sub(1);
        self.first_valid_level[index].set(Some(final_level));
        final_level
    }

    /// Mirrors `ChaosEntity.updateOurHedge` followed by
    /// `ExchEntityBase.updateOurHedge`, including the pinned double-quantity
    /// update when a more-aggressive price first replaces the pending level.
    pub(super) fn update_our_hedge(
        &mut self,
        hedge_side: Side,
        price: Price,
        qty: Quantity,
        now_ms: TimeMs,
    ) {
        self.first_valid_level[side_index(hedge_side.reverse())].set(None);
        let pending = &mut self.pending_hedge[side_index(hedge_side)];
        if java_more_aggressive(hedge_side, price, pending.price) {
            *pending = PendingHedge {
                price,
                qty,
                updated_ms: now_ms,
            };
        }
        if price == pending.price {
            pending.qty += qty;
            pending.updated_ms = now_ms;
        }
    }

    fn clear_first_valid_levels(&self) {
        for cached in &self.first_valid_level {
            cached.set(None);
        }
    }
}

impl ChaosStrategy {
    /// Runs the caller's local-send reservation with the genuine moved intent
    /// and commits any private implied-depth hedge transition only on success,
    /// sampling the supplied clock after that reservation returns.
    ///
    /// The genuine non-Clone strategy intent is consumed exactly once; no
    /// deferred state token crosses the public crate boundary.
    pub fn with_locally_sent_intent<T, E>(
        &mut self,
        mut intent: crate::ChaosExecutionIntent,
        local_send_clock: impl FnOnce() -> TimeMs,
        reserve: impl FnOnce(crate::ChaosExecutionIntent) -> Result<T, E>,
    ) -> Result<T, E> {
        let commit = intent.take_hedge_commit();
        let accepted = reserve(intent)?;
        if let Some(commit) = commit {
            self.commit_sent_hedge(commit, local_send_clock());
        }
        Ok(accepted)
    }

    fn commit_sent_hedge(&mut self, commit: ChaosHedgeCommit, observed_now_ms: TimeMs) {
        let (symbol, side, price, qty) = commit.into_parts();
        let Some(entity) = self.entities.get_mut(symbol.as_ref()) else {
            self.halt_reason = Some(format!(
                "accepted hedge implied-depth state references unknown symbol {symbol}"
            ));
            return;
        };
        entity
            .implied_depth
            .update_our_hedge(side, price, qty, observed_now_ms);
    }
}

const fn side_index(side: Side) -> usize {
    match side {
        Side::Buy => 0,
        Side::Sell => 1,
    }
}

fn java_more_aggressive(side: Side, left: f64, right: f64) -> bool {
    match side {
        Side::Buy => matches!(java_compare(left, right), Some(std::cmp::Ordering::Greater)),
        Side::Sell => matches!(java_compare(left, right), Some(std::cmp::Ordering::Less)),
    }
}

fn java_more_passive(side: Side, left: f64, right: f64) -> bool {
    match side {
        Side::Buy => matches!(java_compare(left, right), Some(std::cmp::Ordering::Less)),
        Side::Sell => matches!(java_compare(left, right), Some(std::cmp::Ordering::Greater)),
    }
}

fn java_equal_or_more_passive(side: Side, left: f64, right: f64) -> bool {
    match side {
        Side::Buy => matches!(
            java_compare(left, right),
            Some(std::cmp::Ordering::Less | std::cmp::Ordering::Equal)
        ),
        Side::Sell => matches!(
            java_compare(left, right),
            Some(std::cmp::Ordering::Greater | std::cmp::Ordering::Equal)
        ),
    }
}

fn java_compare(left: f64, right: f64) -> Option<std::cmp::Ordering> {
    if left.is_nan() || right.is_nan() {
        // NumberUtil's relational helpers fall back to Java's primitive
        // comparison for NaN, for which every <, >, <=, and >= is false.
        return None;
    }
    if left == right || (left - right).abs() <= java_epsilon(left, right) {
        Some(std::cmp::Ordering::Equal)
    } else {
        Some(left.total_cmp(&right))
    }
}

fn java_epsilon(left: f64, right: f64) -> f64 {
    let magnitude = left.abs().min(right.abs());
    if magnitude < 1e-10 {
        1e-25
    } else if magnitude < 1e-8 {
        1e-23
    } else if magnitude < 1e-6 {
        1e-21
    } else if magnitude <= 1e3 {
        1e-12
    } else if magnitude <= 1e5 {
        1e-10
    } else if magnitude <= 1e7 {
        1e-8
    } else if magnitude <= 1e9 {
        1e-6
    } else if magnitude <= 1e11 {
        1e-4
    } else {
        1e-2
    }
}

#[cfg(test)]
mod tests {
    use reap_core::Side;

    use super::{java_equal_or_more_passive, java_more_aggressive, java_more_passive};

    #[test]
    fn java_relational_helpers_keep_nan_unordered() {
        for side in [Side::Buy, Side::Sell] {
            assert!(!java_more_aggressive(side, f64::NAN, 1.0));
            assert!(!java_more_aggressive(side, 1.0, f64::NAN));
            assert!(!java_more_passive(side, f64::NAN, 1.0));
            assert!(!java_more_passive(side, 1.0, f64::NAN));
            assert!(!java_equal_or_more_passive(side, f64::NAN, 1.0));
            assert!(!java_equal_or_more_passive(side, 1.0, f64::NAN));
        }
    }
}
