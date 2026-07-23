use reap_pm_core::{
    EventEnvelope, PmCompleteAccountSnapshot, PmOrderEvent,
};
use reap_pm_live::PmReadOnlyMonitor;

fn inject(
    root: &mut PmReadOnlyMonitor,
    order: EventEnvelope<PmOrderEvent>,
    account: EventEnvelope<PmCompleteAccountSnapshot>,
) {
    root.observe_order(order);
    root.apply_account_snapshot(account);
}

fn main() {}
