use reap_pm_live_contracts::PmFakeExecutionProfile;

fn main() {
    let profile = PmFakeExecutionProfile::goal_f();
    profile.set_order_type("market");
    profile.enable_cancel_all();
}
