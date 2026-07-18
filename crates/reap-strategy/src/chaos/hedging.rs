use std::collections::HashMap;
use std::sync::Arc;

use reap_core::{Price, Quantity, Side, Symbol, round_down_to_lot};

use crate::ChaosExecutionIntent;

use super::{ChaosStrategy, EPS, RiskGroupKindConfig, approx_eq};

const HEDGE_VOL_TO_DELTA_RATIO: f64 = 1.5;

#[derive(Debug, Clone)]
pub struct HedgeLevel {
    pub symbol: Symbol,
    pub priority: i32,
    pub level: usize,
    pub px: Price,
    pub qty: Quantity,
    pub hedge_rate: f64,
    pub notional_usd: f64,
    pub acc_qty: Quantity,
}

#[derive(Debug, Clone)]
pub(super) struct HedgeCandidate {
    pub(super) symbol: Arc<str>,
    pub(super) priority: i32,
    pub(super) level: usize,
    pub(super) px: Price,
    pub(super) qty: Quantity,
    pub(super) hedge_rate: f64,
    pub(super) notional_usd: f64,
    pub(super) acc_qty: Quantity,
}

impl HedgeCandidate {
    pub(super) fn to_owned_level(&self) -> HedgeLevel {
        HedgeLevel {
            symbol: self.symbol.to_string(),
            priority: self.priority,
            level: self.level,
            px: self.px,
            qty: self.qty,
            hedge_rate: self.hedge_rate,
            notional_usd: self.notional_usd,
            acc_qty: self.acc_qty,
        }
    }
}

#[derive(Debug, Clone)]
pub(super) struct HedgeTarget {
    pub(super) symbol: Symbol,
    pub(super) orig_px: Price,
    pub(super) hedge_px: Price,
    pub(super) qty: Quantity,
    pub(super) cur_level_acc_qty: Quantity,
    pub(super) notional_usd: f64,
}

impl ChaosStrategy {
    pub(super) fn update_best_hedges(&mut self) {
        for entity in self.entities.values_mut() {
            entity.refresh_trade_permissions(self.now_ms);
        }
        let Some(ref_mid) = self.ref_mid() else {
            return;
        };
        for entity in self.entities.values_mut() {
            entity.update_take_rate(Side::Buy, ref_mid);
            entity.update_take_rate(Side::Sell, ref_mid);
        }

        self.best_hedges.entry(Side::Buy).or_default().clear();
        self.best_hedges.entry(Side::Sell).or_default().clear();

        let mut levels = std::mem::take(&mut self.hedge_candidate_scratch);
        let mut group_names = self.risk_groups.keys().cloned().collect::<Vec<_>>();
        group_names.sort();
        for group_name in group_names {
            let candidate_notional_limit = self
                .hedge_selection_target(&group_name)
                .unwrap_or(f64::INFINITY);
            for side in [Side::Buy, Side::Sell] {
                levels.clear();
                if let Some(rg) = self.risk_groups.get(&group_name) {
                    for symbol in &rg.ordered_symbols {
                        let Some(entity) = self.entities.get(symbol) else {
                            continue;
                        };
                        if !entity.can_take(side) {
                            continue;
                        }
                        let own_quotes = self
                            .execution
                            .active_quotes
                            .iter()
                            .filter(|((quote_symbol, _, _), _)| quote_symbol == symbol)
                            .map(|((_, quote_side, _), quote)| {
                                (*quote_side, quote.price, quote.qty)
                            })
                            .collect::<Vec<_>>();
                        entity.append_hedge_candidates(
                            side,
                            ref_mid,
                            &own_quotes,
                            candidate_notional_limit,
                            &mut levels,
                        );
                    }
                }

                sort_hedge_candidates(side, &mut levels);
                let selected = self.select_required_hedges(&group_name, side, &levels);
                if let Some(rg) = self.risk_groups.get_mut(&group_name) {
                    rg.best_hedges.insert(side, selected);
                }
            }
        }
        levels.clear();
        self.hedge_candidate_scratch = levels;

        if self.risk_groups.len() == 1 {
            if let Some(rg) = self.risk_groups.values().next() {
                self.best_hedges
                    .insert(Side::Buy, rg.best_hedges_for(Side::Buy).to_vec());
                self.best_hedges
                    .insert(Side::Sell, rg.best_hedges_for(Side::Sell).to_vec());
            }
            return;
        }

        for side in [Side::Buy, Side::Sell] {
            let mut levels = Vec::new();
            for rg in self.risk_groups.values() {
                if rg.can_increase_delta_with_quote_buffer(side) {
                    levels.extend_from_slice(rg.best_hedges_for(side));
                }
            }
            sort_hedge_levels(side, &mut levels);
            self.best_hedges.insert(side, levels);
        }
    }

    pub(super) fn check_hedge_availability(&mut self) -> bool {
        let all_halted = self.all_hedges_halted();
        if all_halted {
            let since = self.all_hedges_halted_since.get_or_insert(self.now_ms);
            if self.now_ms.saturating_sub(*since) > self.config.all_hedges_halted_stop_ms {
                self.halt_reason = Some(format!(
                    "all hedge-enabled instruments halted for more than {}ms",
                    self.config.all_hedges_halted_stop_ms
                ));
                return false;
            }
        } else {
            self.all_hedges_halted_since = None;
        }

        let no_hedges = self.best_hedges.get(&Side::Buy).is_none_or(Vec::is_empty)
            && self.best_hedges.get(&Side::Sell).is_none_or(Vec::is_empty);
        if no_hedges && !all_halted {
            let since = self.no_hedge_found_since.get_or_insert(self.now_ms);
            if self.now_ms.saturating_sub(*since) > self.config.no_hedge_stop_ms {
                self.halt_reason = Some(format!(
                    "neither buy nor sell hedge found for more than {}ms",
                    self.config.no_hedge_stop_ms
                ));
                return false;
            }
        } else {
            self.no_hedge_found_since = None;
        }
        true
    }

    pub(super) fn all_hedges_halted(&self) -> bool {
        self.entities
            .values()
            .filter(|entity| {
                entity.hedge_profit_margin() < 1.0
                    && self
                        .risk_groups
                        .get(&entity.config.risk_group)
                        .is_none_or(|group| group.config.kind != RiskGroupKindConfig::RefOnly)
            })
            .all(|entity| entity.config.halted || entity.interval_halted || entity.system_halted)
    }

    pub(super) fn select_required_hedges(
        &self,
        group_name: &str,
        _side: Side,
        levels: &[HedgeCandidate],
    ) -> Vec<HedgeLevel> {
        let Some(rg) = self.risk_groups.get(group_name) else {
            return levels.iter().map(HedgeCandidate::to_owned_level).collect();
        };
        let target = self
            .hedge_selection_target(group_name)
            .unwrap_or(f64::INFINITY);

        let mut selected = Vec::new();
        let mut total = 0.0;
        let mut per_symbol: HashMap<&str, f64> = HashMap::new();
        for level in levels {
            total += level.notional_usd;
            *per_symbol.entry(level.symbol.as_ref()).or_default() += level.notional_usd;
            selected.push(level.to_owned_level());
            if total >= target
                && all_symbols_have_hedge(
                    &per_symbol,
                    total,
                    rg.max_quote_size_usd,
                    rg.symbols.len(),
                )
            {
                break;
            }
        }
        selected
    }

    pub(super) fn hedge_selection_target(&self, group_name: &str) -> Option<f64> {
        let rg = self.risk_groups.get(group_name)?;
        let hedge_required_for_quote = rg
            .ordered_symbols
            .iter()
            .filter_map(|symbol| self.entities.get(symbol))
            .map(|entity| entity.config.max_order_size_usd)
            .sum::<f64>()
            * 2.0;
        let min_hedge_usd = self.config.delta_limit_usd.min(hedge_required_for_quote);
        let delta_need =
            self.delta_usd.abs().max(self.pending_delta_usd.abs()) * HEDGE_VOL_TO_DELTA_RATIO;
        Some(min_hedge_usd.max(delta_need))
    }

    pub(super) fn should_hedge_strategy_delta(&self) -> bool {
        if self.config.master_strategy.is_some() {
            return false;
        }
        if self.now_ms < self.last_hedge_ms + self.config.min_hedge_interval_ms {
            return false;
        }
        self.delta_to_hedge().abs() >= self.config.active_hedge_threshold_usd
    }

    pub(super) fn hedge_delta(
        &mut self,
        delta_to_hedge: f64,
        source_symbol: Option<&str>,
        strategy_delta_hedge: bool,
    ) -> Vec<ChaosExecutionIntent> {
        if self.config.master_strategy.is_some() {
            return Vec::new();
        }
        self.update_risk();
        self.update_best_hedges();

        let hedge_side = if delta_to_hedge > 0.0 {
            Side::Sell
        } else {
            Side::Buy
        };
        let group_name = source_symbol
            .and_then(|symbol| self.symbol_to_group.get(symbol))
            .cloned();
        let mut targets = Vec::new();
        if let Some(group_name) = group_name
            && let Some(rg) = self.risk_groups.get(&group_name)
            && rg.must_hedge_within_group(delta_to_hedge)
        {
            targets = self.summarize_hedges(
                rg.best_hedges_for(hedge_side),
                hedge_side,
                delta_to_hedge.abs(),
                source_symbol,
            );
        }
        if targets.is_empty() {
            let hedges = self
                .best_hedges
                .get(&hedge_side)
                .map(Vec::as_slice)
                .unwrap_or(&[]);
            targets =
                self.summarize_hedges(hedges, hedge_side, delta_to_hedge.abs(), source_symbol);
        }

        if targets.is_empty() {
            if !self.all_hedges_halted() {
                let since = self.hedge_not_found_since.get_or_insert(self.now_ms);
                if self.now_ms.saturating_sub(*since) > self.config.hedge_not_found_stop_ms {
                    self.halt_reason = Some(format!(
                        "delta hedge unavailable for more than {}ms",
                        self.config.hedge_not_found_stop_ms
                    ));
                }
            }
            return Vec::new();
        }
        self.hedge_not_found_since = None;

        if strategy_delta_hedge {
            self.last_hedge_ms = self.now_ms;
        }
        let source_label = source_symbol.unwrap_or("timer");
        targets
            .into_iter()
            .filter_map(|target| {
                let entity = self.entities.get(&target.symbol)?;
                if target.qty < entity.config.min_trade_size {
                    return None;
                }
                Some(ChaosExecutionIntent::hedge(
                    target.symbol,
                    hedge_side,
                    target.qty,
                    target.hedge_px,
                    format!("hedge:{}:{}", source_label, target.orig_px),
                ))
            })
            .collect()
    }

    pub(super) fn delta_to_hedge(&self) -> f64 {
        if self.pending_delta_usd * self.delta_usd < 0.0 {
            return 0.0;
        }
        if self.delta_usd > 0.0 && self.pending_delta_usd > 0.0 {
            self.delta_usd.min(self.pending_delta_usd)
        } else {
            self.delta_usd.max(self.pending_delta_usd)
        }
    }

    pub(super) fn summarize_hedges(
        &self,
        hedges: &[HedgeLevel],
        hedge_side: Side,
        usd_amt: f64,
        exclude_symbol: Option<&str>,
    ) -> Vec<HedgeTarget> {
        let mut out: HashMap<Symbol, HedgeTarget> = HashMap::new();
        let mut total = 0.0;

        for level in hedges {
            if exclude_symbol.is_some_and(|symbol| symbol == level.symbol) {
                continue;
            }
            if total >= usd_amt {
                break;
            }
            let Some(entity) = self.entities.get(&level.symbol) else {
                continue;
            };
            let pending = self
                .execution
                .active_hedges
                .values()
                .filter(|hedge| {
                    hedge.symbol == level.symbol
                        && hedge.signed_open_qty.signum() == hedge_side.factor()
                })
                .collect::<Vec<_>>();
            if pending
                .iter()
                .any(|hedge| hedge_side.is_more_passive(level.px, hedge.reference_price))
            {
                continue;
            }
            let pending_qty_at_level = pending
                .iter()
                .filter(|hedge| approx_eq(level.px, hedge.reference_price))
                .map(|hedge| hedge.signed_open_qty.abs())
                .sum::<f64>();
            let available_level_qty = (level.qty - pending_qty_at_level).max(0.0);
            if available_level_qty <= 0.0 {
                continue;
            }
            let available_level_notional =
                level.notional_usd * available_level_qty / level.qty.max(EPS);
            let gap = usd_amt - total;
            let use_notional = available_level_notional.min(gap);
            let qty = if available_level_notional > gap {
                round_down_to_lot(
                    available_level_qty * gap / available_level_notional,
                    entity.config.lot_size,
                )
            } else {
                round_down_to_lot(available_level_qty, entity.config.lot_size)
            };
            if qty <= 0.0 {
                continue;
            }
            let notional = if available_level_qty > 0.0 {
                qty * available_level_notional / available_level_qty
            } else {
                use_notional
            }
            .min(use_notional);
            let hedge_aggression =
                if self.config.act_on_burst && hedge_side.factor() * self.burst > 0.0 {
                    entity.hedge_aggression().max(self.burst.abs())
                } else {
                    entity.hedge_aggression()
                };
            let hedge_px = entity.hedge_px(hedge_side, level.px, hedge_aggression);
            out.entry(level.symbol.clone())
                .and_modify(|target| {
                    target.qty += qty;
                    target.notional_usd += notional;
                    target.orig_px = level.px;
                    target.hedge_px = hedge_px;
                    target.cur_level_acc_qty = qty;
                })
                .or_insert_with(|| HedgeTarget {
                    symbol: level.symbol.clone(),
                    orig_px: level.px,
                    hedge_px,
                    qty,
                    cur_level_acc_qty: qty,
                    notional_usd: notional,
                });
            total += use_notional;
        }

        let mut targets = out.into_values().collect::<Vec<_>>();
        targets.sort_by(|left, right| left.symbol.cmp(&right.symbol));
        targets
    }
}

fn sort_hedge_levels(side: Side, levels: &mut [HedgeLevel]) {
    match side {
        Side::Buy => levels.sort_by(|a, b| {
            a.hedge_rate
                .total_cmp(&b.hedge_rate)
                .then_with(|| b.priority.cmp(&a.priority))
                .then_with(|| b.symbol.cmp(&a.symbol))
                .then_with(|| a.level.cmp(&b.level))
        }),
        Side::Sell => levels.sort_by(|a, b| {
            b.hedge_rate
                .total_cmp(&a.hedge_rate)
                .then_with(|| b.priority.cmp(&a.priority))
                .then_with(|| b.symbol.cmp(&a.symbol))
                .then_with(|| a.level.cmp(&b.level))
        }),
    }
}

fn sort_hedge_candidates(side: Side, levels: &mut [HedgeCandidate]) {
    match side {
        Side::Buy => levels.sort_by(|a, b| {
            a.hedge_rate
                .total_cmp(&b.hedge_rate)
                .then_with(|| b.priority.cmp(&a.priority))
                .then_with(|| b.symbol.cmp(&a.symbol))
                .then_with(|| a.level.cmp(&b.level))
        }),
        Side::Sell => levels.sort_by(|a, b| {
            b.hedge_rate
                .total_cmp(&a.hedge_rate)
                .then_with(|| b.priority.cmp(&a.priority))
                .then_with(|| b.symbol.cmp(&a.symbol))
                .then_with(|| a.level.cmp(&b.level))
        }),
    }
}

fn all_symbols_have_hedge(
    per_symbol: &HashMap<&str, f64>,
    total_hedge_size: f64,
    max_quote_size: f64,
    symbol_count: usize,
) -> bool {
    if symbol_count <= 1 {
        return true;
    }
    if per_symbol.len() < 2 {
        return false;
    }
    per_symbol
        .values()
        .all(|hedge_size| total_hedge_size - hedge_size >= max_quote_size)
}
