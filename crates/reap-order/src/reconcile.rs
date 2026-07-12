use std::collections::{HashMap, HashSet};

use reap_venue::{PrivateOrderState, RemoteFill, RemoteOrder};
use serde::{Deserialize, Serialize};

use crate::OrderReducer;

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
    pub report: ReconcileReport,
}

pub fn reconcile(
    local: &OrderReducer,
    known_fill_ids: &HashSet<String>,
    remote_orders: &[RemoteOrder],
    remote_fills: &[RemoteFill],
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
        .map(|order| (remote_id(order), order))
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
        let order_id = if fill.client_order_id.is_empty() {
            &fill.exchange_order_id
        } else {
            &fill.client_order_id
        };
        if !known_fill_ids.contains(&fill.fill_id) {
            issues.push(ReconcileIssue::UnknownFill {
                fill_id: fill.fill_id.clone(),
                order_id: order_id.clone(),
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

fn remote_id(order: &RemoteOrder) -> String {
    if order.client_order_id.is_empty() {
        order.exchange_order_id.clone()
    } else {
        order.client_order_id.clone()
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
    use reap_core::{NewOrder, Side, TimeInForce};

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
            ts_ms: 3,
        };

        let report = reconcile(&local, &HashSet::new(), &[remote], &[fill]);
        assert!(!report.is_clean());
        assert_eq!(report.issues.len(), 2);
    }
}
