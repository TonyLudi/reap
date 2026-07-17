use reap_okx_live_adapter::{
    ForbiddenOrderObserver, LiveReadiness, LiveSafety, PrivateStateSessionFactory,
    RegularExecution, RegularOrderSessionFactory, RegularReconciliation,
};

fn reach_for_raw_authority(
    readiness: &LiveReadiness,
    reconciliation: &RegularReconciliation,
    execution: &RegularExecution,
    safety: &LiveSafety,
    forbidden: &ForbiddenOrderObserver,
    private_state: &PrivateStateSessionFactory,
    order_sessions: &RegularOrderSessionFactory,
) {
    let _ = readiness.credentials();
    let _ = reconciliation.signer();
    let _ = execution.wire();
    let _ = safety.transport();
    let _ = forbidden.get("/api/v5/account/config");
    let _ = private_state.post("/api/v5/trade/order", "{}");
    let _ = order_sessions.get("/api/v5/account/config");
    let _ = order_sessions.post("/api/v5/trade/order", "{}");
}

fn main() {}
