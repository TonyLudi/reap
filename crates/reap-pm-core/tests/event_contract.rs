use reap_core::Venue;
use reap_pm_core::{
    EvmAddress, MAX_PM_BOOK_LEVELS, PmAccountHandle, PmAllowanceEvent, PmAllowanceValue, PmAssetId,
    PmBalanceEvent, PmBookDeltaBatch, PmBookEvent, PmBookLevel, PmBookPoint, PmBookQuantity,
    PmBookSide, PmBookSnapshot, PmBookTop, PmBookTopCheck, PmBookUpdate, PmChainId,
    PmClientOrderId, PmClientOrderKey, PmConditionId, PmErc1155OperatorApproval, PmEventError,
    PmFillEvent, PmFillExecution, PmFillFee, PmFillId, PmFillKey, PmFillRole,
    PmFillSettlementStatus, PmInstrumentHandle, PmMarketEvent, PmMarketHandle, PmMarketId,
    PmMarketLifecycle, PmMarketMetadata, PmOrderEvent, PmOrderIdentity, PmOrderProgress,
    PmOrderSide, PmOrderStatus, PmOutcomeLabel, PmOutcomeMetadata, PmPositionAvailability,
    PmPositionEvent, PmPrice, PmProductSource, PmQuantity, PmSign, PmSignedUnits,
    PmSnapshotEvidence, PmSourceHandle, PmSpenderDomain, PmSpenderId, PmSpenderRequirement, PmTick,
    PmTokenHandle, PmTokenId, PmVenueChangeHash, PmVenueOrderId, PmVenueOrderKey, SnapshotRevision,
    U256,
};

const CONDITION: &str = "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const MARKET: &str = "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const COLLATERAL: &str = "0x1111111111111111111111111111111111111111";
const EXCHANGE: &str = "0x2222222222222222222222222222222222222222";
const CONDITIONAL_TOKENS: &str = "0x3333333333333333333333333333333333333333";

fn instrument() -> PmInstrumentHandle {
    PmInstrumentHandle::new(
        PmMarketHandle::from_ordinal(4),
        PmTokenHandle::from_ordinal(9),
    )
}

fn market_source() -> PmProductSource {
    PmProductSource::polymarket_market(PmSourceHandle::from_ordinal(1), instrument().token())
}

fn account() -> PmAccountHandle {
    PmAccountHandle::from_ordinal(3)
}

fn account_source() -> PmProductSource {
    PmProductSource::polymarket_account(PmSourceHandle::from_ordinal(2), account())
}

fn collateral_asset() -> PmAssetId {
    PmAssetId::collateral(EvmAddress::parse(COLLATERAL).unwrap())
}

fn outcome_asset() -> PmAssetId {
    PmAssetId::outcome(
        EvmAddress::parse(CONDITIONAL_TOKENS).unwrap(),
        PmTokenId::new(U256::from_u64(77)).unwrap(),
    )
}

fn spender(asset: PmAssetId) -> PmSpenderId {
    PmSpenderId::new(
        account(),
        PmSpenderRequirement::new(
            PmChainId::new(137).unwrap(),
            EvmAddress::parse(EXCHANGE).unwrap(),
            PmSpenderDomain::Standard,
            asset,
        ),
    )
}

fn metadata() -> PmMarketMetadata {
    let mut required_spenders = [None; 8];
    required_spenders[0] = Some(spender(collateral_asset()).requirement());
    PmMarketMetadata::new(
        PmConditionId::parse(CONDITION).unwrap(),
        PmMarketId::parse(MARKET).unwrap(),
        PmOutcomeMetadata::new(
            PmTokenId::new(U256::from_u64(77)).unwrap(),
            PmOutcomeLabel::new("YES").unwrap(),
        ),
        PmMarketLifecycle::new(true, false, false, true, true),
        PmTick::parse_decimal("0.0001").unwrap(),
        PmQuantity::parse_decimal("0.01").unwrap(),
        false,
        PmChainId::new(137).unwrap(),
        EvmAddress::parse(EXCHANGE).unwrap(),
        required_spenders,
        1,
    )
    .unwrap()
}

fn complete_snapshot() -> PmSnapshotEvidence {
    PmSnapshotEvidence::new(SnapshotRevision::new(12)).unwrap()
}

fn order_identity() -> PmOrderIdentity {
    order_identity_for(account())
}

fn order_identity_for(account: PmAccountHandle) -> PmOrderIdentity {
    PmOrderIdentity::new(
        Some(PmClientOrderKey::new(
            account,
            PmClientOrderId::parse("00112233445566778899aabbccddeeff").unwrap(),
        )),
        Some(PmVenueOrderKey::new(
            account,
            PmVenueOrderId::new("venue-order-1").unwrap(),
        )),
    )
    .unwrap()
}

#[test]
fn market_events_retain_metadata_source_and_structural_instrument() {
    let event = PmMarketEvent::new(
        market_source(),
        instrument(),
        SnapshotRevision::new(5),
        metadata(),
    )
    .unwrap();
    assert_eq!(event.source().venue(), Venue::Polymarket);
    assert_eq!(event.instrument(), instrument());
    assert_eq!(event.metadata_revision().value(), 5);
    assert_eq!(event.metadata().market(), metadata().market());
    assert_eq!(event.metadata().outcome(), metadata().outcome());

    assert_eq!(
        PmMarketEvent::new(
            account_source(),
            instrument(),
            SnapshotRevision::new(5),
            metadata(),
        ),
        Err(PmEventError::WrongMarketSource)
    );
    assert_eq!(
        PmMarketEvent::new(
            PmProductSource::polymarket_market(
                PmSourceHandle::from_ordinal(1),
                PmTokenHandle::from_ordinal(10),
            ),
            instrument(),
            SnapshotRevision::new(5),
            metadata(),
        ),
        Err(PmEventError::MarketSourceTokenMismatch)
    );
    assert_eq!(
        PmMarketEvent::new(
            market_source(),
            instrument(),
            SnapshotRevision::new(0),
            metadata(),
        ),
        Err(PmEventError::ZeroRevision)
    );
}

#[test]
fn book_events_keep_whole_snapshots_and_delta_frames_atomic() {
    let bid = PmBookLevel::new(
        PmBookSide::Bid,
        PmPrice::parse_decimal("0.42").unwrap(),
        PmBookQuantity::from_protocol_units(U256::from_u64(2_000_000)),
    );
    let snapshot = PmBookSnapshot::new(vec![bid].into_boxed_slice()).unwrap();
    let event = PmBookEvent::new(
        market_source(),
        instrument(),
        SnapshotRevision::new(6),
        PmBookUpdate::Snapshot(snapshot.clone()),
    )
    .unwrap();
    assert_eq!(event.source(), market_source());
    assert_eq!(event.instrument(), instrument());
    assert_eq!(event.metadata_revision().value(), 6);
    assert_eq!(event.update(), &PmBookUpdate::Snapshot(snapshot.clone()));
    assert_eq!(snapshot.levels(), &[bid]);
    assert_eq!(bid.side(), PmBookSide::Bid);
    assert_eq!(bid.price().units(), 420_000);

    let delete = PmBookLevel::new(
        PmBookSide::Ask,
        PmPrice::parse_decimal("0.58").unwrap(),
        PmBookQuantity::Delete,
    );
    assert_eq!(
        PmBookSnapshot::new(vec![delete].into_boxed_slice()),
        Err(PmEventError::SnapshotLevelIsDelete)
    );
    let top = PmBookTopCheck::new(
        Some(PmPrice::parse_decimal("0.42").unwrap()),
        Some(PmPrice::parse_decimal("0.58").unwrap()),
    );
    let deltas = PmBookDeltaBatch::new(vec![delete].into_boxed_slice(), top).unwrap();
    assert_eq!(deltas.changes(), &[delete]);
    assert_eq!(deltas.venue_change_hashes(), &[None]);
    assert_eq!(deltas.expected_top(), top);
    let change_hash = PmVenueChangeHash::new("tx-delete").unwrap();
    let evidenced = PmBookDeltaBatch::new_with_venue_hashes(
        vec![delete].into_boxed_slice(),
        vec![Some(change_hash)].into_boxed_slice(),
        top,
    )
    .unwrap();
    assert_eq!(evidenced.venue_change_hashes(), &[Some(change_hash)]);
    assert_eq!(
        evidenced.venue_change_hashes()[0].unwrap().as_str(),
        "tx-delete"
    );
    assert_eq!(
        PmBookDeltaBatch::new_with_venue_hashes(
            vec![delete].into_boxed_slice(),
            Box::default(),
            top,
        ),
        Err(PmEventError::ChangeHashCountMismatch)
    );
    assert_eq!(
        PmVenueChangeHash::new(""),
        Err(PmEventError::EmptyVenueChangeHash)
    );
    assert_eq!(
        PmVenueChangeHash::new(&"a".repeat(97)),
        Err(PmEventError::VenueChangeHashTooLong)
    );
    assert_eq!(
        PmVenueChangeHash::new("é"),
        Err(PmEventError::NonAsciiVenueChangeHash)
    );
    assert!(PmBookSnapshot::new(Box::default()).is_ok());
    assert!(PmBookDeltaBatch::new(Box::default(), top).is_ok());
    assert_eq!(
        PmBookSnapshot::new(vec![bid; usize::from(MAX_PM_BOOK_LEVELS) + 1].into_boxed_slice()),
        Err(PmEventError::TooManyBookLevels)
    );
    assert_eq!(
        PmBookDeltaBatch::new(
            vec![delete; usize::from(MAX_PM_BOOK_LEVELS) + 1].into_boxed_slice(),
            top,
        ),
        Err(PmEventError::TooManyBookLevels)
    );

    assert_eq!(top.bid().unwrap().units(), 420_000);
    assert_eq!(top.ask().unwrap().units(), 580_000);
    assert!(
        PmBookEvent::new(
            market_source(),
            instrument(),
            SnapshotRevision::new(6),
            PmBookUpdate::TickSizeChanged {
                old: PmTick::parse_decimal("0.01").unwrap(),
                new: PmTick::parse_decimal("0.001").unwrap(),
            },
        )
        .is_ok()
    );
}

#[test]
fn reduced_book_top_rejects_equal_or_crossed_prices_and_keeps_empty_sides() {
    let quantity = PmQuantity::parse_decimal("1").unwrap();
    let bid = PmBookPoint::new(PmPrice::parse_decimal("0.49").unwrap(), quantity);
    let ask = PmBookPoint::new(PmPrice::parse_decimal("0.51").unwrap(), quantity);
    let top = PmBookTop::new(Some(bid), Some(ask)).unwrap();
    assert_eq!(top.bid(), Some(bid));
    assert_eq!(top.ask(), Some(ask));
    assert!(PmBookTop::new(None, None).is_ok());

    let equal = PmBookPoint::new(PmPrice::parse_decimal("0.49").unwrap(), quantity);
    assert_eq!(
        PmBookTop::new(Some(bid), Some(equal)),
        Err(PmEventError::CrossedBookTop)
    );
    let crossed = PmBookPoint::new(PmPrice::parse_decimal("0.48").unwrap(), quantity);
    assert_eq!(
        PmBookTop::new(Some(bid), Some(crossed)),
        Err(PmEventError::CrossedBookTop)
    );
}

#[test]
fn order_identity_and_progress_are_checked_without_float_or_redundant_remaining_state() {
    assert_eq!(
        PmOrderIdentity::new(None, None),
        Err(PmEventError::MissingOrderIdentity)
    );
    assert!(
        PmOrderIdentity::new(
            Some(PmClientOrderKey::new(
                account(),
                PmClientOrderId::parse("00112233445566778899aabbccddeeff").unwrap(),
            )),
            None,
        )
        .is_ok()
    );
    assert!(
        PmOrderIdentity::new(
            None,
            Some(PmVenueOrderKey::new(
                account(),
                PmVenueOrderId::new("remote-only").unwrap(),
            )),
        )
        .is_ok()
    );
    let other_account = PmAccountHandle::from_ordinal(4);
    assert_eq!(
        PmOrderIdentity::new(
            Some(PmClientOrderKey::new(
                account(),
                PmClientOrderId::parse("00112233445566778899aabbccddeeff").unwrap(),
            )),
            Some(PmVenueOrderKey::new(
                other_account,
                PmVenueOrderId::new("venue-order-1").unwrap(),
            )),
        ),
        Err(PmEventError::OrderIdentityAccountMismatch)
    );
    assert_ne!(order_identity(), order_identity_for(other_account));

    let original = PmQuantity::parse_decimal("2").unwrap();
    let partial = PmOrderProgress::new(
        original,
        U256::from_u64(500_000),
        PmOrderStatus::PartiallyFilled,
    )
    .unwrap();
    assert_eq!(
        partial.remaining_quantity_units(),
        U256::from_u64(1_500_000)
    );
    assert!(!partial.status().is_terminal());

    assert_eq!(
        PmOrderProgress::new(original, U256::from_u64(1), PmOrderStatus::Open),
        Err(PmEventError::OrderStatusFillMismatch)
    );
    assert_eq!(
        PmOrderProgress::new(original, U256::ZERO, PmOrderStatus::PartiallyFilled),
        Err(PmEventError::OrderStatusFillMismatch)
    );
    assert_eq!(
        PmOrderProgress::new(original, U256::from_u64(2_000_001), PmOrderStatus::Filled,),
        Err(PmEventError::CumulativeFillExceedsOriginal)
    );
    assert!(
        PmOrderProgress::new(original, original.protocol_units(), PmOrderStatus::Filled,)
            .unwrap()
            .status()
            .is_terminal()
    );
    assert_eq!(
        PmOrderProgress::new(original, U256::from_u64(1), PmOrderStatus::Rejected),
        Err(PmEventError::OrderStatusFillMismatch)
    );
    assert!(PmOrderProgress::new(original, U256::from_u64(1), PmOrderStatus::Cancelled,).is_ok());
    assert!(PmOrderProgress::new(original, U256::from_u64(1), PmOrderStatus::Expired,).is_ok());
}

#[test]
fn order_and_fill_events_retain_account_token_and_both_order_identities() {
    let progress = PmOrderProgress::new(
        PmQuantity::parse_decimal("2").unwrap(),
        U256::ZERO,
        PmOrderStatus::Open,
    )
    .unwrap();
    let order = PmOrderEvent::new(
        account_source(),
        instrument(),
        order_identity(),
        PmOrderSide::Buy,
        PmPrice::parse_decimal("0.4").unwrap(),
        progress,
    )
    .unwrap();
    assert_eq!(order.source().venue(), Venue::Polymarket);
    assert_eq!(order.account(), account());
    assert_eq!(order.instrument().token(), instrument().token());
    assert_eq!(
        order.order().client_order_key().unwrap().account(),
        account()
    );
    assert_eq!(
        order.order().venue_order_key().unwrap().id().as_str(),
        "venue-order-1"
    );
    assert_eq!(order.side(), PmOrderSide::Buy);
    assert_eq!(order.price().units(), 400_000);
    assert_eq!(order.progress(), progress);

    let fee_delta = PmSignedUnits::from_parts(PmSign::Negative, U256::from_u64(25)).unwrap();
    let execution = PmFillExecution::new(
        PmOrderSide::Buy,
        PmFillRole::Maker,
        PmFillSettlementStatus::Confirmed,
        PmPrice::parse_decimal("0.4").unwrap(),
        PmQuantity::parse_decimal("0.5").unwrap(),
        PmFillFee::Known {
            asset: collateral_asset(),
            delta: fee_delta,
        },
    );
    let fill_key = PmFillKey::new(
        order_identity().venue_order_key().unwrap(),
        PmFillId::new("fill-1").unwrap(),
    );
    let fill = PmFillEvent::new(
        account_source(),
        instrument(),
        fill_key,
        order_identity(),
        execution,
    )
    .unwrap();
    assert_eq!(fill.fill_key(), fill_key);
    assert_eq!(fill.fill_key().id().as_str(), "fill-1");
    assert_eq!(
        fill.fill_key().venue_order(),
        order_identity().venue_order_key().unwrap()
    );
    assert_eq!(fill.account(), account());
    assert_eq!(fill.instrument(), instrument());
    assert_eq!(fill.order(), order_identity());
    assert_eq!(fill.execution().role(), PmFillRole::Maker);
    assert_eq!(
        fill.execution().settlement(),
        PmFillSettlementStatus::Confirmed
    );
    assert_eq!(fill.execution().side(), PmOrderSide::Buy);
    assert_eq!(
        fill.execution().quantity().protocol_units(),
        U256::from_u64(500_000)
    );
    assert_eq!(
        fill.execution().fee(),
        PmFillFee::Known {
            asset: collateral_asset(),
            delta: fee_delta,
        }
    );
    let other_account = PmAccountHandle::from_ordinal(4);
    let other_fill_key = PmFillKey::new(
        order_identity_for(other_account).venue_order_key().unwrap(),
        PmFillId::new("fill-1").unwrap(),
    );
    assert_ne!(fill_key, other_fill_key);
    assert_eq!(
        PmFillEvent::new(
            account_source(),
            instrument(),
            fill_key,
            order_identity_for(other_account),
            execution,
        ),
        Err(PmEventError::FillVenueOrderMismatch)
    );

    let client_only = PmOrderIdentity::new(
        Some(PmClientOrderKey::new(
            account(),
            PmClientOrderId::parse("ffeeddccbbaa99887766554433221100").unwrap(),
        )),
        None,
    )
    .unwrap();
    assert_eq!(
        PmFillEvent::new(
            account_source(),
            instrument(),
            fill_key,
            client_only,
            execution,
        ),
        Err(PmEventError::MissingFillVenueOrderIdentity)
    );

    let second_leg_identity = PmOrderIdentity::new(
        None,
        Some(PmVenueOrderKey::new(
            account(),
            PmVenueOrderId::new("venue-order-2").unwrap(),
        )),
    )
    .unwrap();
    let second_leg_key = PmFillKey::new(
        second_leg_identity.venue_order_key().unwrap(),
        PmFillId::new("fill-1").unwrap(),
    );
    assert_ne!(
        fill_key, second_leg_key,
        "one trade ID on two maker-order legs must retain two fill identities"
    );
    assert!(
        PmFillEvent::new(
            account_source(),
            instrument(),
            second_leg_key,
            second_leg_identity,
            execution,
        )
        .is_ok()
    );

    let wrong_account_source = PmProductSource::polymarket_account(
        PmSourceHandle::from_ordinal(2),
        PmAccountHandle::from_ordinal(99),
    );
    assert_eq!(
        PmOrderEvent::new(
            wrong_account_source,
            instrument(),
            order_identity(),
            PmOrderSide::Buy,
            PmPrice::parse_decimal("0.4").unwrap(),
            progress,
        ),
        Err(PmEventError::AccountSourceMismatch)
    );
}

#[test]
fn snapshot_rows_keep_only_revision_and_zero_balances_explicit() {
    assert_eq!(
        PmSnapshotEvidence::new(SnapshotRevision::new(0)),
        Err(PmEventError::ZeroRevision)
    );

    let balance = PmBalanceEvent::new(
        account_source(),
        account(),
        collateral_asset(),
        U256::ZERO,
        complete_snapshot(),
    )
    .unwrap();
    assert_eq!(balance.source(), account_source());
    assert_eq!(balance.account(), account());
    assert_eq!(balance.asset(), collateral_asset());
    assert_eq!(balance.balance(), U256::ZERO);
    assert_eq!(balance.snapshot().revision().value(), 12);

    assert_eq!(
        PmBalanceEvent::new(
            market_source(),
            account(),
            collateral_asset(),
            U256::ZERO,
            complete_snapshot(),
        ),
        Err(PmEventError::WrongAccountSource)
    );
}

#[test]
fn allowances_bind_exact_account_asset_and_spender_kind() {
    let collateral_spender = spender(collateral_asset());
    let collateral = PmAllowanceEvent::new(
        account_source(),
        collateral_spender,
        PmAllowanceValue::Erc20(U256::from_u64(9_000_000)),
        complete_snapshot(),
    )
    .unwrap();
    assert_eq!(collateral.source(), account_source());
    assert_eq!(collateral.account(), account());
    assert_eq!(collateral.spender(), collateral_spender);
    assert_eq!(collateral.asset(), collateral_asset());
    assert_eq!(
        collateral.value(),
        PmAllowanceValue::Erc20(U256::from_u64(9_000_000))
    );
    assert_eq!(collateral.snapshot(), complete_snapshot());

    assert_eq!(
        PmAllowanceEvent::new(
            account_source(),
            collateral_spender,
            PmAllowanceValue::Erc1155Operator(PmErc1155OperatorApproval::from_bool(true)),
            complete_snapshot(),
        ),
        Err(PmEventError::AllowanceAssetKindMismatch)
    );

    let outcome_spender = spender(outcome_asset());
    let approval = PmAllowanceEvent::new(
        account_source(),
        outcome_spender,
        PmAllowanceValue::Erc1155Operator(PmErc1155OperatorApproval::from_bool(false)),
        complete_snapshot(),
    )
    .unwrap();
    assert_eq!(approval.asset(), outcome_asset());
    assert_eq!(
        approval.value(),
        PmAllowanceValue::Erc1155Operator(PmErc1155OperatorApproval::from_bool(false))
    );
}

#[test]
fn positions_retain_account_token_exact_units_and_nontradable_state() {
    let position = PmPositionEvent::new(
        account_source(),
        account(),
        instrument(),
        U256::from_u64(4_500_000),
        PmPositionAvailability::ResolvedUnredeemed,
        complete_snapshot(),
    )
    .unwrap();
    assert_eq!(position.source().venue(), Venue::Polymarket);
    assert_eq!(position.account(), account());
    assert_eq!(position.instrument().token(), instrument().token());
    assert_eq!(position.quantity(), U256::from_u64(4_500_000));
    assert_eq!(
        position.availability(),
        PmPositionAvailability::ResolvedUnredeemed
    );
    assert_eq!(position.snapshot(), complete_snapshot());
}

#[test]
fn every_normalized_family_rejects_a_wrong_source_scope() {
    let wrong_market_source = PmProductSource::polymarket_market(
        PmSourceHandle::from_ordinal(1),
        PmTokenHandle::from_ordinal(99),
    );
    assert_eq!(
        PmBookEvent::new(
            wrong_market_source,
            instrument(),
            SnapshotRevision::new(1),
            PmBookUpdate::Snapshot(PmBookSnapshot::new(Box::default()).unwrap()),
        ),
        Err(PmEventError::MarketSourceTokenMismatch)
    );
    assert_eq!(
        PmBookEvent::new(
            account_source(),
            instrument(),
            SnapshotRevision::new(1),
            PmBookUpdate::Snapshot(PmBookSnapshot::new(Box::default()).unwrap()),
        ),
        Err(PmEventError::WrongMarketSource)
    );

    let open_progress = PmOrderProgress::new(
        PmQuantity::parse_decimal("1").unwrap(),
        U256::ZERO,
        PmOrderStatus::Open,
    )
    .unwrap();
    assert_eq!(
        PmOrderEvent::new(
            market_source(),
            instrument(),
            order_identity(),
            PmOrderSide::Sell,
            PmPrice::parse_decimal("0.6").unwrap(),
            open_progress,
        ),
        Err(PmEventError::WrongAccountSource)
    );
    assert_eq!(
        PmFillEvent::new(
            market_source(),
            instrument(),
            PmFillKey::new(
                order_identity().venue_order_key().unwrap(),
                PmFillId::new("fill-wrong-source").unwrap(),
            ),
            order_identity(),
            PmFillExecution::new(
                PmOrderSide::Sell,
                PmFillRole::Taker,
                PmFillSettlementStatus::Matched,
                PmPrice::parse_decimal("0.6").unwrap(),
                PmQuantity::parse_decimal("1").unwrap(),
                PmFillFee::Unknown,
            ),
        ),
        Err(PmEventError::WrongAccountSource)
    );
    assert_eq!(
        PmBalanceEvent::new(
            market_source(),
            account(),
            collateral_asset(),
            U256::ZERO,
            complete_snapshot(),
        ),
        Err(PmEventError::WrongAccountSource)
    );
    assert_eq!(
        PmAllowanceEvent::new(
            market_source(),
            spender(collateral_asset()),
            PmAllowanceValue::Erc20(U256::ZERO),
            complete_snapshot(),
        ),
        Err(PmEventError::WrongAccountSource)
    );
    assert_eq!(
        PmPositionEvent::new(
            market_source(),
            account(),
            instrument(),
            U256::ZERO,
            PmPositionAvailability::Unavailable,
            complete_snapshot(),
        ),
        Err(PmEventError::WrongAccountSource)
    );

    let different_account_source = PmProductSource::polymarket_account(
        PmSourceHandle::from_ordinal(2),
        PmAccountHandle::from_ordinal(99),
    );
    assert_eq!(
        PmAllowanceEvent::new(
            different_account_source,
            spender(collateral_asset()),
            PmAllowanceValue::Erc20(U256::ZERO),
            complete_snapshot(),
        ),
        Err(PmEventError::AccountSourceMismatch)
    );
    assert_eq!(
        PmPositionEvent::new(
            different_account_source,
            account(),
            instrument(),
            U256::ZERO,
            PmPositionAvailability::Unavailable,
            complete_snapshot(),
        ),
        Err(PmEventError::AccountSourceMismatch)
    );
}
