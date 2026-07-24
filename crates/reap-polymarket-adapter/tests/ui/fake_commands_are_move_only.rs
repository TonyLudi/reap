use reap_polymarket_adapter::{PmFakeCancelCommand, PmFakePlaceCommand};

fn require_clone<T: Clone>() {}

fn main() {
    require_clone::<PmFakePlaceCommand>();
    require_clone::<PmFakeCancelCommand>();
}
