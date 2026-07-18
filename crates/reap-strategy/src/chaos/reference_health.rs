use std::collections::{HashMap, HashSet};

use reap_core::{Price, Side, Symbol, SystemEvent, SystemEventKind, TimeMs};

use crate::ChaosExecutionIntent;

use super::ChaosStrategy;

#[derive(Debug, Clone, Copy)]
pub(super) struct TimedPrice {
    pub(super) price: Price,
    pub(super) updated_ms: TimeMs,
}

#[derive(Debug, Clone, Default)]
pub(super) struct DebouncedCondition {
    last_disqualify_ms: Option<TimeMs>,
}

impl DebouncedCondition {
    pub(super) fn check(&mut self, qualifies: bool, now_ms: TimeMs, interval_ms: TimeMs) -> bool {
        if !qualifies {
            self.last_disqualify_ms = Some(now_ms);
            return false;
        }
        self.last_disqualify_ms
            .is_none_or(|last| now_ms.saturating_sub(last) > interval_ms)
    }
}

#[derive(Debug, Clone)]
pub(super) struct ReferenceHealthState {
    pub(super) index_symbols: HashSet<Symbol>,
    pub(super) index_prices: HashMap<Symbol, TimedPrice>,
    pub(super) index_debouncers: HashMap<String, DebouncedCondition>,
    pub(super) basis_debouncers: HashMap<String, DebouncedCondition>,
    pub(super) basis_breaches: HashMap<String, (Symbol, f64)>,
    pub(super) startup_basis_checked: bool,
    pub(super) insufficient_valid_since: Option<TimeMs>,
}

impl ChaosStrategy {
    pub(super) fn check_validity(&mut self) -> bool {
        if self.halt_reason.is_some() {
            return false;
        }
        let valid_count = self
            .entities
            .values()
            .filter(|entity| entity.market_data_is_valid_at(self.now_ms) && !entity.feed_stale)
            .count();
        if valid_count < 2 {
            let since = self
                .reference_health
                .insufficient_valid_since
                .get_or_insert(self.now_ms);
            if self.now_ms.saturating_sub(*since) > self.config.insufficient_valid_stop_ms {
                self.halt_reason = Some(format!(
                    "fewer than two instruments valid for more than {}ms",
                    self.config.insufficient_valid_stop_ms
                ));
                return false;
            }
        } else {
            self.reference_health.insufficient_valid_since = None;
        }
        true
    }

    pub(super) fn update_interval_halts(&mut self) {
        let second_in_day = ((self.now_ms / 1_000) % 86_400) as u32;
        for entity in self.entities.values_mut() {
            entity.interval_halted = entity.config.kind.is_derivative()
                && entity.config.halt_intervals.iter().any(|interval| {
                    second_in_day >= interval.start_sec_utc && second_in_day <= interval.end_sec_utc
                });
        }
    }

    pub(super) fn update_funding_window(&mut self) {
        if !self.config.use_funding_rate_manager {
            for entity in self.entities.values_mut() {
                entity.funding_rate_active = true;
            }
            return;
        }
        let current_window = self
            .entities
            .values()
            .filter(|entity| entity.config.kind.is_swap())
            .map(|entity| entity.funding_time_ms)
            .filter(|funding_time| *funding_time > self.now_ms)
            .min();
        for entity in self.entities.values_mut() {
            entity.funding_rate_active = entity.config.kind.is_swap()
                && current_window.is_some_and(|window| entity.funding_time_ms == window);
        }
    }

    pub(super) fn check_index_deviation(&mut self) -> bool {
        if self.halt_reason.is_some() {
            return false;
        }
        if !self.configured_indexes_ready_at(self.now_ms) {
            return false;
        }
        let mut group_names = self.risk_groups.keys().cloned().collect::<Vec<_>>();
        group_names.sort();
        for group_name in group_names {
            let mut worst: Option<(Symbol, Symbol, f64)> = None;
            if let Some(group) = self.risk_groups.get(&group_name) {
                for symbol in &group.ordered_symbols {
                    let Some(entity) = self.entities.get(symbol) else {
                        continue;
                    };
                    if !entity.config.kind.is_spot() {
                        continue;
                    }
                    let Some(index_symbol) = entity.config.index_symbol.as_ref() else {
                        continue;
                    };
                    let (Some(spot_mid), Some(index_price)) = (
                        entity.mid(),
                        self.reference_health
                            .index_prices
                            .get(index_symbol)
                            .map(|value| value.price),
                    ) else {
                        continue;
                    };
                    let deviation =
                        spot_mid / index_price - 1.0 - entity.config.index_deviation_adjustment;
                    if deviation.abs() > self.config.index_deviation_limit
                        && worst
                            .as_ref()
                            .is_none_or(|(_, _, current)| deviation.abs() > current.abs())
                    {
                        worst = Some((symbol.clone(), index_symbol.clone(), deviation));
                    }
                }
            }
            let should_stop = self
                .reference_health
                .index_debouncers
                .entry(group_name)
                .or_default()
                .check(
                    worst.is_some(),
                    self.now_ms,
                    self.config.index_deviation_debounce_ms,
                );
            if should_stop && let Some((symbol, index_symbol, deviation)) = worst {
                self.halt_reason = Some(format!(
                    "{} index deviation {} versus {} exceeds {}",
                    symbol, deviation, index_symbol, self.config.index_deviation_limit
                ));
                return false;
            }
        }
        true
    }

    pub(super) fn configured_indexes_ready_at(&self, now_ms: TimeMs) -> bool {
        let Some(max_age_ms) = self.config.reference_data_stale_threshold_ms else {
            return true;
        };
        self.reference_health.index_symbols.iter().all(|symbol| {
            self.reference_health
                .index_prices
                .get(symbol)
                .is_some_and(|value| timestamp_is_fresh(value.updated_ms, now_ms, max_age_ms))
        })
    }

    pub(super) fn pricing_ready(&self) -> bool {
        self.configured_indexes_ready_at(self.now_ms)
            && self
                .entities
                .values()
                .all(|entity| entity.market_data_is_valid_at(self.now_ms) && !entity.feed_stale)
    }

    pub(super) fn check_basis(&mut self, first_run: bool) -> bool {
        let mut group_names = self.risk_groups.keys().cloned().collect::<Vec<_>>();
        group_names.sort();
        self.reference_health.basis_breaches.clear();
        for group_name in group_names {
            let Some(group) = self.risk_groups.get(&group_name) else {
                continue;
            };
            let mut max_basis = 0.0;
            let mut max_symbol = String::new();
            for symbol in &group.ordered_symbols {
                let Some(entity) = self.entities.get(symbol) else {
                    continue;
                };
                if !entity.config.kind.is_derivative() {
                    continue;
                }
                let basis =
                    (entity.take_rate(Side::Buy) + entity.take_rate(Side::Sell)) * 0.5 - 1.0;
                if basis.is_finite() && basis.abs() > max_basis {
                    max_basis = basis.abs();
                    max_symbol = symbol.clone();
                }
            }
            let limit = if first_run {
                group.config.basis_limit / 3.0
            } else {
                group.config.basis_limit
            };
            let breached = self
                .reference_health
                .basis_debouncers
                .entry(group_name.clone())
                .or_default()
                .check(
                    max_basis > limit,
                    self.now_ms,
                    self.config.basis_breach_debounce_ms,
                );
            if breached {
                self.reference_health
                    .basis_breaches
                    .insert(group_name, (max_symbol, max_basis));
                return false;
            }
        }
        true
    }

    pub(super) fn on_system_event(&mut self, event: &SystemEvent) -> Vec<ChaosExecutionIntent> {
        self.advance_time(event.ts_ms);
        if event.kind == SystemEventKind::AccountHalted {
            let Some(account_id) = event.account_id.as_deref() else {
                return Vec::new();
            };
            let affected_groups = self
                .risk_groups
                .iter()
                .filter(|(_, group)| group.config.account_id.as_deref() == Some(account_id))
                .map(|(name, _)| name.clone())
                .collect::<HashSet<_>>();
            let mut changed = false;
            for entity in self.entities.values_mut() {
                if affected_groups.contains(&entity.config.risk_group) {
                    changed |= update_flag(&mut entity.system_halted, true);
                }
            }
            return if changed {
                self.refresh_quotes()
            } else {
                Vec::new()
            };
        }
        let Some(symbol) = event.symbol.as_deref() else {
            return Vec::new();
        };
        let Some(entity) = self.entities.get_mut(symbol) else {
            return Vec::new();
        };
        let changed = match event.kind {
            SystemEventKind::SymbolHalted => update_flag(&mut entity.system_halted, true),
            SystemEventKind::SymbolResumed => update_flag(&mut entity.system_halted, false),
            SystemEventKind::FeedStale
            | SystemEventKind::FeedGap
            | SystemEventKind::BookRecoveryStarted
            | SystemEventKind::BookRecoveryFailed => update_flag(&mut entity.feed_stale, true),
            SystemEventKind::FeedHeartbeat | SystemEventKind::FeedRecovered => {
                entity.feed_stale = false;
                return Vec::new();
            }
            _ => return Vec::new(),
        };
        if !changed {
            return Vec::new();
        }
        self.refresh_quotes()
    }
}

fn update_flag(value: &mut bool, next: bool) -> bool {
    let changed = *value != next;
    *value = next;
    changed
}

pub(super) fn timestamp_is_fresh(updated_ms: TimeMs, now_ms: TimeMs, max_age_ms: TimeMs) -> bool {
    now_ms.saturating_sub(updated_ms) <= max_age_ms
}

pub(super) fn should_accept_timestamp(current_ms: Option<TimeMs>, next_ms: TimeMs) -> bool {
    current_ms.is_none_or(|current_ms| next_ms >= current_ms)
}
