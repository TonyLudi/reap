use reap_pm_live::PmReadOnlyMonitor;

fn escalate(root: &PmReadOnlyMonitor) {
    let _ = root.execution();
    root.place();
}

fn main() {}
