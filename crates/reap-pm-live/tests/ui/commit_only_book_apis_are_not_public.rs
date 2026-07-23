use reap_pm_live::{PmPublicBookDelivery, PmPublicCaptureRun, PmPublicSnapshotFlow};

fn commit_snapshot_only(
    run: &mut PmPublicCaptureRun,
    delivery: PmPublicBookDelivery,
    flow: PmPublicSnapshotFlow,
) {
    let _ = run.commit_pm_snapshot(delivery, flow);
}

fn commit_update_only(run: &mut PmPublicCaptureRun, delivery: PmPublicBookDelivery) {
    let _ = run.commit_pm_book_update(delivery);
}

fn main() {}
