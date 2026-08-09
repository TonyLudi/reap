use reap_pm_core::{
    EvmAddress, PmAssetId, PmFillExecution, PmFillFee, PmFillRole, PmFillSettlementStatus,
    PmOrderIdentity, PmOrderSide, PmPrice, PmQuantity, PmSign, PmSignedUnits, PmVenueOrderId,
    PmVenueOrderKey, U256, exact_order_amounts,
};
use reap_polymarket_wire::{PmLiveMakerOrder, PmLiveTrade};
use sha2::Digest;

use crate::live_diagnostics::{encode_ascii, semantic_hash};
use crate::{PmAccountFillLeg, PmAccountFillLegKey, PmUnresolvedTradeReason};

use super::{LiveNormalizationScope, PmLiveNormalizationError};

const TRADE_LEG_KEY_DOMAIN: &[u8] = b"reap.pm.live.trade-leg-key.v1\0";
const TRADE_LEG_FACTS_DOMAIN: &[u8] = b"reap.pm.live.trade-leg-facts.v1\0";
const UNRESOLVED_TRADE_KEY_DOMAIN: &[u8] = b"reap.pm.live.unresolved-trade-key.v1\0";
const UNRESOLVED_TRADE_FACTS_DOMAIN: &[u8] = b"reap.pm.live.unresolved-trade-facts.v1\0";

#[derive(Debug, Clone, Copy)]
pub(crate) struct LiveTradeCandidate {
    pub(crate) key: PmAccountFillLegKey,
    pub(crate) key_digest: [u8; 32],
    pub(crate) facts_digest: [u8; 32],
    pub(crate) leg: Option<PmAccountFillLeg>,
}

#[derive(Debug)]
pub(crate) struct NormalizedTrade {
    pub(crate) candidates: Vec<LiveTradeCandidate>,
    pub(crate) unresolved: Option<PmUnresolvedTradeReason>,
    pub(crate) relevant_to_configured: bool,
    pub(crate) settlement: PmFillSettlementStatus,
    pub(crate) unresolved_diagnostic: Option<([u8; 32], [u8; 32])>,
}

pub(crate) fn normalize_trade(
    scope: LiveNormalizationScope,
    trade: &PmLiveTrade,
) -> Result<NormalizedTrade, PmLiveNormalizationError> {
    let expected_maker = scope.expected_order_maker();
    let settlement = parse_trade_status(trade.status())?;
    if scope.is_configured(trade.condition(), trade.token()) {
        validate_configured_fill(scope, trade.side(), trade.price(), trade.size())?;
    }
    match trade.trader_side() {
        Some("TAKER") => normalize_taker_trade(scope, trade, settlement),
        Some("MAKER") => normalize_maker_trade(scope, trade, settlement, expected_maker),
        _ => Ok(NormalizedTrade {
            candidates: Vec::new(),
            unresolved: Some(PmUnresolvedTradeReason::MissingDirectOrderRole),
            relevant_to_configured: scope.is_configured(trade.condition(), trade.token()),
            settlement,
            unresolved_diagnostic: Some(unresolved_trade_digests(trade)),
        }),
    }
}

fn normalize_taker_trade(
    scope: LiveNormalizationScope,
    trade: &PmLiveTrade,
    settlement: PmFillSettlementStatus,
) -> Result<NormalizedTrade, PmLiveNormalizationError> {
    let order = match (trade.order_id(), trade.taker_order_id()) {
        (Some(order), None) | (None, Some(order)) => order,
        (None, None) => {
            return Ok(NormalizedTrade {
                candidates: Vec::new(),
                unresolved: Some(PmUnresolvedTradeReason::MissingExactOrderLinkage),
                relevant_to_configured: scope.is_configured(trade.condition(), trade.token()),
                settlement,
                unresolved_diagnostic: Some(unresolved_trade_digests(trade)),
            });
        }
        (Some(_), Some(_)) => {
            return Ok(NormalizedTrade {
                candidates: Vec::new(),
                unresolved: Some(PmUnresolvedTradeReason::MultipleOrderReferenceKinds),
                relevant_to_configured: scope.is_configured(trade.condition(), trade.token()),
                settlement,
                unresolved_diagnostic: Some(unresolved_trade_digests(trade)),
            });
        }
    };
    let leg = make_fill_leg(
        scope,
        trade.id(),
        order,
        trade.condition(),
        trade.token(),
        trade.side(),
        PmFillRole::Taker,
        settlement,
        trade.price(),
        trade.size(),
        trade.fee_rate_bps(),
    )?;
    let mut candidates = Vec::with_capacity(1 + trade.maker_orders().len());
    candidates.push(candidate_for_leg(leg, None));
    candidates.extend(
        trade
            .maker_orders()
            .iter()
            .map(|maker| candidate_for_foreign_maker(trade, maker, settlement)),
    );
    Ok(NormalizedTrade {
        candidates,
        unresolved: None,
        relevant_to_configured: scope.is_configured(trade.condition(), trade.token()),
        settlement,
        unresolved_diagnostic: None,
    })
}

fn normalize_maker_trade(
    scope: LiveNormalizationScope,
    trade: &PmLiveTrade,
    settlement: PmFillSettlementStatus,
    expected_maker: EvmAddress,
) -> Result<NormalizedTrade, PmLiveNormalizationError> {
    if trade.maker_orders().is_empty() {
        return Ok(NormalizedTrade {
            candidates: Vec::new(),
            unresolved: Some(PmUnresolvedTradeReason::MissingLocalMakerOrderProof),
            relevant_to_configured: scope.is_configured(trade.condition(), trade.token()),
            settlement,
            unresolved_diagnostic: Some(unresolved_trade_digests(trade)),
        });
    }
    let mut candidates = Vec::with_capacity(trade.maker_orders().len());
    let mut relevant_to_configured = false;
    let mut local_count = 0;
    for maker in trade.maker_orders() {
        let configured = scope.is_configured(trade.condition(), maker.token());
        relevant_to_configured |= configured;
        if maker.maker() == expected_maker {
            let leg = make_maker_leg(scope, trade, maker, settlement)?;
            candidates.push(candidate_for_leg(leg, Some(maker.maker())));
            local_count += 1;
        } else {
            candidates.push(candidate_for_foreign_maker(trade, maker, settlement));
        }
    }
    Ok(NormalizedTrade {
        candidates,
        unresolved: (local_count == 0)
            .then_some(PmUnresolvedTradeReason::MissingLocalMakerOrderProof),
        relevant_to_configured,
        settlement,
        unresolved_diagnostic: (local_count == 0).then(|| unresolved_trade_digests(trade)),
    })
}

fn make_maker_leg(
    scope: LiveNormalizationScope,
    trade: &PmLiveTrade,
    maker: &PmLiveMakerOrder,
    settlement: PmFillSettlementStatus,
) -> Result<PmAccountFillLeg, PmLiveNormalizationError> {
    make_fill_leg(
        scope,
        trade.id(),
        maker.order_id(),
        trade.condition(),
        maker.token(),
        maker.side(),
        PmFillRole::Maker,
        settlement,
        maker.price(),
        maker.matched_amount(),
        maker.fee_rate_bps(),
    )
}

#[allow(clippy::too_many_arguments)]
fn make_fill_leg(
    scope: LiveNormalizationScope,
    fill: reap_pm_core::PmFillId,
    venue_order: PmVenueOrderId,
    condition: reap_pm_core::PmConditionId,
    token: reap_pm_core::PmTokenId,
    side: PmOrderSide,
    role: PmFillRole,
    settlement: PmFillSettlementStatus,
    price: PmPrice,
    quantity: PmQuantity,
    fee_rate_bps: Option<U256>,
) -> Result<PmAccountFillLeg, PmLiveNormalizationError> {
    exact_order_amounts(side, price, quantity)
        .map_err(|_| PmLiveNormalizationError::NonIntegralProtocolAmounts)?;
    if scope.is_configured(condition, token) {
        validate_configured_fill(scope, side, price, quantity)?;
    }
    Ok(PmAccountFillLeg::new(
        PmAccountFillLegKey::new(venue_order, fill),
        condition,
        token,
        side,
        role,
        settlement,
        price,
        quantity,
        normalize_fee(scope, condition, token, fee_rate_bps),
    ))
}

fn normalize_fee(
    scope: LiveNormalizationScope,
    condition: reap_pm_core::PmConditionId,
    token: reap_pm_core::PmTokenId,
    fee_rate_bps: Option<U256>,
) -> PmFillFee {
    if !scope.is_configured(condition, token) {
        return PmFillFee::Unknown;
    }
    match fee_rate_bps {
        None => PmFillFee::Unknown,
        Some(rate) if rate.is_zero() => PmFillFee::Known {
            asset: scope.instrument.trading_domain().collateral(),
            delta: PmSignedUnits::ZERO,
        },
        Some(_) => PmFillFee::Incomplete,
    }
}

fn encode_fee_evidence(digest: &mut sha2::Sha256, fee: PmFillFee) {
    match fee {
        PmFillFee::Unknown => {}
        PmFillFee::Incomplete => digest.update([1]),
        PmFillFee::Known { asset, delta } => {
            digest.update([2]);
            match asset {
                PmAssetId::Collateral { contract } => {
                    digest.update([0]);
                    digest.update(contract.bytes());
                }
                PmAssetId::Outcome { contract, token } => {
                    digest.update([1]);
                    digest.update(contract.bytes());
                    digest.update(token.units().to_be_bytes());
                }
            }
            digest.update([match delta.sign() {
                PmSign::Positive => 0,
                PmSign::Negative => 1,
            }]);
            digest.update(delta.magnitude().to_be_bytes());
        }
    }
}

fn validate_configured_fill(
    scope: LiveNormalizationScope,
    side: PmOrderSide,
    price: PmPrice,
    quantity: PmQuantity,
) -> Result<(), PmLiveNormalizationError> {
    price
        .validate_tick(scope.instrument.tick())
        .map_err(|_| PmLiveNormalizationError::PriceOffTick)?;
    exact_order_amounts(side, price, quantity)
        .map_err(|_| PmLiveNormalizationError::NonIntegralProtocolAmounts)?;
    Ok(())
}

fn candidate_for_leg(leg: PmAccountFillLeg, maker: Option<EvmAddress>) -> LiveTradeCandidate {
    LiveTradeCandidate {
        key: leg.key(),
        key_digest: trade_leg_key_digest(leg.key()),
        facts_digest: trade_leg_facts_digest(leg, maker),
        leg: Some(leg),
    }
}

fn candidate_for_foreign_maker(
    trade: &PmLiveTrade,
    maker: &PmLiveMakerOrder,
    settlement: PmFillSettlementStatus,
) -> LiveTradeCandidate {
    let key = PmAccountFillLegKey::new(maker.order_id(), trade.id());
    let synthetic = PmAccountFillLeg::new(
        key,
        trade.condition(),
        maker.token(),
        maker.side(),
        PmFillRole::Maker,
        settlement,
        maker.price(),
        maker.matched_amount(),
        PmFillFee::Unknown,
    );
    LiveTradeCandidate {
        key,
        key_digest: trade_leg_key_digest(key),
        facts_digest: trade_leg_facts_digest(synthetic, Some(maker.maker())),
        leg: None,
    }
}

pub(crate) fn fill_event_from_leg(
    scope: LiveNormalizationScope,
    leg: PmAccountFillLeg,
) -> Result<reap_pm_core::PmFillEvent, PmLiveNormalizationError> {
    debug_assert!(scope.is_configured(leg.condition(), leg.token()));
    let venue_order = PmVenueOrderKey::new(scope.account.handle(), leg.key().venue_order());
    let identity = PmOrderIdentity::new(None, Some(venue_order))
        .map_err(|_| PmLiveNormalizationError::EventContract)?;
    reap_pm_core::PmFillEvent::new(
        scope.source,
        scope.instrument.handle(),
        reap_pm_core::PmFillKey::new(venue_order, leg.key().fill()),
        identity,
        PmFillExecution::new(
            leg.side(),
            leg.role(),
            leg.settlement(),
            leg.price(),
            leg.quantity(),
            leg.fee(),
        ),
    )
    .map_err(|_| PmLiveNormalizationError::EventContract)
}

fn parse_trade_status(status: &str) -> Result<PmFillSettlementStatus, PmLiveNormalizationError> {
    match status {
        "MATCHED_NOT_BROADCASTED" => Ok(PmFillSettlementStatus::MatchedNotBroadcasted),
        "MATCHED" => Ok(PmFillSettlementStatus::Matched),
        "MINED" => Ok(PmFillSettlementStatus::Mined),
        "CONFIRMED" => Ok(PmFillSettlementStatus::Confirmed),
        "RETRYING" => Ok(PmFillSettlementStatus::Retrying),
        "FAILED" => Ok(PmFillSettlementStatus::Failed),
        _ => Err(PmLiveNormalizationError::UnknownTradeStatus),
    }
}

fn trade_leg_key_digest(key: PmAccountFillLegKey) -> [u8; 32] {
    semantic_hash(TRADE_LEG_KEY_DOMAIN, |digest| {
        encode_ascii(digest, key.venue_order().as_str());
        encode_ascii(digest, key.fill().as_str());
    })
}

fn trade_leg_facts_digest(leg: PmAccountFillLeg, maker: Option<EvmAddress>) -> [u8; 32] {
    semantic_hash(TRADE_LEG_FACTS_DOMAIN, |digest| {
        digest.update(leg.condition().bytes());
        digest.update(leg.token().units().to_be_bytes());
        digest.update([side_tag(leg.side())]);
        digest.update([match leg.role() {
            PmFillRole::Maker => 0,
            PmFillRole::Taker => 1,
        }]);
        digest.update([match leg.settlement() {
            PmFillSettlementStatus::Matched => 0,
            PmFillSettlementStatus::Mined => 1,
            PmFillSettlementStatus::Confirmed => 2,
            PmFillSettlementStatus::Retrying => 3,
            PmFillSettlementStatus::Failed => 4,
            PmFillSettlementStatus::MatchedNotBroadcasted => 5,
        }]);
        digest.update(leg.price().units().to_be_bytes());
        digest.update(leg.quantity().protocol_units().to_be_bytes());
        match maker {
            None => digest.update([0]),
            Some(maker) => {
                digest.update([1]);
                digest.update(maker.bytes());
            }
        }
        // Unknown appends nothing to preserve the established omitted-fee
        // facts digest. Explicit partial and known evidence get disjoint tags.
        encode_fee_evidence(digest, leg.fee());
    })
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

fn unresolved_trade_digests(trade: &PmLiveTrade) -> ([u8; 32], [u8; 32]) {
    let key = semantic_hash(UNRESOLVED_TRADE_KEY_DOMAIN, |digest| {
        encode_ascii(digest, trade.id().as_str());
    });
    let facts = semantic_hash(UNRESOLVED_TRADE_FACTS_DOMAIN, |digest| {
        digest.update(trade.condition().bytes());
        digest.update(trade.token().units().to_be_bytes());
        digest.update([side_tag(trade.side())]);
        digest.update(trade.price().units().to_be_bytes());
        digest.update(trade.size().protocol_units().to_be_bytes());
        encode_ascii(digest, trade.status());
        encode_optional_ascii(digest, trade.trader_side());
        match trade.order_id() {
            None => digest.update([0]),
            Some(order) => {
                digest.update([1]);
                encode_ascii(digest, order.as_str());
            }
        }
        match trade.taker_order_id() {
            None => digest.update([0]),
            Some(order) => {
                digest.update([1]);
                encode_ascii(digest, order.as_str());
            }
        }
    });
    (key, facts)
}
