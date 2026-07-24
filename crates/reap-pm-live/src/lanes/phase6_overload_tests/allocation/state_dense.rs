use reap_benchmark_allocator::MeasurementWindow;
use reap_pm_core::{
    ConnectionEpoch, EventClock, EventEnvelope, EventOrdering, IngressSequence, PmClientOrderId,
    PmClientOrderKey, PmConnectionId, PmOrderEvent, PmOrderIdentity, PmOrderProgress, PmOrderSide,
    PmOrderStatus, PmPrice, PmProductSource, PmQuantity, PmSourceHandle, PmVenueOrderId,
    PmVenueOrderKey, U256, exact_order_amounts,
};
use reap_pm_state::{
    MAX_PM_OWNED_ORDER_HISTORY, MAX_PM_PRIVATE_ORDERS, PmExactReservation, PmOwnedIntentId,
    PmOwnedOrderLifecycle, PmOwnedQuoteAdmission, PmOwnedQuoteIntent, PmOwnedQuoteSlotKey,
    PmOwnedSubmitApply, PmOwnedSubmitResult, PmPrivateState, PmPrivateStateConfig,
    PmRemoteOrderKnowledge, PmReservationKnowledge,
};

use crate::evidence::{account_scope, instrument, market_metadata, risk_limits};

pub(super) fn measure_dense_state_indexes(window: &mut MeasurementWindow) {
    measure_canonical_order_indexes(window);
    measure_owned_lifecycle_indexes(window);
}

fn measure_canonical_order_indexes(window: &mut MeasurementWindow) {
    let scope = account_scope();
    let instrument = instrument();
    let source =
        PmProductSource::polymarket_account(PmSourceHandle::from_ordinal(2), scope.handle());
    let config = PmPrivateStateConfig::new(source, scope, instrument, market_metadata())
        .expect("fixed dense-index private configuration");
    let mut state =
        PmPrivateState::new(config, risk_limits()).expect("fixed dense-index private state");
    state
        .observe_reconnect(ConnectionEpoch::new(1), 1)
        .expect("fixed private reconnect");
    let connection = PmConnectionId::new("phase6-dense-orders").expect("fixed connection");
    let entries = shuffled(MAX_PM_PRIVATE_ORDERS, 0x517c_c1b7_2722_0a95)
        .into_iter()
        .enumerate()
        .map(|(index, ordinal)| {
            order_envelope(
                source,
                scope.handle(),
                instrument,
                connection,
                u64::try_from(index + 1).expect("bounded ingress"),
                ordinal,
            )
        })
        .collect::<Vec<_>>();
    let before = window
        .checkpoint()
        .expect("paused canonical-order allocation baseline");

    window
        .resume()
        .expect("measure canonical-order dense indexes");
    for envelope in entries.iter().copied() {
        state
            .observe_order(
                envelope,
                PmRemoteOrderKnowledge::Unmanaged(PmReservationKnowledge::Unknown),
            )
            .expect("bounded canonical order inserts");
    }
    let after = window
        .checkpoint()
        .expect("canonical-order allocation checkpoint");
    window
        .pause()
        .expect("pause after canonical-order dense indexes");

    assert_eq!(after, before, "canonical order dense indexes allocated");
    assert_eq!(
        state.cardinalities().canonical_orders,
        MAX_PM_PRIVATE_ORDERS
    );
    assert_eq!(
        state.order_counters().observations(),
        u64::try_from(MAX_PM_PRIVATE_ORDERS).expect("bounded order count")
    );
}

fn order_envelope(
    source: PmProductSource,
    account: reap_pm_core::PmAccountHandle,
    instrument: reap_pm_core::PmInstrumentHandle,
    connection: PmConnectionId,
    sequence: u64,
    ordinal: usize,
) -> EventEnvelope<PmOrderEvent> {
    let identity = if ordinal % 2 == 0 {
        PmOrderIdentity::new(Some(client_order(account, ordinal)), None)
            .expect("fixed client-only identity")
    } else {
        let venue = PmVenueOrderId::new(&format!("dense-{ordinal:04}"))
            .expect("fixed venue order identity");
        PmOrderIdentity::new(None, Some(PmVenueOrderKey::new(account, venue)))
            .expect("fixed venue-only identity")
    };
    let event = PmOrderEvent::new(
        source,
        instrument,
        identity,
        PmOrderSide::Buy,
        PmPrice::parse_decimal("0.40").expect("fixed price"),
        PmOrderProgress::new(
            PmQuantity::parse_decimal("5").expect("fixed quantity"),
            U256::ZERO,
            PmOrderStatus::Open,
        )
        .expect("fixed open progress"),
    )
    .expect("fixed order event");
    let receive_ns = sequence.saturating_mul(2).saturating_add(10);
    EventEnvelope::new(
        source.venue(),
        source,
        connection,
        EventClock::new(
            None,
            1_000_000_u64.saturating_add(sequence),
            receive_ns,
            receive_ns.saturating_add(1),
        )
        .expect("fixed event clock"),
        EventOrdering::new(
            ConnectionEpoch::new(1),
            None,
            None,
            None,
            IngressSequence::new(sequence),
        )
        .expect("fixed event ordering"),
        event,
    )
    .expect("fixed order envelope")
}

fn measure_owned_lifecycle_indexes(window: &mut MeasurementWindow) {
    let scope = account_scope();
    let instrument = instrument();
    let clients = shuffled(MAX_PM_OWNED_ORDER_HISTORY, 0x9e37_79b9_7f4a_7c15);
    let intents = shuffled(MAX_PM_OWNED_ORDER_HISTORY, 0xd1b5_4a32_d192_ed03);
    let fixtures = clients
        .into_iter()
        .zip(intents)
        .enumerate()
        .map(|(index, (client, intent))| {
            quote_intent(
                scope,
                instrument,
                client,
                intent,
                if index % 2 == 0 {
                    PmOrderSide::Buy
                } else {
                    PmOrderSide::Sell
                },
            )
        })
        .collect::<Vec<_>>();
    let mut lifecycle = PmOwnedOrderLifecycle::new(scope, instrument);
    let before = window
        .checkpoint()
        .expect("paused owned-lifecycle allocation baseline");

    window
        .resume()
        .expect("measure owned-lifecycle dense indexes");
    for intent in fixtures.iter().copied() {
        let client = intent.client_order();
        assert_eq!(
            lifecycle
                .admit_quote(intent)
                .expect("bounded owned quote admission"),
            PmOwnedQuoteAdmission::Admitted(client)
        );
        assert_eq!(
            lifecycle
                .apply_submit_result(client, PmOwnedSubmitResult::Rejected)
                .expect("fixed terminal submit result"),
            PmOwnedSubmitApply::Rejected
        );
    }
    let after = window
        .checkpoint()
        .expect("owned-lifecycle allocation checkpoint");
    window
        .pause()
        .expect("pause after owned-lifecycle dense indexes");

    assert_eq!(after, before, "owned lifecycle dense indexes allocated");
    assert_eq!(lifecycle.orders().count(), MAX_PM_OWNED_ORDER_HISTORY);
    assert_eq!(
        lifecycle.counters().admissions(),
        u64::try_from(MAX_PM_OWNED_ORDER_HISTORY).expect("bounded lifecycle count")
    );
    assert_eq!(
        lifecycle.counters().submit_rejections(),
        u64::try_from(MAX_PM_OWNED_ORDER_HISTORY).expect("bounded lifecycle count")
    );
}

fn quote_intent(
    scope: reap_pm_core::PmAccountScope,
    instrument: reap_pm_core::PmInstrumentHandle,
    client_ordinal: usize,
    intent_ordinal: usize,
    side: PmOrderSide,
) -> PmOwnedQuoteIntent {
    let price = PmPrice::parse_decimal("0.40").expect("fixed price");
    let quantity = PmQuantity::parse_decimal("5").expect("fixed quantity");
    let maker = exact_order_amounts(side, price, quantity)
        .expect("fixed exact amounts")
        .maker();
    let reservation = match side {
        PmOrderSide::Buy => PmExactReservation::policy_approved(maker, U256::ZERO),
        PmOrderSide::Sell => PmExactReservation::policy_approved(U256::ZERO, maker),
    }
    .expect("fixed exact reservation");
    PmOwnedQuoteIntent::new(
        PmOwnedIntentId::new(
            u64::try_from(intent_ordinal)
                .expect("bounded intent ordinal")
                .saturating_add(1),
        )
        .expect("nonzero intent identity"),
        PmOwnedQuoteSlotKey::new(scope, instrument, side),
        client_order(scope.handle(), client_ordinal),
        price,
        quantity,
        reservation,
    )
    .expect("fixed owned quote intent")
}

fn client_order(account: reap_pm_core::PmAccountHandle, ordinal: usize) -> PmClientOrderKey {
    let mut bytes = [0_u8; 16];
    bytes[8..].copy_from_slice(
        &u64::try_from(ordinal)
            .expect("bounded client ordinal")
            .saturating_add(1)
            .to_be_bytes(),
    );
    PmClientOrderKey::new(
        account,
        PmClientOrderId::from_bytes(bytes).expect("nonzero client identity"),
    )
}

fn shuffled(count: usize, mut seed: u64) -> Vec<usize> {
    let mut ordinals = (0..count).collect::<Vec<_>>();
    for upper in (1..ordinals.len()).rev() {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        let index =
            usize::try_from(seed % u64::try_from(upper + 1).expect("bounded shuffle range"))
                .expect("shuffle index fits usize");
        ordinals.swap(upper, index);
    }
    ordinals
}
