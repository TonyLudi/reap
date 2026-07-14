use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use reap_core::{AccountUpdate, OrderStatus, OrderUpdate};

use crate::LiveConfig;

const MAX_PENDING_FILL_LATENCY_OBSERVATIONS: usize = 8_192;

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
enum StateTarget {
    Balance(String),
    Position(String),
}

impl StateTarget {
    fn label(&self) -> String {
        match self {
            Self::Balance(currency) => format!("balance:{currency}"),
            Self::Position(symbol) => format!("position:{symbol}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ScopedTarget {
    account_id: String,
    target: StateTarget,
}

#[derive(Debug, Clone)]
struct PendingFill {
    first_observed_ms: u64,
    latest_observed_ms: u64,
    exchange_ms: u64,
    order_id: String,
    symbol: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
struct FillLatencyKey {
    account_id: String,
    order_id: String,
    symbol: String,
    exchange_ms: u64,
    cumulative_fill_bits: u64,
}

#[derive(Debug, Clone)]
struct PendingFillLatency {
    first_observed_ns: u64,
    targets: BTreeSet<StateTarget>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FillConvergenceObservation {
    pub symbol: String,
    pub first_observed_ns: u64,
    pub state_visible_ns: u64,
}

#[derive(Debug, Default)]
pub(crate) struct FillConvergenceResult {
    pub observations: Vec<FillConvergenceObservation>,
    pub dropped_latency_observation: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FillConvergenceBreach {
    pub account_id: String,
    pub symbol: Option<String>,
    pub reason: String,
}

#[derive(Debug, Default)]
pub(crate) struct FillConvergenceGuard {
    timeout_ms: u64,
    routes: HashMap<String, HashMap<String, Vec<StateTarget>>>,
    known_targets: HashSet<ScopedTarget>,
    last_state_exchange_ms: HashMap<ScopedTarget, u64>,
    pending: HashMap<ScopedTarget, PendingFill>,
    latency_pending: HashMap<FillLatencyKey, PendingFillLatency>,
    reported_accounts: HashSet<String>,
}

impl FillConvergenceGuard {
    pub fn new(config: &LiveConfig) -> Self {
        let routes = config
            .strategy
            .instruments
            .iter()
            .map(|instrument| {
                let account_id = config
                    .account_for_symbol(&instrument.symbol)
                    .expect("validated live instrument must have an account route")
                    .id
                    .clone();
                let targets = if instrument.kind.is_spot() {
                    vec![
                        StateTarget::Balance(instrument.base_currency.clone()),
                        StateTarget::Balance(instrument.quote_currency.clone()),
                    ]
                } else {
                    vec![StateTarget::Position(instrument.symbol.clone())]
                };
                (account_id, instrument.symbol.clone(), targets)
            })
            .collect::<Vec<_>>();
        Self::from_routes(config.runtime.fill_state_convergence_timeout_ms, routes)
    }

    fn from_routes(
        timeout_ms: u64,
        routes: impl IntoIterator<Item = (String, String, Vec<StateTarget>)>,
    ) -> Self {
        let mut route_map = HashMap::new();
        let mut known_targets = HashSet::new();
        for (account_id, symbol, mut targets) in routes {
            targets.sort();
            targets.dedup();
            for target in &targets {
                known_targets.insert(ScopedTarget {
                    account_id: account_id.clone(),
                    target: target.clone(),
                });
            }
            route_map
                .entry(account_id)
                .or_insert_with(HashMap::new)
                .insert(symbol, targets);
        }
        Self {
            timeout_ms,
            routes: route_map,
            known_targets,
            last_state_exchange_ms: HashMap::new(),
            pending: HashMap::new(),
            latency_pending: HashMap::new(),
            reported_accounts: HashSet::new(),
        }
    }

    #[cfg(test)]
    fn observe_fill(&mut self, account_id: &str, update: &OrderUpdate, observed_ms: u64) {
        let _ = self.observe_fill_at(
            account_id,
            update,
            observed_ms,
            observed_ms.saturating_mul(1_000_000),
            true,
        );
    }

    pub fn observe_fill_at(
        &mut self,
        account_id: &str,
        update: &OrderUpdate,
        observed_ms: u64,
        observed_ns: u64,
        track_latency: bool,
    ) -> FillConvergenceResult {
        if !update.has_fill() {
            return FillConvergenceResult::default();
        }
        let Some(targets) = self
            .routes
            .get(account_id)
            .and_then(|routes| routes.get(&update.symbol))
            .cloned()
        else {
            return FillConvergenceResult::default();
        };
        let mut latency_targets = BTreeSet::new();
        for target in targets {
            let key = ScopedTarget {
                account_id: account_id.to_string(),
                target: target.clone(),
            };
            let last_state_ms = self.last_state_exchange_ms.get(&key).copied().unwrap_or(0);
            if update.ts_ms > 0 && last_state_ms >= update.ts_ms {
                continue;
            }
            latency_targets.insert(target);
            self.pending
                .entry(key)
                .and_modify(|pending| {
                    pending.first_observed_ms = pending.first_observed_ms.min(observed_ms);
                    pending.latest_observed_ms = pending.latest_observed_ms.max(observed_ms);
                    if update.ts_ms >= pending.exchange_ms {
                        pending.exchange_ms = update.ts_ms;
                        pending.order_id.clone_from(&update.order_id);
                        pending.symbol.clone_from(&update.symbol);
                    }
                })
                .or_insert_with(|| PendingFill {
                    first_observed_ms: observed_ms,
                    latest_observed_ms: observed_ms,
                    exchange_ms: update.ts_ms,
                    order_id: update.order_id.clone(),
                    symbol: update.symbol.clone(),
                });
        }
        if !track_latency {
            return FillConvergenceResult::default();
        }
        if latency_targets.is_empty() {
            return FillConvergenceResult {
                observations: vec![FillConvergenceObservation {
                    symbol: update.symbol.clone(),
                    first_observed_ns: observed_ns,
                    state_visible_ns: observed_ns,
                }],
                dropped_latency_observation: false,
            };
        }
        let latency_key = FillLatencyKey {
            account_id: account_id.to_string(),
            order_id: update.order_id.clone(),
            symbol: update.symbol.clone(),
            exchange_ms: update.ts_ms,
            cumulative_fill_bits: update.filled_qty.to_bits(),
        };
        if !self.latency_pending.contains_key(&latency_key)
            && self.latency_pending.len() >= MAX_PENDING_FILL_LATENCY_OBSERVATIONS
        {
            return FillConvergenceResult {
                observations: Vec::new(),
                dropped_latency_observation: true,
            };
        }
        self.latency_pending
            .entry(latency_key)
            .and_modify(|pending| {
                pending.first_observed_ns = pending.first_observed_ns.min(observed_ns);
                pending.targets.extend(latency_targets.iter().cloned());
            })
            .or_insert(PendingFillLatency {
                first_observed_ns: observed_ns,
                targets: latency_targets,
            });
        FillConvergenceResult::default()
    }

    #[cfg(test)]
    fn observe_account(&mut self, account_id: &str, update: &AccountUpdate, observed_ms: u64) {
        let _ = self.observe_account_at(
            account_id,
            update,
            observed_ms,
            observed_ms.saturating_mul(1_000_000),
        );
    }

    pub fn observe_account_at(
        &mut self,
        account_id: &str,
        update: &AccountUpdate,
        observed_ms: u64,
        observed_ns: u64,
    ) -> FillConvergenceResult {
        for balance in &update.balances {
            let target = StateTarget::Balance(balance.currency.clone());
            self.observe_target(
                ScopedTarget {
                    account_id: account_id.to_string(),
                    target: target.clone(),
                },
                update.ts_ms,
                observed_ms,
            );
            self.observe_latency_target(account_id, &target, update.ts_ms, observed_ns);
        }
        for position in &update.positions {
            let target = StateTarget::Position(position.symbol.clone());
            self.observe_target(
                ScopedTarget {
                    account_id: account_id.to_string(),
                    target: target.clone(),
                },
                update.ts_ms,
                observed_ms,
            );
            self.observe_latency_target(account_id, &target, update.ts_ms, observed_ns);
        }
        self.clear_reported_if_converged(account_id);
        let mut completed = self
            .latency_pending
            .iter()
            .filter(|(key, pending)| key.account_id == account_id && pending.targets.is_empty())
            .map(|(key, pending)| {
                (
                    key.clone(),
                    FillConvergenceObservation {
                        symbol: key.symbol.clone(),
                        first_observed_ns: pending.first_observed_ns,
                        state_visible_ns: observed_ns,
                    },
                )
            })
            .collect::<Vec<_>>();
        completed.sort_by(|left, right| left.0.cmp(&right.0));
        for (key, _) in &completed {
            self.latency_pending.remove(key);
        }
        FillConvergenceResult {
            observations: completed
                .into_iter()
                .map(|(_, observation)| observation)
                .collect(),
            dropped_latency_observation: false,
        }
    }

    /// Clear pending convergence against an authoritative REST snapshot.
    ///
    /// Returns the number of live-path latency observations that were censored
    /// rather than completed by websocket account state.
    pub fn observe_authoritative(&mut self, account_id: &str, exchange_ms: u64) -> usize {
        let targets = self
            .known_targets
            .iter()
            .filter(|key| key.account_id == account_id)
            .cloned()
            .collect::<Vec<_>>();
        for target in targets {
            self.last_state_exchange_ms
                .entry(target)
                .and_modify(|last| *last = (*last).max(exchange_ms))
                .or_insert(exchange_ms);
        }
        self.pending.retain(|key, _| key.account_id != account_id);
        let latency_before = self.latency_pending.len();
        self.latency_pending
            .retain(|key, _| key.account_id != account_id);
        let censored_latency = latency_before.saturating_sub(self.latency_pending.len());
        self.reported_accounts.remove(account_id);
        censored_latency
    }

    pub fn expire(&mut self, now_ms: u64) -> Vec<FillConvergenceBreach> {
        let mut expired = BTreeMap::<String, Vec<(ScopedTarget, PendingFill, u64)>>::new();
        for (key, pending) in &self.pending {
            let age_ms = now_ms.saturating_sub(pending.first_observed_ms);
            if age_ms >= self.timeout_ms && !self.reported_accounts.contains(&key.account_id) {
                expired.entry(key.account_id.clone()).or_default().push((
                    key.clone(),
                    pending.clone(),
                    age_ms,
                ));
            }
        }

        expired
            .into_iter()
            .map(|(account_id, mut entries)| {
                entries.sort_by(|left, right| left.0.target.cmp(&right.0.target));
                self.reported_accounts.insert(account_id.clone());
                let symbols = entries
                    .iter()
                    .map(|(_, pending, _)| pending.symbol.clone())
                    .collect::<BTreeSet<_>>();
                let symbol = (symbols.len() == 1)
                    .then(|| symbols.iter().next().expect("one symbol exists").clone());
                let oldest_age_ms = entries
                    .iter()
                    .map(|(_, _, age_ms)| *age_ms)
                    .max()
                    .unwrap_or_default();
                let details = entries
                    .into_iter()
                    .map(|(key, pending, age_ms)| {
                        format!(
                            "{} order={} target={} age_ms={age_ms}",
                            pending.symbol,
                            pending.order_id,
                            key.target.label()
                        )
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                FillConvergenceBreach {
                    account_id,
                    symbol,
                    reason: format!(
                        "fill-to-account-state convergence exceeded {}ms (oldest_age_ms={oldest_age_ms}): {details}",
                        self.timeout_ms
                    ),
                }
            })
            .collect()
    }

    fn observe_target(&mut self, key: ScopedTarget, exchange_ms: u64, observed_ms: u64) {
        if !self.known_targets.contains(&key) {
            return;
        }
        self.last_state_exchange_ms
            .entry(key.clone())
            .and_modify(|last| *last = (*last).max(exchange_ms))
            .or_insert(exchange_ms);
        let covered = self.pending.get(&key).is_some_and(|pending| {
            if exchange_ms > 0 && pending.exchange_ms > 0 {
                exchange_ms >= pending.exchange_ms
            } else {
                observed_ms >= pending.latest_observed_ms
            }
        });
        if covered {
            self.pending.remove(&key);
        }
    }

    fn observe_latency_target(
        &mut self,
        account_id: &str,
        target: &StateTarget,
        exchange_ms: u64,
        observed_ns: u64,
    ) {
        for (key, pending) in &mut self.latency_pending {
            if key.account_id != account_id || !pending.targets.contains(target) {
                continue;
            }
            let covered = if exchange_ms > 0 && key.exchange_ms > 0 {
                exchange_ms >= key.exchange_ms
            } else {
                observed_ns >= pending.first_observed_ns
            };
            if covered {
                pending.targets.remove(target);
            }
        }
    }

    fn clear_reported_if_converged(&mut self, account_id: &str) {
        if !self.pending.keys().any(|key| key.account_id == account_id) {
            self.reported_accounts.remove(account_id);
        }
    }

    #[cfg(test)]
    fn pending_count(&self) -> usize {
        self.pending.len()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
enum PendingOrderTransition {
    Submit,
    Cancel,
}

impl PendingOrderTransition {
    fn label(self) -> &'static str {
        match self {
            Self::Submit => "submit_to_private_state",
            Self::Cancel => "cancel_to_terminal_state",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
struct ScopedOrderTransition {
    account_id: String,
    order_id: String,
    transition: PendingOrderTransition,
}

#[derive(Debug, Clone)]
struct PendingOrderState {
    symbol: String,
    first_observed_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OrderStateConvergenceBreach {
    pub account_id: String,
    pub symbol: Option<String>,
    pub expired_cancel_order_ids: Vec<String>,
    pub reason: String,
}

#[derive(Debug, Default)]
pub(crate) struct OrderStateConvergenceGuard {
    timeout_ms: u64,
    pending: HashMap<ScopedOrderTransition, PendingOrderState>,
}

impl OrderStateConvergenceGuard {
    pub fn new(timeout_ms: u64) -> Self {
        Self {
            timeout_ms,
            pending: HashMap::new(),
        }
    }

    pub fn observe_order(&mut self, account_id: &str, update: &OrderUpdate, observed_ms: u64) {
        let submit = ScopedOrderTransition {
            account_id: account_id.to_string(),
            order_id: update.order_id.clone(),
            transition: PendingOrderTransition::Submit,
        };
        if update.status == OrderStatus::PendingNew {
            self.pending
                .entry(submit)
                .or_insert_with(|| PendingOrderState {
                    symbol: update.symbol.clone(),
                    first_observed_ms: observed_ms,
                });
        } else {
            self.pending.remove(&submit);
        }

        if matches!(
            update.status,
            OrderStatus::Filled | OrderStatus::Cancelled | OrderStatus::Rejected
        ) {
            self.pending.remove(&ScopedOrderTransition {
                account_id: account_id.to_string(),
                order_id: update.order_id.clone(),
                transition: PendingOrderTransition::Cancel,
            });
        }
    }

    pub fn observe_cancel(
        &mut self,
        account_id: &str,
        order_id: &str,
        symbol: &str,
        observed_ms: u64,
    ) {
        self.pending
            .entry(ScopedOrderTransition {
                account_id: account_id.to_string(),
                order_id: order_id.to_string(),
                transition: PendingOrderTransition::Cancel,
            })
            .or_insert_with(|| PendingOrderState {
                symbol: symbol.to_string(),
                first_observed_ms: observed_ms,
            });
    }

    pub fn has_pending_cancel(&self, account_id: &str, order_id: &str) -> bool {
        self.pending.contains_key(&ScopedOrderTransition {
            account_id: account_id.to_string(),
            order_id: order_id.to_string(),
            transition: PendingOrderTransition::Cancel,
        })
    }

    pub fn pending_reason(&self, account_id: &str) -> Option<String> {
        let mut pending = self
            .pending
            .iter()
            .filter(|(key, _)| key.account_id == account_id)
            .map(|(key, state)| {
                format!(
                    "{} order={} transition={}",
                    state.symbol,
                    key.order_id,
                    key.transition.label()
                )
            })
            .collect::<Vec<_>>();
        if pending.is_empty() {
            return None;
        }
        pending.sort();
        Some(format!(
            "order-state transitions remain pending after REST recovery: {}",
            pending.join(", ")
        ))
    }

    pub fn expire(&mut self, now_ms: u64) -> Vec<OrderStateConvergenceBreach> {
        let mut expired = self
            .pending
            .iter()
            .filter_map(|(key, pending)| {
                let age_ms = now_ms.saturating_sub(pending.first_observed_ms);
                (age_ms >= self.timeout_ms).then(|| (key.clone(), pending.clone(), age_ms))
            })
            .collect::<Vec<_>>();
        expired.sort_by(|left, right| left.0.cmp(&right.0));

        let mut by_account =
            BTreeMap::<String, Vec<(ScopedOrderTransition, PendingOrderState, u64)>>::new();
        for (key, pending, age_ms) in expired {
            self.pending.remove(&key);
            by_account
                .entry(key.account_id.clone())
                .or_default()
                .push((key, pending, age_ms));
        }

        by_account
            .into_iter()
            .map(|(account_id, entries)| {
                let symbols = entries
                    .iter()
                    .map(|(_, pending, _)| pending.symbol.clone())
                    .collect::<BTreeSet<_>>();
                let symbol = (symbols.len() == 1)
                    .then(|| symbols.iter().next().expect("one symbol exists").clone());
                let mut expired_cancel_order_ids = entries
                    .iter()
                    .filter(|(key, _, _)| key.transition == PendingOrderTransition::Cancel)
                    .map(|(key, _, _)| key.order_id.clone())
                    .collect::<Vec<_>>();
                expired_cancel_order_ids.sort();
                expired_cancel_order_ids.dedup();
                let oldest_age_ms = entries
                    .iter()
                    .map(|(_, _, age_ms)| *age_ms)
                    .max()
                    .unwrap_or_default();
                let details = entries
                    .into_iter()
                    .map(|(key, pending, age_ms)| {
                        format!(
                            "{} order={} transition={} age_ms={age_ms}",
                            pending.symbol,
                            key.order_id,
                            key.transition.label()
                        )
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                OrderStateConvergenceBreach {
                    account_id,
                    symbol,
                    expired_cancel_order_ids,
                    reason: format!(
                        "order-state convergence exceeded {}ms (oldest_age_ms={oldest_age_ms}): {details}",
                        self.timeout_ms
                    ),
                }
            })
            .collect()
    }

    #[cfg(test)]
    fn pending_count(&self) -> usize {
        self.pending.len()
    }
}

#[cfg(test)]
mod tests {
    use reap_core::{
        AccountUpdate, Balance, FillLiquidity, OrderEvent, OrderStatus, OrderUpdate, Position, Side,
    };

    use super::*;

    fn derivative_guard(timeout_ms: u64) -> FillConvergenceGuard {
        FillConvergenceGuard::from_routes(
            timeout_ms,
            [(
                "main".to_string(),
                "BTC-USDT-SWAP".to_string(),
                vec![StateTarget::Position("BTC-USDT-SWAP".to_string())],
            )],
        )
    }

    fn spot_guard(timeout_ms: u64) -> FillConvergenceGuard {
        FillConvergenceGuard::from_routes(
            timeout_ms,
            [(
                "main".to_string(),
                "BTC-USDT".to_string(),
                vec![
                    StateTarget::Balance("BTC".to_string()),
                    StateTarget::Balance("USDT".to_string()),
                ],
            )],
        )
    }

    fn fill(symbol: &str, ts_ms: u64) -> OrderUpdate {
        OrderUpdate {
            ts_ms,
            order_id: "order-1".to_string(),
            symbol: symbol.to_string(),
            side: Side::Buy,
            event: OrderEvent::PartialFill,
            status: OrderStatus::PartiallyFilled,
            price: 100.0,
            time_in_force: None,
            qty: 2.0,
            open_qty: 1.0,
            filled_qty: 1.0,
            avg_fill_price: 100.0,
            last_fill_qty: 1.0,
            last_fill_price: 100.0,
            last_fill_liquidity: Some(FillLiquidity::Maker),
            last_fill_fee: None,
            reason: "quote".to_string(),
        }
    }

    fn order_state(status: OrderStatus, event: OrderEvent, ts_ms: u64) -> OrderUpdate {
        OrderUpdate {
            ts_ms,
            order_id: "order-1".to_string(),
            symbol: "BTC-USDT-SWAP".to_string(),
            side: Side::Buy,
            event,
            status,
            price: 100.0,
            time_in_force: None,
            qty: 1.0,
            open_qty: if status == OrderStatus::Filled {
                0.0
            } else {
                1.0
            },
            filled_qty: if status == OrderStatus::Filled {
                1.0
            } else {
                0.0
            },
            avg_fill_price: if status == OrderStatus::Filled {
                100.0
            } else {
                0.0
            },
            last_fill_qty: 0.0,
            last_fill_price: 0.0,
            last_fill_liquidity: None,
            last_fill_fee: None,
            reason: "quote".to_string(),
        }
    }

    fn balances(ts_ms: u64, currencies: &[&str]) -> AccountUpdate {
        AccountUpdate {
            ts_ms,
            balances: currencies
                .iter()
                .map(|currency| Balance {
                    account_id: Some("main".to_string()),
                    currency: (*currency).to_string(),
                    total: 1.0,
                    available: 1.0,
                    equity: 1.0,
                    liability: 0.0,
                    max_loan: 0.0,
                    forced_repayment_indicator: None,
                })
                .collect(),
            positions: Vec::new(),
            margins: Vec::new(),
        }
    }

    #[test]
    fn derivative_fill_expires_once_until_position_converges() {
        let mut guard = derivative_guard(2_000);
        guard.observe_fill("main", &fill("BTC-USDT-SWAP", 10), 100);

        assert!(guard.expire(2_099).is_empty());
        let breach = guard.expire(2_100);
        assert_eq!(breach.len(), 1);
        assert_eq!(breach[0].account_id, "main");
        assert_eq!(breach[0].symbol.as_deref(), Some("BTC-USDT-SWAP"));
        assert!(breach[0].reason.contains("position:BTC-USDT-SWAP"));
        assert!(guard.expire(3_000).is_empty());

        guard.observe_account(
            "main",
            &AccountUpdate {
                ts_ms: 11,
                balances: Vec::new(),
                positions: vec![Position {
                    symbol: "BTC-USDT-SWAP".to_string(),
                    qty: 1.0,
                    avg_price: 100.0,
                    margin_mode: None,
                }],
                margins: Vec::new(),
            },
            3_001,
        );
        assert_eq!(guard.pending_count(), 0);
    }

    #[test]
    fn spot_fill_requires_both_base_and_quote_updates() {
        let mut guard = spot_guard(2_000);
        guard.observe_fill("main", &fill("BTC-USDT", 10), 100);
        guard.observe_account("main", &balances(11, &["BTC"]), 101);

        assert_eq!(guard.pending_count(), 1);
        let breach = guard.expire(2_100);
        assert_eq!(breach.len(), 1);
        assert!(breach[0].reason.contains("balance:USDT"));
        assert!(!breach[0].reason.contains("balance:BTC"));
    }

    #[test]
    fn latency_observation_requires_every_fill_target_and_is_emitted_once() {
        let mut guard = spot_guard(2_000);
        assert!(
            guard
                .observe_fill_at("main", &fill("BTC-USDT", 10), 100, 100_000_000, true,)
                .observations
                .is_empty()
        );
        assert!(
            guard
                .observe_account_at("main", &balances(11, &["BTC"]), 120, 120_000_000)
                .observations
                .is_empty()
        );

        let result = guard.observe_account_at("main", &balances(11, &["USDT"]), 135, 135_000_000);
        let observations = result.observations;

        assert_eq!(observations.len(), 1);
        assert_eq!(observations[0].symbol, "BTC-USDT");
        assert_eq!(observations[0].first_observed_ns, 100_000_000);
        assert_eq!(observations[0].state_visible_ns, 135_000_000);
        assert!(
            guard
                .observe_account_at("main", &balances(12, &["USDT"]), 140, 140_000_000)
                .observations
                .is_empty()
        );
    }

    #[test]
    fn already_visible_account_state_produces_zero_fill_visibility_delay() {
        let mut guard = derivative_guard(2_000);
        guard.observe_account(
            "main",
            &AccountUpdate {
                ts_ms: 20,
                balances: Vec::new(),
                positions: vec![Position {
                    symbol: "BTC-USDT-SWAP".to_string(),
                    qty: 1.0,
                    avg_price: 100.0,
                    margin_mode: None,
                }],
                margins: Vec::new(),
            },
            100,
        );

        let observations = guard
            .observe_fill_at("main", &fill("BTC-USDT-SWAP", 19), 101, 101_000_000, true)
            .observations;

        assert_eq!(observations.len(), 1);
        assert_eq!(
            observations[0].first_observed_ns,
            observations[0].state_visible_ns
        );
    }

    #[test]
    fn pending_fill_latency_evidence_is_bounded_and_reports_a_drop() {
        let mut guard = derivative_guard(2_000);
        for ordinal in 0..MAX_PENDING_FILL_LATENCY_OBSERVATIONS {
            let mut update = fill("BTC-USDT-SWAP", ordinal as u64 + 1);
            update.filled_qty = ordinal as f64 + 1.0;
            let result = guard.observe_fill_at(
                "main",
                &update,
                ordinal as u64 + 1,
                ordinal as u64 + 1,
                true,
            );
            assert!(!result.dropped_latency_observation);
        }

        let mut overflow = fill("BTC-USDT-SWAP", 10_000);
        overflow.filled_qty = 10_000.0;
        let result = guard.observe_fill_at("main", &overflow, 10_000, 10_000, true);

        assert!(result.dropped_latency_observation);
        assert_eq!(
            guard.latency_pending.len(),
            MAX_PENDING_FILL_LATENCY_OBSERVATIONS
        );
    }

    #[test]
    fn newer_state_observed_before_fill_already_covers_it() {
        let mut guard = derivative_guard(2_000);
        guard.observe_account(
            "main",
            &AccountUpdate {
                ts_ms: 20,
                balances: Vec::new(),
                positions: vec![Position {
                    symbol: "BTC-USDT-SWAP".to_string(),
                    qty: 1.0,
                    avg_price: 100.0,
                    margin_mode: None,
                }],
                margins: Vec::new(),
            },
            100,
        );
        guard.observe_fill("main", &fill("BTC-USDT-SWAP", 19), 101);

        assert_eq!(guard.pending_count(), 0);
        assert!(guard.expire(5_000).is_empty());
    }

    #[test]
    fn repeated_fills_do_not_extend_the_first_pending_deadline() {
        let mut guard = derivative_guard(2_000);
        guard.observe_fill("main", &fill("BTC-USDT-SWAP", 10), 100);
        let mut second = fill("BTC-USDT-SWAP", 11);
        second.filled_qty = 2.0;
        guard.observe_fill("main", &second, 1_900);

        let breach = guard.expire(2_100);

        assert_eq!(breach.len(), 1);
        assert!(breach[0].reason.contains("oldest_age_ms=2000"));
    }

    #[test]
    fn repeated_fill_requires_state_newer_than_the_latest_fill() {
        let mut guard = derivative_guard(2_000);
        guard.observe_fill("main", &fill("BTC-USDT-SWAP", 10), 100);
        let mut second = fill("BTC-USDT-SWAP", 20);
        second.filled_qty = 2.0;
        guard.observe_fill("main", &second, 200);

        guard.observe_account(
            "main",
            &AccountUpdate {
                ts_ms: 15,
                balances: Vec::new(),
                positions: vec![Position {
                    symbol: "BTC-USDT-SWAP".to_string(),
                    qty: 1.0,
                    avg_price: 100.0,
                    margin_mode: None,
                }],
                margins: Vec::new(),
            },
            201,
        );
        assert_eq!(guard.pending_count(), 1);

        guard.observe_account(
            "main",
            &AccountUpdate {
                ts_ms: 20,
                balances: Vec::new(),
                positions: vec![Position {
                    symbol: "BTC-USDT-SWAP".to_string(),
                    qty: 2.0,
                    avg_price: 100.0,
                    margin_mode: None,
                }],
                margins: Vec::new(),
            },
            202,
        );
        assert_eq!(guard.pending_count(), 0);
    }

    #[test]
    fn authoritative_snapshot_clears_every_pending_target() {
        let mut guard = spot_guard(2_000);
        guard.observe_fill("main", &fill("BTC-USDT", 10), 100);
        assert_eq!(guard.pending_count(), 2);

        let censored = guard.observe_authoritative("main", 11);

        assert_eq!(guard.pending_count(), 0);
        assert_eq!(censored, 1);
        assert!(guard.expire(5_000).is_empty());
    }

    #[test]
    fn live_config_builds_spot_and_derivative_routes() {
        let config =
            LiveConfig::from_toml(include_str!("../../../examples/live-okx-demo.toml")).unwrap();
        let mut guard = FillConvergenceGuard::new(&config);

        guard.observe_fill("main", &fill("BTC-USDT", 10), 100);
        assert_eq!(guard.pending_count(), 2);
        guard.observe_authoritative("main", 11);
        guard.observe_fill("main", &fill("BTC-USDT-SWAP", 12), 200);
        assert_eq!(guard.pending_count(), 1);
    }

    #[test]
    fn pending_submit_expires_once_or_clears_on_private_state() {
        let mut guard = OrderStateConvergenceGuard::new(5_000);
        let pending = order_state(OrderStatus::PendingNew, OrderEvent::PendingNew, 10);
        guard.observe_order("main", &pending, 100);

        assert!(guard.expire(5_099).is_empty());
        let breaches = guard.expire(5_100);
        assert_eq!(breaches.len(), 1);
        assert_eq!(breaches[0].account_id, "main");
        assert_eq!(breaches[0].symbol.as_deref(), Some("BTC-USDT-SWAP"));
        assert!(breaches[0].expired_cancel_order_ids.is_empty());
        assert!(breaches[0].reason.contains("submit_to_private_state"));
        assert!(guard.expire(6_000).is_empty());

        guard.observe_order("main", &pending, 6_100);
        guard.observe_order(
            "main",
            &order_state(OrderStatus::Live, OrderEvent::New, 11),
            6_101,
        );
        assert_eq!(guard.pending_count(), 0);
    }

    #[test]
    fn pending_cancel_survives_nonterminal_updates_and_rearms_after_expiry() {
        let mut guard = OrderStateConvergenceGuard::new(5_000);
        guard.observe_cancel("main", "order-1", "BTC-USDT-SWAP", 100);
        guard.observe_order(
            "main",
            &order_state(OrderStatus::Live, OrderEvent::New, 10),
            200,
        );
        assert!(
            guard
                .pending_reason("main")
                .is_some_and(|reason| reason.contains("cancel_to_terminal_state"))
        );

        let breaches = guard.expire(5_100);
        assert_eq!(breaches.len(), 1);
        assert_eq!(breaches[0].expired_cancel_order_ids, ["order-1"]);
        assert!(breaches[0].reason.contains("cancel_to_terminal_state"));
        assert!(!guard.has_pending_cancel("main", "order-1"));

        guard.observe_cancel("main", "order-1", "BTC-USDT-SWAP", 5_200);
        guard.observe_order(
            "main",
            &order_state(OrderStatus::Cancelled, OrderEvent::Cancelled, 11),
            5_201,
        );
        assert_eq!(guard.pending_count(), 0);
        assert!(guard.pending_reason("main").is_none());
        assert!(guard.expire(20_000).is_empty());
    }
}
