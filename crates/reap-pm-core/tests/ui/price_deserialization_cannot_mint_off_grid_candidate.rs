use reap_pm_core::PmPrice;
use serde::Deserialize;

fn require_deserialize<T: for<'de> Deserialize<'de>>() {}

fn off_grid_candidate_cannot_be_minted_by_deserialization() {
    require_deserialize::<PmPrice>();
}

fn main() {}
