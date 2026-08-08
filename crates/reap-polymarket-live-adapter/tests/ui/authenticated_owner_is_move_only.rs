use reap_polymarket_live_adapter::PmAuthenticatedHttpOwner;

fn require_clone<T: Clone>() {}

fn main() {
    require_clone::<PmAuthenticatedHttpOwner>();
}
