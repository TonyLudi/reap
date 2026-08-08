use reap_polymarket_live_adapter::{
    PmAuthorizedMutationServerTime, PmPendingMutationServerTime, PmReadServerTime,
};

fn clone_pending(value: PmPendingMutationServerTime) {
    let _ = value.clone();
}

fn clone_authorized(value: PmAuthorizedMutationServerTime) {
    let _ = value.clone();
}

fn clone_read(value: PmReadServerTime) {
    let _ = value.clone();
}

fn main() {}
