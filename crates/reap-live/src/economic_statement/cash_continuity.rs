use std::collections::{BTreeMap, BTreeSet};

use reap_venue::okx::{OkxAccountBalanceSnapshot, OkxBalanceDetail, OkxBill};

use super::support::{close_abs, issue, issue_for_bill};
use super::{
    BoundAccountBoundary, BoundEconomicSources, CurrencyBalanceContinuitySample, EconomicIssue,
    EconomicIssueSource, EconomicReconciliationCounts, EconomicReconciliationFailure,
    EconomicReconciliationOptions, IssueSink,
};

#[derive(Debug, Clone, Copy, Default)]
struct BoundaryCurrencyValue {
    cash_balance: f64,
    equity: f64,
    equity_usd: f64,
}

type BillsByCurrency<'a> = BTreeMap<String, Vec<(&'a OkxBill, u128)>>;

pub(super) fn validate_account_balance_continuity(
    sources: &BoundEconomicSources,
    options: &EconomicReconciliationOptions,
    counts: &mut EconomicReconciliationCounts,
    failures: &mut BTreeSet<EconomicReconciliationFailure>,
    issues: &mut IssueSink,
) -> Vec<CurrencyBalanceContinuitySample> {
    validate_bound_account_identity(
        "opening",
        &sources.opening_account_boundary,
        sources,
        options,
        failures,
        issues,
    );
    validate_bound_account_identity(
        "closing",
        &sources.closing_account_boundary,
        sources,
        options,
        failures,
        issues,
    );
    let opening = boundary_currency_values(
        "opening",
        &sources.opening_account_boundary.balance,
        failures,
        issues,
    );
    let closing = boundary_currency_values(
        "closing",
        &sources.closing_account_boundary.balance,
        failures,
        issues,
    );
    let bills_by_currency = collect_bills_by_currency(&sources.bills, failures, issues);

    let currencies = opening
        .keys()
        .chain(closing.keys())
        .chain(bills_by_currency.keys())
        .cloned()
        .collect::<BTreeSet<_>>();
    let mut samples = Vec::new();
    for currency in currencies {
        let opening_value = opening.get(&currency).copied().unwrap_or_default();
        let closing_value = closing.get(&currency).copied().unwrap_or_default();
        let bills = bills_by_currency
            .get(&currency)
            .map(Vec::as_slice)
            .unwrap_or_default();
        samples.push(validate_currency_balance_continuity(
            currency,
            opening_value,
            closing_value,
            bills,
            options,
            counts,
            failures,
            issues,
        ));
    }
    samples
}

fn collect_bills_by_currency<'a>(
    bills: &'a [OkxBill],
    failures: &mut BTreeSet<EconomicReconciliationFailure>,
    issues: &mut IssueSink,
) -> BillsByCurrency<'a> {
    let mut bills_by_currency = BillsByCurrency::new();
    let mut seen_bill_ids = BTreeSet::new();
    for bill in bills {
        if !valid_currency(&bill.currency) {
            issues.push(
                EconomicReconciliationFailure::InvalidOrDuplicateBalanceCurrencies,
                issue_for_bill(
                    EconomicIssueSource::BillCollection,
                    bill,
                    "ccy",
                    "1-32 uppercase ASCII letters or digits",
                    &bill.currency,
                    "bill currency cannot be joined to certified account balances",
                ),
                failures,
            );
            continue;
        }
        let numeric_id = match parse_numeric_bill_id(&bill.bill_id) {
            Some(value) => value,
            None => {
                issues.push(
                    EconomicReconciliationFailure::InvalidBillBalanceChain,
                    issue_for_bill(
                        EconomicIssueSource::BillCollection,
                        bill,
                        "billId",
                        "positive base-10 integer",
                        &bill.bill_id,
                        "bill balance ordering requires a numeric OKX bill id",
                    ),
                    failures,
                );
                0
            }
        };
        if !seen_bill_ids.insert(bill.bill_id.clone()) {
            issues.push(
                EconomicReconciliationFailure::InvalidBillBalanceChain,
                issue_for_bill(
                    EconomicIssueSource::BillCollection,
                    bill,
                    "billId",
                    "globally unique bill id",
                    &bill.bill_id,
                    "bill balance chain contains a duplicate bill id",
                ),
                failures,
            );
        }
        bills_by_currency
            .entry(bill.currency.clone())
            .or_default()
            .push((bill, numeric_id));
    }
    for bills in bills_by_currency.values_mut() {
        bills.sort_by(|(left, left_id), (right, right_id)| {
            (left.timestamp_ms, *left_id, left.bill_id.as_str()).cmp(&(
                right.timestamp_ms,
                *right_id,
                right.bill_id.as_str(),
            ))
        });
    }
    bills_by_currency
}

#[allow(clippy::too_many_arguments)]
fn validate_currency_balance_continuity(
    currency: String,
    opening_value: BoundaryCurrencyValue,
    closing_value: BoundaryCurrencyValue,
    bills: &[(&OkxBill, u128)],
    options: &EconomicReconciliationOptions,
    counts: &mut EconomicReconciliationCounts,
    failures: &mut BTreeSet<EconomicReconciliationFailure>,
    issues: &mut IssueSink,
) -> CurrencyBalanceContinuitySample {
    let mut summed_balance_change = 0.0;
    let links = (bills.len() as u64).saturating_add(1);
    let mut links_validated = 0_u64;
    let mut valid = true;
    let mut previous_post_balance = None::<f64>;

    if bills.is_empty() {
        if close_abs(
            opening_value.cash_balance,
            closing_value.cash_balance,
            options.tolerances.balance_abs,
        ) {
            links_validated = 1;
        } else {
            valid = false;
            push_cash_continuity_issue(
                &currency,
                "boundary_cash_balance",
                opening_value.cash_balance,
                closing_value.cash_balance,
                "currency cash balance changed without an account bill",
                failures,
                issues,
            );
        }
    } else {
        for (offset, (bill, numeric_id)) in bills.iter().enumerate() {
            if *numeric_id == 0 {
                valid = false;
            }
            if !bill.balance_change.is_finite() {
                valid = false;
                issues.push(
                    EconomicReconciliationFailure::InvalidBillBalanceChain,
                    issue_for_bill(
                        EconomicIssueSource::BillCollection,
                        bill,
                        "balChg",
                        "finite balance change",
                        &bill.balance_change.to_string(),
                        "bill balance change is not finite",
                    ),
                    failures,
                );
                continue;
            }
            summed_balance_change += bill.balance_change;
            let Some(post_balance) = bill.balance.filter(|value| value.is_finite()) else {
                valid = false;
                issues.push(
                    EconomicReconciliationFailure::InvalidBillBalanceChain,
                    issue_for_bill(
                        EconomicIssueSource::BillCollection,
                        bill,
                        "bal",
                        "finite post-bill cash balance",
                        "missing or non-finite",
                        "bill does not expose the post-change balance needed for continuity",
                    ),
                    failures,
                );
                previous_post_balance = None;
                continue;
            };
            let pre_balance = post_balance - bill.balance_change;
            let (expected, field, message, failure) = if offset == 0 {
                (
                    opening_value.cash_balance,
                    "opening_cash_balance",
                    "first bill pre-balance does not match the opening account snapshot",
                    EconomicReconciliationFailure::CashBalanceContinuityMismatches,
                )
            } else if let Some(previous) = previous_post_balance {
                (
                    previous,
                    "bill_balance_chain",
                    "adjacent bill post/pre balances are discontinuous",
                    EconomicReconciliationFailure::InvalidBillBalanceChain,
                )
            } else {
                valid = false;
                previous_post_balance = Some(post_balance);
                continue;
            };
            if close_abs(expected, pre_balance, options.tolerances.balance_abs) {
                links_validated = links_validated.saturating_add(1);
            } else {
                valid = false;
                issues.push(
                    failure,
                    issue_for_bill(
                        EconomicIssueSource::BillCollection,
                        bill,
                        field,
                        &expected.to_string(),
                        &pre_balance.to_string(),
                        message,
                    ),
                    failures,
                );
            }
            previous_post_balance = Some(post_balance);
        }
        if let Some(last_post_balance) = previous_post_balance {
            if close_abs(
                last_post_balance,
                closing_value.cash_balance,
                options.tolerances.balance_abs,
            ) {
                links_validated = links_validated.saturating_add(1);
            } else {
                valid = false;
                push_cash_continuity_issue(
                    &currency,
                    "closing_cash_balance",
                    last_post_balance,
                    closing_value.cash_balance,
                    "last bill post-balance does not match the closing account snapshot",
                    failures,
                    issues,
                );
            }
        } else {
            valid = false;
        }
    }

    let expected_closing_cash_balance = opening_value.cash_balance + summed_balance_change;
    let aggregate_absolute_difference =
        (expected_closing_cash_balance - closing_value.cash_balance).abs();
    if !expected_closing_cash_balance.is_finite()
        || aggregate_absolute_difference > options.tolerances.balance_abs
    {
        valid = false;
        push_cash_continuity_issue(
            &currency,
            "aggregate_cash_balance",
            expected_closing_cash_balance,
            closing_value.cash_balance,
            "opening cash plus all bill balance changes does not equal closing cash",
            failures,
            issues,
        );
    }
    if links_validated != links {
        valid = false;
    }
    counts.cash_balance_chain_links = counts.cash_balance_chain_links.saturating_add(links);
    counts.cash_balance_chain_links_validated = counts
        .cash_balance_chain_links_validated
        .saturating_add(links_validated);
    counts.cash_balance_currencies = counts.cash_balance_currencies.saturating_add(1);
    if valid {
        counts.cash_balance_currencies_validated =
            counts.cash_balance_currencies_validated.saturating_add(1);
    }
    CurrencyBalanceContinuitySample {
        currency,
        opening_cash_balance: opening_value.cash_balance,
        closing_cash_balance: closing_value.cash_balance,
        opening_equity: opening_value.equity,
        closing_equity: closing_value.equity,
        opening_equity_usd: opening_value.equity_usd,
        closing_equity_usd: closing_value.equity_usd,
        bill_count: bills.len() as u64,
        first_bill_id: bills.first().map(|(bill, _)| bill.bill_id.clone()),
        last_bill_id: bills.last().map(|(bill, _)| bill.bill_id.clone()),
        summed_balance_change,
        expected_closing_cash_balance,
        aggregate_absolute_difference,
        effective_tolerance: options.tolerances.balance_abs,
        balance_chain_links: links,
        balance_chain_links_validated: links_validated,
        validated: valid,
    }
}

fn validate_bound_account_identity(
    label: &str,
    boundary: &BoundAccountBoundary,
    sources: &BoundEconomicSources,
    options: &EconomicReconciliationOptions,
    failures: &mut BTreeSet<EconomicReconciliationFailure>,
    issues: &mut IssueSink,
) {
    let identity_valid = boundary.passed
        && boundary.account_id == sources.account_id
        && boundary.environment == sources.environment
        && boundary.account_identity_sha256 == sources.account_identity_sha256
        && boundary.config_fingerprint == sources.config_fingerprint
        && boundary.config_source_path == sources.config_file.path
        && boundary.config_sha256 == sources.config_file.sha256
        && boundary.evidence.start_server_ms <= boundary.evidence.finish_server_ms
        && boundary.evidence.balance_currencies == boundary.balance.details.len() as u64
        && boundary.evidence.total_equity_usd.is_finite()
        && boundary.balance.total_equity_usd.is_some_and(|value| {
            close_abs(
                value,
                boundary.evidence.total_equity_usd,
                options.tolerances.balance_abs,
            )
        });
    let expected_gap = if label == "opening" {
        options
            .begin_ms
            .checked_sub(boundary.evidence.finish_server_ms)
    } else {
        boundary
            .evidence
            .start_server_ms
            .checked_sub(options.end_ms)
    };
    let timing_valid = expected_gap.is_some_and(|gap| {
        gap == boundary.evidence.window_gap_ms && gap <= options.maximum_account_boundary_gap_ms
    });
    if !identity_valid || !timing_valid {
        issues.push(
            EconomicReconciliationFailure::InvalidAccountBoundaries,
            issue(
                EconomicIssueSource::AccountBoundary,
                None,
                None,
                None,
                &format!("{label}_account_boundary"),
                "passing, bound certification on the correct side of the window within the configured gap",
                &format!(
                    "account={}, environment={:?}, passed={}, start={}, finish={}, gap={}",
                    boundary.account_id,
                    boundary.environment,
                    boundary.passed,
                    boundary.evidence.start_server_ms,
                    boundary.evidence.finish_server_ms,
                    boundary.evidence.window_gap_ms
                ),
                "account boundary identity, timing, or certified total equity is invalid",
            ),
            failures,
        );
    }
}

fn boundary_currency_values(
    label: &str,
    balance: &OkxAccountBalanceSnapshot,
    failures: &mut BTreeSet<EconomicReconciliationFailure>,
    issues: &mut IssueSink,
) -> BTreeMap<String, BoundaryCurrencyValue> {
    let mut values = BTreeMap::new();
    if balance.details.is_empty() {
        issues.push(
            EconomicReconciliationFailure::InvalidOrDuplicateBalanceCurrencies,
            issue(
                EconomicIssueSource::AccountBoundary,
                None,
                None,
                None,
                &format!("{label}_balance_details"),
                "at least one certified currency",
                "empty",
                "account boundary has no currency balances",
            ),
            failures,
        );
    }
    for detail in &balance.details {
        if !valid_currency(&detail.currency) {
            issues.push(
                EconomicReconciliationFailure::InvalidOrDuplicateBalanceCurrencies,
                boundary_currency_issue(
                    label,
                    detail,
                    "ccy",
                    "1-32 uppercase ASCII letters or digits",
                    &detail.currency,
                    "account boundary contains an invalid currency",
                ),
                failures,
            );
            continue;
        }
        let Some(cash_balance) = finite_optional(detail.cash_balance) else {
            issues.push(
                EconomicReconciliationFailure::InvalidOrDuplicateBalanceCurrencies,
                boundary_currency_issue(
                    label,
                    detail,
                    "cashBal",
                    "finite value",
                    "missing or non-finite",
                    "account boundary cash balance is unavailable",
                ),
                failures,
            );
            continue;
        };
        let Some(equity) = finite_optional(detail.equity) else {
            issues.push(
                EconomicReconciliationFailure::InvalidOrDuplicateBalanceCurrencies,
                boundary_currency_issue(
                    label,
                    detail,
                    "eq",
                    "finite value",
                    "missing or non-finite",
                    "account boundary native equity is unavailable",
                ),
                failures,
            );
            continue;
        };
        let Some(equity_usd) = finite_optional(detail.equity_usd) else {
            issues.push(
                EconomicReconciliationFailure::InvalidOrDuplicateBalanceCurrencies,
                boundary_currency_issue(
                    label,
                    detail,
                    "eqUsd",
                    "finite value",
                    "missing or non-finite",
                    "account boundary converted equity is unavailable",
                ),
                failures,
            );
            continue;
        };
        if values
            .insert(
                detail.currency.clone(),
                BoundaryCurrencyValue {
                    cash_balance,
                    equity,
                    equity_usd,
                },
            )
            .is_some()
        {
            issues.push(
                EconomicReconciliationFailure::InvalidOrDuplicateBalanceCurrencies,
                boundary_currency_issue(
                    label,
                    detail,
                    "ccy",
                    "unique currency",
                    &detail.currency,
                    "account boundary contains duplicate currency balances",
                ),
                failures,
            );
        }
    }
    values
}

fn boundary_currency_issue(
    label: &str,
    detail: &OkxBalanceDetail,
    field: &str,
    expected: &str,
    observed: &str,
    message: &str,
) -> EconomicIssue {
    issue(
        EconomicIssueSource::AccountBoundary,
        None,
        None,
        None,
        &format!("{label}.{}.{}", detail.currency, field),
        expected,
        observed,
        message,
    )
}

fn push_cash_continuity_issue(
    currency: &str,
    field: &str,
    expected: f64,
    observed: f64,
    message: &str,
    failures: &mut BTreeSet<EconomicReconciliationFailure>,
    issues: &mut IssueSink,
) {
    issues.push(
        EconomicReconciliationFailure::CashBalanceContinuityMismatches,
        issue(
            EconomicIssueSource::AccountBoundary,
            None,
            None,
            None,
            &format!("{currency}.{field}"),
            &expected.to_string(),
            &observed.to_string(),
            message,
        ),
        failures,
    );
}

fn valid_currency(currency: &str) -> bool {
    !currency.is_empty()
        && currency.len() <= 32
        && currency
            .bytes()
            .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit())
}

fn parse_numeric_bill_id(value: &str) -> Option<u128> {
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    value.parse::<u128>().ok().filter(|value| *value > 0)
}

fn finite_optional(value: Option<f64>) -> Option<f64> {
    value.filter(|value| value.is_finite())
}
