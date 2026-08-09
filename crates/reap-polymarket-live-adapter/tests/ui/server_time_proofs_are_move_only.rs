use reap_polymarket_live_adapter::{
    PmCancelMutationTimeProof, PmPlaceMutationTimeProof, PmReadServerTime,
};

fn clone_place(value: PmPlaceMutationTimeProof) {
    let _ = value.clone();
}

fn clone_cancel(value: PmCancelMutationTimeProof) {
    let _ = value.clone();
}

fn clone_read(value: PmReadServerTime) {
    let _ = value.clone();
}

fn main() {}
