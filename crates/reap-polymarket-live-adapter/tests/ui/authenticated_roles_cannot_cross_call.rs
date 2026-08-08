use reap_polymarket_live_adapter::{PmAccountHttpRole, PmReadServerTime, PmReconciliationHttpRole};

async fn cross_capabilities(
    mut reconciliation: PmReconciliationHttpRole<'_>,
    mut account: PmAccountHttpRole<'_>,
    timestamp: PmReadServerTime,
) {
    let _ = reconciliation.collateral_balance_allowance(timestamp).await;
    let _ = account.begin_open_orders(timestamp).await;
}

fn main() {}
