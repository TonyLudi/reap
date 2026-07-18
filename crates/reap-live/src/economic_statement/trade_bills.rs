use std::collections::BTreeSet;

use reap_core::{FillLiquidity, Side};
use reap_strategy::InstrumentConfig;
use reap_venue::RemoteFill;
use reap_venue::okx::{OkxBill, OkxBillExecutionType, OkxInstrumentType};

use super::support::{
    close_abs, compare_number, compare_number_value, compare_text, execution_name,
    expected_bill_margin_mode, instrument, instrument_type, issue_for_bill, margin_mode_name,
    push_bill_issue, side_name, trade_subtype_side,
};
use super::{
    DerivativePnlFormulaSample, EconomicIssueSource, EconomicReconciliationFailure,
    EconomicReconciliationOptions, IssueSink, JournalTradeEvidence,
};
use crate::LiveConfig;

#[derive(Debug)]
pub(super) struct TradeBillValidation {
    pub(super) valid: bool,
    pub(super) derivative_sample: Option<DerivativePnlFormulaSample>,
}

#[allow(clippy::too_many_arguments)]
pub(super) fn validate_trade_bill(
    bill: &OkxBill,
    fill: &RemoteFill,
    journal_candidates: &[JournalTradeEvidence],
    config: &LiveConfig,
    account_id: &str,
    options: &EconomicReconciliationOptions,
    failures: &mut BTreeSet<EconomicReconciliationFailure>,
    issues: &mut IssueSink,
) -> TradeBillValidation {
    let before = issues.total;
    let Some(instrument) = instrument(config, &bill.symbol) else {
        push_bill_issue(
            failures,
            issues,
            EconomicReconciliationFailure::InvalidTradeBills,
            bill,
            "instId",
            "configured strategy instrument",
            &bill.symbol,
            "trade bill references an instrument outside the exact live config",
        );
        return TradeBillValidation {
            valid: false,
            derivative_sample: None,
        };
    };
    let journal_trade =
        validate_journal_trade_fill(bill, fill, journal_candidates, options, failures, issues);
    let expected_side = trade_subtype_side(&bill.sub_type);
    validate_trade_identity(
        bill,
        fill,
        instrument,
        expected_side,
        config,
        account_id,
        failures,
        issues,
    );
    validate_trade_contract_execution(
        bill, fill, instrument, config, account_id, options, failures, issues,
    );
    validate_trade_fill_amounts(
        bill,
        fill,
        instrument,
        expected_side,
        options,
        failures,
        issues,
    );
    validate_trade_accounting(bill, instrument, options, failures, issues);
    let derivative_sample = if instrument.kind.is_derivative() {
        validate_derivative_trade_pnl(
            bill,
            fill,
            journal_trade,
            instrument,
            options,
            failures,
            issues,
        )
    } else {
        None
    };
    TradeBillValidation {
        valid: before == issues.total,
        derivative_sample,
    }
}

#[allow(clippy::too_many_arguments)]
fn validate_trade_identity(
    bill: &OkxBill,
    fill: &RemoteFill,
    instrument: &InstrumentConfig,
    expected_side: Option<Side>,
    config: &LiveConfig,
    account_id: &str,
    failures: &mut BTreeSet<EconomicReconciliationFailure>,
    issues: &mut IssueSink,
) {
    if expected_side.is_none()
        || (instrument.kind.is_spot() && !matches!(bill.sub_type.as_str(), "1" | "2"))
    {
        push_bill_issue(
            failures,
            issues,
            EconomicReconciliationFailure::InvalidTradeBills,
            bill,
            "subType",
            if instrument.kind.is_spot() {
                "1 or 2"
            } else {
                "1 through 6"
            },
            &bill.sub_type,
            "trade bill subtype is not a supported normal strategy trade",
        );
    }
    if let Some(expected_side) = expected_side
        && fill.side != expected_side
    {
        push_bill_issue(
            failures,
            issues,
            EconomicReconciliationFailure::InvalidTradeBills,
            bill,
            "side",
            side_name(expected_side),
            side_name(fill.side),
            "bill subtype side does not match the verified fill",
        );
    }
    compare_text(
        bill,
        "ordId",
        &fill.exchange_order_id,
        &bill.order_id,
        failures,
        issues,
    );
    if fill.client_order_id.is_empty() || bill.client_order_id.is_empty() {
        push_bill_issue(
            failures,
            issues,
            EconomicReconciliationFailure::InvalidTradeBills,
            bill,
            "clOrdId",
            "non-empty Reap client order id",
            &bill.client_order_id,
            "normal strategy trade must retain its client order identity",
        );
    } else {
        compare_text(
            bill,
            "clOrdId",
            &fill.client_order_id,
            &bill.client_order_id,
            failures,
            issues,
        );
        if let Some(account) = config.account(account_id)
            && !bill.client_order_id.starts_with(&account.id_prefix)
        {
            push_bill_issue(
                failures,
                issues,
                EconomicReconciliationFailure::InvalidTradeBills,
                bill,
                "clOrdId",
                &format!("prefix {}", account.id_prefix),
                &bill.client_order_id,
                "trade bill is not attributable to the configured Reap client-id namespace",
            );
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn validate_trade_contract_execution(
    bill: &OkxBill,
    fill: &RemoteFill,
    instrument: &InstrumentConfig,
    config: &LiveConfig,
    account_id: &str,
    options: &EconomicReconciliationOptions,
    failures: &mut BTreeSet<EconomicReconciliationFailure>,
    issues: &mut IssueSink,
) {
    let expected_instrument_type = instrument_type(instrument.kind);
    if bill.instrument_type != Some(expected_instrument_type) {
        push_bill_issue(
            failures,
            issues,
            EconomicReconciliationFailure::InvalidTradeBills,
            bill,
            "instType",
            expected_instrument_type.as_str(),
            bill.instrument_type
                .map_or("missing", OkxInstrumentType::as_str),
            "bill instrument type does not match the configured contract model",
        );
    }
    match expected_bill_margin_mode(config, account_id, &bill.symbol) {
        Some(expected_margin) if bill.margin_mode != Some(expected_margin) => push_bill_issue(
            failures,
            issues,
            EconomicReconciliationFailure::InvalidTradeBills,
            bill,
            "mgnMode",
            margin_mode_name(expected_margin),
            bill.margin_mode.map_or("missing", margin_mode_name),
            "trade bill margin mode does not match the configured account trade mode",
        ),
        None => push_bill_issue(
            failures,
            issues,
            EconomicReconciliationFailure::InvalidTradeBills,
            bill,
            "mgnMode",
            "configured account trade mode",
            bill.margin_mode.map_or("missing", margin_mode_name),
            "trade bill cannot be bound to an account trade-mode configuration",
        ),
        Some(_) => {}
    }
    let expected_execution = match fill.liquidity {
        FillLiquidity::Maker => OkxBillExecutionType::Maker,
        FillLiquidity::Taker => OkxBillExecutionType::Taker,
    };
    if bill.execution_type != Some(expected_execution) {
        push_bill_issue(
            failures,
            issues,
            EconomicReconciliationFailure::InvalidTradeBills,
            bill,
            "execType",
            execution_name(expected_execution),
            bill.execution_type.map_or("missing", execution_name),
            "bill liquidity does not match the verified fill",
        );
    }
    let fill_time_ms = bill.fill_time_ms;
    match fill_time_ms {
        Some(fill_time_ms) => {
            if fill_time_ms != fill.ts_ms {
                push_bill_issue(
                    failures,
                    issues,
                    EconomicReconciliationFailure::InvalidTradeBills,
                    bill,
                    "fillTime",
                    &fill.ts_ms.to_string(),
                    &fill_time_ms.to_string(),
                    "bill fill timestamp does not match the verified fill",
                );
            }
            if bill.timestamp_ms < fill_time_ms
                || bill.timestamp_ms - fill_time_ms > options.maximum_trade_bill_delay_ms
            {
                push_bill_issue(
                    failures,
                    issues,
                    EconomicReconciliationFailure::InvalidTradeBills,
                    bill,
                    "ts",
                    &format!(
                        "fillTime..=fillTime+{}",
                        options.maximum_trade_bill_delay_ms
                    ),
                    &bill.timestamp_ms.to_string(),
                    "trade bill completion time is outside the bounded causal delay",
                );
            }
        }
        None => push_bill_issue(
            failures,
            issues,
            EconomicReconciliationFailure::InvalidTradeBills,
            bill,
            "fillTime",
            &fill.ts_ms.to_string(),
            "missing",
            "trade bill does not retain an exact fill timestamp",
        ),
    }
}

fn validate_trade_fill_amounts(
    bill: &OkxBill,
    fill: &RemoteFill,
    instrument: &InstrumentConfig,
    expected_side: Option<Side>,
    options: &EconomicReconciliationOptions,
    failures: &mut BTreeSet<EconomicReconciliationFailure>,
    issues: &mut IssueSink,
) {
    compare_number(
        bill,
        "px",
        fill.price,
        bill.price,
        options.tolerances.price_abs,
        failures,
        issues,
    );

    let expected_currency = if instrument.kind.is_spot() {
        match expected_side {
            Some(Side::Buy) => instrument.base_currency.as_str(),
            Some(Side::Sell) => instrument.quote_currency.as_str(),
            None => "",
        }
    } else {
        instrument.settle_currency.as_str()
    }
    .to_ascii_uppercase();
    if expected_currency.is_empty() || bill.currency != expected_currency {
        let expected = if expected_currency.is_empty() {
            "configured accounting currency"
        } else {
            &expected_currency
        };
        push_bill_issue(
            failures,
            issues,
            EconomicReconciliationFailure::InvalidTradeBills,
            bill,
            "ccy",
            expected,
            &bill.currency,
            "trade bill currency does not match the configured received/settlement currency",
        );
    }
    let expected_quantity = if instrument.kind.is_spot() {
        match expected_side {
            Some(Side::Buy) => Some(fill.qty),
            Some(Side::Sell) => Some(fill.qty * fill.price),
            None => None,
        }
    } else {
        Some(fill.qty)
    };
    if let Some(expected_quantity) = expected_quantity {
        compare_number(
            bill,
            "sz",
            expected_quantity,
            bill.quantity,
            options.tolerances.quantity_abs,
            failures,
            issues,
        );
    }

    match (&fill.fee, bill.fee) {
        (Some(fill_fee), Some(bill_fee)) => {
            if fill_fee.currency.trim().to_ascii_uppercase() != bill.currency {
                push_bill_issue(
                    failures,
                    issues,
                    EconomicReconciliationFailure::InvalidTradeBills,
                    bill,
                    "feeCcy",
                    &fill_fee.currency.trim().to_ascii_uppercase(),
                    &bill.currency,
                    "bill currency does not match the verified fill fee currency",
                );
            }
            compare_number_value(
                bill,
                "fee",
                fill_fee.amount,
                bill_fee,
                options.tolerances.fee_abs,
                EconomicReconciliationFailure::InvalidTradeBills,
                failures,
                issues,
            );
        }
        _ => push_bill_issue(
            failures,
            issues,
            EconomicReconciliationFailure::InvalidTradeBills,
            bill,
            "fee",
            "exact fee on both bill and verified fill",
            "missing",
            "trade economics cannot be accepted without exact signed fee evidence",
        ),
    }
}

fn validate_trade_accounting(
    bill: &OkxBill,
    instrument: &InstrumentConfig,
    options: &EconomicReconciliationOptions,
    failures: &mut BTreeSet<EconomicReconciliationFailure>,
    issues: &mut IssueSink,
) {
    if let Some(interest) = bill.interest
        && !close_abs(interest, 0.0, options.tolerances.balance_abs)
    {
        push_bill_issue(
            failures,
            issues,
            EconomicReconciliationFailure::InvalidTradeBills,
            bill,
            "interest",
            "0",
            &interest.to_string(),
            "normal controlled strategy trade unexpectedly accrued interest",
        );
    }
    if bill.from_account.is_some() || bill.to_account.is_some() {
        push_bill_issue(
            failures,
            issues,
            EconomicReconciliationFailure::InvalidTradeBills,
            bill,
            "from/to",
            "empty for non-transfer trade",
            "populated",
            "trade bill unexpectedly identifies an account transfer",
        );
    }
    let expected_balance_change = match (bill.fee, instrument.kind.is_spot()) {
        (Some(fee), true) => bill.quantity.map(|quantity| quantity + fee),
        (Some(fee), false) => bill.pnl.map(|pnl| pnl + fee),
        _ => None,
    };
    if let Some(expected_balance_change) = expected_balance_change.filter(|value| value.is_finite())
    {
        compare_number_value(
            bill,
            "balChg",
            expected_balance_change,
            bill.balance_change,
            options.tolerances.balance_abs,
            EconomicReconciliationFailure::InvalidTradeBills,
            failures,
            issues,
        );
    } else {
        push_bill_issue(
            failures,
            issues,
            EconomicReconciliationFailure::InvalidTradeBills,
            bill,
            if instrument.kind.is_spot() {
                "sz/fee"
            } else {
                "pnl/fee"
            },
            "complete finite balance equation inputs",
            "missing",
            "trade bill balance change cannot be checked for internal consistency",
        );
    }
}

fn validate_journal_trade_fill<'a>(
    bill: &OkxBill,
    fill: &RemoteFill,
    candidates: &'a [JournalTradeEvidence],
    options: &EconomicReconciliationOptions,
    failures: &mut BTreeSet<EconomicReconciliationFailure>,
    issues: &mut IssueSink,
) -> Option<&'a JournalTradeEvidence> {
    let [candidate] = candidates else {
        issues.push(
            EconomicReconciliationFailure::TradeJournalFillMismatches,
            issue_for_bill(
                EconomicIssueSource::Journal,
                bill,
                "journal_fill",
                "exactly one same-account critical fill record",
                &format!("{} candidates", candidates.len()),
                "verified exchange fill cannot be bound to one durable runtime fill",
            ),
            failures,
        );
        return None;
    };
    let journal = &candidate.observation.fill;
    let mut valid = true;
    let mut compare = |field: &str, matches: bool, expected: String, observed: String| {
        if !matches {
            valid = false;
            issues.push(
                EconomicReconciliationFailure::TradeJournalFillMismatches,
                issue_for_bill(
                    EconomicIssueSource::Journal,
                    bill,
                    field,
                    &expected,
                    &observed,
                    "critical journal fill does not match the independently collected exchange fill",
                ),
                failures,
            );
        }
    };
    compare(
        "journal_fill_time",
        journal.ts_ms == fill.ts_ms,
        fill.ts_ms.to_string(),
        journal.ts_ms.to_string(),
    );
    compare(
        "journal_order_id",
        journal.order_id == fill.exchange_order_id
            || (!fill.client_order_id.is_empty() && journal.order_id == fill.client_order_id),
        format!("{} or {}", fill.exchange_order_id, fill.client_order_id),
        journal.order_id.clone(),
    );
    compare(
        "journal_side",
        journal.side == fill.side,
        side_name(fill.side).to_string(),
        side_name(journal.side).to_string(),
    );
    compare(
        "journal_price",
        close_abs(journal.price, fill.price, options.tolerances.price_abs),
        fill.price.to_string(),
        journal.price.to_string(),
    );
    compare(
        "journal_quantity",
        close_abs(journal.qty, fill.qty, options.tolerances.quantity_abs),
        fill.qty.to_string(),
        journal.qty.to_string(),
    );
    compare(
        "journal_liquidity",
        journal.liquidity == Some(fill.liquidity),
        execution_name(match fill.liquidity {
            FillLiquidity::Maker => OkxBillExecutionType::Maker,
            FillLiquidity::Taker => OkxBillExecutionType::Taker,
        })
        .to_string(),
        journal
            .liquidity
            .map(|liquidity| match liquidity {
                FillLiquidity::Maker => "maker",
                FillLiquidity::Taker => "taker",
            })
            .unwrap_or("missing")
            .to_string(),
    );
    match (&fill.fee, &journal.fee) {
        (Some(expected), Some(observed)) => {
            compare(
                "journal_fee_currency",
                expected
                    .currency
                    .trim()
                    .eq_ignore_ascii_case(observed.currency.trim()),
                expected.currency.trim().to_ascii_uppercase(),
                observed.currency.trim().to_ascii_uppercase(),
            );
            compare(
                "journal_fee",
                close_abs(expected.amount, observed.amount, options.tolerances.fee_abs),
                expected.amount.to_string(),
                observed.amount.to_string(),
            );
        }
        _ => compare(
            "journal_fee",
            false,
            "exact signed fee on collection and journal".to_string(),
            "missing".to_string(),
        ),
    }
    valid.then_some(candidate)
}

#[allow(clippy::too_many_arguments)]
fn validate_derivative_trade_pnl(
    bill: &OkxBill,
    fill: &RemoteFill,
    journal_trade: Option<&JournalTradeEvidence>,
    instrument: &InstrumentConfig,
    options: &EconomicReconciliationOptions,
    failures: &mut BTreeSet<EconomicReconciliationFailure>,
    issues: &mut IssueSink,
) -> Option<DerivativePnlFormulaSample> {
    let journal_trade = journal_trade?;
    let Some(evidence) = journal_trade.derivative.as_ref() else {
        issues.push(
            EconomicReconciliationFailure::DerivativeOpeningBasisMissingOrInvalid,
            issue_for_bill(
                EconomicIssueSource::Journal,
                bill,
                "opening_basis",
                "same-session authoritative REST avgPx snapshot before the critical fill",
                "missing or invalid",
                "derivative PnL cannot be independently reconstructed from the stopped journal",
            ),
            failures,
        );
        return None;
    };
    if evidence.basis.snapshot_line <= evidence.runtime_session_start_line
        || evidence.basis.snapshot_line >= evidence.fill_line
        || evidence.basis.snapshot_time_ms >= fill.ts_ms
    {
        issues.push(
            EconomicReconciliationFailure::DerivativeOpeningBasisMissingOrInvalid,
            issue_for_bill(
                EconomicIssueSource::Journal,
                bill,
                "opening_basis_order",
                "same-session snapshot line/time before fill line/time",
                &format!(
                    "session_line={}, snapshot_line={}, snapshot_ts={}, fill_line={}, fill_ts={}",
                    evidence.runtime_session_start_line,
                    evidence.basis.snapshot_line,
                    evidence.basis.snapshot_time_ms,
                    evidence.fill_line,
                    fill.ts_ms
                ),
                "authoritative position basis is not causally ordered before the target fill",
            ),
            failures,
        );
        return None;
    }
    let Some(observed_pnl) = bill.pnl.filter(|pnl| pnl.is_finite()) else {
        push_bill_issue(
            failures,
            issues,
            EconomicReconciliationFailure::InvalidTradeBills,
            bill,
            "pnl",
            "finite derivative trade PnL",
            &format!("{:?}", bill.pnl),
            "derivative trade bill does not expose a finite realized-PnL value",
        );
        return None;
    };
    let subtype_valid = bill.sub_type == evidence.expected_sub_type;
    if !subtype_valid {
        push_bill_issue(
            failures,
            issues,
            EconomicReconciliationFailure::DerivativePnlFormulaMismatches,
            bill,
            "subType",
            &evidence.expected_sub_type,
            &bill.sub_type,
            "bill open/close direction contradicts the journal-reconstructed pre-fill position",
        );
    }
    let absolute_difference = (evidence.expected_pnl - observed_pnl).abs();
    let scale = evidence.expected_pnl.abs().max(observed_pnl.abs());
    let relative_difference = absolute_difference / scale.max(f64::MIN_POSITIVE);
    let effective_tolerance = options
        .tolerances
        .trade_pnl_abs
        .max(options.tolerances.trade_pnl_relative * evidence.expected_pnl.abs());
    let formula_valid = evidence.expected_pnl.is_finite()
        && absolute_difference.is_finite()
        && effective_tolerance.is_finite()
        && absolute_difference <= effective_tolerance;
    if !formula_valid {
        push_bill_issue(
            failures,
            issues,
            EconomicReconciliationFailure::DerivativePnlFormulaMismatches,
            bill,
            "pnl_formula",
            &format!("{} +/- {}", evidence.expected_pnl, effective_tolerance),
            &observed_pnl.to_string(),
            "derivative trade PnL does not match the attested opening basis and configured contract formula",
        );
    }
    Some(DerivativePnlFormulaSample {
        bill_id: bill.bill_id.clone(),
        symbol: bill.symbol.clone(),
        trade_id: bill.trade_id.clone(),
        runtime_session_id: evidence.runtime_session_id.clone(),
        runtime_session_start_line: evidence.runtime_session_start_line,
        snapshot_line: evidence.basis.snapshot_line,
        snapshot_time_ms: evidence.basis.snapshot_time_ms,
        fill_line: evidence.fill_line,
        fill_time_ms: fill.ts_ms,
        inverse: instrument.kind.is_inverse(),
        currency: instrument.settle_currency.trim().to_ascii_uppercase(),
        pre_quantity: evidence.basis.quantity,
        pre_avg_price: evidence.basis.avg_price,
        fill_side: fill.side,
        fill_price: fill.price,
        fill_quantity: fill.qty,
        close_quantity: evidence.close_quantity,
        contract_value: instrument.contract_value,
        post_quantity: evidence.post_quantity,
        post_avg_price: evidence.post_avg_price,
        expected_sub_type: evidence.expected_sub_type.clone(),
        observed_sub_type: bill.sub_type.clone(),
        expected_pnl: evidence.expected_pnl,
        observed_pnl,
        absolute_difference,
        relative_difference,
        effective_tolerance,
        validated: subtype_valid && formula_valid,
    })
}
