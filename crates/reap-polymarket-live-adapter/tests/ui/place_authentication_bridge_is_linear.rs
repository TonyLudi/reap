use reap_polymarket_auth::{L2Credentials, SerializedPlaceRequest};
use reap_polymarket_live_adapter::{PmPlaceMutationTimeFinalizer, PmPlaceMutationTimeProof};

fn authenticate_twice(
    finalizer: &mut PmPlaceMutationTimeFinalizer,
    proof: PmPlaceMutationTimeProof,
    credentials: &L2Credentials,
    first: SerializedPlaceRequest,
    second: SerializedPlaceRequest,
) {
    let _ = finalizer.authenticate_exact_place(proof, 1_700_000_000, credentials, first);
    let _ = finalizer.authenticate_exact_place(proof, 1_700_000_000, credentials, second);
}

fn main() {}
