use reap_pm_core::{
    PmBookQuantity, PmOrderEvent, PmOrderIdentity, PmOrderProgress, PmOrderSide, PmOrderStatus,
    PmPrice, PmQuantity, PmVenueOrderId, PmVenueOrderKey, U256, exact_order_amounts,
};
use reap_polymarket_wire::{PmLiveOrder, PmLiveUserOrder};
use sha2::Digest;

use crate::live_diagnostics::{encode_ascii, semantic_hash};

use super::{LiveNormalizationScope, PmLiveNormalizationError};

const ORDER_KEY_DOMAIN: &[u8] = b"reap.pm.live.order-key.v1\0";
const ORDER_FACTS_DOMAIN: &[u8] = b"reap.pm.live.order-facts.v1\0";
const USER_ORDER_FACTS_DOMAIN: &[u8] = b"reap.pm.live.user-order-facts.v1\0";

#[derive(Debug, Clone, Copy)]
pub(crate) struct NormalizedOrderRow {
    pub(crate) venue_order: PmVenueOrderId,
    pub(crate) key_digest: [u8; 32],
    pub(crate) facts_digest: [u8; 32],
    pub(crate) configured: Option<PmOrderEvent>,
}

pub(crate) fn normalize_rest_order(
    scope: LiveNormalizationScope,
    order: &PmLiveOrder,
    require_open: bool,
) -> Result<NormalizedOrderRow, PmLiveNormalizationError> {
    let expected_maker = scope.expected_order_maker();
    let key_digest = order_key_digest(order.id());
    let facts_digest = rest_order_facts_digest(order);
    if !scope.is_configured(order.condition(), order.token()) {
        return Ok(NormalizedOrderRow {
            venue_order: order.id(),
            key_digest,
            facts_digest,
            configured: None,
        });
    }
    if order.maker() != expected_maker {
        return Err(PmLiveNormalizationError::AccountProfileMismatch);
    }
    if order
        .order_type()
        .is_some_and(|order_type| order_type != "GTC")
    {
        return Err(PmLiveNormalizationError::UnsupportedOrderType);
    }
    if order.expiration() != 0 {
        return Err(PmLiveNormalizationError::UnexpectedExpiration);
    }
    if order
        .outcome()
        .is_some_and(|outcome| outcome != scope.instrument.metadata().outcome().label().as_str())
    {
        return Err(PmLiveNormalizationError::OutcomeMismatch);
    }
    let cumulative = book_quantity_units(order.size_matched());
    let status = parse_rest_order_status(order.status(), cumulative)?;
    if require_open && status.is_terminal() {
        return Err(PmLiveNormalizationError::OpenOrderIsTerminal);
    }
    let event = make_order_event(
        scope,
        order.id(),
        order.side(),
        order.price(),
        order.original_size(),
        cumulative,
        status,
    )?;
    Ok(NormalizedOrderRow {
        venue_order: order.id(),
        key_digest,
        facts_digest,
        configured: Some(event),
    })
}

pub(crate) fn normalize_user_order(
    scope: LiveNormalizationScope,
    order: &PmLiveUserOrder,
) -> Result<NormalizedOrderRow, PmLiveNormalizationError> {
    let key_digest = order_key_digest(order.id());
    let facts_digest = user_order_facts_digest(order);
    if !scope.is_configured(order.condition(), order.token()) {
        return Ok(NormalizedOrderRow {
            venue_order: order.id(),
            key_digest,
            facts_digest,
            configured: None,
        });
    }
    let maker = order
        .maker()
        .ok_or(PmLiveNormalizationError::MissingUserOrderProfileFact(
            "maker_address",
        ))?;
    let order_type =
        order
            .order_type()
            .ok_or(PmLiveNormalizationError::MissingUserOrderProfileFact(
                "order_type",
            ))?;
    let expiration =
        order
            .expiration()
            .ok_or(PmLiveNormalizationError::MissingUserOrderProfileFact(
                "expiration",
            ))?;
    let outcome = order
        .outcome()
        .ok_or(PmLiveNormalizationError::MissingUserOrderProfileFact(
            "outcome",
        ))?;
    let wire_status =
        order
            .status()
            .ok_or(PmLiveNormalizationError::MissingUserOrderProfileFact(
                "status",
            ))?;
    if maker != scope.expected_order_maker() {
        return Err(PmLiveNormalizationError::AccountProfileMismatch);
    }
    if order_type != "GTC" {
        return Err(PmLiveNormalizationError::UnsupportedOrderType);
    }
    if expiration != 0 {
        return Err(PmLiveNormalizationError::UnexpectedExpiration);
    }
    if outcome != scope.instrument.metadata().outcome().label().as_str() {
        return Err(PmLiveNormalizationError::OutcomeMismatch);
    }
    let cumulative = book_quantity_units(order.size_matched());
    let original_units = order.original_size().protocol_units();
    let status = match order.event_kind() {
        "PLACEMENT" if cumulative.is_zero() => PmOrderStatus::Open,
        "PLACEMENT" => {
            return Err(PmLiveNormalizationError::UserOrderKindProgressMismatch);
        }
        "UPDATE" if cumulative.is_zero() => {
            return Err(PmLiveNormalizationError::UserOrderKindProgressMismatch);
        }
        "UPDATE" if cumulative < original_units => PmOrderStatus::PartiallyFilled,
        "UPDATE" if cumulative == original_units => PmOrderStatus::Filled,
        "UPDATE" => return Err(PmLiveNormalizationError::InvalidOrderProgress),
        "CANCELLATION" if cumulative <= original_units => PmOrderStatus::Cancelled,
        "CANCELLATION" => return Err(PmLiveNormalizationError::InvalidOrderProgress),
        _ => return Err(PmLiveNormalizationError::UnknownUserOrderKind),
    };
    let status_matches = matches!(
        (order.event_kind(), wire_status, status),
        ("PLACEMENT", "LIVE", PmOrderStatus::Open)
            | ("UPDATE", "LIVE", PmOrderStatus::PartiallyFilled)
            | ("UPDATE", "MATCHED", PmOrderStatus::Filled)
            | ("CANCELLATION", "CANCELED", PmOrderStatus::Cancelled)
    );
    if !status_matches {
        return Err(PmLiveNormalizationError::UserOrderStatusProgressMismatch);
    }
    let event = make_order_event(
        scope,
        order.id(),
        order.side(),
        order.price(),
        order.original_size(),
        cumulative,
        status,
    )?;
    Ok(NormalizedOrderRow {
        venue_order: order.id(),
        key_digest,
        facts_digest,
        configured: Some(event),
    })
}

fn make_order_event(
    scope: LiveNormalizationScope,
    venue_order: PmVenueOrderId,
    side: PmOrderSide,
    price: PmPrice,
    original: PmQuantity,
    cumulative: U256,
    status: PmOrderStatus,
) -> Result<PmOrderEvent, PmLiveNormalizationError> {
    validate_configured_order(scope, side, price, original)?;
    let progress = PmOrderProgress::new(original, cumulative, status)
        .map_err(|_| PmLiveNormalizationError::InvalidOrderProgress)?;
    let venue_order = PmVenueOrderKey::new(scope.account.handle(), venue_order);
    let identity = PmOrderIdentity::new(None, Some(venue_order))
        .map_err(|_| PmLiveNormalizationError::EventContract)?;
    PmOrderEvent::new(
        scope.source,
        scope.instrument.handle(),
        identity,
        side,
        price,
        progress,
    )
    .map_err(|_| PmLiveNormalizationError::EventContract)
}

fn validate_configured_order(
    scope: LiveNormalizationScope,
    side: PmOrderSide,
    price: PmPrice,
    quantity: PmQuantity,
) -> Result<(), PmLiveNormalizationError> {
    price
        .validate_tick(scope.instrument.tick())
        .map_err(|_| PmLiveNormalizationError::PriceOffTick)?;
    quantity
        .validate_order(scope.instrument.minimum_order_size())
        .map_err(|_| PmLiveNormalizationError::InvalidOrderQuantityContract)?;
    exact_order_amounts(side, price, quantity)
        .map_err(|_| PmLiveNormalizationError::NonIntegralProtocolAmounts)?;
    Ok(())
}

fn parse_rest_order_status(
    status: &str,
    cumulative: U256,
) -> Result<PmOrderStatus, PmLiveNormalizationError> {
    match status {
        "LIVE" | "ORDER_STATUS_LIVE" if cumulative.is_zero() => Ok(PmOrderStatus::Open),
        "LIVE" | "ORDER_STATUS_LIVE" => Ok(PmOrderStatus::PartiallyFilled),
        "MATCHED" | "ORDER_STATUS_MATCHED" => Ok(PmOrderStatus::Filled),
        "CANCELED" | "ORDER_STATUS_CANCELED" | "CANCELLED" | "ORDER_STATUS_CANCELLED" => {
            Ok(PmOrderStatus::Cancelled)
        }
        "EXPIRED" | "ORDER_STATUS_EXPIRED" => Ok(PmOrderStatus::Expired),
        "INVALID" | "REJECTED" | "ORDER_STATUS_INVALID" | "ORDER_STATUS_REJECTED" => {
            Ok(PmOrderStatus::Rejected)
        }
        _ => Err(PmLiveNormalizationError::UnknownOrderStatus),
    }
}

fn book_quantity_units(quantity: PmBookQuantity) -> U256 {
    match quantity {
        PmBookQuantity::Delete => U256::ZERO,
        PmBookQuantity::Quantity(quantity) => quantity.protocol_units(),
    }
}

fn order_key_digest(order: PmVenueOrderId) -> [u8; 32] {
    semantic_hash(ORDER_KEY_DOMAIN, |digest| {
        encode_ascii(digest, order.as_str());
    })
}

fn rest_order_facts_digest(order: &PmLiveOrder) -> [u8; 32] {
    semantic_hash(ORDER_FACTS_DOMAIN, |digest| {
        digest.update(order.condition().bytes());
        digest.update(order.token().units().to_be_bytes());
        encode_order_execution(
            digest,
            order.side(),
            order.price(),
            order.original_size(),
            order.size_matched(),
        );
        encode_ascii(digest, order.status());
        digest.update(order.maker().bytes());
        digest.update(order.expiration().to_be_bytes());
        encode_optional_ascii(digest, order.outcome());
        encode_optional_ascii(digest, order.order_type());
    })
}

fn user_order_facts_digest(order: &PmLiveUserOrder) -> [u8; 32] {
    semantic_hash(USER_ORDER_FACTS_DOMAIN, |digest| {
        digest.update(order.condition().bytes());
        digest.update(order.token().units().to_be_bytes());
        encode_order_execution(
            digest,
            order.side(),
            order.price(),
            order.original_size(),
            order.size_matched(),
        );
        encode_ascii(digest, order.event_kind());
        match order.maker() {
            Some(maker) => {
                digest.update([1]);
                digest.update(maker.bytes());
            }
            None => digest.update([0]),
        }
        match order.expiration() {
            Some(expiration) => {
                digest.update([1]);
                digest.update(expiration.to_be_bytes());
            }
            None => digest.update([0]),
        }
        encode_optional_ascii(digest, order.order_type());
        encode_optional_ascii(digest, order.outcome());
        encode_optional_ascii(digest, order.status());
        match order.created_at() {
            Some(created_at) => {
                digest.update([1]);
                digest.update(created_at.to_be_bytes());
            }
            None => digest.update([0]),
        }
        if let Some(trades) = order.associate_trades() {
            digest.update([1]);
            for trade in trades {
                encode_ascii(digest, trade.as_str());
            }
        } else {
            digest.update([0]);
        }
    })
}

fn encode_order_execution(
    digest: &mut sha2::Sha256,
    side: PmOrderSide,
    price: PmPrice,
    original: PmQuantity,
    cumulative: PmBookQuantity,
) {
    digest.update([side_tag(side)]);
    digest.update(price.units().to_be_bytes());
    digest.update(original.protocol_units().to_be_bytes());
    digest.update(book_quantity_units(cumulative).to_be_bytes());
}

fn side_tag(side: PmOrderSide) -> u8 {
    match side {
        PmOrderSide::Buy => 0,
        PmOrderSide::Sell => 1,
    }
}

fn encode_optional_ascii(digest: &mut sha2::Sha256, value: Option<&str>) {
    match value {
        None => digest.update([0]),
        Some(value) => {
            digest.update([1]);
            encode_ascii(digest, value);
        }
    }
}
