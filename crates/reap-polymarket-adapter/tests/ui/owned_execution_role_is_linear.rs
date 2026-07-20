use reap_polymarket_adapter::PmFixtureOwnedExecution;

fn require_clone<T: Clone>() {}

fn main() {
    require_clone::<PmFixtureOwnedExecution>();
}
