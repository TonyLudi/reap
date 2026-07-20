use reap_pm_core::{
    EvmAddress, PmAccountHandle, PmAccountScope, PmChainId, PmConnectionId, PmEnvironmentId,
    PmFunderId, PmInstrumentHandle, PmMarketHandle, PmProductSource, PmSignerId,
    PmSnapshotCompleteness, PmSnapshotEvidence, PmSourceHandle, PmSpenderId, PmTokenHandle,
    SnapshotRevision,
};
use reap_polymarket_adapter::{
    PmAccountPositionSnapshotRole, PmCompleteFillPage, PmCompleteOpenOrdersSnapshot,
    PmFixtureAccountPositionSnapshot, PmFixtureFillWatermark, PmFixtureOwnedExecution,
    PmFixturePrivateLifecycle, PmFixtureReconciliation, PmOwnedExecutionRole,
    PmPrivateLifecycleRole, PmPublicObservationRole, PmPublicRole, PmReconciliationContractError,
    PmReconciliationRole,
};

fn instrument() -> PmInstrumentHandle {
    PmInstrumentHandle::new(
        PmMarketHandle::from_ordinal(2),
        PmTokenHandle::from_ordinal(3),
    )
}

fn account_scope() -> PmAccountScope {
    let eoa = EvmAddress::from_bytes([7; 20]).unwrap();
    PmAccountScope::new(
        PmEnvironmentId::new("fixture").unwrap(),
        PmChainId::new(137).unwrap(),
        PmSignerId::new(eoa),
        PmFunderId::new(eoa),
        PmAccountHandle::from_ordinal(7),
    )
}

fn connection() -> PmConnectionId {
    PmConnectionId::new("fixture-connection").unwrap()
}

fn account_source(scope: PmAccountScope) -> PmProductSource {
    PmProductSource::polymarket_account(PmSourceHandle::from_ordinal(4), scope.handle())
}

fn market_source() -> PmProductSource {
    PmProductSource::polymarket_market(PmSourceHandle::from_ordinal(3), instrument().token())
}

#[test]
fn five_role_types_retain_exact_scope_source_and_connection() {
    let scope = account_scope();
    let source = account_source(scope);
    let public = PmPublicRole::new(instrument(), market_source(), connection()).unwrap();
    let private = PmFixturePrivateLifecycle::new(scope, source, connection()).unwrap();
    let reconciliation = PmFixtureReconciliation::new(scope, source, connection()).unwrap();
    let snapshots = PmFixtureAccountPositionSnapshot::new(
        scope,
        instrument(),
        source,
        connection(),
        Vec::<PmSpenderId>::new(),
    )
    .expect("empty spender set is structurally valid at the role seam");
    let execution = PmFixtureOwnedExecution::new(scope, instrument());

    assert_eq!(public.instrument(), instrument());
    assert_eq!(public.source(), market_source());
    assert_eq!(private.account_scope(), scope);
    assert_eq!(private.source(), source);
    assert_eq!(reconciliation.account_scope(), scope);
    assert_eq!(reconciliation.connection(), connection());
    assert_eq!(snapshots.account_scope(), scope);
    assert_eq!(snapshots.instrument(), instrument());
    assert_eq!(snapshots.required_spenders(), &[]);
    assert_eq!(execution.account_scope(), scope);
    assert_eq!(execution.instrument(), instrument());
}

#[test]
fn role_traits_are_static_and_expose_only_their_exact_scope() {
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
    fn execution<R: PmOwnedExecutionRole>(role: &R) -> (PmAccountScope, PmInstrumentHandle) {
        (role.account_scope(), role.instrument())
    }

    let scope = account_scope();
    let source = account_source(scope);
    assert_eq!(
        public(&PmPublicRole::new(instrument(), market_source(), connection()).unwrap()),
        instrument()
    );
    assert_eq!(
        private(&PmFixturePrivateLifecycle::new(scope, source, connection()).unwrap()),
        scope.handle()
    );
    assert_eq!(
        reconciliation(&PmFixtureReconciliation::new(scope, source, connection()).unwrap()),
        scope.handle()
    );
    assert_eq!(
        snapshots(
            &PmFixtureAccountPositionSnapshot::new(
                scope,
                instrument(),
                source,
                connection(),
                Vec::new(),
            )
            .unwrap()
        ),
        scope.handle()
    );
    assert_eq!(
        execution(&PmFixtureOwnedExecution::new(scope, instrument())),
        (scope, instrument())
    );
}

#[test]
fn reconciliation_outputs_are_atomic_complete_and_watermark_opaque() {
    let scope = account_scope();
    let source = account_source(scope);
    let complete =
        PmSnapshotEvidence::new(SnapshotRevision::new(1), PmSnapshotCompleteness::Complete)
            .unwrap();
    let incomplete =
        PmSnapshotEvidence::new(SnapshotRevision::new(2), PmSnapshotCompleteness::Incomplete)
            .unwrap();

    let open = PmCompleteOpenOrdersSnapshot::new(source, scope, complete, Vec::new()).unwrap();
    assert_eq!(open.account_scope(), scope);
    assert!(open.orders().is_empty());
    assert_eq!(
        PmCompleteOpenOrdersSnapshot::new(source, scope, incomplete, Vec::new()),
        Err(PmReconciliationContractError::IncompleteSnapshot)
    );

    let requested = PmFixtureFillWatermark::new(scope, [1; 32]);
    let next = PmFixtureFillWatermark::new(scope, [9; 32]);
    let page = PmCompleteFillPage::new(
        source,
        scope,
        complete,
        Some(requested),
        Some(next),
        Vec::new(),
    )
    .unwrap();
    assert_eq!(page.requested_after(), Some(requested));
    assert_eq!(page.next_after(), Some(next));
    assert!(page.fills().is_empty());

    let different_scope_with_same_handle = PmAccountScope::new(
        scope.environment(),
        scope.chain(),
        scope.signer(),
        PmFunderId::new(EvmAddress::from_bytes([8; 20]).unwrap()),
        scope.handle(),
    );
    let wrong_scope_watermark =
        PmFixtureFillWatermark::new(different_scope_with_same_handle, [1; 32]);
    assert_eq!(
        PmCompleteFillPage::new(
            source,
            scope,
            complete,
            Some(wrong_scope_watermark),
            None,
            Vec::new(),
        ),
        Err(PmReconciliationContractError::WatermarkAccountMismatch)
    );
}
