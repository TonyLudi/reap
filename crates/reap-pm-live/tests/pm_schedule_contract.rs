use std::path::Path;

use reap_pm_core::{
    EvmAddress, PmAccountHandle, PmAccountScope, PmChainId, PmEnvironmentId, PmFunderId,
    PmInstrumentHandle, PmMarketHandle, PmOrderSide, PmSignerId, PmTokenHandle,
};
use reap_pm_live::{
    MAX_PM_SCHEDULED_ACTIONS, PmLaneKind, PmLanePolicy, PmScheduleMetrics, PmScheduleProjection,
    PmScheduledActionKey, PmScheduledActionKind, PmScheduledActionView, SaturationAction,
};

fn instrument(market: u16, token: u16) -> PmInstrumentHandle {
    PmInstrumentHandle::new(
        PmMarketHandle::from_ordinal(market),
        PmTokenHandle::from_ordinal(token),
    )
}

fn scope(account: u16) -> PmAccountScope {
    PmAccountScope::new(
        PmEnvironmentId::new("schedule-contract").unwrap(),
        PmChainId::new(137).unwrap(),
        PmSignerId::new(EvmAddress::from_bytes([1; 20]).unwrap()),
        PmFunderId::new(EvmAddress::from_bytes([2; 20]).unwrap()),
        PmAccountHandle::from_ordinal(account),
    )
}

#[test]
fn public_schedule_surface_is_read_only_identity_and_projection() {
    fn assert_copy_eq<T: Copy + Eq>() {}

    assert_copy_eq::<PmScheduledActionKey>();
    assert_copy_eq::<PmScheduledActionView>();
    assert_copy_eq::<PmScheduleMetrics>();
    assert_copy_eq::<PmScheduleProjection>();

    let instrument = instrument(8, 13);
    let cancel = PmScheduledActionKey::new(
        scope(2),
        instrument,
        PmOrderSide::Sell,
        PmScheduledActionKind::CancelOwnedQuote,
    );
    let replace = PmScheduledActionKey::new(
        scope(1),
        instrument,
        PmOrderSide::Buy,
        PmScheduledActionKind::QuoteEvaluation,
    );

    assert_eq!(cancel.account(), PmAccountHandle::from_ordinal(2));
    assert_eq!(cancel.account_scope(), scope(2));
    assert_eq!(cancel.instrument(), instrument);
    assert_eq!(cancel.side(), PmOrderSide::Sell);
    assert_eq!(cancel.kind(), PmScheduledActionKind::CancelOwnedQuote);
    assert!(
        cancel < replace,
        "equal-time ordering must make every owned cancel precede replacement"
    );
}

#[test]
fn scheduled_capacity_and_fail_closed_policy_are_one_frozen_contract() {
    let policy = PmLanePolicy::for_lane(PmLaneKind::Scheduled);
    assert_eq!(MAX_PM_SCHEDULED_ACTIONS, 4_096);
    assert_eq!(policy.capacity(), MAX_PM_SCHEDULED_ACTIONS);
    assert_eq!(policy.nominal_high_water(), 64);
    assert_eq!(policy.maximum_age_ns(), Some(100_000_000));
    assert_eq!(
        policy.saturation_action(),
        SaturationAction::SuppressQuoteAndCancelOwned
    );
    assert_eq!(policy.service_burst(), Some(16));
}

#[test]
fn scheduled_mutation_owner_stays_private_single_and_task_free() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/schedule.rs");
    let source = std::fs::read_to_string(&path).unwrap();

    assert!(source.contains("pub(crate) struct PmQuoteScheduleRole"));
    assert!(source.contains("actions: Vec<ScheduledEntry>"));
    assert!(source.contains("Vec::with_capacity(MAX_PM_SCHEDULED_ACTIONS)"));
    assert!(!source.contains("pub struct PmQuoteScheduleRole"));
    assert!(!source.contains("pub fn schedule("));
    assert!(!source.contains("pub fn pop_due("));
    for forbidden in [
        "tokio::spawn",
        "spawn_local",
        "std::thread",
        "async fn",
        "HashMap",
        "HashSet",
        "Arc<",
        "Mutex<",
        "PreparedPmQuote",
        "PreparedPmCancel",
        "reap_polymarket_wire",
    ] {
        assert!(
            !source.contains(forbidden),
            "schedule owner contains forbidden capability token {forbidden}"
        );
    }
}
