use reap_polymarket_auth::{L2Credentials, SerializedOwnedCancelRequest, SerializedPlaceRequest};
use reap_polymarket_live_adapter::{
    PmCancelMutationTimeFinalizer, PmCancelMutationTimeProof, PmPlaceMutationTimeFinalizer,
    PmPlaceMutationTimeProof,
};

fn place_auth_cannot_consume_cancel(
    mut finalizer: PmPlaceMutationTimeFinalizer,
    proof: PmCancelMutationTimeProof,
    credentials: &L2Credentials,
    request: SerializedPlaceRequest,
) {
    let _ = finalizer.authenticate_exact_place(proof, 1_700_000_000, credentials, request);
}

fn cancel_auth_cannot_consume_place(
    mut finalizer: PmCancelMutationTimeFinalizer,
    proof: PmPlaceMutationTimeProof,
    credentials: &L2Credentials,
    request: SerializedOwnedCancelRequest,
) {
    let _ = finalizer.authenticate_exact_owned_cancel(proof, 1_700_000_000, credentials, request);
}

fn main() {}
