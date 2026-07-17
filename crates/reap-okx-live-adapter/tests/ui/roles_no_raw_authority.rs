use reap_feed::SocketPlan;
use reap_okx_live_adapter::{
    DemoRoles, ForbiddenOrderObserver, LiveReadiness, LiveSafety, PrivateStateSessionFactory,
    RegularExecution, RegularOrderSessionFactory, RegularReconciliation,
};
use reap_order::{OkxOrderTransport, RegularExecution as RegularExecutionPort};
use reap_venue::okx::{OkxCancelOrder, OkxPlaceOrder};

fn require_clone<T: Clone>() {}

fn retain_mutation_roles(roles: &DemoRoles) {
    require_clone::<RegularExecution>();
    require_clone::<RegularOrderSessionFactory>();
    let _ = roles.execution();
    let _ = roles.regular_order_sessions();
    let _ = roles.take_execution();
    let _ = roles.take_regular_order_sessions();
}

fn reach_for_raw_authority(
    readiness: &LiveReadiness,
    reconciliation: &RegularReconciliation,
    execution: &RegularExecution,
    safety: &LiveSafety,
    forbidden: &ForbiddenOrderObserver,
    private_state: &PrivateStateSessionFactory,
    order_sessions: &RegularOrderSessionFactory,
    transport: &dyn OkxOrderTransport,
    place: &OkxPlaceOrder,
    cancel: &OkxCancelOrder,
    plan: &SocketPlan,
) {
    let _ = readiness.credentials();
    let _ = reconciliation.signer();
    let _ = execution.wire();
    let _ = safety.transport();
    let _ = forbidden.get("/api/v5/account/config");
    let _ = private_state.post("/api/v5/trade/order", "{}");
    let bootstrap = private_state
        .bootstrap_factory("wss://wspap.okx.com:8443/ws/v5/private")
        .unwrap();
    let _ = bootstrap(plan);
    let _ = bootstrap.generate(plan, "wss://wspap.okx.com:8443/ws/v5/private");
    let _ = order_sessions.get("/api/v5/account/config");
    let _ = order_sessions.post("/api/v5/trade/order", "{}");
    let _ = order_sessions.place_request("place1", 1, place);
    let _ = order_sessions.cancel_request("cancel1", cancel);
    let _ = OkxOrderTransport::place_order(transport, place);
    let _ = OkxOrderTransport::cancel_order(transport, cancel);
    let _ = RegularExecutionPort::cancel_regular_order(execution, cancel);
}

fn main() {}
