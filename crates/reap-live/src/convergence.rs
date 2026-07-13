use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use reap_core::{AccountUpdate, OrderUpdate};

use crate::LiveConfig;

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
            reported_accounts: HashSet::new(),
        }
    }

    pub fn observe_fill(&mut self, account_id: &str, update: &OrderUpdate, observed_ms: u64) {
        if !update.has_fill() {
            return;
        }
        let Some(targets) = self
            .routes
            .get(account_id)
            .and_then(|routes| routes.get(&update.symbol))
            .cloned()
        else {
            return;
        };
        for target in targets {
            let key = ScopedTarget {
                account_id: account_id.to_string(),
                target,
            };
            let last_state_ms = self.last_state_exchange_ms.get(&key).copied().unwrap_or(0);
            if update.ts_ms > 0 && last_state_ms >= update.ts_ms {
                continue;
            }
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
    }

    pub fn observe_account(&mut self, account_id: &str, update: &AccountUpdate, observed_ms: u64) {
        for balance in &update.balances {
            self.observe_target(
                ScopedTarget {
                    account_id: account_id.to_string(),
                    target: StateTarget::Balance(balance.currency.clone()),
                },
                update.ts_ms,
                observed_ms,
            );
        }
        for position in &update.positions {
            self.observe_target(
                ScopedTarget {
                    account_id: account_id.to_string(),
                    target: StateTarget::Position(position.symbol.clone()),
                },
                update.ts_ms,
                observed_ms,
            );
        }
        self.clear_reported_if_converged(account_id);
    }

    pub fn observe_authoritative(&mut self, account_id: &str, exchange_ms: u64) {
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
        self.reported_accounts.remove(account_id);
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
            qty: 2.0,
            open_qty: 1.0,
            filled_qty: 1.0,
            avg_fill_price: 100.0,
            last_fill_qty: 1.0,
            last_fill_price: 100.0,
            last_fill_liquidity: Some(FillLiquidity::Maker),
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

        guard.observe_authoritative("main", 11);

        assert_eq!(guard.pending_count(), 0);
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
}
