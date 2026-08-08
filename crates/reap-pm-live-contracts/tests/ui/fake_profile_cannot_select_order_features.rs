use reap_pm_live_contracts::PmFixedExecutionProfile;

fn main() {
    let profile = PmFixedExecutionProfile::gtc_post_only_owned_cancel();
    profile.set_order_type("market");
    profile.set_order_type("FOK");
    profile.set_order_type("FAK");
    profile.set_order_type("IOC");
    profile.set_order_type("GTD");
    profile.enable_batch_place();
    profile.enable_batch_cancel();
    profile.enable_cancel_all();
    profile.enable_amend();
    profile.enable_allowance_mutation();
    profile.enable_settlement();
    profile.enable_redemption();
}
