use std::collections::BTreeSet;

use reap_core::Side;
use reap_strategy::{InstrumentConfig, InstrumentKindConfig};
use reap_venue::okx::{
    OkxBill, OkxBillExecutionType, OkxBillMarginMode, OkxInstrumentType, OkxTradeMode,
};

use super::{
    EconomicIssue, EconomicIssueSource, EconomicReconciliationFailure, IssueSink, LiveConfig,
};

pub(super) fn instrument<'a>(config: &'a LiveConfig, symbol: &str) -> Option<&'a InstrumentConfig> {
    config
        .strategy
        .instruments
        .iter()
        .find(|instrument| instrument.symbol == symbol)
}

pub(super) fn instrument_type(kind: InstrumentKindConfig) -> OkxInstrumentType {
    match kind {
        InstrumentKindConfig::Spot => OkxInstrumentType::Spot,
        InstrumentKindConfig::Future
        | InstrumentKindConfig::LinearFuture
        | InstrumentKindConfig::InverseFuture => OkxInstrumentType::Futures,
        InstrumentKindConfig::LinearSwap | InstrumentKindConfig::InverseSwap => {
            OkxInstrumentType::Swap
        }
    }
}

pub(super) fn expected_bill_margin_mode(
    config: &LiveConfig,
    account_id: &str,
    symbol: &str,
) -> Option<OkxBillMarginMode> {
    let account = config.account(account_id)?;
    match account.trade_mode(symbol)? {
        OkxTradeMode::Cash => Some(OkxBillMarginMode::Cash),
        OkxTradeMode::Cross => Some(OkxBillMarginMode::Cross),
        OkxTradeMode::Isolated => Some(OkxBillMarginMode::Isolated),
    }
}

pub(super) fn trade_subtype_side(sub_type: &str) -> Option<Side> {
    match sub_type {
        "1" | "3" | "6" => Some(Side::Buy),
        "2" | "4" | "5" => Some(Side::Sell),
        _ => None,
    }
}

pub(super) fn compare_text(
    bill: &OkxBill,
    field: &str,
    expected: &str,
    observed: &str,
    failures: &mut BTreeSet<EconomicReconciliationFailure>,
    issues: &mut IssueSink,
) {
    if expected != observed {
        push_bill_issue(
            failures,
            issues,
            EconomicReconciliationFailure::InvalidTradeBills,
            bill,
            field,
            expected,
            observed,
            "trade bill field does not match the verified fill",
        );
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn compare_number(
    bill: &OkxBill,
    field: &str,
    expected: f64,
    observed: Option<f64>,
    tolerance: f64,
    failures: &mut BTreeSet<EconomicReconciliationFailure>,
    issues: &mut IssueSink,
) {
    let Some(observed) = observed else {
        push_bill_issue(
            failures,
            issues,
            EconomicReconciliationFailure::InvalidTradeBills,
            bill,
            field,
            &expected.to_string(),
            "missing",
            "trade bill omits a required numeric field",
        );
        return;
    };
    compare_number_value(
        bill,
        field,
        expected,
        observed,
        tolerance,
        EconomicReconciliationFailure::InvalidTradeBills,
        failures,
        issues,
    );
}

#[allow(clippy::too_many_arguments)]
pub(super) fn compare_number_value(
    bill: &OkxBill,
    field: &str,
    expected: f64,
    observed: f64,
    tolerance: f64,
    failure: EconomicReconciliationFailure,
    failures: &mut BTreeSet<EconomicReconciliationFailure>,
    issues: &mut IssueSink,
) {
    if !close_abs(expected, observed, tolerance) {
        push_bill_issue(
            failures,
            issues,
            failure,
            bill,
            field,
            &expected.to_string(),
            &observed.to_string(),
            &format!("absolute difference exceeds {tolerance}"),
        );
    }
}

pub(super) fn close_abs(left: f64, right: f64, tolerance: f64) -> bool {
    left.is_finite() && right.is_finite() && (left - right).abs() <= tolerance
}

pub(super) fn is_lower_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[allow(clippy::too_many_arguments)]
pub(super) fn push_bill_issue(
    failures: &mut BTreeSet<EconomicReconciliationFailure>,
    issues: &mut IssueSink,
    failure: EconomicReconciliationFailure,
    bill: &OkxBill,
    field: &str,
    expected: &str,
    observed: &str,
    message: &str,
) {
    issues.push(
        failure,
        issue_for_bill(
            EconomicIssueSource::BillCollection,
            bill,
            field,
            expected,
            observed,
            message,
        ),
        failures,
    );
}

pub(super) fn issue_for_bill(
    source: EconomicIssueSource,
    bill: &OkxBill,
    field: &str,
    expected: &str,
    observed: &str,
    message: &str,
) -> EconomicIssue {
    issue(
        source,
        Some(&bill.bill_id),
        (!bill.symbol.is_empty()).then_some(bill.symbol.as_str()),
        (!bill.trade_id.is_empty()).then_some(bill.trade_id.as_str()),
        field,
        expected,
        observed,
        message,
    )
}

#[allow(clippy::too_many_arguments)]
pub(super) fn issue(
    source: EconomicIssueSource,
    bill_id: Option<&str>,
    symbol: Option<&str>,
    trade_id: Option<&str>,
    field: &str,
    expected: &str,
    observed: &str,
    message: &str,
) -> EconomicIssue {
    EconomicIssue {
        source,
        bill_id: bill_id.map(str::to_string),
        symbol: symbol.map(str::to_string),
        trade_id: trade_id.map(str::to_string),
        field: field.to_string(),
        expected: expected.to_string(),
        observed: observed.to_string(),
        message: message.to_string(),
    }
}

pub(super) fn side_name(side: Side) -> &'static str {
    match side {
        Side::Buy => "buy",
        Side::Sell => "sell",
    }
}

pub(super) fn execution_name(execution: OkxBillExecutionType) -> &'static str {
    match execution {
        OkxBillExecutionType::Maker => "maker",
        OkxBillExecutionType::Taker => "taker",
    }
}

pub(super) fn margin_mode_name(mode: OkxBillMarginMode) -> &'static str {
    match mode {
        OkxBillMarginMode::Cash => "cash",
        OkxBillMarginMode::Cross => "cross",
        OkxBillMarginMode::Isolated => "isolated",
    }
}
