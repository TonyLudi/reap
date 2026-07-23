use reap_pm_core::{
    EvmAddress, IngressSequence, MAX_PM_ACCOUNT_DIAGNOSTIC_EXTRA_ROWS,
    MAX_PM_ACCOUNT_EXPECTED_ASSETS, MAX_PM_ACCOUNT_EXPECTED_INSTRUMENTS,
    MAX_PM_ACCOUNT_EXPECTED_SPENDERS, MAX_PM_ACCOUNT_SNAPSHOT_ROWS, MAX_PM_RECONCILIATION_FILLS,
    MAX_PM_RECONCILIATION_ORDERS, PmAccountHandle, PmAccountScope, PmAggregateError,
    PmAllowanceEvent, PmAllowanceValue, PmAssetId, PmBalanceEvent, PmChainId, PmClientOrderId,
    PmClientOrderKey, PmCompleteAccountSnapshot, PmCompleteFillQuery, PmCompleteOpenOrdersSnapshot,
    PmEnvironmentId, PmExactOrderDetail, PmFillEvent, PmFillExecution, PmFillFee, PmFillId,
    PmFillKey, PmFillQueryCursor, PmFillRole, PmFillSettlementStatus, PmFunderId,
    PmInstrumentHandle, PmMarketHandle, PmOrderEvent, PmOrderIdentity, PmOrderProgress,
    PmOrderSide, PmOrderStatus, PmPositionAvailability, PmPositionEvent, PmPrice, PmProductSource,
    PmQuantity, PmReconciliationRequestBoundary, PmSignerId, PmSnapshotEvidence, PmSourceBound,
    PmSourceHandle, PmSpenderDomain, PmSpenderId, PmSpenderRequirement, PmTokenHandle,
    PmVenueOrderId, PmVenueOrderKey, SnapshotRevision, U256,
};

fn address(byte: u8) -> EvmAddress {
    EvmAddress::from_bytes([byte; 20]).unwrap()
}

fn account() -> PmAccountHandle {
    PmAccountHandle::from_ordinal(7)
}

fn other_account() -> PmAccountHandle {
    PmAccountHandle::from_ordinal(8)
}

fn scope() -> PmAccountScope {
    PmAccountScope::new(
        PmEnvironmentId::new("fixture").unwrap(),
        PmChainId::new(137).unwrap(),
        PmSignerId::new(address(1)),
        PmFunderId::new(address(2)),
        account(),
    )
}

fn same_handle_other_funder_scope() -> PmAccountScope {
    PmAccountScope::new(
        scope().environment(),
        scope().chain(),
        scope().signer(),
        PmFunderId::new(address(3)),
        account(),
    )
}

fn source() -> PmProductSource {
    source_for(account(), 4)
}

fn source_for(account: PmAccountHandle, ordinal: u16) -> PmProductSource {
    PmProductSource::polymarket_account(PmSourceHandle::from_ordinal(ordinal), account)
}

fn snapshot(revision: u64) -> PmSnapshotEvidence {
    PmSnapshotEvidence::new(SnapshotRevision::new(revision)).unwrap()
}

fn boundary() -> PmReconciliationRequestBoundary {
    PmReconciliationRequestBoundary::new(IngressSequence::new(10), IngressSequence::new(11))
        .unwrap()
}

fn instrument(ordinal: u16) -> PmInstrumentHandle {
    PmInstrumentHandle::new(
        PmMarketHandle::from_ordinal(20),
        PmTokenHandle::from_ordinal(ordinal),
    )
}

fn asset(byte: u8) -> PmAssetId {
    PmAssetId::collateral(address(byte))
}

fn spender_for(
    account: PmAccountHandle,
    chain: u64,
    spender_byte: u8,
    asset: PmAssetId,
) -> PmSpenderId {
    PmSpenderId::new(
        account,
        PmSpenderRequirement::new(
            PmChainId::new(chain).unwrap(),
            address(spender_byte),
            PmSpenderDomain::Standard,
            asset,
        ),
    )
}

fn venue_order(account: PmAccountHandle, id: &str) -> PmVenueOrderKey {
    PmVenueOrderKey::new(account, PmVenueOrderId::new(id).unwrap())
}

fn order_identity(account: PmAccountHandle, client_byte: u8, venue_id: &str) -> PmOrderIdentity {
    PmOrderIdentity::new(
        Some(PmClientOrderKey::new(
            account,
            PmClientOrderId::from_bytes([client_byte; 16]).unwrap(),
        )),
        Some(venue_order(account, venue_id)),
    )
    .unwrap()
}

fn order_for(
    event_source: PmProductSource,
    account: PmAccountHandle,
    client_byte: u8,
    venue_id: &str,
) -> PmOrderEvent {
    PmOrderEvent::new(
        event_source,
        instrument(30),
        order_identity(account, client_byte, venue_id),
        PmOrderSide::Buy,
        PmPrice::parse_decimal("0.40").unwrap(),
        PmOrderProgress::new(
            PmQuantity::parse_decimal("1").unwrap(),
            U256::ZERO,
            PmOrderStatus::Open,
        )
        .unwrap(),
    )
    .unwrap()
}

fn client_only_order(event_source: PmProductSource) -> PmOrderEvent {
    let identity = PmOrderIdentity::new(
        Some(PmClientOrderKey::new(
            account(),
            PmClientOrderId::from_bytes([99; 16]).unwrap(),
        )),
        None,
    )
    .unwrap();
    PmOrderEvent::new(
        event_source,
        instrument(30),
        identity,
        PmOrderSide::Buy,
        PmPrice::parse_decimal("0.40").unwrap(),
        PmOrderProgress::new(
            PmQuantity::parse_decimal("1").unwrap(),
            U256::ZERO,
            PmOrderStatus::Open,
        )
        .unwrap(),
    )
    .unwrap()
}

fn fill_for(
    event_source: PmProductSource,
    account: PmAccountHandle,
    venue_id: &str,
    fill_id: &str,
) -> PmFillEvent {
    let venue_order = venue_order(account, venue_id);
    let identity = PmOrderIdentity::new(None, Some(venue_order)).unwrap();
    PmFillEvent::new(
        event_source,
        instrument(30),
        PmFillKey::new(venue_order, PmFillId::new(fill_id).unwrap()),
        identity,
        PmFillExecution::new(
            PmOrderSide::Buy,
            PmFillRole::Maker,
            PmFillSettlementStatus::Confirmed,
            PmPrice::parse_decimal("0.40").unwrap(),
            PmQuantity::parse_decimal("0.25").unwrap(),
            PmFillFee::Unknown,
        ),
    )
    .unwrap()
}

fn balance_for(
    event_source: PmProductSource,
    event_account: PmAccountHandle,
    asset: PmAssetId,
    revision: u64,
) -> PmBalanceEvent {
    PmBalanceEvent::new(
        event_source,
        event_account,
        asset,
        U256::from_u64(50),
        snapshot(revision),
    )
    .unwrap()
}

fn allowance_for(
    event_source: PmProductSource,
    spender: PmSpenderId,
    revision: u64,
) -> PmAllowanceEvent {
    PmAllowanceEvent::new(
        event_source,
        spender,
        PmAllowanceValue::Erc20(U256::from_u64(60)),
        snapshot(revision),
    )
    .unwrap()
}

fn position_for(
    event_source: PmProductSource,
    event_account: PmAccountHandle,
    instrument: PmInstrumentHandle,
    revision: u64,
) -> PmPositionEvent {
    PmPositionEvent::new(
        event_source,
        event_account,
        instrument,
        U256::from_u64(70),
        PmPositionAvailability::Tradable,
        snapshot(revision),
    )
    .unwrap()
}

#[allow(clippy::too_many_arguments)]
fn account_snapshot(
    expected_assets: Vec<PmAssetId>,
    expected_spenders: Vec<PmSpenderId>,
    expected_instruments: Vec<PmInstrumentHandle>,
    balances: Vec<PmBalanceEvent>,
    allowances: Vec<PmAllowanceEvent>,
    positions: Vec<PmPositionEvent>,
) -> Result<PmCompleteAccountSnapshot, PmAggregateError> {
    PmCompleteAccountSnapshot::new(
        source(),
        scope(),
        snapshot(12),
        boundary(),
        expected_assets.into_boxed_slice(),
        expected_spenders.into_boxed_slice(),
        expected_instruments.into_boxed_slice(),
        balances.into_boxed_slice(),
        allowances.into_boxed_slice(),
        positions.into_boxed_slice(),
    )
}

#[test]
fn request_boundary_is_nonzero_and_strictly_causal() {
    assert_eq!(
        PmReconciliationRequestBoundary::new(IngressSequence::new(0), IngressSequence::new(2)),
        Err(PmAggregateError::ZeroRequestSequence)
    );
    assert_eq!(
        PmReconciliationRequestBoundary::new(IngressSequence::new(1), IngressSequence::new(0)),
        Err(PmAggregateError::ZeroCompletionSequence)
    );
    assert_eq!(
        PmReconciliationRequestBoundary::new(IngressSequence::new(2), IngressSequence::new(2)),
        Err(PmAggregateError::CompletionDoesNotFollowRequest)
    );
    assert_eq!(
        PmReconciliationRequestBoundary::new(IngressSequence::new(3), IngressSequence::new(2)),
        Err(PmAggregateError::CompletionDoesNotFollowRequest)
    );
    assert_eq!(boundary().request_sequence().value(), 10);
    assert_eq!(boundary().completion_sequence().value(), 11);
}

#[test]
fn complete_open_orders_validate_source_scope_rows_duplicates_and_bound() {
    let first = order_for(source(), account(), 1, "venue-1");
    let second = order_for(source(), account(), 2, "venue-2");
    let complete = PmCompleteOpenOrdersSnapshot::new(
        source(),
        scope(),
        snapshot(12),
        boundary(),
        vec![first, second].into_boxed_slice(),
    )
    .unwrap();
    assert_eq!(complete.source(), source());
    assert_eq!(complete.account_scope(), scope());
    assert_eq!(complete.snapshot(), snapshot(12));
    assert_eq!(complete.boundary(), boundary());
    assert_eq!(complete.orders(), &[first, second]);
    assert_eq!(complete.clone().into_orders().as_ref(), &[first, second]);

    let wrong_kind =
        PmProductSource::polymarket_market(PmSourceHandle::from_ordinal(4), instrument(30).token());
    assert_eq!(
        PmCompleteOpenOrdersSnapshot::new(
            wrong_kind,
            scope(),
            snapshot(12),
            boundary(),
            Box::default(),
        ),
        Err(PmAggregateError::WrongSource)
    );
    assert_eq!(
        PmCompleteOpenOrdersSnapshot::new(
            source_for(other_account(), 4),
            scope(),
            snapshot(12),
            boundary(),
            Box::default(),
        ),
        Err(PmAggregateError::SourceAccountMismatch)
    );

    let wrong_account_row = order_for(
        source_for(other_account(), 4),
        other_account(),
        3,
        "venue-3",
    );
    assert_eq!(
        PmCompleteOpenOrdersSnapshot::new(
            source(),
            scope(),
            snapshot(12),
            boundary(),
            vec![wrong_account_row].into_boxed_slice(),
        ),
        Err(PmAggregateError::RowAccountMismatch)
    );
    let wrong_source_row = order_for(source_for(account(), 99), account(), 3, "venue-3");
    assert_eq!(
        PmCompleteOpenOrdersSnapshot::new(
            source(),
            scope(),
            snapshot(12),
            boundary(),
            vec![wrong_source_row].into_boxed_slice(),
        ),
        Err(PmAggregateError::RowSourceMismatch)
    );
    assert_eq!(
        PmCompleteOpenOrdersSnapshot::new(
            source(),
            scope(),
            snapshot(12),
            boundary(),
            vec![client_only_order(source())].into_boxed_slice(),
        ),
        Err(PmAggregateError::MissingOpenOrderVenueKey)
    );

    let duplicate_client = order_for(source(), account(), 1, "venue-2");
    assert_eq!(
        PmCompleteOpenOrdersSnapshot::new(
            source(),
            scope(),
            snapshot(12),
            boundary(),
            vec![first, duplicate_client].into_boxed_slice(),
        ),
        Err(PmAggregateError::DuplicateOrderKey)
    );
    let duplicate_venue = order_for(source(), account(), 2, "venue-1");
    assert_eq!(
        PmCompleteOpenOrdersSnapshot::new(
            source(),
            scope(),
            snapshot(12),
            boundary(),
            vec![first, duplicate_venue].into_boxed_slice(),
        ),
        Err(PmAggregateError::DuplicateOrderKey)
    );
    assert_eq!(
        PmCompleteOpenOrdersSnapshot::new(
            source(),
            scope(),
            snapshot(12),
            boundary(),
            vec![first; MAX_PM_RECONCILIATION_ORDERS + 1].into_boxed_slice(),
        ),
        Err(PmAggregateError::TooManyOrders)
    );
}

#[test]
fn exact_order_detail_represents_present_and_explicit_absent_results() {
    let requested = venue_order(account(), "venue-1");
    let row = order_for(source(), account(), 1, "venue-1");
    let present = PmExactOrderDetail::new(
        source(),
        scope(),
        snapshot(12),
        boundary(),
        requested,
        Some(row),
    )
    .unwrap();
    assert_eq!(present.source(), source());
    assert_eq!(present.account_scope(), scope());
    assert_eq!(present.snapshot(), snapshot(12));
    assert_eq!(present.boundary(), boundary());
    assert_eq!(present.requested_order(), requested);
    assert_eq!(present.order(), Some(row));

    let absent =
        PmExactOrderDetail::new(source(), scope(), snapshot(12), boundary(), requested, None)
            .unwrap();
    assert_eq!(absent.order(), None);
    assert_eq!(
        PmExactOrderDetail::new(
            source(),
            scope(),
            snapshot(12),
            boundary(),
            venue_order(other_account(), "venue-1"),
            None,
        ),
        Err(PmAggregateError::RequestedOrderAccountMismatch)
    );
    assert_eq!(
        PmExactOrderDetail::new(
            source(),
            scope(),
            snapshot(12),
            boundary(),
            requested,
            Some(client_only_order(source())),
        ),
        Err(PmAggregateError::MissingOrderDetailVenueKey)
    );
    assert_eq!(
        PmExactOrderDetail::new(
            source(),
            scope(),
            snapshot(12),
            boundary(),
            requested,
            Some(order_for(source(), account(), 2, "venue-2")),
        ),
        Err(PmAggregateError::OrderDetailVenueMismatch)
    );
}

#[test]
fn complete_fill_query_is_terminal_scoped_and_deduplicates_exact_legs() {
    let first = fill_for(source(), account(), "maker-leg-1", "shared-trade");
    let second = fill_for(source(), account(), "maker-leg-2", "shared-trade");
    assert_ne!(first.fill_key(), second.fill_key());

    let requested = PmFillQueryCursor::new(scope(), [u8::MAX; 32]);
    let resulting = PmFillQueryCursor::new(scope(), [0; 32]);
    let complete = PmCompleteFillQuery::new(
        source(),
        scope(),
        snapshot(12),
        boundary(),
        Some(requested),
        resulting,
        vec![first, second].into_boxed_slice(),
    )
    .unwrap();
    assert_eq!(complete.source(), source());
    assert_eq!(complete.account_scope(), scope());
    assert_eq!(complete.snapshot(), snapshot(12));
    assert_eq!(complete.boundary(), boundary());
    assert_eq!(complete.requested_after(), Some(requested));
    assert_eq!(complete.resulting_watermark(), resulting);
    assert_eq!(complete.fills(), &[first, second]);
    assert_eq!(complete.clone().into_fills().as_ref(), &[first, second]);
    assert_eq!(requested.account_scope(), scope());
    assert_eq!(requested.opaque(), [u8::MAX; 32]);

    assert_eq!(
        PmCompleteFillQuery::new(
            source(),
            scope(),
            snapshot(12),
            boundary(),
            Some(PmFillQueryCursor::new(
                same_handle_other_funder_scope(),
                [1; 32],
            )),
            resulting,
            Box::default(),
        ),
        Err(PmAggregateError::CursorAccountScopeMismatch)
    );
    assert_eq!(
        PmCompleteFillQuery::new(
            source(),
            scope(),
            snapshot(12),
            boundary(),
            None,
            PmFillQueryCursor::new(same_handle_other_funder_scope(), [2; 32]),
            Box::default(),
        ),
        Err(PmAggregateError::CursorAccountScopeMismatch)
    );
    assert_eq!(
        PmCompleteFillQuery::new(
            source(),
            scope(),
            snapshot(12),
            boundary(),
            None,
            resulting,
            vec![first, first].into_boxed_slice(),
        ),
        Err(PmAggregateError::DuplicateFillKey)
    );
    assert_eq!(
        PmCompleteFillQuery::new(
            source(),
            scope(),
            snapshot(12),
            boundary(),
            None,
            resulting,
            vec![first; MAX_PM_RECONCILIATION_FILLS + 1].into_boxed_slice(),
        ),
        Err(PmAggregateError::TooManyFills)
    );
}

#[test]
fn complete_fill_query_rejects_rows_outside_its_account_source() {
    let resulting = PmFillQueryCursor::new(scope(), [3; 32]);
    let wrong_account_fill = fill_for(
        source_for(other_account(), 4),
        other_account(),
        "other-account-order",
        "fill",
    );
    assert_eq!(
        PmCompleteFillQuery::new(
            source(),
            scope(),
            snapshot(12),
            boundary(),
            None,
            resulting,
            vec![wrong_account_fill].into_boxed_slice(),
        ),
        Err(PmAggregateError::RowAccountMismatch)
    );

    let wrong_source_fill = fill_for(
        source_for(account(), 99),
        account(),
        "same-account-order",
        "fill",
    );
    assert_eq!(
        PmCompleteFillQuery::new(
            source(),
            scope(),
            snapshot(12),
            boundary(),
            None,
            resulting,
            vec![wrong_source_fill].into_boxed_slice(),
        ),
        Err(PmAggregateError::RowSourceMismatch)
    );
}

#[test]
fn account_snapshot_makes_expected_absence_explicit_and_extra_rows_non_authoritative() {
    let expected_asset = asset(10);
    let extra_asset = asset(11);
    let expected_spender = spender_for(account(), 137, 12, expected_asset);
    let extra_spender = spender_for(account(), 137, 13, extra_asset);
    let expected_instrument = instrument(40);
    let extra_instrument = instrument(41);
    let extra_balance = balance_for(source(), account(), extra_asset, 12);
    let extra_allowance = allowance_for(source(), extra_spender, 12);
    let extra_position = position_for(source(), account(), extra_instrument, 12);

    let complete = account_snapshot(
        vec![expected_asset],
        vec![expected_spender],
        vec![expected_instrument],
        vec![extra_balance],
        vec![extra_allowance],
        vec![extra_position],
    )
    .unwrap();
    assert_eq!(complete.source(), source());
    assert_eq!(complete.account_scope(), scope());
    assert_eq!(complete.snapshot(), snapshot(12));
    assert_eq!(complete.boundary(), boundary());
    assert_eq!(complete.expected_assets(), &[expected_asset]);
    assert_eq!(complete.expected_spenders(), &[expected_spender]);
    assert_eq!(complete.expected_instruments(), &[expected_instrument]);
    assert_eq!(complete.balances(), &[extra_balance]);
    assert_eq!(complete.allowances(), &[extra_allowance]);
    assert_eq!(complete.positions(), &[extra_position]);
    assert_eq!(complete.expected_balance(expected_asset), Some(None));
    assert_eq!(complete.expected_allowance(expected_spender), Some(None));
    assert_eq!(complete.expected_position(expected_instrument), Some(None));
    assert_eq!(complete.expected_balance(extra_asset), None);
    assert_eq!(complete.expected_allowance(extra_spender), None);
    assert_eq!(complete.expected_position(extra_instrument), None);

    let expected_balance = balance_for(source(), account(), expected_asset, 12);
    let expected_allowance = allowance_for(source(), expected_spender, 12);
    let expected_position = position_for(source(), account(), expected_instrument, 12);
    let populated = account_snapshot(
        vec![expected_asset],
        vec![expected_spender],
        vec![expected_instrument],
        vec![expected_balance],
        vec![expected_allowance],
        vec![expected_position],
    )
    .unwrap();
    assert_eq!(
        populated.expected_balance(expected_asset),
        Some(Some(&expected_balance))
    );
    assert_eq!(
        populated.expected_allowance(expected_spender),
        Some(Some(&expected_allowance))
    );
    assert_eq!(
        populated.expected_position(expected_instrument),
        Some(Some(&expected_position))
    );
}

#[test]
fn full_expected_scope_retains_bounded_non_authoritative_diagnostics() {
    assert_eq!(
        MAX_PM_ACCOUNT_SNAPSHOT_ROWS,
        MAX_PM_ACCOUNT_EXPECTED_ASSETS + MAX_PM_ACCOUNT_DIAGNOSTIC_EXTRA_ROWS
    );

    let expected_assets = (1..=MAX_PM_ACCOUNT_EXPECTED_ASSETS)
        .map(|ordinal| asset(ordinal as u8))
        .collect::<Vec<_>>();
    let mut balances = expected_assets
        .iter()
        .copied()
        .map(|asset| balance_for(source(), account(), asset, 12))
        .collect::<Vec<_>>();
    let diagnostic_asset = asset(99);
    balances.push(balance_for(source(), account(), diagnostic_asset, 12));

    let complete = account_snapshot(
        expected_assets.clone(),
        vec![],
        vec![],
        balances,
        vec![],
        vec![],
    )
    .unwrap();
    assert_eq!(complete.expected_assets(), expected_assets);
    assert_eq!(complete.expected_balance(diagnostic_asset), None);
    assert_eq!(
        complete.balances().len(),
        MAX_PM_ACCOUNT_EXPECTED_ASSETS + 1
    );
}

#[test]
fn account_snapshot_caps_diagnostics_independently_of_expected_scope_size() {
    let expected_assets = vec![asset(10), asset(11)];
    let expected_spenders = vec![
        spender_for(account(), 137, 12, expected_assets[0]),
        spender_for(account(), 137, 13, expected_assets[1]),
    ];
    let expected_instruments = vec![instrument(40)];

    let mut balances = expected_assets
        .iter()
        .copied()
        .map(|asset| balance_for(source(), account(), asset, 12))
        .collect::<Vec<_>>();
    balances.extend(
        (0..=MAX_PM_ACCOUNT_DIAGNOSTIC_EXTRA_ROWS)
            .map(|index| balance_for(source(), account(), asset(30 + index as u8), 12)),
    );
    assert_eq!(
        account_snapshot(
            expected_assets.clone(),
            expected_spenders.clone(),
            expected_instruments.clone(),
            balances,
            vec![],
            vec![],
        ),
        Err(PmAggregateError::TooManyBalanceRows)
    );

    let mut allowances = expected_spenders
        .iter()
        .copied()
        .map(|spender| allowance_for(source(), spender, 12))
        .collect::<Vec<_>>();
    allowances.extend((0..=MAX_PM_ACCOUNT_DIAGNOSTIC_EXTRA_ROWS).map(|index| {
        allowance_for(
            source(),
            spender_for(account(), 137, 50 + index as u8, asset(70 + index as u8)),
            12,
        )
    }));
    assert_eq!(
        account_snapshot(
            expected_assets.clone(),
            expected_spenders.clone(),
            expected_instruments.clone(),
            vec![],
            allowances,
            vec![],
        ),
        Err(PmAggregateError::TooManyAllowanceRows)
    );

    let mut positions = expected_instruments
        .iter()
        .copied()
        .map(|instrument| position_for(source(), account(), instrument, 12))
        .collect::<Vec<_>>();
    positions.extend(
        (0..=MAX_PM_ACCOUNT_DIAGNOSTIC_EXTRA_ROWS)
            .map(|index| position_for(source(), account(), instrument(60 + index as u16), 12)),
    );
    assert_eq!(
        account_snapshot(
            expected_assets,
            expected_spenders,
            expected_instruments,
            vec![],
            vec![],
            positions,
        ),
        Err(PmAggregateError::TooManyPositionRows)
    );
}

#[test]
fn account_snapshot_validates_exact_expected_scopes() {
    let expected_asset = asset(10);
    let expected_spender = spender_for(account(), 137, 12, expected_asset);
    let expected_instrument = instrument(40);

    assert_eq!(
        account_snapshot(
            vec![expected_asset, expected_asset],
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
        ),
        Err(PmAggregateError::DuplicateExpectedAsset)
    );
    assert_eq!(
        account_snapshot(
            vec![expected_asset],
            vec![expected_spender, expected_spender],
            vec![],
            vec![],
            vec![],
            vec![],
        ),
        Err(PmAggregateError::DuplicateExpectedSpender)
    );
    assert_eq!(
        account_snapshot(
            vec![],
            vec![],
            vec![expected_instrument, expected_instrument],
            vec![],
            vec![],
            vec![],
        ),
        Err(PmAggregateError::DuplicateExpectedInstrument)
    );
    assert_eq!(
        account_snapshot(
            vec![expected_asset],
            vec![spender_for(other_account(), 137, 12, expected_asset)],
            vec![],
            vec![],
            vec![],
            vec![],
        ),
        Err(PmAggregateError::ExpectedSpenderAccountMismatch)
    );
    assert_eq!(
        account_snapshot(
            vec![expected_asset],
            vec![spender_for(account(), 1, 12, expected_asset)],
            vec![],
            vec![],
            vec![],
            vec![],
        ),
        Err(PmAggregateError::ExpectedSpenderChainMismatch)
    );
    assert_eq!(
        account_snapshot(
            vec![expected_asset],
            vec![spender_for(account(), 137, 12, asset(11))],
            vec![],
            vec![],
            vec![],
            vec![],
        ),
        Err(PmAggregateError::ExpectedSpenderAssetNotExpected)
    );

    assert_eq!(
        account_snapshot(
            vec![expected_asset; MAX_PM_ACCOUNT_EXPECTED_ASSETS + 1],
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
        ),
        Err(PmAggregateError::TooManyExpectedAssets)
    );
    assert_eq!(
        account_snapshot(
            vec![expected_asset],
            vec![expected_spender; MAX_PM_ACCOUNT_EXPECTED_SPENDERS + 1],
            vec![],
            vec![],
            vec![],
            vec![],
        ),
        Err(PmAggregateError::TooManyExpectedSpenders)
    );
    assert_eq!(
        account_snapshot(
            vec![],
            vec![],
            vec![expected_instrument; MAX_PM_ACCOUNT_EXPECTED_INSTRUMENTS + 1],
            vec![],
            vec![],
            vec![],
        ),
        Err(PmAggregateError::TooManyExpectedInstruments)
    );
}

#[test]
fn account_snapshot_validates_row_scope_revision_duplicates_and_bounds() {
    let first_asset = asset(10);
    let second_asset = asset(11);
    let first_spender = spender_for(account(), 137, 12, first_asset);
    let second_spender = spender_for(account(), 137, 13, second_asset);
    let first_instrument = instrument(40);
    let second_instrument = instrument(41);

    assert_eq!(
        account_snapshot(
            vec![],
            vec![],
            vec![],
            vec![balance_for(
                source_for(other_account(), 4),
                other_account(),
                first_asset,
                12,
            )],
            vec![],
            vec![],
        ),
        Err(PmAggregateError::RowAccountMismatch)
    );
    assert_eq!(
        account_snapshot(
            vec![],
            vec![],
            vec![],
            vec![balance_for(
                source_for(account(), 99),
                account(),
                first_asset,
                12,
            )],
            vec![],
            vec![],
        ),
        Err(PmAggregateError::RowSourceMismatch)
    );
    assert_eq!(
        account_snapshot(
            vec![],
            vec![],
            vec![],
            vec![balance_for(source(), account(), first_asset, 13)],
            vec![],
            vec![],
        ),
        Err(PmAggregateError::RowRevisionMismatch)
    );
    assert_eq!(
        account_snapshot(
            vec![],
            vec![],
            vec![],
            vec![],
            vec![allowance_for(source(), first_spender, 13)],
            vec![],
        ),
        Err(PmAggregateError::RowRevisionMismatch)
    );
    assert_eq!(
        account_snapshot(
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
            vec![position_for(source(), account(), first_instrument, 13,)],
        ),
        Err(PmAggregateError::RowRevisionMismatch)
    );
    assert_eq!(
        account_snapshot(
            vec![],
            vec![],
            vec![],
            vec![],
            vec![allowance_for(
                source(),
                spender_for(account(), 1, 12, first_asset),
                12,
            )],
            vec![],
        ),
        Err(PmAggregateError::AllowanceSpenderChainMismatch)
    );

    let first_balance = balance_for(source(), account(), first_asset, 12);
    assert_eq!(
        account_snapshot(
            vec![],
            vec![],
            vec![],
            vec![first_balance, first_balance],
            vec![],
            vec![],
        ),
        Err(PmAggregateError::DuplicateBalanceAsset)
    );
    let first_allowance = allowance_for(source(), first_spender, 12);
    assert_eq!(
        account_snapshot(
            vec![],
            vec![],
            vec![],
            vec![],
            vec![first_allowance, first_allowance],
            vec![],
        ),
        Err(PmAggregateError::DuplicateAllowanceSpender)
    );
    let first_position = position_for(source(), account(), first_instrument, 12);
    assert_eq!(
        account_snapshot(
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
            vec![first_position, first_position],
        ),
        Err(PmAggregateError::DuplicatePositionInstrument)
    );

    let second_balance = balance_for(source(), account(), second_asset, 12);
    assert_eq!(
        account_snapshot(
            vec![],
            vec![],
            vec![],
            vec![first_balance; MAX_PM_ACCOUNT_SNAPSHOT_ROWS + 1],
            vec![],
            vec![],
        ),
        Err(PmAggregateError::TooManyBalanceRows)
    );
    let second_allowance = allowance_for(source(), second_spender, 12);
    assert_eq!(
        account_snapshot(
            vec![],
            vec![],
            vec![],
            vec![],
            vec![second_allowance; MAX_PM_ACCOUNT_SNAPSHOT_ROWS + 1],
            vec![],
        ),
        Err(PmAggregateError::TooManyAllowanceRows)
    );
    let second_position = position_for(source(), account(), second_instrument, 12);
    assert_eq!(
        account_snapshot(
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
            vec![second_position; MAX_PM_ACCOUNT_SNAPSHOT_ROWS + 1],
        ),
        Err(PmAggregateError::TooManyPositionRows)
    );
    assert_ne!(first_balance.asset(), second_balance.asset());
    assert_ne!(first_allowance.spender(), second_allowance.spender());
    assert_ne!(first_position.instrument(), second_position.instrument());
}
