use reap_polymarket_adapter::{
    PmFixtureAccountPositionSnapshot, PmFixturePrivateLifecycle, PmFixtureReconciliation,
};

fn cannot_mutate(
    private: PmFixturePrivateLifecycle,
    reconciliation: PmFixtureReconciliation,
    account: PmFixtureAccountPositionSnapshot,
) {
    private.place();
    reconciliation.cancel_owned();
    account.execution();
}

fn main() {}
