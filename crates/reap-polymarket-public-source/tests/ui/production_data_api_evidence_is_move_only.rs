use reap_polymarket_public_source::PmProductionDataApiPositionObservation;

fn requires_clone<T: Clone>() {}

fn main() {
    requires_clone::<PmProductionDataApiPositionObservation>();
}
