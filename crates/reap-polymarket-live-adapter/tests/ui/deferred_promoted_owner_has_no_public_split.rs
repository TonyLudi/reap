use reap_polymarket_live_adapter::PmProductionSelectedPlaceCancelTimeOwner;

fn split_without_the_private_actor_bridge(value: PmProductionSelectedPlaceCancelTimeOwner) {
    let _ = value.into_purpose_owners();
}

fn main() {}
