use super::support::{OWNER_MEMORY_BOUND_BYTES, account_ingress, internal_ingress};
use super::*;

fn assert_complete_lane_overload(
    lane_kind: PmLaneKind,
    expected_capacity: usize,
    expected_action: SaturationAction,
    product_source: bool,
) {
    let mut lane = PmCompleteLane::<u64>::new(lane_kind);
    let reserved = lane.reserved_capacity_bytes();
    assert_eq!(
        PmLanePolicy::for_lane(lane_kind).capacity(),
        expected_capacity
    );
    let source = if product_source {
        PmCompleteSourceKind::PolymarketAccount
    } else {
        PmCompleteSourceKind::InternalSignal
    };
    for attempt in 1..=expected_capacity {
        let attempt = u64::try_from(attempt).expect("bounded");
        let ingress = if product_source {
            account_ingress(attempt, attempt)
        } else {
            internal_ingress(attempt, attempt)
        };
        lane.enqueue(ingress, attempt, 0, source)
            .expect("within capacity");
    }
    let final_attempt = u64::try_from(expected_capacity + 1).expect("bounded");
    let ingress = if product_source {
        account_ingress(final_attempt, final_attempt)
    } else {
        internal_ingress(final_attempt, final_attempt)
    };
    assert!(matches!(
        lane.enqueue(ingress, final_attempt, 0, source),
        Err(PmCompleteLaneEnqueueError::Full { action, .. }) if action == expected_action
    ));
    let metrics = lane.metrics();
    assert_eq!(metrics.queue().depth(), expected_capacity);
    assert_eq!(metrics.queue().high_water(), expected_capacity);
    assert_eq!(metrics.queue().rejected_full(), 1);
    assert_eq!(lane.reserved_capacity_bytes(), reserved);
    assert!(reserved <= OWNER_MEMORY_BOUND_BYTES);
}

#[test]
fn persistence_and_complete_snapshot_queue_mechanics_are_exact() {
    assert_complete_lane_overload(
        PmLaneKind::Persistence,
        512,
        SaturationAction::GlobalStop,
        false,
    );
    assert_complete_lane_overload(
        PmLaneKind::Reconciliation,
        128,
        SaturationAction::KeepUnreadyAndRetry,
        true,
    );
}
