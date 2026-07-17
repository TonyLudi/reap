use reap_okx_live_adapter::ObserveRoles;

fn escalate(observe: &ObserveRoles) {
    let _ = observe.execution();
    let _ = observe.safety();
    let _ = observe.regular_order_sessions();
    let _ = observe.take_private_state_sessions();
}

fn main() {}
