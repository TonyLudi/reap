use reap_pm_live::PmReadOnlyMonitor;

fn extract_state(root: &mut PmReadOnlyMonitor) {
    let _ = root.private_state();
    let _ = root.state_mut();
}

fn main() {}
