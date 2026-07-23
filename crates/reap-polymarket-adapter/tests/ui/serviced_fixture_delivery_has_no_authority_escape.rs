use reap_pm_core::PmCompleteOpenOrdersSnapshot;
use reap_polymarket_adapter::PmFixtureServicedAggregate;

fn cannot_escape(delivery: PmFixtureServicedAggregate<PmCompleteOpenOrdersSnapshot>) {
    let _ = delivery.payload();
    let _ = delivery.owner_id();
    let _ = delivery.into_envelope();
}

fn main() {}
