use reap_pm_core::{PM_PROTOCOL_SCALE, PmMarketMetadata};

use crate::readiness::{
    PmMetadataContract, PmMetadataDrift, PmProtocolProfile, PmPublicReadinessReason, PmUnitContract,
};

pub(super) fn validate_expected_contract(
    contract: PmMetadataContract,
) -> Result<(), PmPublicReadinessReason> {
    if contract.protocol() != PmProtocolProfile::ClobV2 {
        return Err(PmPublicReadinessReason::MetadataDrift(
            PmMetadataDrift::Protocol,
        ));
    }
    let expected_units = PmUnitContract::goal_f_clob_v2();
    if contract.units().lot_units() != expected_units.lot_units() {
        return Err(PmPublicReadinessReason::MetadataDrift(PmMetadataDrift::Lot));
    }
    if contract.units().price_units_per_one() != PM_PROTOCOL_SCALE
        || contract.units().quantity_units_per_one() != expected_units.quantity_units_per_one()
        || contract.units().collateral_units_per_one() != expected_units.collateral_units_per_one()
    {
        return Err(PmPublicReadinessReason::MetadataDrift(
            PmMetadataDrift::Units,
        ));
    }
    validate_lifecycle(contract.market())
}

pub(super) fn validate_observed_contract(
    expected: PmMetadataContract,
    actual: PmMetadataContract,
) -> Result<(), PmPublicReadinessReason> {
    if actual.protocol() != expected.protocol() {
        return Err(PmPublicReadinessReason::MetadataDrift(
            PmMetadataDrift::Protocol,
        ));
    }
    if actual.units().lot_units() != expected.units().lot_units() {
        return Err(PmPublicReadinessReason::MetadataDrift(PmMetadataDrift::Lot));
    }
    if actual.units().price_units_per_one() != expected.units().price_units_per_one()
        || actual.units().quantity_units_per_one() != expected.units().quantity_units_per_one()
        || actual.units().collateral_units_per_one() != expected.units().collateral_units_per_one()
    {
        return Err(PmPublicReadinessReason::MetadataDrift(
            PmMetadataDrift::Units,
        ));
    }

    let expected_market = expected.market();
    let actual_market = actual.market();
    if actual_market.condition() != expected_market.condition()
        || actual_market.market() != expected_market.market()
        || actual_market.outcome().token() != expected_market.outcome().token()
    {
        return Err(PmPublicReadinessReason::MetadataDrift(
            PmMetadataDrift::Identity,
        ));
    }
    if actual_market.outcome().label() != expected_market.outcome().label() {
        return Err(PmPublicReadinessReason::MetadataDrift(
            PmMetadataDrift::OutcomeLabel,
        ));
    }
    validate_lifecycle(actual_market)?;
    if actual_market.tick() != expected_market.tick() {
        return Err(PmPublicReadinessReason::MetadataDrift(
            PmMetadataDrift::Grid,
        ));
    }
    if actual_market.minimum_order_size() != expected_market.minimum_order_size() {
        return Err(PmPublicReadinessReason::MetadataDrift(
            PmMetadataDrift::Minimum,
        ));
    }
    if actual_market.negative_risk() != expected_market.negative_risk() {
        return Err(PmPublicReadinessReason::MetadataDrift(
            PmMetadataDrift::NegativeRisk,
        ));
    }
    if actual.domain() != expected.domain()
        || actual_market.chain() != expected_market.chain()
        || actual_market.exchange() != expected_market.exchange()
    {
        return Err(PmPublicReadinessReason::MetadataDrift(
            PmMetadataDrift::Domain,
        ));
    }
    if !actual_market
        .required_spenders()
        .eq(expected_market.required_spenders())
    {
        return Err(PmPublicReadinessReason::MetadataDrift(
            PmMetadataDrift::RequiredSpenders,
        ));
    }
    Ok(())
}

fn validate_lifecycle(metadata: PmMarketMetadata) -> Result<(), PmPublicReadinessReason> {
    let lifecycle = metadata.lifecycle();
    if !lifecycle.active() {
        return Err(PmPublicReadinessReason::MarketInactive);
    }
    if lifecycle.closed() {
        return Err(PmPublicReadinessReason::MarketClosed);
    }
    if lifecycle.archived() {
        return Err(PmPublicReadinessReason::MarketArchived);
    }
    if !lifecycle.accepting_orders() {
        return Err(PmPublicReadinessReason::OrdersNotAccepted);
    }
    if !lifecycle.order_book_enabled() {
        return Err(PmPublicReadinessReason::OrderBookDisabled);
    }
    Ok(())
}
