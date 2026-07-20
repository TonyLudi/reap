use reap_polymarket_adapter::{PmFixtureOwnedExecution, PmPrivateLifecycleRole, PmPublicRole};

fn require_private<R: PmPrivateLifecycleRole>(_role: R) {}

fn roles_stay_distinct(public: PmPublicRole, execution: PmFixtureOwnedExecution) {
    require_private(public);
    let _: PmPublicRole = execution.into();
}

fn main() {}
