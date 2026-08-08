use reap_polymarket_adapter::{PmCompletionOccurrence, PmPrivateLifecycle};
use reap_polymarket_wire::PmLiveUserFrame;

fn bare_frame_cannot_claim_authenticated_scope(
    role: &mut PmPrivateLifecycle,
    occurrence: PmCompletionOccurrence,
    frame: PmLiveUserFrame,
) {
    let _ = role.receive_live_user_frame(occurrence, frame);
}

fn main() {}
