use reap_polymarket_live_adapter::{
    PmOpenOrdersAssembly, PmReadServerTime, PmReconciliationHttpRole,
};

async fn reuse(
    role: &mut PmReconciliationHttpRole<'_>,
    timestamp: PmReadServerTime,
    assembly: PmOpenOrdersAssembly,
) {
    let _ = role.continue_open_orders(timestamp, assembly).await;
    let _ = role.continue_open_orders(timestamp, assembly).await;
}

fn main() {}
