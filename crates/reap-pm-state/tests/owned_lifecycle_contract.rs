use reap_pm_core::{
    ConnectionEpoch, EvmAddress, IngressSequence, PmAccountHandle, PmAccountScope, PmChainId,
    PmClientOrderId, PmClientOrderKey, PmEnvironmentId, PmFillId, PmFillKey, PmFunderId,
    PmInstrumentHandle, PmMarketHandle, PmOrderIdentity, PmOrderProgress, PmOrderSide,
    PmOrderStatus, PmPrice, PmQuantity, PmSignerId, PmTokenHandle, PmVenueOrderId, PmVenueOrderKey,
    U256, exact_order_amounts,
};
use reap_pm_state::{
    PmExactReservation, PmOwnedCancelApply, PmOwnedCancelOutcome, PmOwnedCancelRequestApply,
    PmOwnedCancelState, PmOwnedFillApply, PmOwnedFillObservation, PmOwnedIntentId,
    PmOwnedObservationOccurrence, PmOwnedObservationSource, PmOwnedOrderLifecycle,
    PmOwnedOrderLifecycleError, PmOwnedOrderProgressObservation, PmOwnedProgressApply,
    PmOwnedQuoteAdmission, PmOwnedQuoteIntent, PmOwnedQuoteSlotKey, PmOwnedReductionSequence,
    PmOwnedRemoteOrderApply, PmOwnedReplacementBlock, PmOwnedSubmitApply, PmOwnedSubmitResult,
    PmOwnedSubmitState,
};

fn account() -> PmAccountHandle {
    PmAccountHandle::from_ordinal(7)
}

fn other_account() -> PmAccountHandle {
    PmAccountHandle::from_ordinal(8)
}

fn scope() -> PmAccountScope {
    PmAccountScope::new(
        PmEnvironmentId::new("owned-lifecycle").unwrap(),
        PmChainId::new(137).unwrap(),
        PmSignerId::new(EvmAddress::from_bytes([1; 20]).unwrap()),
        PmFunderId::new(EvmAddress::from_bytes([2; 20]).unwrap()),
        account(),
    )
}

fn instrument() -> PmInstrumentHandle {
    PmInstrumentHandle::new(
        PmMarketHandle::from_ordinal(20),
        PmTokenHandle::from_ordinal(30),
    )
}

fn client(id: u8) -> PmClientOrderKey {
    PmClientOrderKey::new(account(), PmClientOrderId::from_bytes([id; 16]).unwrap())
}

fn other_client(id: u8) -> PmClientOrderKey {
    PmClientOrderKey::new(
        other_account(),
        PmClientOrderId::from_bytes([id; 16]).unwrap(),
    )
}

fn venue(id: &str) -> PmVenueOrderKey {
    PmVenueOrderKey::new(account(), PmVenueOrderId::new(id).unwrap())
}

fn fill_key(venue: PmVenueOrderKey, id: &str) -> PmFillKey {
    PmFillKey::new(venue, PmFillId::new(id).unwrap())
}

fn price(value: &str) -> PmPrice {
    PmPrice::parse_decimal(value).unwrap()
}

fn quantity(value: &str) -> PmQuantity {
    PmQuantity::parse_decimal(value).unwrap()
}

fn units(value: &str) -> U256 {
    quantity(value).protocol_units()
}

fn occurrence(epoch: u64, ingress: u64) -> PmOwnedObservationOccurrence {
    PmOwnedObservationOccurrence::new(
        PmOwnedReductionSequence::new(ingress).unwrap(),
        Some(reap_pm_state::PmPrivateOccurrence::new(
            ConnectionEpoch::new(epoch),
            IngressSequence::new(ingress),
        )),
        None,
    )
    .unwrap()
}

fn intent(
    intent_id: u64,
    client_id: u8,
    side: PmOrderSide,
    price_value: &str,
) -> PmOwnedQuoteIntent {
    let price = price(price_value);
    let quantity = quantity("1");
    let maker = exact_order_amounts(side, price, quantity).unwrap().maker();
    let reservation = match side {
        PmOrderSide::Buy => PmExactReservation::policy_approved(maker, U256::ZERO),
        PmOrderSide::Sell => PmExactReservation::policy_approved(U256::ZERO, maker),
    }
    .unwrap();
    PmOwnedQuoteIntent::new(
        PmOwnedIntentId::new(intent_id).unwrap(),
        PmOwnedQuoteSlotKey::new(scope(), instrument(), side),
        client(client_id),
        price,
        quantity,
        reservation,
    )
    .unwrap()
}

fn sequenced_intent(id: u64) -> PmOwnedQuoteIntent {
    let mut bytes = [0_u8; 16];
    bytes[8..].copy_from_slice(&id.to_be_bytes());
    let client_order =
        PmClientOrderKey::new(account(), PmClientOrderId::from_bytes(bytes).unwrap());
    let price = price("0.40");
    let quantity = quantity("1");
    PmOwnedQuoteIntent::new(
        PmOwnedIntentId::new(id).unwrap(),
        PmOwnedQuoteSlotKey::new(scope(), instrument(), PmOrderSide::Buy),
        client_order,
        price,
        quantity,
        PmExactReservation::policy_approved(U256::from_u64(400_000), U256::ZERO).unwrap(),
    )
    .unwrap()
}

fn lifecycle() -> PmOwnedOrderLifecycle {
    let mut lifecycle = PmOwnedOrderLifecycle::new(scope(), instrument());
    lifecycle.begin_epoch(ConnectionEpoch::new(1)).unwrap();
    lifecycle
}

fn accepted(
    lifecycle: &mut PmOwnedOrderLifecycle,
    intent_id: u64,
    client_id: u8,
    side: PmOrderSide,
    venue_id: &str,
) -> (PmClientOrderKey, PmVenueOrderKey) {
    let quote = intent(intent_id, client_id, side, "0.40");
    let client = quote.client_order();
    let venue = venue(venue_id);
    assert_eq!(
        lifecycle.admit_quote(quote).unwrap(),
        PmOwnedQuoteAdmission::Admitted(client)
    );
    assert_eq!(
        lifecycle
            .apply_submit_result(client, PmOwnedSubmitResult::Accepted(venue))
            .unwrap(),
        PmOwnedSubmitApply::Accepted
    );
    (client, venue)
}

fn fill_observation(
    venue: PmVenueOrderKey,
    id: &str,
    quantity_value: &str,
    cumulative: Option<&str>,
    epoch: u64,
    ingress: u64,
    source: PmOwnedObservationSource,
) -> PmOwnedFillObservation {
    PmOwnedFillObservation::new(
        fill_key(venue, id),
        quantity(quantity_value),
        cumulative.map(units),
        occurrence(epoch, ingress),
        source,
    )
    .unwrap()
}

fn progress(
    client: PmClientOrderKey,
    venue: PmVenueOrderKey,
    cumulative: &str,
    status: PmOrderStatus,
    epoch: u64,
    ingress: u64,
    source: PmOwnedObservationSource,
) -> PmOwnedOrderProgressObservation {
    PmOwnedOrderProgressObservation::new(
        client,
        venue,
        PmOrderProgress::new(quantity("1"), units(cumulative), status).unwrap(),
        occurrence(epoch, ingress),
        source,
    )
}

#[test]
fn immediate_ws_and_rest_fills_converge_without_double_counting() {
    let mut lifecycle = lifecycle();
    let (client, venue) = accepted(&mut lifecycle, 1, 1, PmOrderSide::Buy, "venue-1");

    assert_eq!(
        lifecycle
            .observe_fill(fill_observation(
                venue,
                "fill-b",
                "0.25",
                Some("0.25"),
                1,
                1,
                PmOwnedObservationSource::ImmediateAcknowledgement,
            ))
            .unwrap(),
        PmOwnedFillApply::Applied {
            client_order: client,
            cumulative_filled: units("0.25"),
            remaining: units("0.75"),
        }
    );
    assert_eq!(
        lifecycle
            .observe_fill(fill_observation(
                venue,
                "fill-b",
                "0.25",
                Some("0.25"),
                1,
                2,
                PmOwnedObservationSource::PrivateWebSocket,
            ))
            .unwrap(),
        PmOwnedFillApply::Duplicate {
            client_order: client,
            cumulative_filled: units("0.25"),
            remaining: units("0.75"),
            source_added: true,
            cumulative_advanced: false,
        }
    );
    lifecycle
        .observe_fill(fill_observation(
            venue,
            "fill-a",
            "0.25",
            Some("0.50"),
            1,
            3,
            PmOwnedObservationSource::RestReconciliation,
        ))
        .unwrap();
    lifecycle
        .observe_fill(fill_observation(
            venue,
            "fill-c",
            "0.50",
            Some("1"),
            1,
            4,
            PmOwnedObservationSource::PrivateWebSocket,
        ))
        .unwrap();

    let order = lifecycle.orders().next().unwrap();
    assert_eq!(order.status(), Some(PmOrderStatus::Filled));
    assert_eq!(order.cumulative_filled(), units("1"));
    assert_eq!(order.known_fill_total(), units("1"));
    assert_eq!(order.remaining(), U256::ZERO);

    let fills: Vec<_> = lifecycle.fills().collect();
    assert_eq!(fills.len(), 3);
    assert_eq!(fills[0].key(), fill_key(venue, "fill-a"));
    assert_eq!(fills[1].key(), fill_key(venue, "fill-b"));
    assert!(fills[1].observed_from(PmOwnedObservationSource::ImmediateAcknowledgement));
    assert!(fills[1].observed_from(PmOwnedObservationSource::PrivateWebSocket));
    assert_eq!(fills[1].first_occurrence(), occurrence(1, 1));
    assert_eq!(fills[1].last_occurrence(), occurrence(1, 2));
    assert_eq!(lifecycle.counters().fills(), 3);
    assert_eq!(lifecycle.counters().fill_duplicates(), 1);
}

#[test]
fn out_of_order_fills_apply_exact_keys_but_newer_cumulative_cannot_move_backwards() {
    let mut lifecycle = lifecycle();
    let (client, venue) = accepted(&mut lifecycle, 1, 1, PmOrderSide::Buy, "venue-1");

    lifecycle
        .observe_fill(fill_observation(
            venue,
            "later",
            "0.75",
            Some("0.75"),
            1,
            10,
            PmOwnedObservationSource::PrivateWebSocket,
        ))
        .unwrap();
    lifecycle
        .observe_fill(fill_observation(
            venue,
            "earlier",
            "0.25",
            Some("0.25"),
            1,
            2,
            PmOwnedObservationSource::RestReconciliation,
        ))
        .unwrap();
    assert_eq!(
        lifecycle.orders().next().unwrap().cumulative_filled(),
        units("1")
    );

    let backwards = lifecycle
        .observe_fill(fill_observation(
            venue,
            "later",
            "0.75",
            Some("0.75"),
            1,
            11,
            PmOwnedObservationSource::RestReconciliation,
        ))
        .unwrap_err();
    assert_eq!(backwards, PmOwnedOrderLifecycleError::BackwardsCumulative);

    let overfill = lifecycle
        .observe_fill(fill_observation(
            venue,
            "overfill",
            "0.01",
            None,
            1,
            12,
            PmOwnedObservationSource::PrivateWebSocket,
        ))
        .unwrap_err();
    assert_eq!(overfill, PmOwnedOrderLifecycleError::Overfill);

    let backwards_progress = lifecycle
        .observe_progress(progress(
            client,
            venue,
            "0.75",
            PmOrderStatus::PartiallyFilled,
            1,
            13,
            PmOwnedObservationSource::RestReconciliation,
        ))
        .unwrap_err();
    assert_eq!(
        backwards_progress,
        PmOwnedOrderLifecycleError::BackwardsCumulative
    );
}

#[test]
fn rejection_timeout_late_ack_and_remote_ambiguity_preserve_ownership_authority() {
    let mut lifecycle = lifecycle();
    let rejected = intent(1, 1, PmOrderSide::Buy, "0.40");
    assert!(matches!(
        lifecycle.admit_quote(rejected).unwrap(),
        PmOwnedQuoteAdmission::Admitted(_)
    ));
    assert_eq!(
        lifecycle
            .apply_submit_result(rejected.client_order(), PmOwnedSubmitResult::Rejected)
            .unwrap(),
        PmOwnedSubmitApply::Rejected
    );
    assert_eq!(
        lifecycle
            .apply_submit_result(
                rejected.client_order(),
                PmOwnedSubmitResult::Accepted(venue("too-late")),
            )
            .unwrap_err(),
        PmOwnedOrderLifecycleError::TerminalNonResurrection
    );

    let timed_out = intent(2, 2, PmOrderSide::Sell, "0.40");
    lifecycle.admit_quote(timed_out).unwrap();
    assert_eq!(
        lifecycle
            .apply_submit_result(timed_out.client_order(), PmOwnedSubmitResult::Ambiguous)
            .unwrap(),
        PmOwnedSubmitApply::MarkedAmbiguous
    );
    let accepted_venue = venue("late-ack");
    assert_eq!(
        lifecycle
            .apply_submit_result(
                timed_out.client_order(),
                PmOwnedSubmitResult::Accepted(accepted_venue),
            )
            .unwrap(),
        PmOwnedSubmitApply::LateAccepted
    );

    assert_eq!(
        lifecycle
            .observe_remote_order(
                PmOrderIdentity::new(None, Some(venue("unknown-remote"))).unwrap()
            )
            .unwrap(),
        PmOwnedRemoteOrderApply::AmbiguousRemote
    );
    assert_eq!(
        lifecycle
            .observe_remote_order(
                PmOrderIdentity::new(Some(timed_out.client_order()), Some(venue("wrong-remote")),)
                    .unwrap(),
            )
            .unwrap(),
        PmOwnedRemoteOrderApply::AmbiguousRemote
    );
    let order = lifecycle
        .orders()
        .find(|order| order.client_order() == timed_out.client_order())
        .unwrap();
    assert_eq!(order.venue_order(), Some(accepted_venue));
    assert!(order.reconciliation_required());

    let mut bindings = PmOwnedOrderLifecycle::new(scope(), instrument());
    bindings.begin_epoch(ConnectionEpoch::new(1)).unwrap();
    let (_, shared) = accepted(&mut bindings, 10, 10, PmOrderSide::Buy, "shared");
    let second = intent(11, 11, PmOrderSide::Sell, "0.40");
    bindings.admit_quote(second).unwrap();
    assert_eq!(
        bindings
            .apply_submit_result(second.client_order(), PmOwnedSubmitResult::Accepted(shared),)
            .unwrap_err(),
        PmOwnedOrderLifecycleError::VenueBindingConflict
    );
}

#[test]
fn cancel_results_and_late_fills_converge_without_terminal_resurrection() {
    let mut lifecycle = lifecycle();
    let (client_1, venue_1) = accepted(&mut lifecycle, 1, 1, PmOrderSide::Buy, "cancel-retry");
    let cancel_1 = match lifecycle.request_cancel(client_1).unwrap() {
        PmOwnedCancelRequestApply::Issued(intent) => intent,
        other => panic!("expected issued cancel, got {other:?}"),
    };
    assert_eq!(
        lifecycle
            .apply_cancel_result(cancel_1, PmOwnedCancelOutcome::Rejected)
            .unwrap(),
        PmOwnedCancelApply::Rejected
    );
    let cancel_1 = match lifecycle.request_cancel(client_1).unwrap() {
        PmOwnedCancelRequestApply::Issued(intent) => intent,
        other => panic!("expected retry cancel, got {other:?}"),
    };
    assert_eq!(
        lifecycle
            .apply_cancel_result(cancel_1, PmOwnedCancelOutcome::Accepted)
            .unwrap(),
        PmOwnedCancelApply::Cancelled
    );
    assert_eq!(
        lifecycle.compact_proven_terminal(client_1).unwrap_err(),
        PmOwnedOrderLifecycleError::TerminalCompactionUnavailable
    );
    lifecycle
        .observe_fill(fill_observation(
            venue_1,
            "late-partial",
            "0.25",
            Some("0.25"),
            1,
            1,
            PmOwnedObservationSource::PrivateWebSocket,
        ))
        .unwrap();
    assert_eq!(
        lifecycle.orders().next().unwrap().status(),
        Some(PmOrderStatus::Cancelled)
    );
    assert_eq!(
        lifecycle.compact_proven_terminal(client_1).unwrap_err(),
        PmOwnedOrderLifecycleError::TerminalCompactionUnavailable
    );
    lifecycle
        .observe_fill(fill_observation(
            venue_1,
            "late-rest",
            "0.75",
            Some("1"),
            1,
            2,
            PmOwnedObservationSource::RestReconciliation,
        ))
        .unwrap();
    let first = lifecycle.orders().next().unwrap();
    assert_eq!(first.status(), Some(PmOrderStatus::Filled));
    assert_eq!(first.cancel(), PmOwnedCancelState::FilledRace);
    assert!(!first.reconciliation_required());
    assert_eq!(
        lifecycle
            .compact_proven_terminal(client_1)
            .unwrap()
            .fill_keys_removed(),
        2
    );

    let (client_2, venue_2) = accepted(&mut lifecycle, 2, 2, PmOrderSide::Buy, "already-filled");
    let cancel_2 = match lifecycle.request_cancel(client_2).unwrap() {
        PmOwnedCancelRequestApply::Issued(intent) => intent,
        other => panic!("expected issued cancel, got {other:?}"),
    };
    assert_eq!(
        lifecycle
            .apply_cancel_result(cancel_2, PmOwnedCancelOutcome::AlreadyFilled)
            .unwrap(),
        PmOwnedCancelApply::Filled
    );
    assert_eq!(
        lifecycle.compact_proven_terminal(client_2).unwrap_err(),
        PmOwnedOrderLifecycleError::TerminalCompactionUnavailable
    );
    lifecycle
        .observe_progress(progress(
            client_2,
            venue_2,
            "1",
            PmOrderStatus::Filled,
            1,
            3,
            PmOwnedObservationSource::RestReconciliation,
        ))
        .unwrap();
    assert_eq!(
        lifecycle.compact_proven_terminal(client_2).unwrap_err(),
        PmOwnedOrderLifecycleError::TerminalCompactionUnavailable
    );
    lifecycle
        .observe_fill(fill_observation(
            venue_2,
            "already-filled-rest",
            "1",
            Some("1"),
            1,
            4,
            PmOwnedObservationSource::RestReconciliation,
        ))
        .unwrap();
    assert_eq!(
        lifecycle
            .compact_proven_terminal(client_2)
            .unwrap()
            .fill_keys_removed(),
        1
    );

    let (client_3, venue_3) = accepted(&mut lifecycle, 3, 3, PmOrderSide::Buy, "ambiguous-cancel");
    let cancel_3 = match lifecycle.request_cancel(client_3).unwrap() {
        PmOwnedCancelRequestApply::Issued(intent) => intent,
        other => panic!("expected issued cancel, got {other:?}"),
    };
    assert_eq!(
        lifecycle
            .apply_cancel_result(cancel_3, PmOwnedCancelOutcome::Ambiguous)
            .unwrap(),
        PmOwnedCancelApply::MarkedAmbiguous
    );
    lifecycle
        .observe_progress(progress(
            client_3,
            venue_3,
            "1",
            PmOrderStatus::Filled,
            1,
            5,
            PmOwnedObservationSource::RestReconciliation,
        ))
        .unwrap();
    assert_eq!(
        lifecycle
            .apply_cancel_result(cancel_3, PmOwnedCancelOutcome::Rejected)
            .unwrap(),
        PmOwnedCancelApply::ConvergedFilled
    );
}

#[test]
fn quote_slots_enforce_cancel_before_replace_and_canonical_iteration() {
    let mut lifecycle = lifecycle();
    let buy = intent(20, 20, PmOrderSide::Buy, "0.40");
    let sell = intent(10, 10, PmOrderSide::Sell, "0.40");

    lifecycle.admit_quote(buy).unwrap();
    assert_eq!(
        lifecycle
            .admit_quote(intent(21, 21, PmOrderSide::Buy, "0.41"))
            .unwrap(),
        PmOwnedQuoteAdmission::ReplacementBlocked {
            current: buy.client_order(),
            reason: PmOwnedReplacementBlock::SubmitPending,
        }
    );
    let buy_venue = venue("buy");
    lifecycle
        .apply_submit_result(buy.client_order(), PmOwnedSubmitResult::Accepted(buy_venue))
        .unwrap();
    lifecycle.admit_quote(sell).unwrap();
    lifecycle
        .apply_submit_result(
            sell.client_order(),
            PmOwnedSubmitResult::Accepted(venue("sell")),
        )
        .unwrap();

    assert_eq!(
        lifecycle.admit_quote(buy).unwrap(),
        PmOwnedQuoteAdmission::DuplicateIntent(buy.client_order())
    );
    assert_eq!(
        lifecycle
            .admit_quote(intent(22, 22, PmOrderSide::Buy, "0.40"))
            .unwrap(),
        PmOwnedQuoteAdmission::DuplicateQuote(buy.client_order())
    );
    let replacement = intent(23, 23, PmOrderSide::Buy, "0.41");
    let cancel = match lifecycle.admit_quote(replacement).unwrap() {
        PmOwnedQuoteAdmission::CancelBeforeReplace(cancel) => cancel,
        other => panic!("expected cancel-before-replace, got {other:?}"),
    };
    assert_eq!(cancel.client_order(), buy.client_order());
    assert_eq!(cancel.venue_order(), buy_venue);
    assert_eq!(
        lifecycle
            .apply_cancel_result(cancel, PmOwnedCancelOutcome::Accepted)
            .unwrap(),
        PmOwnedCancelApply::Cancelled
    );
    assert_eq!(
        lifecycle.admit_quote(replacement).unwrap(),
        PmOwnedQuoteAdmission::Admitted(replacement.client_order())
    );

    let clients: Vec<_> = lifecycle
        .orders()
        .map(|order| order.client_order())
        .collect();
    assert_eq!(
        clients,
        vec![
            sell.client_order(),
            buy.client_order(),
            replacement.client_order()
        ]
    );
    let slots: Vec<_> = lifecycle.slots().collect();
    assert_eq!(slots[0].key().side(), PmOrderSide::Buy);
    assert_eq!(slots[0].current(), Some(replacement.client_order()));
    assert_eq!(slots[1].key().side(), PmOrderSide::Sell);
    assert_eq!(slots[1].current(), Some(sell.client_order()));
}

#[test]
fn reconnect_progress_and_recovery_replay_are_deterministic() {
    let mut lifecycle = lifecycle();
    let quote = intent(1, 1, PmOrderSide::Buy, "0.40");
    lifecycle.admit_quote(quote).unwrap();
    lifecycle.begin_epoch(ConnectionEpoch::new(2)).unwrap();
    let pending = lifecycle.orders().next().unwrap();
    assert_eq!(pending.submit(), PmOwnedSubmitState::Ambiguous);
    assert!(pending.reconciliation_required());

    let recovered_venue = venue("recovered");
    assert_eq!(
        lifecycle
            .apply_submit_result(
                quote.client_order(),
                PmOwnedSubmitResult::Accepted(recovered_venue),
            )
            .unwrap(),
        PmOwnedSubmitApply::LateAccepted
    );
    lifecycle
        .observe_progress(progress(
            quote.client_order(),
            recovered_venue,
            "0.50",
            PmOrderStatus::PartiallyFilled,
            2,
            10,
            PmOwnedObservationSource::PrivateWebSocket,
        ))
        .unwrap();
    assert_eq!(
        lifecycle
            .observe_progress(progress(
                quote.client_order(),
                recovered_venue,
                "0.25",
                PmOrderStatus::PartiallyFilled,
                2,
                9,
                PmOwnedObservationSource::PrivateWebSocket,
            ))
            .unwrap(),
        PmOwnedProgressApply::IgnoredOutOfOrder
    );
    assert_eq!(
        lifecycle
            .observe_progress(progress(
                quote.client_order(),
                recovered_venue,
                "0.25",
                PmOrderStatus::PartiallyFilled,
                2,
                11,
                PmOwnedObservationSource::RestReconciliation,
            ))
            .unwrap_err(),
        PmOwnedOrderLifecycleError::BackwardsCumulative
    );
    lifecycle.begin_epoch(ConnectionEpoch::new(3)).unwrap();
    assert_eq!(
        lifecycle
            .observe_fill(fill_observation(
                recovered_venue,
                "old-epoch",
                "0.50",
                Some("1"),
                2,
                12,
                PmOwnedObservationSource::PrivateWebSocket,
            ))
            .unwrap(),
        PmOwnedFillApply::IgnoredOldEpoch
    );
    lifecycle
        .observe_progress(progress(
            quote.client_order(),
            recovered_venue,
            "0.50",
            PmOrderStatus::PartiallyFilled,
            3,
            1,
            PmOwnedObservationSource::RestReconciliation,
        ))
        .unwrap();
    assert!(lifecycle.orders().next().unwrap().reconciliation_required());
    lifecycle
        .observe_fill(fill_observation(
            recovered_venue,
            "current-epoch",
            "0.50",
            Some("0.50"),
            3,
            2,
            PmOwnedObservationSource::RestReconciliation,
        ))
        .unwrap();
    assert!(!lifecycle.orders().next().unwrap().reconciliation_required());

    let replay = || {
        let mut state = PmOwnedOrderLifecycle::new(scope(), instrument());
        state.begin_epoch(ConnectionEpoch::new(1)).unwrap();
        let quote = intent(50, 50, PmOrderSide::Buy, "0.40");
        state.admit_quote(quote).unwrap();
        state
            .apply_submit_result(quote.client_order(), PmOwnedSubmitResult::Ambiguous)
            .unwrap();
        state.begin_epoch(ConnectionEpoch::new(2)).unwrap();
        let venue = venue("journal-order");
        state
            .apply_submit_result(quote.client_order(), PmOwnedSubmitResult::Accepted(venue))
            .unwrap();
        state
            .observe_fill(fill_observation(
                venue,
                "journal-fill",
                "0.25",
                Some("0.25"),
                2,
                1,
                PmOwnedObservationSource::PrivateWebSocket,
            ))
            .unwrap();
        let cancel = match state.request_cancel(quote.client_order()).unwrap() {
            PmOwnedCancelRequestApply::Issued(intent) => intent,
            other => panic!("expected issued cancel, got {other:?}"),
        };
        state
            .apply_cancel_result(cancel, PmOwnedCancelOutcome::Ambiguous)
            .unwrap();
        state
            .observe_progress(progress(
                quote.client_order(),
                venue,
                "1",
                PmOrderStatus::Filled,
                2,
                2,
                PmOwnedObservationSource::RestReconciliation,
            ))
            .unwrap();
        state
    };

    let left = replay();
    let right = replay();
    assert_eq!(
        left.orders().collect::<Vec<_>>(),
        right.orders().collect::<Vec<_>>()
    );
    assert_eq!(
        left.fills().collect::<Vec<_>>(),
        right.fills().collect::<Vec<_>>()
    );
    assert_eq!(
        left.slots().collect::<Vec<_>>(),
        right.slots().collect::<Vec<_>>()
    );
    assert_eq!(left.counters(), right.counters());
}

#[test]
fn fill_identity_keeps_multiple_maker_legs_and_scope_is_exact() {
    let mut lifecycle = lifecycle();
    let (_, buy_venue) = accepted(&mut lifecycle, 2, 2, PmOrderSide::Buy, "maker-buy");
    let (_, sell_venue) = accepted(&mut lifecycle, 1, 1, PmOrderSide::Sell, "maker-sell");

    lifecycle
        .observe_fill(fill_observation(
            buy_venue,
            "shared-trade",
            "1",
            Some("1"),
            1,
            1,
            PmOwnedObservationSource::PrivateWebSocket,
        ))
        .unwrap();
    lifecycle
        .observe_fill(fill_observation(
            sell_venue,
            "shared-trade",
            "1",
            Some("1"),
            1,
            1,
            PmOwnedObservationSource::PrivateWebSocket,
        ))
        .unwrap();
    let fills: Vec<_> = lifecycle.fills().collect();
    assert_eq!(fills.len(), 2);
    assert_ne!(fills[0].key(), fills[1].key());
    assert_eq!(fills[0].key().id(), fills[1].key().id());

    let valid = intent(3, 3, PmOrderSide::Buy, "0.40");
    let wrong_client = PmOwnedQuoteIntent::new(
        PmOwnedIntentId::new(4).unwrap(),
        valid.slot(),
        other_client(4),
        valid.price(),
        valid.quantity(),
        valid.reservation(),
    )
    .unwrap_err();
    assert_eq!(wrong_client, PmOwnedOrderLifecycleError::ScopeMismatch);

    let wrong_instrument = PmOwnedQuoteIntent::new(
        PmOwnedIntentId::new(5).unwrap(),
        PmOwnedQuoteSlotKey::new(
            scope(),
            PmInstrumentHandle::new(
                PmMarketHandle::from_ordinal(21),
                PmTokenHandle::from_ordinal(31),
            ),
            PmOrderSide::Buy,
        ),
        client(5),
        valid.price(),
        valid.quantity(),
        valid.reservation(),
    )
    .unwrap();
    assert_eq!(
        lifecycle.admit_quote(wrong_instrument).unwrap_err(),
        PmOwnedOrderLifecycleError::ScopeMismatch
    );
}

#[test]
fn proven_terminal_compaction_reuses_fixed_storage_for_ten_thousand_orders() {
    let mut lifecycle = lifecycle();

    for id in 1..=10_000 {
        let quote = sequenced_intent(id);
        let client = quote.client_order();
        lifecycle.admit_quote(quote).unwrap();
        if id == 1 {
            assert_eq!(
                lifecycle.compact_proven_terminal(client).unwrap_err(),
                PmOwnedOrderLifecycleError::TerminalCompactionUnavailable
            );
        }

        let venue_id = format!("venue-{id}");
        let venue = venue(&venue_id);
        lifecycle
            .apply_submit_result(client, PmOwnedSubmitResult::Accepted(venue))
            .unwrap();
        let fill_id = format!("fill-{id}");
        lifecycle
            .observe_fill(fill_observation(
                venue,
                &fill_id,
                "1",
                Some("1"),
                1,
                id,
                PmOwnedObservationSource::PrivateWebSocket,
            ))
            .unwrap();

        let compacted = lifecycle.compact_proven_terminal(client).unwrap();
        assert_eq!(compacted.client_order(), client);
        assert_eq!(compacted.intent(), PmOwnedIntentId::new(id).unwrap());
        assert_eq!(compacted.fill_keys_removed(), 1);
        assert_eq!(lifecycle.orders().count(), 0);
        assert_eq!(lifecycle.fills().count(), 0);
    }

    assert_eq!(lifecycle.counters().admissions(), 10_000);
    assert_eq!(lifecycle.counters().fills(), 10_000);
    assert_eq!(lifecycle.counters().terminal_compactions(), 10_000);
    assert_eq!(
        lifecycle.admit_quote(sequenced_intent(10_000)).unwrap_err(),
        PmOwnedOrderLifecycleError::CompactedIntentIdentity
    );
}
