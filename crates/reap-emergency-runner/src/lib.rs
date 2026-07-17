//! Separate emergency-stop composition preserving the pre-Phase-5 combined workflow.

use std::collections::{BTreeSet, HashSet};
use std::future::Future;
use std::path::Path;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use chrono::{SecondsFormat, Utc};
use reap_core::PINNED_JAVA_REVISION;
use reap_emergency_core::*;
use reap_okx_emergency_adapter::OkxEmergencyAccountStopFactory;
use reap_order::{PacingPolicy, RequestKind, RequestPacer};
use reap_telemetry::{
    current_executable_sha256, host_identity_sha256, identity_sha256, sha256_bytes,
};
use tokio::task::JoinSet;

#[derive(Debug)]
struct EmergencyProvenance {
    config_file_sha256: String,
    executable_sha256: Option<String>,
    host_identity_sha256: Option<String>,
    incidents: Vec<String>,
}

#[derive(Debug, Clone)]
struct AccountCancelSettings {
    environment: TradingEnvironment,
    account_timeout: Duration,
    poll_interval: Duration,
    verification_delay: Duration,
    pacing_policy: PacingPolicy,
    max_exchange_clock_skew_ms: u64,
    deadman_timeout_secs: u64,
    max_order_reconciliation_pages: usize,
}

#[derive(Debug, Clone, Default)]
struct AccountPendingOrders {
    regular: Vec<RegularOrder>,
    algo: Vec<AlgoOrder>,
    spread: Vec<SpreadOrder>,
}

impl AccountPendingOrders {
    fn is_empty(&self) -> bool {
        self.regular.is_empty() && self.algo.is_empty() && self.spread.is_empty()
    }
}

#[derive(Debug, Clone)]
struct ExchangeClock {
    exchange_ms: u64,
    sampled_at: Instant,
}

impl ExchangeClock {
    fn local() -> Self {
        Self {
            exchange_ms: unix_time_ms(),
            sampled_at: Instant::now(),
        }
    }

    fn timestamp(&self) -> Result<String, EmergencyRoleError> {
        let elapsed_ms = self.sampled_at.elapsed().as_millis().min(u64::MAX as u128) as u64;
        format_okx_timestamp_ms(self.exchange_ms.saturating_add(elapsed_ms))
    }
}

pub async fn run_emergency_cancel_path(
    path: impl AsRef<Path>,
    options: EmergencyCancelOptions,
) -> Result<EmergencyCancelReport, EmergencyCancelError> {
    run_emergency_cancel_path_with_factory(path, options, &OkxEmergencyAccountStopFactory).await
}

/// Testable composition seam. Production callers should use
/// [`run_emergency_cancel_path`], whose factory is restricted to the OKX
/// emergency adapter's narrow account-stop authority.
pub async fn run_emergency_cancel_path_with_factory(
    path: impl AsRef<Path>,
    options: EmergencyCancelOptions,
    factory: &dyn EmergencyAccountStopFactory,
) -> Result<EmergencyCancelReport, EmergencyCancelError> {
    let path = path.as_ref();
    let text = std::fs::read_to_string(path).map_err(|source| EmergencyCancelError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    let provenance = collect_emergency_provenance(sha256_bytes(text.as_bytes()));
    let config = parse_emergency_config(&text)?;
    run_emergency_cancel(config, options, provenance, factory).await
}

async fn run_emergency_cancel(
    config: EmergencyFileConfig,
    options: EmergencyCancelOptions,
    provenance: EmergencyProvenance,
    factory: &dyn EmergencyAccountStopFactory,
) -> Result<EmergencyCancelReport, EmergencyCancelError> {
    let selected = validate_and_select_accounts(&config, &options)?;
    let started_at_ms = unix_time_ms();
    let report_id = format!("{:x}", unix_time_ns());
    let started = Instant::now();
    let verification_delay =
        Duration::from_secs(options.deadman_timeout_secs).saturating_add(Duration::from_secs(2));
    let pacing = config.runtime.pacing_policy();
    let settings = AccountCancelSettings {
        environment: config.venue.environment,
        account_timeout: options.account_timeout,
        poll_interval: options.poll_interval,
        verification_delay,
        pacing_policy: PacingPolicy {
            submit_requests: pacing.submit_requests,
            cancel_requests: pacing.cancel_requests,
            reconcile_requests: pacing.reconcile_requests,
            window: pacing.window,
        },
        max_exchange_clock_skew_ms: config.runtime.max_exchange_clock_skew_ms,
        deadman_timeout_secs: options.deadman_timeout_secs,
        max_order_reconciliation_pages: config.runtime.max_order_reconciliation_pages,
    };
    let mut selected_accounts = selected
        .iter()
        .map(|account| account.id.clone())
        .collect::<Vec<_>>();
    selected_accounts.sort();
    let mut reports = Vec::new();
    let mut tasks = JoinSet::new();

    for account in selected {
        let account_id = account.id.clone();
        let client = match factory.create(&config.venue, &config.runtime, &account) {
            Ok(client) => client,
            Err(EmergencyRoleSetupError::Credential(error)) => {
                reports.push(EmergencyAccountReport::setup_failure(
                    account_id,
                    format!("credential setup failed: {error}"),
                ));
                continue;
            }
            Err(EmergencyRoleSetupError::Transport(error)) => {
                reports.push(EmergencyAccountReport::setup_failure(
                    account_id,
                    format!("REST transport setup failed: {error}"),
                ));
                continue;
            }
        };
        let managed_symbols = account.trade_modes.keys().cloned().collect::<HashSet<_>>();
        let account_settings = settings.clone();
        tasks.spawn(async move {
            run_account_cancel(client, account_id, managed_symbols, account_settings).await
        });
    }
    let mut execution_incident_count = 0;
    let mut execution_incidents = Vec::new();
    collect_account_reports(
        &mut tasks,
        &mut reports,
        &mut execution_incident_count,
        &mut execution_incidents,
    )
    .await;
    reports.sort_by(|left, right| left.account_id.cmp(&right.account_id));
    let provenance_incident_count = provenance.incidents.len() as u64;
    let completion = emergency_completion(
        &selected_accounts,
        &reports,
        &provenance.config_file_sha256,
        provenance.executable_sha256.as_deref(),
        provenance.host_identity_sha256.as_deref(),
        provenance.incidents.is_empty(),
        execution_incident_count,
    );
    Ok(EmergencyCancelReport {
        schema_version: EMERGENCY_CANCEL_REPORT_SCHEMA_VERSION,
        report_id,
        config_file_sha256: provenance.config_file_sha256,
        java_reference_revision: PINNED_JAVA_REVISION.to_string(),
        reap_version: env!("CARGO_PKG_VERSION").to_string(),
        executable_sha256: provenance.executable_sha256,
        host_identity_sha256: provenance.host_identity_sha256,
        provenance_incident_count,
        provenance_incidents: provenance.incidents,
        environment: config.venue.environment,
        scope: ACCOUNT_WIDE_ORDER_SCOPE.to_string(),
        excluded_order_classes: EXCLUDED_ORDER_CLASSES
            .into_iter()
            .map(str::to_string)
            .collect(),
        started_at_ms,
        elapsed_ms: elapsed_ms(&started),
        account_timeout_ms: duration_ms(options.account_timeout),
        poll_interval_ms: duration_ms(options.poll_interval),
        deadman_timeout_secs: options.deadman_timeout_secs,
        selected_accounts,
        accounts: reports,
        execution_incident_count,
        execution_incidents,
        regular_orders_all_clear: completion.regular_orders_all_clear,
        algo_orders_all_clear: completion.algo_orders_all_clear,
        spread_orders_all_clear: completion.spread_orders_all_clear,
        account_wide_orders_all_clear: completion.account_wide_orders_all_clear,
        evidence_complete: completion.evidence_complete,
        all_clear: completion.all_clear,
    })
}

fn collect_emergency_provenance(config_file_sha256: String) -> EmergencyProvenance {
    let mut incidents = Vec::new();
    let executable_sha256 = current_executable_sha256()
        .map_err(|error| incidents.push(format!("executable provenance failed: {error}")))
        .ok();
    let host_identity_sha256 = host_identity_sha256()
        .map_err(|error| incidents.push(format!("host provenance failed: {error}")))
        .ok();
    EmergencyProvenance {
        config_file_sha256,
        executable_sha256,
        host_identity_sha256,
        incidents,
    }
}

async fn collect_account_reports(
    tasks: &mut JoinSet<EmergencyAccountReport>,
    reports: &mut Vec<EmergencyAccountReport>,
    incident_count: &mut u64,
    incidents: &mut Vec<String>,
) {
    while let Some(result) = tasks.join_next().await {
        match result {
            Ok(report) => reports.push(report),
            Err(error) => push_incident(
                incident_count,
                incidents,
                format!("emergency account task failed: {error}"),
            ),
        }
    }
}

fn push_incident(count: &mut u64, incidents: &mut Vec<String>, message: String) {
    *count = count.saturating_add(1);
    if incidents.len() < MAX_INCIDENTS {
        incidents.push(truncate_utf8(message, MAX_INCIDENT_MESSAGE_BYTES));
    }
}

fn truncate_utf8(mut value: String, maximum_bytes: usize) -> String {
    if value.len() <= maximum_bytes {
        return value;
    }
    let mut boundary = maximum_bytes;
    while !value.is_char_boundary(boundary) {
        boundary -= 1;
    }
    value.truncate(boundary);
    value
}

async fn run_account_cancel(
    client: Box<dyn EmergencyAccountStopRole>,
    account_id: String,
    managed_symbols: HashSet<String>,
    settings: AccountCancelSettings,
) -> EmergencyAccountReport {
    let started = Instant::now();
    let mut report = EmergencyAccountReport::new(account_id.clone());
    let (clock, sampled, skew_ms) = match run_bounded(
        started,
        settings.account_timeout,
        sample_exchange_clock(client.as_ref()),
    )
    .await
    {
        Some(Ok((clock, skew_ms))) => (clock, true, Some(skew_ms)),
        Some(Err(error)) => {
            report.push_incident(format!(
                "exchange clock sampling failed; using local UTC for cancellation: {error}"
            ));
            (ExchangeClock::local(), false, None)
        }
        None => {
            report.push_incident(
                "account timeout expired while sampling exchange clock; cancellation could not start",
            );
            (ExchangeClock::local(), false, None)
        }
    };
    report.exchange_clock_sampled = sampled;
    report.exchange_clock_skew_ms = skew_ms;
    if skew_ms.is_some_and(|skew| skew > settings.max_exchange_clock_skew_ms) {
        report.push_incident(format!(
            "local/exchange clock skew {}ms exceeds configured maximum {}ms; exchange-adjusted timestamps are in use",
            skew_ms.unwrap_or_default(),
            settings.max_exchange_clock_skew_ms
        ));
    }

    match clock.timestamp() {
        Ok(timestamp) => match run_bounded(
            started,
            settings.account_timeout,
            client.cancel_all_after_at(&timestamp, settings.deadman_timeout_secs),
        )
        .await
        {
            Some(Ok(())) => report.deadman_armed = true,
            Some(Err(error)) => {
                report.push_incident(format!("failed to arm Cancel All After: {error}"));
            }
            None => report.push_incident("account timeout expired while arming Cancel All After"),
        },
        Err(error) => report.push_incident(format!("failed to format deadman timestamp: {error}")),
    }
    match clock.timestamp() {
        Ok(timestamp) => match run_bounded(
            started,
            settings.account_timeout,
            client.spread_cancel_all_after_at(&timestamp, settings.deadman_timeout_secs),
        )
        .await
        {
            Some(Ok(())) => report.spread_deadman_armed = true,
            Some(Err(error)) => {
                report.push_incident(format!("failed to arm spread Cancel All After: {error}"));
            }
            None => {
                report.push_incident("account timeout expired while arming spread Cancel All After")
            }
        },
        Err(error) => report.push_incident(format!(
            "failed to format spread deadman timestamp: {error}"
        )),
    }
    let verify_after = Instant::now() + settings.verification_delay;
    let mut pacer = RequestPacer::new(settings.pacing_policy.clone());
    let mut seen_regular_orders = BTreeSet::new();
    let mut seen_algo_orders = BTreeSet::new();
    let mut seen_spread_orders = BTreeSet::new();
    let mut unmanaged_symbols = BTreeSet::new();
    let mut last_orders: Option<AccountPendingOrders> = None;

    while started.elapsed() < settings.account_timeout {
        let orders = match enumerate_pending_orders(
            client.as_ref(),
            &clock,
            &account_id,
            &mut pacer,
            &mut report,
            started,
            &settings,
        )
        .await
        {
            Ok(orders) => orders,
            Err(error) => {
                report.push_incident(error);
                if started.elapsed() >= settings.account_timeout {
                    break;
                }
                sleep_bounded(settings.poll_interval, started, settings.account_timeout).await;
                continue;
            }
        };
        if report.initial_open_orders.is_none() {
            report.initial_open_orders = Some(orders.regular.len());
            report.initial_algo_orders = Some(orders.algo.len());
            report.initial_spread_orders = Some(orders.spread.len());
        }
        observe_orders(
            &orders.regular,
            &managed_symbols,
            &mut seen_regular_orders,
            &mut unmanaged_symbols,
            &mut report,
        );
        observe_algo_orders(
            &orders.algo,
            &managed_symbols,
            &mut seen_algo_orders,
            &mut unmanaged_symbols,
        );
        observe_spread_orders(&orders.spread, &mut seen_spread_orders);
        last_orders = Some(orders.clone());
        if orders.is_empty() {
            if Instant::now() >= verify_after {
                report.verified_zero_after_deadman = true;
                report.verified_algo_zero_after_deadman = true;
                report.verified_spread_zero_after_deadman = true;
                break;
            }
        } else {
            if !orders.regular.is_empty()
                && !cancel_pending_orders(
                    client.as_ref(),
                    &clock,
                    &orders.regular,
                    &mut pacer,
                    &mut report,
                    started,
                    &settings,
                )
                .await
            {
                break;
            }
            if !orders.algo.is_empty()
                && !cancel_pending_algo_orders(
                    client.as_ref(),
                    &clock,
                    &orders.algo,
                    &mut pacer,
                    &mut report,
                    started,
                    &settings,
                )
                .await
            {
                break;
            }
            if !orders.spread.is_empty()
                && !cancel_pending_spread_orders(
                    client.as_ref(),
                    &clock,
                    &mut pacer,
                    &mut report,
                    started,
                    &settings,
                )
                .await
            {
                break;
            }
        }
        sleep_bounded(settings.poll_interval, started, settings.account_timeout).await;
    }

    report.unique_orders_seen = seen_regular_orders.len();
    report.unique_algo_orders_seen = seen_algo_orders.len();
    report.unique_spread_orders_seen = seen_spread_orders.len();
    report.unmanaged_symbols = unmanaged_symbols.into_iter().collect();
    report.final_open_orders = last_orders.as_ref().map(|orders| orders.regular.len());
    report.final_algo_orders = last_orders.as_ref().map(|orders| orders.algo.len());
    report.final_spread_orders = last_orders.as_ref().map(|orders| orders.spread.len());
    report.remaining_orders = last_orders
        .as_ref()
        .map(|orders| orders.regular.clone())
        .unwrap_or_default()
        .into_iter()
        .take(MAX_REMAINING_ORDER_DETAILS)
        .map(order_ref)
        .collect();
    report.remaining_algo_orders = last_orders
        .as_ref()
        .map(|orders| orders.algo.clone())
        .unwrap_or_default()
        .into_iter()
        .take(MAX_REMAINING_ORDER_DETAILS)
        .map(algo_order_ref)
        .collect();
    report.remaining_spread_orders = last_orders
        .unwrap_or_default()
        .spread
        .into_iter()
        .take(MAX_REMAINING_ORDER_DETAILS)
        .map(spread_order_ref)
        .collect();
    if !report.verified_zero_after_deadman && started.elapsed() >= settings.account_timeout {
        report.push_incident("account timeout expired before every order domain was proven zero");
    }
    report.all_clear = report.deadman_armed
        && report.spread_deadman_armed
        && report.verified_zero_after_deadman
        && report.verified_algo_zero_after_deadman
        && report.verified_spread_zero_after_deadman;
    if report.all_clear {
        let account_identity_sha256 = sample_account_identity(
            client.as_ref(),
            &clock,
            &account_id,
            settings.environment,
            started,
            settings.account_timeout,
            &mut report,
        )
        .await;
        report.account_identity_sha256 = account_identity_sha256;
    }
    report.elapsed_ms = elapsed_ms(&started);
    report
}

async fn enumerate_pending_orders(
    client: &dyn EmergencyAccountStopRole,
    clock: &ExchangeClock,
    account_id: &str,
    pacer: &mut RequestPacer,
    report: &mut EmergencyAccountReport,
    started: Instant,
    settings: &AccountCancelSettings,
) -> Result<AccountPendingOrders, String> {
    let mut regular = RegularOrderPagination::new(settings.max_order_reconciliation_pages)
        .map_err(|error| enumeration_failure(report, "regular", error.to_string()))?;
    loop {
        let timestamp = prepare_enumeration_request(
            clock, account_id, pacer, report, started, settings, "regular",
        )
        .await?;
        let page = match run_bounded(
            started,
            settings.account_timeout,
            client.regular_pending_orders_page_at(&timestamp, regular.after()),
        )
        .await
        {
            Some(Ok(page)) => page,
            Some(Err(error)) => {
                return Err(enumeration_failure(report, "regular", error.to_string()));
            }
            None => {
                return Err(enumeration_failure(
                    report,
                    "regular",
                    "account timeout expired during request".to_string(),
                ));
            }
        };
        match regular.accept(page) {
            Ok(true) => break,
            Ok(false) => {}
            Err(error) => {
                return Err(enumeration_failure(report, "regular", error.to_string()));
            }
        }
    }

    let mut algo_orders = Vec::new();
    let mut algo_ids = BTreeSet::new();
    for query in AlgoOrderQuery::ALL {
        let mut algo = AlgoOrderPagination::new(settings.max_order_reconciliation_pages)
            .map_err(|error| enumeration_failure(report, "algo", error.to_string()))?;
        loop {
            let timestamp = prepare_enumeration_request(
                clock, account_id, pacer, report, started, settings, "algo",
            )
            .await?;
            let page = match run_bounded(
                started,
                settings.account_timeout,
                client.algo_pending_orders_page_at(&timestamp, query, algo.after()),
            )
            .await
            {
                Some(Ok(page)) => page,
                Some(Err(error)) => {
                    return Err(enumeration_failure(report, "algo", error.to_string()));
                }
                None => {
                    return Err(enumeration_failure(
                        report,
                        "algo",
                        "account timeout expired during request".to_string(),
                    ));
                }
            };
            match algo.accept(page) {
                Ok(true) => break,
                Ok(false) => {}
                Err(error) => {
                    return Err(enumeration_failure(report, "algo", error.to_string()));
                }
            }
        }
        for order in algo.into_orders() {
            if !algo_ids.insert(order.algo_id.clone()) {
                return Err(enumeration_failure(
                    report,
                    "algo",
                    format!(
                        "duplicate algo order {} across order-type queries",
                        order.algo_id
                    ),
                ));
            }
            algo_orders.push(order);
        }
    }

    let mut spread = SpreadOrderPagination::new(settings.max_order_reconciliation_pages)
        .map_err(|error| enumeration_failure(report, "spread", error.to_string()))?;
    loop {
        let timestamp = prepare_enumeration_request(
            clock, account_id, pacer, report, started, settings, "spread",
        )
        .await?;
        let page = match run_bounded(
            started,
            settings.account_timeout,
            client.spread_pending_orders_page_at(&timestamp, spread.end_id()),
        )
        .await
        {
            Some(Ok(page)) => page,
            Some(Err(error)) => {
                return Err(enumeration_failure(report, "spread", error.to_string()));
            }
            None => {
                return Err(enumeration_failure(
                    report,
                    "spread",
                    "account timeout expired during request".to_string(),
                ));
            }
        };
        match spread.accept(page) {
            Ok(true) => break,
            Ok(false) => {}
            Err(error) => {
                return Err(enumeration_failure(report, "spread", error.to_string()));
            }
        }
    }

    Ok(AccountPendingOrders {
        regular: regular.into_orders(),
        algo: algo_orders,
        spread: spread.into_orders(),
    })
}

async fn prepare_enumeration_request(
    clock: &ExchangeClock,
    account_id: &str,
    pacer: &mut RequestPacer,
    report: &mut EmergencyAccountReport,
    started: Instant,
    settings: &AccountCancelSettings,
    domain: &'static str,
) -> Result<String, String> {
    report.enumeration_attempts = report.enumeration_attempts.saturating_add(1);
    if run_bounded(
        started,
        settings.account_timeout,
        pacer.pace(RequestKind::Reconcile, account_id),
    )
    .await
    .is_none()
    {
        return Err(enumeration_failure(
            report,
            domain,
            "account timeout expired while pacing request".to_string(),
        ));
    }
    let timestamp = clock
        .timestamp()
        .map_err(|error| enumeration_failure(report, domain, error.to_string()))?;
    Ok(timestamp)
}

fn enumeration_failure(
    report: &mut EmergencyAccountReport,
    domain: &'static str,
    message: String,
) -> String {
    report.enumeration_failures = report.enumeration_failures.saturating_add(1);
    format!("{domain} pending-order enumeration failed: {message}")
}

fn observe_algo_orders(
    orders: &[AlgoOrder],
    managed_symbols: &HashSet<String>,
    seen_orders: &mut BTreeSet<String>,
    unmanaged_symbols: &mut BTreeSet<String>,
) {
    for order in orders {
        if !managed_symbols.contains(&order.symbol) {
            unmanaged_symbols.insert(order.symbol.clone());
        }
        seen_orders.insert(order.algo_id.clone());
    }
}

fn observe_spread_orders(orders: &[SpreadOrder], seen_orders: &mut BTreeSet<String>) {
    seen_orders.extend(orders.iter().map(|order| order.exchange_order_id.clone()));
}

async fn sample_account_identity(
    client: &dyn EmergencyAccountStopRole,
    clock: &ExchangeClock,
    account_id: &str,
    environment: TradingEnvironment,
    started: Instant,
    account_timeout: Duration,
    report: &mut EmergencyAccountReport,
) -> Option<String> {
    let timestamp = match clock.timestamp() {
        Ok(timestamp) => timestamp,
        Err(error) => {
            report.push_incident(format!(
                "failed to format account-identity timestamp: {error}"
            ));
            return None;
        }
    };
    match run_bounded(
        started,
        account_timeout,
        client.account_identity_at(&timestamp),
    )
    .await
    {
        Some(Ok(config))
            if !config.user_id.trim().is_empty() && !config.main_user_id.trim().is_empty() =>
        {
            Some(okx_account_identity_sha256(
                environment,
                account_id,
                &config.user_id,
                &config.main_user_id,
            ))
        }
        Some(Ok(_)) => {
            report.push_incident("exchange account identity response was empty");
            None
        }
        Some(Err(error)) => {
            report.push_incident(format!("exchange account identity query failed: {error}"));
            None
        }
        None => {
            report
                .push_incident("account timeout expired while querying exchange account identity");
            None
        }
    }
}

fn observe_orders(
    orders: &[RegularOrder],
    managed_symbols: &HashSet<String>,
    seen_orders: &mut BTreeSet<(String, String)>,
    unmanaged_symbols: &mut BTreeSet<String>,
    report: &mut EmergencyAccountReport,
) {
    for (index, order) in orders.iter().enumerate() {
        if !managed_symbols.contains(&order.symbol) {
            unmanaged_symbols.insert(order.symbol.clone());
        }
        let identity = if !order.exchange_order_id.is_empty() {
            order.exchange_order_id.clone()
        } else if !order.client_order_id.is_empty() {
            order.client_order_id.clone()
        } else {
            report.push_incident(format!(
                "pending order {} at response index {index} has no exchange or client id",
                order.symbol
            ));
            format!("missing-id-{index}")
        };
        seen_orders.insert((order.symbol.clone(), identity));
    }
}

#[allow(clippy::too_many_arguments)]
async fn cancel_pending_orders(
    client: &dyn EmergencyAccountStopRole,
    clock: &ExchangeClock,
    orders: &[RegularOrder],
    pacer: &mut RequestPacer,
    report: &mut EmergencyAccountReport,
    started: Instant,
    settings: &AccountCancelSettings,
) -> bool {
    let cancels = orders
        .iter()
        .filter_map(|order| {
            let exchange_order_id =
                (!order.exchange_order_id.is_empty()).then(|| order.exchange_order_id.clone());
            let client_order_id =
                (!order.client_order_id.is_empty()).then(|| order.client_order_id.clone());
            (exchange_order_id.is_some() || client_order_id.is_some()).then(|| CancelOrder {
                symbol: order.symbol.clone(),
                exchange_order_id,
                client_order_id,
            })
        })
        .collect::<Vec<_>>();
    for batch in cancels.chunks(20) {
        if started.elapsed() >= settings.account_timeout {
            report.push_incident("account timeout expired before all cancel batches were sent");
            return false;
        }
        for cancel in batch {
            if run_bounded(
                started,
                settings.account_timeout,
                pacer.pace(RequestKind::Cancel, &cancel.symbol),
            )
            .await
            .is_none()
            {
                report.push_incident("account timeout expired while pacing cancel requests");
                return false;
            }
        }
        let timestamp = match clock.timestamp() {
            Ok(timestamp) => timestamp,
            Err(error) => {
                report.push_incident(format!("failed to format cancel timestamp: {error}"));
                return false;
            }
        };
        report.cancel_batches = report.cancel_batches.saturating_add(1);
        match run_bounded(
            started,
            settings.account_timeout,
            client.cancel_batch_orders_at(&timestamp, batch),
        )
        .await
        {
            Some(Ok(results)) => {
                if results.len() != batch.len() {
                    report.push_incident(format!(
                        "batch cancel returned {} results for {} orders",
                        results.len(),
                        batch.len()
                    ));
                }
                let mut matched = HashSet::new();
                for result in results {
                    match matching_cancel_index(batch, &result) {
                        Some(index) if matched.insert(index) => {}
                        Some(_) => report.push_incident(format!(
                            "batch cancel returned a duplicate result for order {}/{}",
                            result.exchange_order_id, result.client_order_id
                        )),
                        None => report.push_incident(format!(
                            "batch cancel returned an unknown result for order {}/{}",
                            result.exchange_order_id, result.client_order_id
                        )),
                    }
                    if result.accepted() {
                        report.accepted_cancel_requests =
                            report.accepted_cancel_requests.saturating_add(1);
                    } else {
                        report.rejected_cancel_requests =
                            report.rejected_cancel_requests.saturating_add(1);
                        report.push_incident(format!(
                            "cancel rejected for order {}/{}: {} {}",
                            result.exchange_order_id,
                            result.client_order_id,
                            result.code,
                            result.message
                        ));
                    }
                }
                let unacknowledged = batch.len().saturating_sub(matched.len()) as u64;
                report.unacknowledged_cancel_requests = report
                    .unacknowledged_cancel_requests
                    .saturating_add(unacknowledged);
                if unacknowledged > 0 {
                    report.push_incident(format!(
                        "batch cancel left {unacknowledged} request(s) without a matching acknowledgement"
                    ));
                }
            }
            Some(Err(error)) => {
                report.cancel_batch_failures = report.cancel_batch_failures.saturating_add(1);
                report.unacknowledged_cancel_requests = report
                    .unacknowledged_cancel_requests
                    .saturating_add(batch.len() as u64);
                report.push_incident(format!("batch cancel request failed: {error}"));
            }
            None => {
                report.cancel_batch_failures = report.cancel_batch_failures.saturating_add(1);
                report.unacknowledged_cancel_requests = report
                    .unacknowledged_cancel_requests
                    .saturating_add(batch.len() as u64);
                report.push_incident("account timeout expired during a batch cancel request");
                return false;
            }
        }
    }
    true
}

#[allow(clippy::too_many_arguments)]
async fn cancel_pending_algo_orders(
    client: &dyn EmergencyAccountStopRole,
    clock: &ExchangeClock,
    orders: &[AlgoOrder],
    pacer: &mut RequestPacer,
    report: &mut EmergencyAccountReport,
    started: Instant,
    settings: &AccountCancelSettings,
) -> bool {
    let cancels = orders
        .iter()
        .map(|order| CancelAlgoOrder {
            symbol: order.symbol.clone(),
            algo_id: order.algo_id.clone(),
        })
        .collect::<Vec<_>>();
    for batch in cancels.chunks(ALGO_CANCEL_BATCH_LIMIT) {
        for cancel in batch {
            if run_bounded(
                started,
                settings.account_timeout,
                pacer.pace(RequestKind::Cancel, &cancel.symbol),
            )
            .await
            .is_none()
            {
                report.push_incident("account timeout expired while pacing algo cancel requests");
                return false;
            }
        }
        let timestamp = match clock.timestamp() {
            Ok(timestamp) => timestamp,
            Err(error) => {
                report.push_incident(format!("failed to format algo cancel timestamp: {error}"));
                return false;
            }
        };
        report.algo_cancel_batches = report.algo_cancel_batches.saturating_add(1);
        match run_bounded(
            started,
            settings.account_timeout,
            client.cancel_algo_orders_at(&timestamp, batch),
        )
        .await
        {
            Some(Ok(results)) => {
                if results.len() != batch.len() {
                    report.push_incident(format!(
                        "algo cancel returned {} results for {} orders",
                        results.len(),
                        batch.len()
                    ));
                }
                let mut matched = HashSet::new();
                for result in results {
                    match matching_algo_cancel_index(batch, &result) {
                        Some(index) if matched.insert(index) => {}
                        Some(_) => report.push_incident(format!(
                            "algo cancel returned a duplicate result for {}",
                            result.algo_id
                        )),
                        None => report.push_incident(format!(
                            "algo cancel returned an unknown result for {}",
                            result.algo_id
                        )),
                    }
                    if result.accepted() {
                        report.accepted_algo_cancel_requests =
                            report.accepted_algo_cancel_requests.saturating_add(1);
                    } else {
                        report.rejected_algo_cancel_requests =
                            report.rejected_algo_cancel_requests.saturating_add(1);
                        report.push_incident(format!(
                            "algo cancel rejected for {}: {} {}",
                            result.algo_id, result.code, result.message
                        ));
                    }
                }
                let unacknowledged = batch.len().saturating_sub(matched.len()) as u64;
                report.unacknowledged_algo_cancel_requests = report
                    .unacknowledged_algo_cancel_requests
                    .saturating_add(unacknowledged);
                if unacknowledged > 0 {
                    report.push_incident(format!(
                        "algo cancel left {unacknowledged} request(s) without a matching acknowledgement"
                    ));
                }
            }
            Some(Err(error)) => {
                report.algo_cancel_batch_failures =
                    report.algo_cancel_batch_failures.saturating_add(1);
                report.unacknowledged_algo_cancel_requests = report
                    .unacknowledged_algo_cancel_requests
                    .saturating_add(batch.len() as u64);
                report.push_incident(format!("algo cancel request failed: {error}"));
            }
            None => {
                report.algo_cancel_batch_failures =
                    report.algo_cancel_batch_failures.saturating_add(1);
                report.unacknowledged_algo_cancel_requests = report
                    .unacknowledged_algo_cancel_requests
                    .saturating_add(batch.len() as u64);
                report.push_incident("account timeout expired during an algo cancel request");
                return false;
            }
        }
    }
    true
}

#[allow(clippy::too_many_arguments)]
async fn cancel_pending_spread_orders(
    client: &dyn EmergencyAccountStopRole,
    clock: &ExchangeClock,
    pacer: &mut RequestPacer,
    report: &mut EmergencyAccountReport,
    started: Instant,
    settings: &AccountCancelSettings,
) -> bool {
    if run_bounded(
        started,
        settings.account_timeout,
        pacer.pace(RequestKind::Cancel, &report.account_id),
    )
    .await
    .is_none()
    {
        report.push_incident("account timeout expired while pacing spread mass cancel");
        return false;
    }
    let timestamp = match clock.timestamp() {
        Ok(timestamp) => timestamp,
        Err(error) => {
            report.push_incident(format!("failed to format spread cancel timestamp: {error}"));
            return false;
        }
    };
    report.spread_mass_cancel_attempts = report.spread_mass_cancel_attempts.saturating_add(1);
    match run_bounded(
        started,
        settings.account_timeout,
        client.spread_mass_cancel_at(&timestamp),
    )
    .await
    {
        Some(Ok(())) => true,
        Some(Err(error)) => {
            report.spread_mass_cancel_failures =
                report.spread_mass_cancel_failures.saturating_add(1);
            report.push_incident(format!("spread mass cancel request failed: {error}"));
            true
        }
        None => {
            report.spread_mass_cancel_failures =
                report.spread_mass_cancel_failures.saturating_add(1);
            report.push_incident("account timeout expired during spread mass cancel");
            false
        }
    }
}

fn matching_cancel_index(batch: &[CancelOrder], result: &CancelOrderResult) -> Option<usize> {
    batch.iter().position(|cancel| {
        (!result.exchange_order_id.is_empty()
            && cancel.exchange_order_id.as_deref() == Some(result.exchange_order_id.as_str()))
            || (!result.client_order_id.is_empty()
                && cancel.client_order_id.as_deref() == Some(result.client_order_id.as_str()))
    })
}

fn matching_algo_cancel_index(
    batch: &[CancelAlgoOrder],
    result: &AlgoCancelResult,
) -> Option<usize> {
    batch
        .iter()
        .position(|cancel| cancel.algo_id == result.algo_id)
}

async fn sample_exchange_clock(
    client: &dyn EmergencyAccountStopRole,
) -> Result<(ExchangeClock, u64), EmergencyRoleError> {
    let before_ms = unix_time_ms();
    let before = Instant::now();
    let exchange_ms = client.server_time_ms().await?;
    let after_ms = unix_time_ms();
    let round_trip = before.elapsed();
    let midpoint_ms = before_ms.saturating_add(after_ms.saturating_sub(before_ms) / 2);
    Ok((
        ExchangeClock {
            exchange_ms,
            sampled_at: before + round_trip / 2,
        },
        midpoint_ms.abs_diff(exchange_ms),
    ))
}

async fn sleep_bounded(interval: Duration, started: Instant, timeout: Duration) {
    let remaining = timeout.saturating_sub(started.elapsed());
    if !remaining.is_zero() {
        tokio::time::sleep(interval.min(remaining)).await;
    }
}

async fn run_bounded<F, T>(started: Instant, timeout: Duration, future: F) -> Option<T>
where
    F: Future<Output = T>,
{
    let remaining = timeout.saturating_sub(started.elapsed());
    if remaining.is_zero() {
        return None;
    }
    tokio::time::timeout(remaining, future).await.ok()
}

fn order_ref(order: RegularOrder) -> EmergencyOrderRef {
    EmergencyOrderRef {
        symbol: order.symbol,
        exchange_order_id: order.exchange_order_id,
        client_order_id: order.client_order_id,
    }
}

fn algo_order_ref(order: AlgoOrder) -> EmergencyAlgoOrderRef {
    EmergencyAlgoOrderRef {
        symbol: order.symbol,
        algo_id: order.algo_id,
        client_order_id: order.client_order_id,
    }
}

fn spread_order_ref(order: SpreadOrder) -> EmergencySpreadOrderRef {
    EmergencySpreadOrderRef {
        spread_id: order.spread_id,
        exchange_order_id: order.exchange_order_id,
        client_order_id: order.client_order_id,
    }
}

fn okx_account_identity_sha256(
    environment: TradingEnvironment,
    account_id: &str,
    user_id: &str,
    main_user_id: &str,
) -> String {
    let environment = match environment {
        TradingEnvironment::Demo => b"demo".as_slice(),
        TradingEnvironment::Production => b"production".as_slice(),
    };
    identity_sha256(
        b"reap-okx-account-v1",
        &[
            environment,
            account_id.as_bytes(),
            user_id.trim().as_bytes(),
            main_user_id.trim().as_bytes(),
        ],
    )
}

fn format_okx_timestamp_ms(timestamp_ms: u64) -> Result<String, EmergencyRoleError> {
    let original = timestamp_ms.to_string();
    let timestamp_ms = i64::try_from(timestamp_ms).map_err(|error| {
        EmergencyRoleError(format!(
            "invalid OKX response field OK-ACCESS-TIMESTAMP={original:?}: {error}"
        ))
    })?;
    let timestamp = chrono::DateTime::<Utc>::from_timestamp_millis(timestamp_ms).ok_or_else(|| {
        EmergencyRoleError(format!(
            "invalid OKX response field OK-ACCESS-TIMESTAMP={original:?}: timestamp is outside the supported range"
        ))
    })?;
    Ok(timestamp.to_rfc3339_opts(SecondsFormat::Millis, true))
}

fn unix_time_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u64::MAX as u128) as u64
}

fn unix_time_ns() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
}

fn elapsed_ms(started: &Instant) -> u64 {
    duration_ms(started.elapsed())
}

fn duration_ms(duration: Duration) -> u64 {
    duration.as_millis().min(u64::MAX as u128) as u64
}

#[cfg(test)]
mod tests {
    use std::collections::{HashMap, VecDeque};
    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;

    use super::*;

    type RoleResult<T> = Result<T, EmergencyRoleError>;

    #[derive(Default)]
    struct Script {
        calls: Vec<String>,
        server_times: VecDeque<RoleResult<u64>>,
        identities: VecDeque<RoleResult<EmergencyAccountIdentity>>,
        regular_pages: VecDeque<RoleResult<RegularOrderPage>>,
        algo_pages: VecDeque<RoleResult<AlgoOrderPage>>,
        spread_pages: VecDeque<RoleResult<SpreadOrderPage>>,
        regular_deadman: VecDeque<RoleResult<()>>,
        spread_deadman: VecDeque<RoleResult<()>>,
        regular_cancels: VecDeque<RoleResult<Vec<CancelOrderResult>>>,
        algo_cancels: VecDeque<RoleResult<Vec<AlgoCancelResult>>>,
        spread_cancels: VecDeque<RoleResult<()>>,
    }

    #[derive(Clone)]
    struct ScriptedRole(Arc<Mutex<Script>>);

    impl ScriptedRole {
        fn new(script: Script) -> (Self, Arc<Mutex<Script>>) {
            let script = Arc::new(Mutex::new(script));
            (Self(Arc::clone(&script)), script)
        }
    }

    fn next<T>(queue: &mut VecDeque<RoleResult<T>>, operation: &str) -> RoleResult<T> {
        queue
            .pop_front()
            .unwrap_or_else(|| panic!("missing scripted response for {operation}"))
    }

    #[async_trait]
    impl EmergencyAccountStopRole for ScriptedRole {
        async fn server_time_ms(&self) -> RoleResult<u64> {
            let mut script = self.0.lock().unwrap();
            script.calls.push("server_time".to_string());
            next(&mut script.server_times, "server_time")
        }

        async fn account_identity_at(
            &self,
            _timestamp: &str,
        ) -> RoleResult<EmergencyAccountIdentity> {
            let mut script = self.0.lock().unwrap();
            script.calls.push("account_identity".to_string());
            next(&mut script.identities, "account_identity")
        }

        async fn regular_pending_orders_page_at(
            &self,
            _timestamp: &str,
            _after: Option<&str>,
        ) -> RoleResult<RegularOrderPage> {
            let mut script = self.0.lock().unwrap();
            script.calls.push("enumerate_regular".to_string());
            next(&mut script.regular_pages, "enumerate_regular")
        }

        async fn algo_pending_orders_page_at(
            &self,
            _timestamp: &str,
            query: AlgoOrderQuery,
            _after: Option<&str>,
        ) -> RoleResult<AlgoOrderPage> {
            let mut script = self.0.lock().unwrap();
            script.calls.push(format!("enumerate_algo:{query:?}"));
            next(&mut script.algo_pages, "enumerate_algo")
        }

        async fn spread_pending_orders_page_at(
            &self,
            _timestamp: &str,
            _end_id: Option<&str>,
        ) -> RoleResult<SpreadOrderPage> {
            let mut script = self.0.lock().unwrap();
            script.calls.push("enumerate_spread".to_string());
            next(&mut script.spread_pages, "enumerate_spread")
        }

        async fn cancel_all_after_at(
            &self,
            _timestamp: &str,
            _timeout_secs: u64,
        ) -> RoleResult<()> {
            let mut script = self.0.lock().unwrap();
            script.calls.push("arm_regular_deadman".to_string());
            next(&mut script.regular_deadman, "arm_regular_deadman")
        }

        async fn spread_cancel_all_after_at(
            &self,
            _timestamp: &str,
            _timeout_secs: u64,
        ) -> RoleResult<()> {
            let mut script = self.0.lock().unwrap();
            script.calls.push("arm_spread_deadman".to_string());
            next(&mut script.spread_deadman, "arm_spread_deadman")
        }

        async fn cancel_batch_orders_at(
            &self,
            _timestamp: &str,
            _orders: &[CancelOrder],
        ) -> RoleResult<Vec<CancelOrderResult>> {
            let mut script = self.0.lock().unwrap();
            script.calls.push("cancel_regular".to_string());
            next(&mut script.regular_cancels, "cancel_regular")
        }

        async fn cancel_algo_orders_at(
            &self,
            _timestamp: &str,
            _orders: &[CancelAlgoOrder],
        ) -> RoleResult<Vec<AlgoCancelResult>> {
            let mut script = self.0.lock().unwrap();
            script.calls.push("cancel_algo".to_string());
            next(&mut script.algo_cancels, "cancel_algo")
        }

        async fn spread_mass_cancel_at(&self, _timestamp: &str) -> RoleResult<()> {
            let mut script = self.0.lock().unwrap();
            script.calls.push("cancel_spread".to_string());
            next(&mut script.spread_cancels, "cancel_spread")
        }
    }

    fn empty_regular_page() -> RegularOrderPage {
        RegularOrderPage {
            orders: Vec::new(),
            next_after: None,
        }
    }

    fn empty_algo_page() -> AlgoOrderPage {
        AlgoOrderPage {
            orders: Vec::new(),
            next_after: None,
        }
    }

    fn empty_spread_page() -> SpreadOrderPage {
        SpreadOrderPage {
            orders: Vec::new(),
            next_end_id: None,
        }
    }

    fn settings() -> AccountCancelSettings {
        AccountCancelSettings {
            environment: TradingEnvironment::Demo,
            account_timeout: Duration::from_secs(1),
            poll_interval: Duration::from_millis(1),
            verification_delay: Duration::ZERO,
            pacing_policy: PacingPolicy {
                submit_requests: 100,
                cancel_requests: 100,
                reconcile_requests: 100,
                window: Duration::from_millis(1),
            },
            max_exchange_clock_skew_ms: 250,
            deadman_timeout_secs: 10,
            max_order_reconciliation_pages: 2,
        }
    }

    fn complete_provenance() -> EmergencyProvenance {
        EmergencyProvenance {
            config_file_sha256: "1".repeat(64),
            executable_sha256: Some("2".repeat(64)),
            host_identity_sha256: Some("3".repeat(64)),
            incidents: Vec::new(),
        }
    }

    #[tokio::test]
    async fn account_task_join_failure_becomes_bounded_evidence() {
        let mut tasks = JoinSet::new();
        let task = tasks.spawn(std::future::pending::<EmergencyAccountReport>());
        task.abort();
        let mut reports = Vec::new();
        let mut incident_count = 0;
        let mut incidents = Vec::new();

        collect_account_reports(
            &mut tasks,
            &mut reports,
            &mut incident_count,
            &mut incidents,
        )
        .await;

        assert!(reports.is_empty());
        assert_eq!(incident_count, 1);
        assert_eq!(incidents.len(), 1);
        assert!(incidents[0].contains("emergency account task failed"));
    }

    #[tokio::test]
    async fn combined_workflow_preserves_domain_enumeration_and_cancel_ordering() {
        let mut script = Script {
            server_times: [Ok(unix_time_ms())].into(),
            identities: [Ok(EmergencyAccountIdentity {
                user_id: "7".to_string(),
                main_user_id: "6".to_string(),
            })]
            .into(),
            regular_pages: [
                Ok(RegularOrderPage {
                    orders: vec![RegularOrder {
                        symbol: "BTC-USDT".to_string(),
                        exchange_order_id: "regular-1".to_string(),
                        client_order_id: "client-1".to_string(),
                    }],
                    next_after: None,
                }),
                Ok(empty_regular_page()),
            ]
            .into(),
            spread_pages: [
                Ok(SpreadOrderPage {
                    orders: vec![SpreadOrder {
                        spread_id: "BTC-USDT_BTC-USDT-SWAP".to_string(),
                        exchange_order_id: "spread-1".to_string(),
                        client_order_id: "spread-client-1".to_string(),
                    }],
                    next_end_id: None,
                }),
                Ok(empty_spread_page()),
            ]
            .into(),
            regular_deadman: [Ok(())].into(),
            spread_deadman: [Ok(())].into(),
            regular_cancels: [Ok(vec![CancelOrderResult {
                exchange_order_id: "regular-1".to_string(),
                client_order_id: "client-1".to_string(),
                code: "0".to_string(),
                message: String::new(),
            }])]
            .into(),
            algo_cancels: [Ok(vec![AlgoCancelResult {
                algo_id: "algo-1".to_string(),
                client_order_id: "algo-client-1".to_string(),
                code: "0".to_string(),
                message: String::new(),
            }])]
            .into(),
            spread_cancels: [Ok(())].into(),
            ..Script::default()
        };
        script.algo_pages.push_back(Ok(AlgoOrderPage {
            orders: vec![AlgoOrder {
                algo_id: "algo-1".to_string(),
                client_order_id: "algo-client-1".to_string(),
                symbol: "BTC-USDT".to_string(),
            }],
            next_after: None,
        }));
        script
            .algo_pages
            .extend((1..AlgoOrderQuery::ALL.len()).map(|_| Ok(empty_algo_page())));
        script
            .algo_pages
            .extend((0..AlgoOrderQuery::ALL.len()).map(|_| Ok(empty_algo_page())));
        let (role, recorded) = ScriptedRole::new(script);

        let report = run_account_cancel(
            Box::new(role),
            "main".to_string(),
            HashSet::from(["BTC-USDT".to_string()]),
            settings(),
        )
        .await;

        assert!(report.all_clear, "{:?}", report.incidents);
        assert_eq!(report.initial_open_orders, Some(1));
        assert_eq!(report.initial_algo_orders, Some(1));
        assert_eq!(report.initial_spread_orders, Some(1));
        assert_eq!(report.accepted_cancel_requests, 1);
        assert_eq!(report.accepted_algo_cancel_requests, 1);
        assert_eq!(report.spread_mass_cancel_attempts, 1);
        assert!(report.account_identity_sha256.is_some());

        let calls = &recorded.lock().unwrap().calls;
        assert_eq!(
            &calls[..3],
            ["server_time", "arm_regular_deadman", "arm_spread_deadman"]
        );
        assert_eq!(calls[3], "enumerate_regular");
        assert!(calls[4].starts_with("enumerate_algo:"));
        assert_eq!(calls[11], "enumerate_spread");
        assert_eq!(
            &calls[12..15],
            ["cancel_regular", "cancel_algo", "cancel_spread"]
        );
        assert_eq!(calls[15], "enumerate_regular");
        assert_eq!(calls[23], "enumerate_spread");
        assert_eq!(calls[24], "account_identity");
    }

    #[tokio::test]
    async fn partial_batch_ack_is_reported_but_final_zero_remains_authoritative() {
        let mut script = Script {
            server_times: [Ok(unix_time_ms())].into(),
            identities: [Ok(EmergencyAccountIdentity {
                user_id: "7".to_string(),
                main_user_id: "6".to_string(),
            })]
            .into(),
            regular_pages: [
                Ok(RegularOrderPage {
                    orders: vec![
                        RegularOrder {
                            symbol: "BTC-USDT".to_string(),
                            exchange_order_id: "123".to_string(),
                            client_order_id: "client-1".to_string(),
                        },
                        RegularOrder {
                            symbol: "BTC-USDT".to_string(),
                            exchange_order_id: "456".to_string(),
                            client_order_id: "client-2".to_string(),
                        },
                    ],
                    next_after: None,
                }),
                Ok(empty_regular_page()),
            ]
            .into(),
            spread_pages: [Ok(empty_spread_page()), Ok(empty_spread_page())].into(),
            regular_deadman: [Ok(())].into(),
            spread_deadman: [Ok(())].into(),
            regular_cancels: [Ok(vec![CancelOrderResult {
                exchange_order_id: "123".to_string(),
                client_order_id: "client-1".to_string(),
                code: "51000".to_string(),
                message: "rejected".to_string(),
            }])]
            .into(),
            ..Script::default()
        };
        script
            .algo_pages
            .extend((0..AlgoOrderQuery::ALL.len() * 2).map(|_| Ok(empty_algo_page())));
        let (role, _) = ScriptedRole::new(script);

        let report = run_account_cancel(
            Box::new(role),
            "main".to_string(),
            HashSet::new(),
            settings(),
        )
        .await;

        assert!(report.all_clear, "{:?}", report.incidents);
        assert!(report.account_identity_sha256.is_some());
        assert_eq!(report.rejected_cancel_requests, 1);
        assert_eq!(report.unacknowledged_cancel_requests, 1);
        assert_eq!(report.cancel_batch_failures, 0);
        assert!(
            report
                .incidents
                .iter()
                .any(|message| { message.contains("without a matching acknowledgement") })
        );
    }

    #[tokio::test]
    async fn failed_algo_enumeration_prevents_every_cancel_domain() {
        let script = Script {
            server_times: [Ok(unix_time_ms())].into(),
            regular_pages: [Ok(RegularOrderPage {
                orders: vec![RegularOrder {
                    symbol: "BTC-USDT".to_string(),
                    exchange_order_id: "regular-1".to_string(),
                    client_order_id: String::new(),
                }],
                next_after: None,
            })]
            .into(),
            algo_pages: [Err(EmergencyRoleError("algo unavailable".to_string()))].into(),
            regular_deadman: [Ok(())].into(),
            spread_deadman: [Ok(())].into(),
            ..Script::default()
        };
        let (role, recorded) = ScriptedRole::new(script);
        let mut bounded = settings();
        bounded.account_timeout = Duration::from_millis(20);
        bounded.poll_interval = Duration::from_millis(20);

        let report =
            run_account_cancel(Box::new(role), "main".to_string(), HashSet::new(), bounded).await;

        assert!(!report.all_clear);
        assert_eq!(report.initial_open_orders, None);
        assert!(report.incidents.iter().any(|message| {
            message.contains("algo pending-order enumeration failed: algo unavailable")
        }));
        assert!(
            recorded
                .lock()
                .unwrap()
                .calls
                .iter()
                .all(|call| !call.starts_with("cancel_"))
        );
    }

    #[tokio::test]
    async fn deadman_failure_is_not_hidden_by_a_zero_snapshot() {
        let mut script = Script {
            server_times: [Ok(unix_time_ms())].into(),
            regular_pages: [Ok(empty_regular_page())].into(),
            spread_pages: [Ok(empty_spread_page())].into(),
            regular_deadman: [Err(EmergencyRoleError("deadman unavailable".to_string()))].into(),
            spread_deadman: [Ok(())].into(),
            ..Script::default()
        };
        script
            .algo_pages
            .extend((0..AlgoOrderQuery::ALL.len()).map(|_| Ok(empty_algo_page())));
        let (role, _) = ScriptedRole::new(script);

        let report = run_account_cancel(
            Box::new(role),
            "main".to_string(),
            HashSet::new(),
            settings(),
        )
        .await;

        assert!(report.verified_zero_after_deadman);
        assert!(!report.deadman_armed);
        assert!(!report.all_clear);
        assert!(
            report
                .incidents
                .iter()
                .any(|message| message.contains("failed to arm Cancel All After"))
        );
    }

    struct FailingFactory(EmergencyRoleSetupError);

    impl EmergencyAccountStopFactory for FailingFactory {
        fn create(
            &self,
            _venue: &EmergencyVenueConfig,
            _runtime: &EmergencyRuntimeConfig,
            _account: &EmergencyAccountConfig,
        ) -> Result<Box<dyn EmergencyAccountStopRole>, EmergencyRoleSetupError> {
            Err(self.0.clone())
        }
    }

    #[tokio::test]
    async fn credential_setup_failure_remains_schema_bound_evidence() {
        let config = EmergencyFileConfig {
            venue: EmergencyVenueConfig::default(),
            runtime: EmergencyRuntimeConfig::default(),
            accounts: vec![EmergencyAccountConfig {
                id: "main".to_string(),
                api_key_env: "API_KEY".to_string(),
                secret_key_env: "SECRET_KEY".to_string(),
                passphrase_env: "PASSPHRASE".to_string(),
                trade_modes: HashMap::new(),
            }],
        };
        let options = EmergencyCancelOptions {
            account_ids: vec!["main".to_string()],
            confirm_account_wide_cancel: true,
            confirm_order_producers_stopped: true,
            ..EmergencyCancelOptions::default()
        };

        let report = run_emergency_cancel(
            config,
            options,
            complete_provenance(),
            &FailingFactory(EmergencyRoleSetupError::Credential(
                "missing credential".to_string(),
            )),
        )
        .await
        .unwrap();

        assert_eq!(
            report.schema_version,
            EMERGENCY_CANCEL_REPORT_SCHEMA_VERSION
        );
        assert_eq!(report.selected_accounts, ["main"]);
        assert_eq!(report.accounts.len(), 1);
        assert_eq!(
            report.accounts[0].incidents,
            ["credential setup failed: missing credential"]
        );
        assert!(!report.evidence_complete);
        assert!(!report.all_clear);
        let encoded = serde_json::to_vec(&report).unwrap();
        let decoded: EmergencyCancelReport = serde_json::from_slice(&encoded).unwrap();
        assert_eq!(decoded.config_file_sha256, "1".repeat(64));
    }

    #[tokio::test]
    async fn collector_failure_report_remains_structurally_verifiable() {
        for name in [
            "REAP_EMERGENCY_VERIFY_TEST_MISSING_KEY",
            "REAP_EMERGENCY_VERIFY_TEST_MISSING_SECRET",
            "REAP_EMERGENCY_VERIFY_TEST_MISSING_PASSPHRASE",
        ] {
            assert!(std::env::var_os(name).is_none());
        }
        let directory = tempfile::tempdir().unwrap();
        let config_path = directory.path().join("live.toml");
        let report_path = directory.path().join("emergency.json");
        std::fs::write(
            &config_path,
            r#"[venue]
environment = "demo"
rest_url = "https://www.okx.com"

[[accounts]]
id = "main"
api_key_env = "REAP_EMERGENCY_VERIFY_TEST_MISSING_KEY"
secret_key_env = "REAP_EMERGENCY_VERIFY_TEST_MISSING_SECRET"
passphrase_env = "REAP_EMERGENCY_VERIFY_TEST_MISSING_PASSPHRASE"
"#,
        )
        .unwrap();

        let report = run_emergency_cancel_path(
            &config_path,
            EmergencyCancelOptions {
                account_ids: vec!["main".to_string()],
                confirm_account_wide_cancel: true,
                confirm_order_producers_stopped: true,
                account_timeout: Duration::from_secs(40),
                ..EmergencyCancelOptions::default()
            },
        )
        .await
        .unwrap();
        assert!(!report.all_clear);
        std::fs::write(&report_path, serde_json::to_vec_pretty(&report).unwrap()).unwrap();

        let verification = verify_emergency_cancel_paths(
            &config_path,
            &report_path,
            EmergencyCancelVerificationOptions {
                require_all_configured_accounts: true,
            },
        )
        .unwrap();

        assert!(verification.evidence_valid, "{:?}", verification.failures);
        assert!(!verification.acceptance_passed);
        assert!(!verification.derived_regular_orders_all_clear);
        assert!(!verification.derived_evidence_complete);
    }

    struct HangingRole;

    #[async_trait]
    impl EmergencyAccountStopRole for HangingRole {
        async fn server_time_ms(&self) -> RoleResult<u64> {
            std::future::pending().await
        }

        async fn account_identity_at(
            &self,
            _timestamp: &str,
        ) -> RoleResult<EmergencyAccountIdentity> {
            unreachable!()
        }

        async fn regular_pending_orders_page_at(
            &self,
            _timestamp: &str,
            _after: Option<&str>,
        ) -> RoleResult<RegularOrderPage> {
            unreachable!()
        }

        async fn algo_pending_orders_page_at(
            &self,
            _timestamp: &str,
            _query: AlgoOrderQuery,
            _after: Option<&str>,
        ) -> RoleResult<AlgoOrderPage> {
            unreachable!()
        }

        async fn spread_pending_orders_page_at(
            &self,
            _timestamp: &str,
            _end_id: Option<&str>,
        ) -> RoleResult<SpreadOrderPage> {
            unreachable!()
        }

        async fn cancel_all_after_at(
            &self,
            _timestamp: &str,
            _timeout_secs: u64,
        ) -> RoleResult<()> {
            unreachable!()
        }

        async fn spread_cancel_all_after_at(
            &self,
            _timestamp: &str,
            _timeout_secs: u64,
        ) -> RoleResult<()> {
            unreachable!()
        }

        async fn cancel_batch_orders_at(
            &self,
            _timestamp: &str,
            _orders: &[CancelOrder],
        ) -> RoleResult<Vec<CancelOrderResult>> {
            unreachable!()
        }

        async fn cancel_algo_orders_at(
            &self,
            _timestamp: &str,
            _orders: &[CancelAlgoOrder],
        ) -> RoleResult<Vec<AlgoCancelResult>> {
            unreachable!()
        }

        async fn spread_mass_cancel_at(&self, _timestamp: &str) -> RoleResult<()> {
            unreachable!()
        }
    }

    #[tokio::test]
    async fn account_deadline_bounds_a_hung_role() {
        let mut bounded = settings();
        bounded.account_timeout = Duration::from_millis(20);
        let started = Instant::now();

        let report = run_account_cancel(
            Box::new(HangingRole),
            "main".to_string(),
            HashSet::new(),
            bounded,
        )
        .await;

        assert!(!report.all_clear);
        assert!(started.elapsed() < Duration::from_millis(500));
        assert!(
            report
                .incidents
                .iter()
                .any(|message| message.contains("account timeout"))
        );
    }
}
