use reap_polymarket_adapter::{
    PmFixtureAccountRoleGrant, PmFixturePrivateRoleGrant, PmFixtureReadOwnerGrant,
    PmFixtureReconciliationRoleGrant,
};

fn require_clone<T: Clone>() {}

fn main() {
    require_clone::<PmFixtureReadOwnerGrant>();
    require_clone::<PmFixturePrivateRoleGrant>();
    require_clone::<PmFixtureReconciliationRoleGrant>();
    require_clone::<PmFixtureAccountRoleGrant>();
}
