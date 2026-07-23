use reap_pm_live::PmReadOnlyMonitor;

fn extract_roles(root: &PmReadOnlyMonitor) {
    let _ = root.private_role();
    let _ = root.reconciliation_role();
    let _ = root.account_role();
}

fn main() {}
