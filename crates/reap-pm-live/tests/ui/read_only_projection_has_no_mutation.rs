use reap_pm_live::PmReadOnlyPrivateProjection;
use reap_pm_state::{
    PmOwnedOrderRegistration, PmRefreshReason,
};

fn mutate(
    projection: &mut PmReadOnlyPrivateProjection<'_>,
    registration: PmOwnedOrderRegistration,
) {
    projection.register_owned_order(registration);
    projection.require_refresh(PmRefreshReason::FillObserved);
}

fn main() {}
