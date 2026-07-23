use reap_pm_live::PmReadOnlyMonitor;

fn inject_callback(root: &mut PmReadOnlyMonitor) {
    root.reduce_private_delivery(|_, _| ());
}

fn main() {}
