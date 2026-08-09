use reap_polymarket_live_adapter::PmReadOnlyPrivateConnectivityOwner;

fn mutation_capability(owner: &mut PmReadOnlyPrivateConnectivityOwner) {
    owner.authenticate_place();
}

fn main() {}
