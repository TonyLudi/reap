use reap_polymarket_live_adapter::PmLoopbackMutationConnectivityBinding;

fn require_clone<T: Clone>() {}

fn main() {
    require_clone::<PmLoopbackMutationConnectivityBinding>();
}
