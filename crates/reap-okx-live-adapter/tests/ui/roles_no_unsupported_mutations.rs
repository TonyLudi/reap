use reap_okx_live_adapter::{
    ForbiddenOrderObserver, LiveSafety, RegularExecution, RegularOrderSessionFactory,
};

fn mutate_outside_regular_scope(
    execution: &RegularExecution,
    safety: &LiveSafety,
    forbidden: &ForbiddenOrderObserver,
    order_sessions: &RegularOrderSessionFactory,
) {
    let _ = execution.place_regular_order();
    let _ = execution.submit_algo_order();
    let _ = execution.submit_spread_order();
    let _ = execution.amend_regular_order();

    let _ = safety.cancel_algo_orders();
    let _ = safety.spread_mass_cancel();
    let _ = safety.spread_cancel_all_after();
    let _ = safety.amend_order();

    let _ = forbidden.submit_algo_order();
    let _ = forbidden.cancel_algo_orders();
    let _ = forbidden.submit_spread_order();
    let _ = forbidden.spread_mass_cancel();
    let _ = forbidden.amend_order();

    let _ = order_sessions.algo_request();
    let _ = order_sessions.spread_request();
    let _ = order_sessions.amend_request();
}

fn main() {}
