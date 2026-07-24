mod support;

use reap_pm_core::{
    EvmAddress, PmAccountHandle, PmAccountScope, PmChainId, PmClientOrderId, PmClientOrderKey,
    PmEnvironmentId, PmFillFee, PmFillId, PmFillRole, PmFillSettlementStatus, PmFunderId,
    PmInstrumentHandle, PmMarketHandle, PmMarketLifecycle, PmNumericError, PmOrderSalt,
    PmOrderSide, PmPrice, PmQuantity, PmSignerId, PmTokenHandle, PmVenueOrderId, PmVenueOrderKey,
};
use reap_polymarket_adapter::{
    MAX_PM_FAKE_ACK_FILL_LEGS, PmFakeCancelOutcome, PmFakeCancelRejectReason, PmFakeCancelScript,
    PmFakeExecutionError, PmFakeImmediateFill, PmFakeOrderType, PmFakePlaceOutcome,
    PmFakePlaceRejectReason, PmFakePlaceScript, PmFixtureInstrumentScope, PmFixtureOwnedExecution,
};

fn client_order(account: PmAccountHandle) -> PmClientOrderKey {
    PmClientOrderKey::new(account, PmClientOrderId::from_bytes([0x11; 16]).unwrap())
}

fn venue_order(account: PmAccountHandle, value: &str) -> PmVenueOrderKey {
    PmVenueOrderKey::new(account, PmVenueOrderId::new(value).unwrap())
}

fn fill(id: &str, price: &str, quantity: &str) -> PmFakeImmediateFill {
    PmFakeImmediateFill::new(
        PmFillId::new(id).unwrap(),
        PmPrice::parse_decimal(price).unwrap(),
        PmQuantity::parse_decimal(quantity).unwrap(),
        PmFillFee::Unknown,
    )
}

fn role() -> PmFixtureOwnedExecution {
    PmFixtureOwnedExecution::new(support::account_scope(), support::instrument())
}

fn place(
    role: &PmFixtureOwnedExecution,
    side: PmOrderSide,
    price: &str,
    quantity: &str,
) -> reap_polymarket_adapter::PmFakePlaceCommand {
    role.place_command(
        support::instrument_scope(),
        client_order(role.account()),
        PmOrderSalt::from_u64(123_456_789).unwrap(),
        side,
        PmPrice::parse_decimal(price).unwrap(),
        PmQuantity::parse_decimal(quantity).unwrap(),
        1_760_000_000_000,
    )
    .unwrap()
}

#[test]
fn place_command_binds_exact_identity_and_the_only_outer_profile() {
    let role = role();
    let command = place(&role, PmOrderSide::Buy, "0.40", "10");

    assert_eq!(command.account_scope(), support::account_scope());
    assert_eq!(command.instrument(), support::instrument());
    assert_eq!(command.instrument_id(), support::instrument_id());
    assert_eq!(command.client_order(), client_order(role.account()));
    assert_eq!(command.side(), PmOrderSide::Buy);
    assert_eq!(command.price(), PmPrice::parse_decimal("0.40").unwrap());
    assert_eq!(command.quantity(), PmQuantity::parse_decimal("10").unwrap());
    assert_eq!(command.profile().order_type(), PmFakeOrderType::Gtc);
    assert!(command.profile().post_only());
    assert!(!command.profile().defer_exec());
    assert_eq!(command.profile().expiration(), 0);
    assert_eq!(
        command.unsigned_order().maker(),
        support::account_scope().funder().address()
    );
    assert_eq!(
        command.unsigned_order().signer(),
        support::account_scope().signer().address()
    );
    assert_eq!(
        command.unsigned_order().token_id(),
        support::instrument_id().token()
    );

    let wire = serde_json::to_string(&command.unsigned_order()).unwrap();
    assert_eq!(
        wire,
        concat!(
            r#"{"builder":"0x0000000000000000000000000000000000000000000000000000000000000000","#,
            r#""maker":"0xabababababababababababababababababababab","#,
            r#""makerAmount":"4000000","#,
            r#""metadata":"0x0000000000000000000000000000000000000000000000000000000000000000","#,
            r#""salt":123456789,"side":"BUY","signatureType":0,"#,
            r#""signer":"0xabababababababababababababababababababab","#,
            r#""takerAmount":"10000000","timestamp":"1760000000000","tokenId":"123"}"#
        )
    );
}

#[test]
fn command_construction_rechecks_scope_account_eoa_grid_lot_and_minimum() {
    let role = role();
    let other_account = PmAccountHandle::from_ordinal(8);
    assert_eq!(
        role.place_command(
            support::instrument_scope(),
            client_order(other_account),
            PmOrderSalt::from_u64(1).unwrap(),
            PmOrderSide::Buy,
            PmPrice::parse_decimal("0.40").unwrap(),
            PmQuantity::parse_decimal("10").unwrap(),
            1,
        ),
        Err(PmFakeExecutionError::AccountMismatch)
    );

    let wrong_instrument = PmFixtureOwnedExecution::new(
        support::account_scope(),
        PmInstrumentHandle::new(
            PmMarketHandle::from_ordinal(20),
            PmTokenHandle::from_ordinal(21),
        ),
    );
    assert_eq!(
        wrong_instrument.place_command(
            support::instrument_scope(),
            client_order(wrong_instrument.account()),
            PmOrderSalt::from_u64(1).unwrap(),
            PmOrderSide::Buy,
            PmPrice::parse_decimal("0.40").unwrap(),
            PmQuantity::parse_decimal("10").unwrap(),
            1,
        ),
        Err(PmFakeExecutionError::InstrumentMismatch)
    );

    let scope = support::account_scope();
    let non_eoa_scope = PmAccountScope::new(
        scope.environment(),
        scope.chain(),
        PmSignerId::new(EvmAddress::parse("0xcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcd").unwrap()),
        scope.funder(),
        scope.handle(),
    );
    let non_eoa = PmFixtureOwnedExecution::new(non_eoa_scope, support::instrument());
    assert_eq!(
        non_eoa.place_command(
            support::instrument_scope(),
            client_order(non_eoa.account()),
            PmOrderSalt::from_u64(1).unwrap(),
            PmOrderSide::Buy,
            PmPrice::parse_decimal("0.40").unwrap(),
            PmQuantity::parse_decimal("10").unwrap(),
            1,
        ),
        Err(PmFakeExecutionError::EoaIdentityMismatch)
    );

    let wrong_chain_scope = PmAccountScope::new(
        PmEnvironmentId::new("wrong-chain").unwrap(),
        PmChainId::new(1).unwrap(),
        scope.signer(),
        PmFunderId::new(scope.funder().address()),
        scope.handle(),
    );
    let wrong_chain = PmFixtureOwnedExecution::new(wrong_chain_scope, support::instrument());
    assert_eq!(
        wrong_chain.place_command(
            support::instrument_scope(),
            client_order(wrong_chain.account()),
            PmOrderSalt::from_u64(1).unwrap(),
            PmOrderSide::Buy,
            PmPrice::parse_decimal("0.40").unwrap(),
            PmQuantity::parse_decimal("10").unwrap(),
            1,
        ),
        Err(PmFakeExecutionError::ChainMismatch)
    );

    assert_eq!(
        role.place_command(
            support::instrument_scope(),
            client_order(role.account()),
            PmOrderSalt::from_u64(1).unwrap(),
            PmOrderSide::Buy,
            PmPrice::parse_decimal("0.405").unwrap(),
            PmQuantity::parse_decimal("10").unwrap(),
            1,
        ),
        Err(PmFakeExecutionError::UnsignedOrder(
            reap_polymarket_wire::PmUnsignedOrderError::Numeric(PmNumericError::PriceOffTick)
        ))
    );
}

#[test]
fn fake_ack_supports_resting_full_and_multi_leg_partial_results() {
    let role = role();
    let venue = venue_order(role.account(), "venue-order-1");

    let resting = role
        .execute_place(
            place(&role, PmOrderSide::Buy, "0.40", "10"),
            PmFakePlaceScript::acknowledged(venue, Box::new([])).unwrap(),
        )
        .unwrap();
    let PmFakePlaceOutcome::Acknowledged(ack) = resting.outcome() else {
        panic!("expected acknowledgement");
    };
    assert_eq!(ack.venue_order(), venue);
    assert!(ack.immediate_fills().is_empty());

    let full = role
        .execute_place(
            place(&role, PmOrderSide::Buy, "0.40", "10"),
            PmFakePlaceScript::acknowledged(
                venue,
                vec![fill("trade-full", "0.40", "10")].into_boxed_slice(),
            )
            .unwrap(),
        )
        .unwrap();
    let PmFakePlaceOutcome::Acknowledged(ack) = full.outcome() else {
        panic!("expected acknowledgement");
    };
    assert_eq!(ack.immediate_fills().len(), 1);
    let leg = ack.immediate_fills()[0];
    assert_eq!(leg.key().venue_order(), venue);
    assert_eq!(leg.key().id(), PmFillId::new("trade-full").unwrap());
    assert_eq!(leg.execution().side(), PmOrderSide::Buy);
    assert_eq!(leg.execution().role(), PmFillRole::Maker);
    assert_eq!(
        leg.execution().settlement(),
        PmFillSettlementStatus::Matched
    );

    let partial = role
        .execute_place(
            place(&role, PmOrderSide::Sell, "0.40", "10"),
            PmFakePlaceScript::acknowledged(
                venue,
                vec![
                    fill("maker-leg-a", "0.40", "2.5"),
                    fill("maker-leg-b", "0.41", "3.5"),
                ]
                .into_boxed_slice(),
            )
            .unwrap(),
        )
        .unwrap();
    let PmFakePlaceOutcome::Acknowledged(ack) = partial.outcome() else {
        panic!("expected acknowledgement");
    };
    assert_eq!(ack.immediate_fills().len(), 2);
    assert_ne!(
        ack.immediate_fills()[0].key(),
        ack.immediate_fills()[1].key()
    );
}

#[test]
fn acknowledgement_rejects_duplicate_overfill_off_grid_and_bad_limit_legs() {
    let role = role();
    let venue = venue_order(role.account(), "venue-order-2");
    let execute = |fills: Vec<PmFakeImmediateFill>| {
        role.execute_place(
            place(&role, PmOrderSide::Buy, "0.40", "10"),
            PmFakePlaceScript::acknowledged(venue, fills.into_boxed_slice()).unwrap(),
        )
    };

    assert_eq!(
        execute(vec![
            fill("duplicate", "0.40", "2"),
            fill("duplicate", "0.40", "2"),
        ]),
        Err(PmFakeExecutionError::DuplicateImmediateFill)
    );
    assert_eq!(
        execute(vec![
            fill("over-a", "0.40", "6"),
            fill("over-b", "0.40", "5"),
        ]),
        Err(PmFakeExecutionError::ImmediateFillExceedsOrder)
    );
    assert_eq!(
        execute(vec![fill("outside-limit", "0.41", "1")]),
        Err(PmFakeExecutionError::ImmediateFillOutsideLimit)
    );
    assert_eq!(
        execute(vec![fill("off-grid", "0.395", "1")]),
        Err(PmFakeExecutionError::Numeric(PmNumericError::PriceOffTick))
    );
}

#[test]
fn accepted_script_enforces_the_exact_immediate_fill_cap() {
    let account = support::account_scope().handle();
    let venue = venue_order(account, "venue-cap");
    let at_cap = (0..MAX_PM_FAKE_ACK_FILL_LEGS)
        .map(|index| fill(&format!("fill-{index}"), "0.40", "0.01"))
        .collect::<Vec<_>>()
        .into_boxed_slice();
    assert!(PmFakePlaceScript::acknowledged(venue, at_cap).is_ok());

    let above_cap = (0..=MAX_PM_FAKE_ACK_FILL_LEGS)
        .map(|index| fill(&format!("fill-{index}"), "0.40", "0.01"))
        .collect::<Vec<_>>()
        .into_boxed_slice();
    assert_eq!(
        PmFakePlaceScript::acknowledged(venue, above_cap),
        Err(PmFakeExecutionError::TooManyImmediateFillLegs)
    );
}

#[test]
fn place_rejection_and_ambiguous_acknowledgement_are_distinct() {
    let role = role();
    let rejected = role
        .execute_place(
            place(&role, PmOrderSide::Buy, "0.40", "10"),
            PmFakePlaceScript::rejected(PmFakePlaceRejectReason::FixtureRejected),
        )
        .unwrap();
    assert_eq!(
        rejected.outcome(),
        &PmFakePlaceOutcome::Rejected(PmFakePlaceRejectReason::FixtureRejected)
    );

    let unknown = role
        .execute_place(
            place(&role, PmOrderSide::Buy, "0.40", "10"),
            PmFakePlaceScript::acknowledgement_unknown(),
        )
        .unwrap();
    assert_eq!(
        unknown.outcome(),
        &PmFakePlaceOutcome::AcknowledgementUnknown
    );
}

#[test]
fn cancel_is_exactly_owned_identity_data_with_all_scripted_outcomes() {
    let role = role();
    let client = client_order(role.account());
    let venue = venue_order(role.account(), "venue-cancel");

    for (script, expected) in [
        (
            PmFakeCancelScript::accepted(),
            PmFakeCancelOutcome::Accepted,
        ),
        (
            PmFakeCancelScript::rejected(PmFakeCancelRejectReason::FixtureRejected),
            PmFakeCancelOutcome::Rejected(PmFakeCancelRejectReason::FixtureRejected),
        ),
        (
            PmFakeCancelScript::already_filled(),
            PmFakeCancelOutcome::AlreadyFilled,
        ),
        (
            PmFakeCancelScript::acknowledgement_unknown(),
            PmFakeCancelOutcome::AcknowledgementUnknown,
        ),
    ] {
        let command = role
            .cancel_command(support::instrument_scope(), client, venue)
            .unwrap();
        let result = role.execute_cancel(command, script).unwrap();
        assert_eq!(result.account_scope(), support::account_scope());
        assert_eq!(result.instrument(), support::instrument());
        assert_eq!(result.instrument_id(), support::instrument_id());
        assert_eq!(result.client_order(), client);
        assert_eq!(result.venue_order(), venue);
        assert_eq!(result.outcome(), expected);
    }
}

#[test]
fn safety_cancel_remains_available_after_quote_lifecycle_closes() {
    let role = role();
    let closed_scope = PmFixtureInstrumentScope::from_metadata(
        support::instrument(),
        support::market_metadata_with_lifecycle(PmMarketLifecycle::new(
            false, true, false, false, false,
        )),
    )
    .unwrap();
    let client = client_order(role.account());
    let venue = venue_order(role.account(), "venue-safety-cancel");

    assert_eq!(
        role.place_command(
            closed_scope,
            client,
            PmOrderSalt::from_u64(123).unwrap(),
            PmOrderSide::Buy,
            PmPrice::parse_decimal("0.40").unwrap(),
            PmQuantity::parse_decimal("10").unwrap(),
            1_760_000_000_000,
        ),
        Err(PmFakeExecutionError::MarketNotReady)
    );

    let command = role.cancel_command(closed_scope, client, venue).unwrap();
    let result = role
        .execute_cancel(command, PmFakeCancelScript::accepted())
        .unwrap();
    assert_eq!(result.account_scope(), support::account_scope());
    assert_eq!(result.instrument(), support::instrument());
    assert_eq!(result.instrument_id(), support::instrument_id());
    assert_eq!(result.client_order(), client);
    assert_eq!(result.venue_order(), venue);
    assert_eq!(result.outcome(), PmFakeCancelOutcome::Accepted);
}

#[test]
fn acknowledgement_and_cancel_reject_cross_account_venue_keys() {
    let role = role();
    let other = PmAccountHandle::from_ordinal(44);
    let other_venue = venue_order(other, "other-venue");
    assert_eq!(
        role.execute_place(
            place(&role, PmOrderSide::Buy, "0.40", "10"),
            PmFakePlaceScript::acknowledged(other_venue, Box::new([])).unwrap(),
        ),
        Err(PmFakeExecutionError::VenueOrderAccountMismatch)
    );
    assert_eq!(
        role.cancel_command(
            support::instrument_scope(),
            client_order(role.account()),
            other_venue,
        ),
        Err(PmFakeExecutionError::AccountMismatch)
    );
}
