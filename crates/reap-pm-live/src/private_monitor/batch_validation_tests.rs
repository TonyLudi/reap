use reap_pm_core::{
    PmAccountHandle, PmClientOrderId, PmClientOrderKey, PmFillEvent, PmFillExecution, PmFillFee,
    PmFillId, PmFillKey, PmFillRole, PmFillSettlementStatus, PmInstrumentHandle, PmOrderEvent,
    PmOrderIdentity, PmOrderProgress, PmOrderSide, PmOrderStatus, PmPrice, PmProductSource,
    PmQuantity, PmVenueOrderId, PmVenueOrderKey, U256,
};
use reap_polymarket_adapter::{
    MAX_PM_PRIVATE_NORMALIZED_OBSERVATIONS, PmPrivateLifecycleObservation,
};

use super::super::PmPrivateMonitorError;
use super::PmPrivateBatchIdentityScratch;
use crate::evidence::connectivity_config;

#[derive(Clone, Copy)]
struct Context {
    account: PmAccountHandle,
    instrument: PmInstrumentHandle,
    source: PmProductSource,
}

fn context() -> Context {
    let config = connectivity_config();
    Context {
        account: config.account().account_scope().handle(),
        instrument: config.account().instrument(),
        source: config.account().account_route().source(),
    }
}

fn venue_order(context: Context, id: &str) -> PmVenueOrderKey {
    PmVenueOrderKey::new(
        context.account,
        PmVenueOrderId::new(id).expect("valid venue order"),
    )
}

fn client_order(context: Context, id: u64) -> PmClientOrderKey {
    let mut bytes = [0_u8; 16];
    bytes[8..].copy_from_slice(&id.to_be_bytes());
    PmClientOrderKey::new(
        context.account,
        PmClientOrderId::from_bytes(bytes).expect("valid client order"),
    )
}

fn order(context: Context, client: u64, venue: &str) -> PmPrivateLifecycleObservation {
    PmPrivateLifecycleObservation::Order(
        PmOrderEvent::new(
            context.source,
            context.instrument,
            PmOrderIdentity::new(
                Some(client_order(context, client)),
                Some(venue_order(context, venue)),
            )
            .expect("valid order identity"),
            PmOrderSide::Buy,
            PmPrice::parse_decimal("0.40").expect("valid price"),
            PmOrderProgress::new(
                PmQuantity::parse_decimal("5").expect("valid quantity"),
                U256::ZERO,
                PmOrderStatus::Open,
            )
            .expect("valid progress"),
        )
        .expect("valid order event"),
    )
}

fn fill(context: Context, venue: &str, id: &str) -> PmPrivateLifecycleObservation {
    let venue = venue_order(context, venue);
    PmPrivateLifecycleObservation::Fill(
        PmFillEvent::new(
            context.source,
            context.instrument,
            PmFillKey::new(venue, PmFillId::new(id).expect("valid fill id")),
            PmOrderIdentity::new(None, Some(venue)).expect("valid fill order"),
            PmFillExecution::new(
                PmOrderSide::Buy,
                PmFillRole::Maker,
                PmFillSettlementStatus::Matched,
                PmPrice::parse_decimal("0.40").expect("valid price"),
                PmQuantity::parse_decimal("5").expect("valid quantity"),
                PmFillFee::Unknown,
            ),
        )
        .expect("valid fill event"),
    )
}

#[test]
fn duplicate_client_venue_and_fill_identities_remain_fail_closed() {
    let context = context();
    let mut scratch = PmPrivateBatchIdentityScratch::new();
    for observations in [
        [order(context, 1, "client-a"), order(context, 1, "client-b")],
        [order(context, 1, "venue-a"), order(context, 2, "venue-a")],
        [
            fill(context, "fill-a", "same-fill"),
            fill(context, "fill-a", "same-fill"),
        ],
    ] {
        assert!(matches!(
            scratch.validate(&observations),
            Err(PmPrivateMonitorError::DuplicateBatchIdentity)
        ));
    }

    assert!(
        scratch
            .validate(&[
                fill(context, "fill-a", "shared-id"),
                fill(context, "fill-b", "shared-id"),
            ])
            .is_ok(),
        "fill identity includes its exact venue order"
    );
}

#[test]
fn exact_capacity_is_reused_and_oversize_is_rejected_before_push() {
    let context = context();
    let mut scratch = PmPrivateBatchIdentityScratch::new();
    let initial_capacities = (
        scratch.client_orders.capacity(),
        scratch.venue_orders.capacity(),
        scratch.fills.capacity(),
        scratch.unresolved.capacity(),
    );
    assert!(
        [
            initial_capacities.0,
            initial_capacities.1,
            initial_capacities.2,
            initial_capacities.3,
        ]
        .into_iter()
        .all(|capacity| capacity >= MAX_PM_PRIVATE_NORMALIZED_OBSERVATIONS)
    );

    let observations = (1..=MAX_PM_PRIVATE_NORMALIZED_OBSERVATIONS)
        .map(|index| order(context, index as u64, &format!("max-order-{index}")))
        .collect::<Vec<_>>();
    scratch
        .validate(&observations)
        .expect("the exact normalized maximum is accepted");
    assert_eq!(scratch.client_orders.len(), observations.len());
    assert_eq!(scratch.venue_orders.len(), observations.len());
    assert_eq!(
        (
            scratch.client_orders.capacity(),
            scratch.venue_orders.capacity(),
            scratch.fills.capacity(),
            scratch.unresolved.capacity(),
        ),
        initial_capacities
    );

    let fills = (1..=MAX_PM_PRIVATE_NORMALIZED_OBSERVATIONS)
        .map(|index| {
            fill(
                context,
                &format!("max-fill-order-{index}"),
                &format!("max-fill-{index}"),
            )
        })
        .collect::<Vec<_>>();
    scratch
        .validate(&fills)
        .expect("the exact normalized fill maximum is accepted");
    assert_eq!(scratch.client_orders.len(), 0);
    assert_eq!(scratch.venue_orders.len(), 0);
    assert_eq!(scratch.fills.len(), fills.len());
    assert_eq!(
        (
            scratch.client_orders.capacity(),
            scratch.venue_orders.capacity(),
            scratch.fills.capacity(),
            scratch.unresolved.capacity(),
        ),
        initial_capacities
    );

    scratch
        .validate(&[fill(context, "reuse-order", "reuse-fill")])
        .expect("a smaller next batch reuses the same storage");
    assert_eq!(scratch.client_orders.len(), 0);
    assert_eq!(scratch.venue_orders.len(), 0);
    assert_eq!(scratch.fills.len(), 1);

    let mut fresh = PmPrivateBatchIdentityScratch::new();
    let oversize = vec![
        fill(context, "oversize-order", "oversize-fill");
        MAX_PM_PRIVATE_NORMALIZED_OBSERVATIONS + 1
    ];
    let fresh_capacities = (
        fresh.client_orders.capacity(),
        fresh.venue_orders.capacity(),
        fresh.fills.capacity(),
        fresh.unresolved.capacity(),
    );
    assert!(matches!(
        fresh.validate(&oversize),
        Err(PmPrivateMonitorError::BatchCounterOverflow)
    ));
    assert_eq!(
        (
            fresh.client_orders.len(),
            fresh.venue_orders.len(),
            fresh.fills.len(),
            fresh.unresolved.len(),
        ),
        (0, 0, 0, 0),
        "the hard bound is checked before scratch mutation"
    );
    assert_eq!(
        (
            fresh.client_orders.capacity(),
            fresh.venue_orders.capacity(),
            fresh.fills.capacity(),
            fresh.unresolved.capacity(),
        ),
        fresh_capacities
    );
    fresh
        .validate(&[fill(context, "after-bound-order", "after-bound-fill")])
        .expect("an oversize rejection leaves scratch reusable");
}
