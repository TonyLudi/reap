use reap_okx_live_adapter::{ForbiddenOrderObserver, LiveSafety};

fn mutate_outside_regular_scope(
    safety: &LiveSafety,
    forbidden: &ForbiddenOrderObserver,
) {
    let _ = safety.cancel_algo_orders();
    let _ = safety.spread_mass_cancel();
    let _ = safety.spread_cancel_all_after();
    let _ = safety.amend_order();

    let _ = forbidden.submit_algo_order();
    let _ = forbidden.cancel_algo_orders();
    let _ = forbidden.submit_spread_order();
    let _ = forbidden.spread_mass_cancel();
    let _ = forbidden.amend_order();

}

fn main() {}
