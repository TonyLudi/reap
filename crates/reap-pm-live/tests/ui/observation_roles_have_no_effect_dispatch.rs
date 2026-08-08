use reap_pm_live::{PmPreparedCancelDispatch, PmPreparedPlaceDispatch};
use reap_polymarket_adapter::{
    PmFixtureAccountPositionSnapshot, PmFixturePrivateLifecycle, PmFixtureReconciliation,
    PmPublicRole,
};

fn public_cannot_dispatch(role: &mut PmPublicRole, dispatch: PmPreparedPlaceDispatch) {
    role.dispatch_place(dispatch);
}

fn private_cannot_dispatch(
    role: &mut PmFixturePrivateLifecycle,
    dispatch: PmPreparedCancelDispatch,
) {
    role.dispatch_cancel(dispatch);
}

fn reconciliation_cannot_dispatch(
    role: &mut PmFixtureReconciliation,
    dispatch: PmPreparedPlaceDispatch,
) {
    role.dispatch_place(dispatch);
}

fn account_cannot_dispatch(
    role: &mut PmFixtureAccountPositionSnapshot,
    dispatch: PmPreparedCancelDispatch,
) {
    role.dispatch_cancel(dispatch);
}

fn main() {}
