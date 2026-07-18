use std::collections::HashMap;

use reap_core::{Price, Quantity, Side, Symbol, TimeMs};
use serde::{Deserialize, Serialize};

use super::{ChaosStrategy, HedgeLevel, InstrumentState, LIVE_ORDER_STOP_QUOTE_THRESHOLD};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TheoQuote {
    pub price: Price,
    pub qty: Quantity,
    pub hedge_px: Price,
    #[serde(skip)]
    pub hedge_symbol: Symbol,
}

#[derive(Debug, Clone)]
pub(super) struct QuoteTargetState {
    levels: Vec<TheoQuote>,
    updated_ms: TimeMs,
}

#[derive(Debug, Clone)]
pub(super) struct JavaRandom {
    seed: u64,
}

impl JavaRandom {
    const MULTIPLIER: u64 = 0x5DEECE66D;
    const ADDEND: u64 = 0xB;
    const MASK: u64 = (1_u64 << 48) - 1;

    pub(super) fn new(seed: u64) -> Self {
        Self {
            seed: (seed ^ Self::MULTIPLIER) & Self::MASK,
        }
    }

    fn next_bits(&mut self, bits: u32) -> u64 {
        self.seed = (self
            .seed
            .wrapping_mul(Self::MULTIPLIER)
            .wrapping_add(Self::ADDEND))
            & Self::MASK;
        self.seed >> (48 - bits)
    }

    pub(super) fn next_f64(&mut self) -> f64 {
        let high = self.next_bits(26);
        let low = self.next_bits(27);
        ((high << 27) + low) as f64 / (1_u64 << 53) as f64
    }
}

impl ChaosStrategy {
    pub(super) fn desired_quote_levels(
        &mut self,
        symbol: &str,
        side: Side,
        desired: Option<TheoQuote>,
    ) -> Vec<TheoQuote> {
        let key = (symbol.to_string(), side);
        let Some(mut top) = desired else {
            self.quote_targets.remove(&key);
            return Vec::new();
        };
        let Some(entity) = self.entities.get(symbol).cloned() else {
            return Vec::new();
        };
        let Some(ref_mid) = self.ref_mid() else {
            return Vec::new();
        };
        let quote_level_count = entity.quote_level_count(side);
        top.price = round_passive_to_tick(top.price, entity.config.tick_size, side);

        if let Some(current) = self.quote_targets.get(&key) {
            let current_top = current.levels.first();
            let price_changed = current_top.is_none_or(|quote| {
                let passive_width = if self.config.quote_only {
                    0.0
                } else {
                    entity.config.debounce_width
                };
                let more_aggressive = side.factor() * (top.price - quote.price) / top.price
                    > entity.config.debounce_width;
                let less_aggressive =
                    side.factor() * (quote.price - top.price) / top.price > passive_width;
                more_aggressive || less_aggressive
            });
            let debounce_qty = entity.size_from_usd(entity.config.debounce_size_usd, ref_mid);
            let size_reduced = current_top.is_some_and(|quote| top.qty < quote.qty - debounce_qty);
            let force_update = self.now_ms.saturating_sub(current.updated_ms)
                >= entity.config.force_quote_update_ms;
            let level_count_changed = current.levels.len() != quote_level_count;
            if !price_changed && !size_reduced && !force_update && !level_count_changed {
                return current.levels.clone();
            }
            let force_debounce = current_top.is_some_and(|quote| match side {
                Side::Buy => top.price < quote.price * 0.999,
                Side::Sell => top.price > quote.price * 1.001,
            });
            if !force_debounce
                && !force_update
                && !level_count_changed
                && self.now_ms.saturating_sub(current.updated_ms) < entity.config.debounce_ms
            {
                return current.levels.clone();
            }
        }

        let mut levels = Vec::with_capacity(quote_level_count);
        levels.push(top.clone());
        let min_qty = entity.size_from_usd(entity.config.min_order_size_usd, ref_mid);
        let max_qty = entity.size_from_usd(entity.config.max_order_size_usd, ref_mid);
        let qty_span = (max_qty - min_qty).max(0.0);
        let spread_span = entity.config.max_level_spread - entity.config.min_level_spread;
        let mut last_px = top.price;
        for _ in 1..quote_level_count {
            let random = self.random.next_f64();
            let spread = entity.config.min_level_spread + random * spread_span;
            let raw_px = match side {
                Side::Buy => last_px * (1.0 - spread),
                Side::Sell => last_px * (1.0 + spread),
            };
            let rounded = round_passive_to_tick(raw_px, entity.config.tick_size, side);
            let price = match side {
                Side::Buy => rounded.min(last_px - entity.config.tick_size),
                Side::Sell => rounded.max(last_px + entity.config.tick_size),
            };
            let qty = min_qty + random * qty_span;
            levels.push(TheoQuote {
                price,
                qty,
                hedge_px: top.hedge_px,
                hedge_symbol: top.hedge_symbol.clone(),
            });
            last_px = price;
        }
        self.quote_targets.insert(
            key,
            QuoteTargetState {
                levels: levels.clone(),
                updated_ms: self.now_ms,
            },
        );
        levels
    }

    pub(super) fn update_theo_quotes(&mut self) {
        let mut group_names = self.risk_groups.keys().cloned().collect::<Vec<_>>();
        group_names.sort();
        for group_name in group_names {
            for side in [Side::Buy, Side::Sell] {
                let Some(rg) = self.risk_groups.get(&group_name) else {
                    continue;
                };
                let symbols = rg.ordered_symbols.clone();
                if !rg.can_quote_side(side)
                    || rg.live_order_size_usd
                        > LIVE_ORDER_STOP_QUOTE_THRESHOLD * rg.config.live_order_limit_usd
                {
                    for symbol in symbols {
                        if let Some(entity) = self.entities.get_mut(&symbol) {
                            entity.clear_theo(side);
                        }
                    }
                    continue;
                }

                let hedge_side = side.reverse();
                let hedges = if rg.can_increase_delta_with_quote_buffer(side) {
                    self.best_hedges
                        .get(&hedge_side)
                        .cloned()
                        .unwrap_or_default()
                } else {
                    rg.best_hedges_for(hedge_side).to_vec()
                };

                for symbol in symbols {
                    self.update_theo_for_symbol(&symbol, side, &hedges);
                }
            }
        }

        for entity in self.entities.values_mut() {
            entity.prevent_self_cross();
        }
    }

    fn update_theo_for_symbol(&mut self, symbol: &str, side: Side, hedges: &[HedgeLevel]) {
        let quote = self.calculate_theo_for_symbol(symbol, side, hedges);
        if let Some(entity) = self.entities.get_mut(symbol) {
            match quote {
                Some(quote) => entity.set_theo(side, quote),
                None => entity.clear_theo(side),
            }
        }
    }

    fn calculate_theo_for_symbol(
        &self,
        symbol: &str,
        side: Side,
        hedges: &[HedgeLevel],
    ) -> Option<TheoQuote> {
        let ref_mid = self.ref_mid()?;
        let pricing_entity = self.entities.get(symbol)?;
        if pricing_entity.quote_profit_margin() >= 1.0 {
            return None;
        }

        let mut px_by_hedge = 0.0;
        let mut current_size_usd = 0.0;
        let mut hedge_notional_by_symbol: HashMap<Symbol, f64> = HashMap::new();
        let mut weighted_hedge_px_by_symbol: HashMap<Symbol, (f64, f64)> = HashMap::new();
        let mut best_hedge_symbol = String::new();

        for hedge_level in hedges {
            if hedge_level.symbol == symbol {
                continue;
            }
            let Some(hedge_entity) = self.entities.get(&hedge_level.symbol) else {
                continue;
            };
            if best_hedge_symbol.is_empty() {
                best_hedge_symbol = hedge_level.symbol.clone();
            }

            let current_px =
                price_by_hedge(side, pricing_entity, hedge_entity, hedge_level, ref_mid);
            if !current_px.is_finite() || current_px <= 0.0 {
                continue;
            }

            let last_size_usd = current_size_usd;
            current_size_usd += hedge_level.notional_usd;

            if current_size_usd < pricing_entity.config.min_order_size_usd {
                px_by_hedge = weighted_avg(
                    px_by_hedge,
                    last_size_usd,
                    current_px,
                    hedge_level.notional_usd,
                );
                *hedge_notional_by_symbol
                    .entry(hedge_level.symbol.clone())
                    .or_default() += hedge_level.notional_usd;
                update_weighted_px(
                    &mut weighted_hedge_px_by_symbol,
                    &hedge_level.symbol,
                    hedge_level.px,
                    hedge_level.notional_usd,
                );
                continue;
            }

            let (this_size_usd, quote_size_usd) =
                if current_size_usd > pricing_entity.config.max_order_size_usd {
                    (
                        pricing_entity.config.max_order_size_usd - last_size_usd,
                        pricing_entity.config.max_order_size_usd,
                    )
                } else {
                    (hedge_level.notional_usd, current_size_usd)
                };
            if this_size_usd <= 0.0 {
                break;
            }

            let px = weighted_avg(px_by_hedge, last_size_usd, current_px, this_size_usd);
            *hedge_notional_by_symbol
                .entry(hedge_level.symbol.clone())
                .or_default() += this_size_usd;
            *hedge_notional_by_symbol
                .entry(symbol.to_string())
                .or_default() += quote_size_usd;
            update_weighted_px(
                &mut weighted_hedge_px_by_symbol,
                &hedge_level.symbol,
                hedge_level.px,
                this_size_usd,
            );

            let pos_skew_adj =
                self.calculate_total_pos_skew_adj(side, &hedge_notional_by_symbol, symbol);
            let burst_adj = if self.config.act_on_burst && side.factor() * self.burst < 0.0 {
                self.burst * ref_mid
            } else {
                0.0
            };
            let raw_px = px + pos_skew_adj + burst_adj;
            let passive_px = side.passive_price(
                raw_px,
                pricing_entity.opposite_touch(side),
                pricing_entity.config.tick_size,
            );
            let quote_px = pricing_entity.px_within_limit(side, passive_px);
            let quote_qty = pricing_entity.quote_qty_from_usd(quote_size_usd, ref_mid);

            return Some(TheoQuote {
                price: quote_px,
                qty: quote_qty,
                hedge_symbol: best_hedge_symbol,
                hedge_px: single_weighted_px(weighted_hedge_px_by_symbol),
            });
        }

        None
    }

    fn calculate_total_pos_skew_adj(
        &self,
        side: Side,
        notionals: &HashMap<Symbol, f64>,
        pricing_symbol: &str,
    ) -> f64 {
        let Some(ref_mid) = self.ref_mid() else {
            return 0.0;
        };
        let mut total = 0.0;
        for (symbol, usd) in notionals {
            let Some(entity) = self.entities.get(symbol) else {
                continue;
            };
            let size = entity.size_from_usd(*usd, ref_mid);
            let is_hedge = symbol != pricing_symbol;
            let end_pos = if is_hedge {
                entity.inventory_position() - side.factor() * size
            } else {
                entity.inventory_position() + side.factor() * size
            };
            let adj_size = if entity.config.kind.is_derivative() && size >= 2.0 {
                (size + 1.0) * 0.5
            } else {
                size * 0.5
            };
            total += -side.factor() * entity.average_skew_rate_to(end_pos) * adj_size * ref_mid;
        }
        total
    }
}

fn price_by_hedge(
    quote_side: Side,
    pricing: &InstrumentState,
    hedge: &InstrumentState,
    hedge_level: &HedgeLevel,
    ref_mid: f64,
) -> f64 {
    let hedge_px_at_spot = hedge_level.px - hedge.fv_adjust() * ref_mid;
    let adjust_bps = quote_side.factor()
        * (hedge.config.taker_fee
            + hedge.hedge_profit_margin()
            + pricing.quote_profit_margin()
            + pricing.config.maker_fee);
    hedge_px_at_spot + (pricing.fv_adjust() - adjust_bps) * ref_mid
}

fn weighted_avg(old_px: f64, old_qty: f64, new_px: f64, new_qty: f64) -> f64 {
    if old_qty + new_qty <= 0.0 {
        return new_px;
    }
    (old_px * old_qty + new_px * new_qty) / (old_qty + new_qty)
}

fn update_weighted_px(map: &mut HashMap<Symbol, (f64, f64)>, symbol: &str, px: f64, qty: f64) {
    map.entry(symbol.to_string())
        .and_modify(|(cur_px, cur_qty)| {
            *cur_px = weighted_avg(*cur_px, *cur_qty, px, qty);
            *cur_qty += qty;
        })
        .or_insert((px, qty));
}

fn single_weighted_px(map: HashMap<Symbol, (f64, f64)>) -> f64 {
    if map.len() == 1 {
        map.into_values()
            .next()
            .map(|(px, _)| px)
            .unwrap_or(f64::NAN)
    } else {
        f64::NAN
    }
}

pub(super) fn round_passive_to_tick(px: f64, tick_size: f64, side: Side) -> f64 {
    if tick_size <= 0.0 || !px.is_finite() {
        return px;
    }
    match side {
        Side::Buy => (px / tick_size).floor() * tick_size,
        Side::Sell => (px / tick_size).ceil() * tick_size,
    }
}
