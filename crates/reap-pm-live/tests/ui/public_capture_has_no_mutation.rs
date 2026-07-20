use reap_pm_live::PmPublicCapture;

fn escalate(root: &PmPublicCapture) {
    let _ = root.execution();
    root.place();
    root.cancel_owned();
}

fn main() {}
