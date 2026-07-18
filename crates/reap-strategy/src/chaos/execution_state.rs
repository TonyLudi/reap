use std::collections::HashMap;

use reap_core::{
    OrderEvent, OrderUpdate, Price, Quantity, Side, Symbol, TimeMs, round_down_to_lot,
};
use serde::{Deserialize, Serialize};

use crate::ChaosExecutionIntent;

use super::{ChaosStrategy, StableMap, TheoQuote, approx_eq, round_passive_to_tick};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MissedHedge {
    pub ts_ms: TimeMs,
    pub order_id: String,
    pub symbol: Symbol,
    pub side: Side,
    pub price: Price,
    pub missed_qty: Quantity,
    pub missed_delta_usd: f64,
    pub reference_symbol: Option<Symbol>,
}

#[derive(Debug, Clone)]
pub(super) struct ActiveQuote {
    pub(super) order_id: String,
    pub(super) price: Price,
    pub(super) qty: Quantity,
}

#[derive(Debug, Clone)]
pub(super) struct ActiveHedge {
    pub(super) symbol: Symbol,
    pub(super) signed_open_qty: Quantity,
    pub(super) price: Price,
    pub(super) reference_price: Price,
    pub(super) updated_ms: TimeMs,
}

#[derive(Debug, Clone)]
pub(super) struct ExecutionTrackingState {
    pub(super) active_quotes: StableMap<(Symbol, Side, usize), ActiveQuote>,
    pub(super) active_hedges: StableMap<String, ActiveHedge>,
    pub(super) last_quote_fill_ms: HashMap<(Symbol, Side), TimeMs>,
    pub(super) missed_hedges: Vec<MissedHedge>,
}

impl ChaosStrategy {
    pub(super) fn sync_quotes(
        &mut self,
        symbol: &str,
        side: Side,
        desired: &[TheoQuote],
        commands: &mut Vec<ChaosExecutionIntent>,
    ) {
        let Some(entity) = self.entities.get(symbol) else {
            return;
        };
        let min_refill_interval_ms = entity.config.min_refill_interval_ms;
        let refill_blocked = self
            .execution
            .last_quote_fill_ms
            .get(&(symbol.to_string(), side))
            .is_some_and(|last_fill_ms| {
                self.now_ms.saturating_sub(*last_fill_ms) < min_refill_interval_ms
            });
        let desired = desired
            .iter()
            .filter_map(|quote| {
                let price = round_passive_to_tick(quote.price, entity.config.tick_size, side);
                let qty = round_down_to_lot(quote.qty, entity.config.lot_size);
                (price.is_finite() && price > 0.0 && qty >= entity.config.min_trade_size)
                    .then_some((price, qty))
            })
            .collect::<Vec<_>>();

        let mut active_levels = self
            .execution
            .active_quotes
            .iter()
            .filter(|((active_symbol, active_side, _), _)| {
                active_symbol == symbol && *active_side == side
            })
            .map(|((_, _, level), quote)| (*level, quote))
            .collect::<Vec<_>>();
        active_levels.sort_unstable_by_key(|(level, _)| *level);

        for (level, active) in &active_levels {
            let matches = desired.get(*level).is_some_and(|(price, qty)| {
                approx_eq(active.price, *price)
                    && (approx_eq(active.qty, *qty)
                        || (*level == 0 && refill_blocked && active.qty < *qty))
            });
            if !matches {
                commands.push(ChaosExecutionIntent::cancel_owned(
                    active.order_id.clone(),
                    if desired.get(*level).is_some() {
                        "replace_quote".to_string()
                    } else {
                        "quote_disabled".to_string()
                    },
                ));
            }
        }

        for (level, (price, qty)) in desired.into_iter().enumerate() {
            let active = active_levels
                .binary_search_by_key(&level, |(active_level, _)| *active_level)
                .ok()
                .map(|index| active_levels[index].1);
            let current_matches = active.is_some_and(|active| {
                approx_eq(active.price, price)
                    && (approx_eq(active.qty, qty)
                        || (level == 0 && refill_blocked && active.qty < qty))
            });
            if current_matches {
                continue;
            }
            if level == 0 && refill_blocked && active.is_none() {
                continue;
            }
            commands.push(ChaosExecutionIntent::quote(
                symbol.to_string(),
                side,
                qty,
                price,
                if level == 0 {
                    "quote".to_string()
                } else {
                    format!("quote:{level}")
                },
            ));
        }
    }

    pub(super) fn on_order_update(&mut self, update: &OrderUpdate) -> Vec<ChaosExecutionIntent> {
        self.advance_time(update.ts_ms);
        if update.event == OrderEvent::Cancelled && update.reason.starts_with("hedge") {
            let missed_qty = (update.qty - update.filled_qty).max(0.0);
            if missed_qty > 0.0
                && let (Some(entity), Some(ref_mid)) =
                    (self.entities.get(&update.symbol), self.ref_mid())
            {
                self.execution.missed_hedges.push(MissedHedge {
                    ts_ms: update.ts_ms,
                    order_id: update.order_id.clone(),
                    symbol: update.symbol.clone(),
                    side: update.side,
                    price: update.price,
                    missed_qty,
                    missed_delta_usd: entity
                        .delta_coin_for_qty(update.side.factor() * missed_qty, update.price)
                        * ref_mid,
                    reference_symbol: update.reason.split(':').nth(1).map(str::to_string),
                });
                if self.execution.missed_hedges.len() > 4_096 {
                    self.execution.missed_hedges.remove(0);
                }
            }
        }
        if update.reason.starts_with("hedge") {
            if matches!(
                update.event,
                OrderEvent::PendingNew | OrderEvent::New | OrderEvent::PartialFill
            ) && update.open_qty > 0.0
            {
                self.execution.active_hedges.insert(
                    update.order_id.clone(),
                    ActiveHedge {
                        symbol: update.symbol.clone(),
                        signed_open_qty: update.side.factor() * update.open_qty,
                        price: update.price,
                        reference_price: hedge_reference_price(&update.reason)
                            .unwrap_or(update.price),
                        updated_ms: update.ts_ms,
                    },
                );
            } else if matches!(
                update.event,
                OrderEvent::Cancelled | OrderEvent::FullyFilled | OrderEvent::Rejected
            ) {
                self.execution.active_hedges.remove(&update.order_id);
            }
        }
        match update.event {
            OrderEvent::PendingNew | OrderEvent::New if update.reason.starts_with("quote") => {
                let level = quote_level_from_reason(&update.reason);
                self.execution.active_quotes.insert(
                    (update.symbol.clone(), update.side, level),
                    ActiveQuote {
                        order_id: update.order_id.clone(),
                        price: update.price,
                        qty: update.open_qty,
                    },
                );
            }
            OrderEvent::PartialFill if update.reason.starts_with("quote") => {
                if let Some(active) = self
                    .execution
                    .active_quotes
                    .values_mut()
                    .find(|quote| quote.order_id == update.order_id)
                {
                    active.qty = update.open_qty;
                }
            }
            OrderEvent::Cancelled | OrderEvent::FullyFilled | OrderEvent::Rejected => {
                self.execution
                    .active_quotes
                    .retain(|_, quote| quote.order_id != update.order_id);
            }
            _ => {}
        }

        if update.has_fill() && update.reason.starts_with("quote") {
            self.execution
                .last_quote_fill_ms
                .insert((update.symbol.clone(), update.side), self.now_ms);
        }

        if update.has_fill() {
            if let Some(ref_mid) = self.ref_mid()
                && let Some(entity) = self.entities.get(&update.symbol)
            {
                let turnover =
                    entity.notional_usd(update.last_fill_qty, update.last_fill_price, ref_mid);
                self.risk.net_filled_delta_usd += entity.delta_coin_for_qty(
                    update.side.factor() * update.last_fill_qty,
                    update.last_fill_price,
                ) * ref_mid;
                *self
                    .risk
                    .turnover_by_group
                    .entry(entity.config.risk_group.clone())
                    .or_default() += turnover;
            }
            if let Some(entity) = self.entities.get_mut(&update.symbol) {
                entity.record_fill(
                    update.side,
                    update.last_fill_qty,
                    update.last_fill_price,
                    update.last_fill_liquidity,
                );
                if !update.reason.starts_with("hedge")
                    && entity.anomalous_fill_should_stop(
                        update.ts_ms,
                        update.side,
                        update.price,
                        update.last_fill_price,
                    )
                {
                    self.halt_reason = Some(format!(
                        "{} received consecutive anomalous fills",
                        update.symbol
                    ));
                }
            }
            self.update_risk();
            return Vec::new();
        }

        Vec::new()
    }
}

fn quote_level_from_reason(reason: &str) -> usize {
    reason
        .split(':')
        .nth(1)
        .and_then(|level| level.parse().ok())
        .unwrap_or(0)
}

fn hedge_reference_price(reason: &str) -> Option<Price> {
    reason.split(':').nth(2)?.parse().ok()
}
