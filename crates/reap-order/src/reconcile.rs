use std::collections::{HashMap, HashSet};

use reap_core::{AccountUpdate, Balance, Position};
use reap_venue::{PrivateOrderState, RemoteFill, RemoteOrder};
use serde::{Deserialize, Serialize};

use crate::{OrderReducer, PrivateStateReducer};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReconcileIssue {
    LocalLiveMissingRemote {
        order_id: String,
        symbol: String,
    },
    RemoteLiveMissingLocal {
        order_id: String,
        symbol: String,
    },
    OrderMismatch {
        order_id: String,
        field: String,
        local: String,
        remote: String,
    },
    UnknownFill {
        fill_id: String,
        order_id: String,
        symbol: String,
    },
    BalanceMissingRemote {
        currency: String,
        local_total: String,
    },
    BalanceMissingLocal {
        currency: String,
        remote_total: String,
    },
    BalanceMismatch {
        currency: String,
        field: String,
        local: String,
        remote: String,
    },
    PositionMissingRemote {
        symbol: String,
        local_qty: String,
    },
    PositionMissingLocal {
        symbol: String,
        remote_qty: String,
    },
    PositionMismatch {
        symbol: String,
        field: String,
        local: String,
        remote: String,
    },
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ReconcileReport {
    pub local_live_orders: usize,
    pub remote_live_orders: usize,
    pub remote_fills: usize,
    pub issues: Vec<ReconcileIssue>,
}

impl ReconcileReport {
    pub fn is_clean(&self) -> bool {
        self.issues.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReconciliationSnapshot {
    pub remote_orders: Vec<RemoteOrder>,
    pub remote_fills: Vec<RemoteFill>,
    pub remote_account: AccountUpdate,
    pub report: ReconcileReport,
}

pub fn reconcile(
    local: &OrderReducer,
    known_fill_ids: &HashSet<String>,
    remote_orders: &[RemoteOrder],
    remote_fills: &[RemoteFill],
) -> ReconcileReport {
    reconcile_with_order_ids(
        local,
        known_fill_ids,
        remote_orders,
        remote_fills,
        reported_order_id,
    )
}

fn reconcile_with_order_ids(
    local: &OrderReducer,
    known_fill_ids: &HashSet<String>,
    remote_orders: &[RemoteOrder],
    remote_fills: &[RemoteFill],
    resolve_order_id: impl Fn(&str, &str) -> String,
) -> ReconcileReport {
    let local_live = local
        .orders()
        .filter(|(_, order)| {
            order.is_live()
                || (order.status == reap_core::OrderStatus::PendingNew && order.open_qty > 0.0)
        })
        .collect::<HashMap<_, _>>();
    let remote_live = remote_orders
        .iter()
        .filter(|order| {
            matches!(
                order.state,
                PrivateOrderState::Live | PrivateOrderState::PartiallyFilled
            )
        })
        .map(|order| {
            (
                resolve_order_id(&order.client_order_id, &order.exchange_order_id),
                order,
            )
        })
        .collect::<HashMap<_, _>>();
    let mut issues = Vec::new();

    for (order_id, order) in &local_live {
        let Some(remote) = remote_live.get(*order_id) else {
            issues.push(ReconcileIssue::LocalLiveMissingRemote {
                order_id: (*order_id).to_string(),
                symbol: order.symbol.clone(),
            });
            continue;
        };
        compare_number(&mut issues, order_id, "price", order.price, remote.price);
        compare_number(&mut issues, order_id, "qty", order.qty, remote.qty);
        compare_number(
            &mut issues,
            order_id,
            "filled_qty",
            order.filled_qty,
            remote.cumulative_filled_qty,
        );
        if order.symbol != remote.symbol {
            issues.push(ReconcileIssue::OrderMismatch {
                order_id: (*order_id).to_string(),
                field: "symbol".to_string(),
                local: order.symbol.clone(),
                remote: remote.symbol.clone(),
            });
        }
        if order.side != remote.side {
            issues.push(ReconcileIssue::OrderMismatch {
                order_id: (*order_id).to_string(),
                field: "side".to_string(),
                local: format!("{:?}", order.side).to_lowercase(),
                remote: format!("{:?}", remote.side).to_lowercase(),
            });
        }
    }
    for (order_id, remote) in &remote_live {
        if !local_live.contains_key(order_id.as_str()) {
            issues.push(ReconcileIssue::RemoteLiveMissingLocal {
                order_id: order_id.clone(),
                symbol: remote.symbol.clone(),
            });
        }
    }
    for fill in remote_fills {
        let order_id = resolve_order_id(&fill.client_order_id, &fill.exchange_order_id);
        if !known_fill_ids.contains(&fill.fill_id) {
            issues.push(ReconcileIssue::UnknownFill {
                fill_id: fill.fill_id.clone(),
                order_id,
                symbol: fill.symbol.clone(),
            });
        }
    }
    issues.sort_by(|left, right| format!("{left:?}").cmp(&format!("{right:?}")));

    ReconcileReport {
        local_live_orders: local_live.len(),
        remote_live_orders: remote_live.len(),
        remote_fills: remote_fills.len(),
        issues,
    }
}

pub fn reconcile_full_state(
    local: &PrivateStateReducer,
    remote_orders: &[RemoteOrder],
    remote_fills: &[RemoteFill],
    remote_account: &AccountUpdate,
) -> ReconcileReport {
    let mut report = reconcile_with_order_ids(
        local.order_reducer(),
        local.seen_fill_ids(),
        remote_orders,
        remote_fills,
        |client_order_id, exchange_order_id| {
            local.resolve_order_id(client_order_id, exchange_order_id)
        },
    );
    compare_account_state(local, remote_account, &mut report.issues);
    report
        .issues
        .sort_by(|left, right| format!("{left:?}").cmp(&format!("{right:?}")));
    report
}

fn compare_account_state(
    local: &PrivateStateReducer,
    remote: &AccountUpdate,
    issues: &mut Vec<ReconcileIssue>,
) {
    let remote_balances = remote
        .balances
        .iter()
        .map(|balance| (balance.currency.as_str(), balance))
        .collect::<HashMap<_, _>>();
    for (currency, local_balance) in local.balances() {
        match remote_balances.get(currency.as_str()) {
            Some(remote_balance) => compare_balance(issues, local_balance, remote_balance),
            None if balance_is_nonzero(local_balance) => {
                issues.push(ReconcileIssue::BalanceMissingRemote {
                    currency: currency.clone(),
                    local_total: local_balance.total.to_string(),
                });
            }
            None => {}
        }
    }
    for (currency, remote_balance) in &remote_balances {
        if !local.balances().contains_key(*currency) && balance_is_nonzero(remote_balance) {
            issues.push(ReconcileIssue::BalanceMissingLocal {
                currency: (*currency).to_string(),
                remote_total: remote_balance.total.to_string(),
            });
        }
    }

    let remote_positions = remote
        .positions
        .iter()
        .map(|position| (position.symbol.as_str(), position))
        .collect::<HashMap<_, _>>();
    for (symbol, local_position) in local.positions() {
        match remote_positions.get(symbol.as_str()) {
            Some(remote_position) => compare_position(issues, local_position, remote_position),
            None if number_is_nonzero(local_position.qty) => {
                issues.push(ReconcileIssue::PositionMissingRemote {
                    symbol: symbol.clone(),
                    local_qty: local_position.qty.to_string(),
                });
            }
            None => {}
        }
    }
    for (symbol, remote_position) in &remote_positions {
        if !local.positions().contains_key(*symbol) && number_is_nonzero(remote_position.qty) {
            issues.push(ReconcileIssue::PositionMissingLocal {
                symbol: (*symbol).to_string(),
                remote_qty: remote_position.qty.to_string(),
            });
        }
    }
}

fn compare_balance(issues: &mut Vec<ReconcileIssue>, local: &Balance, remote: &Balance) {
    for (field, local_value, remote_value) in [
        ("total", local.total, remote.total),
        ("available", local.available, remote.available),
        ("equity", local.equity, remote.equity),
        ("liability", local.liability, remote.liability),
        ("max_loan", local.max_loan, remote.max_loan),
    ] {
        if !numbers_equal(local_value, remote_value) {
            issues.push(ReconcileIssue::BalanceMismatch {
                currency: local.currency.clone(),
                field: field.to_string(),
                local: local_value.to_string(),
                remote: remote_value.to_string(),
            });
        }
    }
    let local_indicator = local.forced_repayment_indicator.unwrap_or(0);
    let remote_indicator = remote.forced_repayment_indicator.unwrap_or(0);
    if local_indicator != remote_indicator {
        issues.push(ReconcileIssue::BalanceMismatch {
            currency: local.currency.clone(),
            field: "forced_repayment_indicator".to_string(),
            local: local_indicator.to_string(),
            remote: remote_indicator.to_string(),
        });
    }
}

fn compare_position(issues: &mut Vec<ReconcileIssue>, local: &Position, remote: &Position) {
    if !numbers_equal(local.qty, remote.qty) {
        issues.push(ReconcileIssue::PositionMismatch {
            symbol: local.symbol.clone(),
            field: "qty".to_string(),
            local: local.qty.to_string(),
            remote: remote.qty.to_string(),
        });
    }
    if (number_is_nonzero(local.qty) || number_is_nonzero(remote.qty))
        && !numbers_equal(local.avg_price, remote.avg_price)
    {
        issues.push(ReconcileIssue::PositionMismatch {
            symbol: local.symbol.clone(),
            field: "avg_price".to_string(),
            local: local.avg_price.to_string(),
            remote: remote.avg_price.to_string(),
        });
    }
    if (number_is_nonzero(local.qty) || number_is_nonzero(remote.qty))
        && local.margin_mode != remote.margin_mode
    {
        issues.push(ReconcileIssue::PositionMismatch {
            symbol: local.symbol.clone(),
            field: "margin_mode".to_string(),
            local: format!("{:?}", local.margin_mode).to_lowercase(),
            remote: format!("{:?}", remote.margin_mode).to_lowercase(),
        });
    }
}

fn balance_is_nonzero(balance: &Balance) -> bool {
    [
        balance.total,
        balance.available,
        balance.equity,
        balance.liability,
        balance.max_loan,
    ]
    .into_iter()
    .any(number_is_nonzero)
}

fn number_is_nonzero(value: f64) -> bool {
    !numbers_equal(value, 0.0)
}

fn numbers_equal(left: f64, right: f64) -> bool {
    if left == right {
        return true;
    }
    if !left.is_finite() || !right.is_finite() {
        return false;
    }
    let scale = left.abs().max(right.abs()).max(1.0);
    (left - right).abs() <= scale * 1e-9
}

fn reported_order_id(client_order_id: &str, exchange_order_id: &str) -> String {
    if client_order_id.is_empty() || client_order_id == "0" {
        exchange_order_id.to_string()
    } else {
        client_order_id.to_string()
    }
}

fn compare_number(
    issues: &mut Vec<ReconcileIssue>,
    order_id: &str,
    field: &str,
    local: f64,
    remote: f64,
) {
    if (local - remote).abs() > 1e-9 {
        issues.push(ReconcileIssue::OrderMismatch {
            order_id: order_id.to_string(),
            field: field.to_string(),
            local: local.to_string(),
            remote: remote.to_string(),
        });
    }
}

#[cfg(test)]
mod tests {
    use reap_core::{NewOrder, PositionMarginMode, Side, TimeInForce};

    use super::*;

    #[test]
    fn reports_remote_drift_and_unknown_fills() {
        let mut local = OrderReducer::new();
        local.ack_new(
            "client-1",
            NewOrder {
                symbol: "BTC-USDT".to_string(),
                side: Side::Buy,
                qty: 1.0,
                price: 100.0,
                time_in_force: TimeInForce::Gtc,
                reduce_only: false,
                self_trade_prevention: None,
                reason: "test".to_string(),
            },
            1,
        );
        let remote = RemoteOrder {
            exchange_order_id: "exchange-1".to_string(),
            client_order_id: "client-1".to_string(),
            symbol: "BTC-USDT".to_string(),
            side: Side::Buy,
            state: PrivateOrderState::Live,
            price: 101.0,
            qty: 1.0,
            cumulative_filled_qty: 0.0,
            average_fill_price: 0.0,
            update_time_ms: 2,
        };
        let fill = RemoteFill {
            fill_id: "fill-unknown".to_string(),
            exchange_order_id: "exchange-2".to_string(),
            client_order_id: "client-2".to_string(),
            symbol: "ETH-USDT".to_string(),
            side: Side::Sell,
            price: 50.0,
            qty: 1.0,
            liquidity: reap_core::FillLiquidity::Taker,
            fee: None,
            ts_ms: 3,
        };

        let report = reconcile(&local, &HashSet::new(), &[remote], &[fill]);
        assert!(!report.is_clean());
        assert_eq!(report.issues.len(), 2);
    }

    #[test]
    fn full_state_reconciliation_resolves_missing_client_id_from_submit_binding() {
        let mut local = PrivateStateReducer::new();
        local.order_reducer_mut().ack_new(
            "client-1",
            NewOrder {
                symbol: "BTC-USDT".to_string(),
                side: Side::Buy,
                qty: 1.0,
                price: 100.0,
                time_in_force: TimeInForce::Gtc,
                reduce_only: false,
                self_trade_prevention: None,
                reason: "test".to_string(),
            },
            1,
        );
        local
            .bind_exchange_order_id("client-1", "exchange-1")
            .unwrap();
        let remote = RemoteOrder {
            exchange_order_id: "exchange-1".to_string(),
            client_order_id: String::new(),
            symbol: "BTC-USDT".to_string(),
            side: Side::Buy,
            state: PrivateOrderState::Live,
            price: 100.0,
            qty: 1.0,
            cumulative_filled_qty: 0.0,
            average_fill_price: 0.0,
            update_time_ms: 2,
        };
        let account = AccountUpdate {
            ts_ms: 2,
            balances: Vec::new(),
            positions: Vec::new(),
            margins: Vec::new(),
        };

        assert!(reconcile_full_state(&local, &[remote], &[], &account).is_clean());
    }

    #[test]
    fn full_state_reconciliation_covers_balances_and_positions() {
        let mut local = PrivateStateReducer::new();
        local.apply_account(AccountUpdate {
            ts_ms: 1,
            balances: vec![Balance {
                account_id: Some("main".to_string()),
                currency: "USDT".to_string(),
                total: 100.0,
                available: 90.0,
                equity: 100.0,
                liability: 0.0,
                max_loan: 10.0,
                forced_repayment_indicator: None,
            }],
            positions: vec![Position {
                symbol: "BTC-USDT-SWAP".to_string(),
                qty: 2.0,
                avg_price: 50_000.0,
                margin_mode: None,
            }],
            margins: Vec::new(),
        });
        let remote = AccountUpdate {
            ts_ms: 2,
            balances: vec![Balance {
                account_id: None,
                currency: "USDT".to_string(),
                total: 100.0,
                available: 90.0,
                equity: 100.0,
                liability: 0.0,
                max_loan: 10.0,
                forced_repayment_indicator: None,
            }],
            positions: vec![Position {
                symbol: "BTC-USDT-SWAP".to_string(),
                qty: 2.0,
                avg_price: 50_000.0,
                margin_mode: None,
            }],
            margins: Vec::new(),
        };

        assert!(reconcile_full_state(&local, &[], &[], &remote).is_clean());
    }

    #[test]
    fn full_state_reconciliation_reports_position_margin_mode_drift() {
        let mut local = PrivateStateReducer::new();
        local.apply_account(AccountUpdate {
            ts_ms: 1,
            balances: Vec::new(),
            positions: vec![Position {
                symbol: "BTC-USDT-SWAP".to_string(),
                qty: 2.0,
                avg_price: 50_000.0,
                margin_mode: Some(PositionMarginMode::Cross),
            }],
            margins: Vec::new(),
        });
        let remote = AccountUpdate {
            ts_ms: 2,
            balances: Vec::new(),
            positions: vec![Position {
                symbol: "BTC-USDT-SWAP".to_string(),
                qty: 2.0,
                avg_price: 50_000.0,
                margin_mode: Some(PositionMarginMode::Isolated),
            }],
            margins: Vec::new(),
        };

        let report = reconcile_full_state(&local, &[], &[], &remote);

        assert!(report.issues.iter().any(|issue| matches!(
            issue,
            ReconcileIssue::PositionMismatch { symbol, field, .. }
                if symbol == "BTC-USDT-SWAP" && field == "margin_mode"
        )));
    }

    #[test]
    fn full_state_reconciliation_reports_forced_repayment_indicator_drift() {
        let mut local = PrivateStateReducer::new();
        local.apply_account(AccountUpdate {
            ts_ms: 1,
            balances: vec![Balance {
                account_id: Some("main".to_string()),
                currency: "USDT".to_string(),
                total: 100.0,
                available: 90.0,
                equity: 100.0,
                liability: 0.0,
                max_loan: 0.0,
                forced_repayment_indicator: Some(1),
            }],
            positions: Vec::new(),
            margins: Vec::new(),
        });
        let remote = AccountUpdate {
            ts_ms: 2,
            balances: vec![Balance {
                account_id: None,
                currency: "USDT".to_string(),
                total: 100.0,
                available: 90.0,
                equity: 100.0,
                liability: 0.0,
                max_loan: 0.0,
                forced_repayment_indicator: Some(2),
            }],
            positions: Vec::new(),
            margins: Vec::new(),
        };

        let report = reconcile_full_state(&local, &[], &[], &remote);

        assert!(report.issues.iter().any(|issue| matches!(
            issue,
            ReconcileIssue::BalanceMismatch { currency, field, .. }
                if currency == "USDT" && field == "forced_repayment_indicator"
        )));
    }

    #[test]
    fn full_state_reconciliation_reports_closed_and_changed_rows() {
        let mut local = PrivateStateReducer::new();
        local.apply_account(AccountUpdate {
            ts_ms: 1,
            balances: vec![Balance {
                account_id: Some("main".to_string()),
                currency: "BTC".to_string(),
                total: 1.0,
                available: 1.0,
                equity: 1.0,
                liability: 0.0,
                max_loan: 0.0,
                forced_repayment_indicator: None,
            }],
            positions: vec![Position {
                symbol: "BTC-USDT-SWAP".to_string(),
                qty: 2.0,
                avg_price: 50_000.0,
                margin_mode: None,
            }],
            margins: Vec::new(),
        });
        let remote = AccountUpdate {
            ts_ms: 2,
            balances: vec![Balance {
                account_id: None,
                currency: "BTC".to_string(),
                total: 1.5,
                available: 1.5,
                equity: 1.5,
                liability: 0.0,
                max_loan: 0.0,
                forced_repayment_indicator: None,
            }],
            positions: Vec::new(),
            margins: Vec::new(),
        };

        let report = reconcile_full_state(&local, &[], &[], &remote);

        assert!(!report.is_clean());
        assert!(report.issues.iter().any(|issue| matches!(
            issue,
            ReconcileIssue::BalanceMismatch {
                currency,
                field,
                ..
            } if currency == "BTC" && field == "total"
        )));
        assert!(report.issues.iter().any(|issue| matches!(
            issue,
            ReconcileIssue::PositionMissingRemote { symbol, .. }
                if symbol == "BTC-USDT-SWAP"
        )));
    }
}
