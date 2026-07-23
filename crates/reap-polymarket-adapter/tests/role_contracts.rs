mod support;

use reap_pm_core::{
    ConnectionEpoch, IngressSequence, OkxInstrumentId, OkxReferenceInstrument, PmAccountHandle,
    PmInstrumentHandle, PmProductSource, PmPublicObservationGrant, PmQuantity, PmSourceHandle,
    PmTick,
};
use reap_polymarket_adapter::{
    PmAccountPositionSnapshotRole, PmFixtureOwnedExecution, PmPrivateLifecycleRole,
    PmPublicObservationRole, PmPublicRole, PmReconciliationRole,
};
use reap_polymarket_wire::{PmBookParserConfig, PmWireScope};

use support::{
    CONDITION, account_scope, account_source, account_with, completion, connection, grants,
    instrument, instrument_scope, private_with, reconciliation_role, reconciliation_with, snapshot,
};

fn market_source() -> PmProductSource {
    PmProductSource::polymarket_market(
        PmSourceHandle::from_ordinal(3),
        observation_grant().instrument().token(),
    )
}

fn parser_config() -> PmBookParserConfig {
    let metadata = instrument_scope().metadata();
    PmBookParserConfig::new(
        PmWireScope::new(
            metadata.condition(),
            metadata.market(),
            metadata.outcome().token(),
        ),
        PmTick::parse_decimal("0.01").unwrap(),
        PmQuantity::parse_decimal("5").unwrap(),
        false,
    )
}

fn observation_grant() -> PmPublicObservationGrant {
    PmPublicObservationGrant::derive_goal_f(
        OkxReferenceInstrument::index(OkxInstrumentId::new("BTC-USDT").unwrap()),
        instrument_scope().id(),
    )
}

#[test]
fn read_grant_builds_exact_three_roles_and_scope_is_metadata_derived() {
    let (private_grant, reconciliation_grant, account_grant) = grants();
    let private = private_with(private_grant);
    let reconciliation = reconciliation_with(reconciliation_grant);
    let account = account_with(account_grant);

    assert_eq!(private.account_scope(), account_scope());
    assert_eq!(private.instrument_scope(), instrument_scope());
    assert_eq!(private.source(), account_source());
    assert_eq!(reconciliation.account_scope(), account_scope());
    assert_eq!(reconciliation.connection(), connection());
    assert_eq!(account.instrument_scope(), instrument_scope());
    assert_eq!(
        account.trading_domain(),
        instrument_scope().trading_domain()
    );
    assert_eq!(account.required_spenders().len(), 2);
    assert_eq!(
        instrument_scope().tick(),
        PmTick::parse_decimal("0.01").unwrap()
    );
    assert_eq!(
        instrument_scope().minimum_order_size(),
        PmQuantity::parse_decimal("5").unwrap()
    );
}

#[test]
fn public_and_owned_execution_roles_remain_disjoint_from_read_grants() {
    fn public<R: PmPublicObservationRole>(role: &R) -> PmInstrumentHandle {
        role.instrument()
    }
    fn private<R: PmPrivateLifecycleRole>(role: &R) -> PmAccountHandle {
        role.account()
    }
    fn reconciliation<R: PmReconciliationRole>(role: &R) -> PmAccountHandle {
        role.account()
    }
    fn snapshots<R: PmAccountPositionSnapshotRole>(role: &R) -> PmAccountHandle {
        role.account()
    }

    let (private_grant, reconciliation_grant, account_grant) = grants();
    let private_role = private_with(private_grant);
    let reconciliation_role = reconciliation_with(reconciliation_grant);
    let account_role = account_with(account_grant);
    let public_instrument = observation_grant().instrument();
    let public_role = PmPublicRole::new(
        observation_grant(),
        public_instrument,
        parser_config(),
        market_source(),
        connection(),
    )
    .unwrap();
    let execution = PmFixtureOwnedExecution::new(account_scope(), instrument());

    assert_eq!(public(&public_role), public_instrument);
    assert_eq!(private(&private_role), account_scope().handle());
    assert_eq!(
        reconciliation(&reconciliation_role),
        account_scope().handle()
    );
    assert_eq!(snapshots(&account_role), account_scope().handle());
    assert_eq!(execution.account_scope(), account_scope());
    assert_eq!(execution.instrument(), instrument());
    assert_eq!(
        parser_config().scope().condition(),
        reap_pm_core::PmConditionId::parse(CONDITION).unwrap()
    );
}

#[test]
fn sibling_equal_config_role_cannot_open_another_owner_delivery() {
    let mut producer = reconciliation_role();
    let sibling = reconciliation_role();
    let delivery = producer
        .request_open_orders(ConnectionEpoch::new(1), IngressSequence::new(10))
        .unwrap()
        .complete(completion(1, 11, Some(1)), snapshot(1), &[])
        .unwrap()
        .service_at(30_000)
        .unwrap();

    let delivery = sibling
        .reduce_open_orders_delivery(delivery, |_, _| ())
        .expect_err("equal configuration is not owner authority");
    let boundary = producer
        .reduce_open_orders_delivery(*delivery, |scope, envelope| {
            assert_eq!(scope.account_scope(), account_scope());
            assert_eq!(scope.instrument_scope(), instrument_scope());
            envelope.payload().boundary()
        })
        .unwrap();
    assert_eq!(boundary.request_sequence(), IngressSequence::new(10));
    assert_eq!(boundary.completion_sequence(), IngressSequence::new(11));
}

#[test]
fn request_order_is_lexicographic_across_reconnect_epochs_and_query_kinds() {
    let mut role = reconciliation_role();
    role.request_open_orders(ConnectionEpoch::new(1), IngressSequence::new(100))
        .unwrap();
    assert!(
        role.request_fills(ConnectionEpoch::new(1), IngressSequence::new(100), None)
            .is_err()
    );
    role.request_fills(ConnectionEpoch::new(2), IngressSequence::new(1), None)
        .expect("a later epoch resets its ingress sequence");
    assert!(
        role.request_open_orders(ConnectionEpoch::new(1), IngressSequence::new(101))
            .is_err()
    );
}
