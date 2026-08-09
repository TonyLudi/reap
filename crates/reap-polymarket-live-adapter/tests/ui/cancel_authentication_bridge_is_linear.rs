use reap_polymarket_auth::{L2Credentials, SerializedOwnedCancelRequest};
use reap_polymarket_live_adapter::{PmCancelMutationTimeFinalizer, PmCancelMutationTimeProof};

fn authenticate_twice(
    finalizer: &mut PmCancelMutationTimeFinalizer,
    proof: PmCancelMutationTimeProof,
    credentials: &L2Credentials,
    first: SerializedOwnedCancelRequest,
    second: SerializedOwnedCancelRequest,
) {
    let _ = finalizer.authenticate_exact_owned_cancel(proof, 1_700_000_000, credentials, first);
    let _ = finalizer.authenticate_exact_owned_cancel(proof, 1_700_000_000, credentials, second);
}

fn main() {}
