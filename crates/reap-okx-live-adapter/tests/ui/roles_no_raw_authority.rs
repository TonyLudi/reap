use reap_feed::SocketPlan;
use reap_okx_live_adapter::{
    DemoRoles, ForbiddenOrderObserver, LiveReadiness, LiveSafety, PrivateStateSessionFactory,
    OrderCommandWebsocketLifecycle, RegularReconciliation,
};

fn require_clone<T: Clone>() {}

fn retain_mutation_roles(roles: &DemoRoles) {
    require_clone::<OrderCommandWebsocketLifecycle>();
    let _ = roles.execution();
    let _ = roles.regular_order_sessions();
    let _ = roles.take_execution();
    let _ = roles.take_regular_order_sessions();
}

fn reach_for_raw_authority(
    readiness: &LiveReadiness,
    reconciliation: &RegularReconciliation,
    safety: &LiveSafety,
    forbidden: &ForbiddenOrderObserver,
    private_state: &PrivateStateSessionFactory,
    plan: &SocketPlan,
) {
    let _ = readiness.credentials();
    let _ = reconciliation.signer();
    let _ = safety.transport();
    let _ = forbidden.get("/api/v5/account/config");
    let _ = private_state.post("/api/v5/trade/order", "{}");
    let bootstrap = private_state
        .bootstrap_factory(
            "main",
            plan.clone(),
            "wss://wspap.okx.com:8443/ws/v5/private",
        )
        .unwrap();
    let _ = bootstrap(plan);
    let _ = bootstrap.generate(plan, "wss://wspap.okx.com:8443/ws/v5/private");
}

fn main() {}
