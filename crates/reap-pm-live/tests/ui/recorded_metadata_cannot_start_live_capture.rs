use std::path::PathBuf;

use reap_pm_live::{
    PmCaptureProvenance, PmCaptureSessionPolicy, PmPublicCapture,
};
use reap_polymarket_adapter::PmRecordedMetadataEvidence;

async fn start_live(
    capture: PmPublicCapture,
    recorded: PmRecordedMetadataEvidence,
    policy: PmCaptureSessionPolicy,
    provenance: PmCaptureProvenance,
) {
    let _ = capture
        .start(PathBuf::from("capture.jsonl"), recorded, policy, provenance)
        .await;
}

fn main() {}
