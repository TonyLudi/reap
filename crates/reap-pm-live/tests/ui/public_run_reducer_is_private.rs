use reap_pm_live::PmPublicCaptureRun;

fn borrow_raw_reducer(run: &mut PmPublicCaptureRun) {
    let _ = &mut run.pm_reducer;
}

fn main() {}
