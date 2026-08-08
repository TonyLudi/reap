use reap_polymarket_adapter::{PmExactOwnedCancelRequest, PmGtcPostOnlyPlaceRequest};

fn require_clone<T: Clone>() {}

fn main() {
    require_clone::<PmGtcPostOnlyPlaceRequest>();
    require_clone::<PmExactOwnedCancelRequest>();
}
