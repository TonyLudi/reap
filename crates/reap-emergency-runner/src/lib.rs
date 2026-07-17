//! Separate emergency-stop composition with independently progressing order domains.

use std::collections::{BTreeSet, HashSet};
use std::future::Future;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use chrono::{SecondsFormat, Utc};
use reap_core::PINNED_JAVA_REVISION;
use reap_emergency_core::*;
use reap_okx_emergency_adapter::OkxEmergencyAccountStopFactory;
use reap_order::{PacingPolicy, RequestKind, RequestPacer};
use reap_telemetry::{
    current_executable_sha256, host_identity_sha256, identity_sha256, sha256_bytes,
};
use tokio::sync::oneshot;
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

#[derive(Debug)]
struct RegularCancelProgress {
    report: EmergencyAccountReport,
}

#[derive(Debug)]
struct AlgoCancelProgress {
    report: EmergencyAccountReport,
}

#[derive(Debug)]
struct SpreadCancelProgress {
    report: EmergencyAccountReport,
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
    let client: Arc<dyn EmergencyAccountStopRole> = Arc::from(client);
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
                "account timeout expired while sampling exchange clock; using local UTC for independently bounded domain workflows",
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

    let (regular_kickoff_tx, regular_kickoff_rx) = oneshot::channel();
    let regular_workflow = run_regular_cancel_domain(
        client.as_ref(),
        &clock,
        &account_id,
        &managed_symbols,
        started,
        &settings,
        regular_kickoff_tx,
    );
    let unsupported_workflows = async {
        // A dropped sender means the regular workflow ended before it could
        // issue CAA. Unsupported-domain mitigation must still run.
        let _ = regular_kickoff_rx.await;
        tokio::join!(
            run_algo_cancel_domain(
                client.as_ref(),
                &clock,
                &account_id,
                &managed_symbols,
                started,
                &settings,
            ),
            run_spread_cancel_domain(client.as_ref(), &clock, &account_id, started, &settings),
        )
    };
    let (regular, (algo, spread)) = tokio::join!(regular_workflow, unsupported_workflows);
    merge_domain_progress(&mut report, regular, algo, spread);

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

async fn run_regular_cancel_domain(
    client: &dyn EmergencyAccountStopRole,
    clock: &ExchangeClock,
    account_id: &str,
    managed_symbols: &HashSet<String>,
    account_started: Instant,
    settings: &AccountCancelSettings,
    regular_kickoff: oneshot::Sender<()>,
) -> RegularCancelProgress {
    let mut report = EmergencyAccountReport::new(account_id.to_string());
    let mut regular_kickoff = Some(regular_kickoff);

    match clock.timestamp() {
        Ok(timestamp) => {
            if let Some(kickoff) = regular_kickoff.take() {
                let _ = kickoff.send(());
            }
            match run_bounded(
                account_started,
                settings.account_timeout,
                client.cancel_all_after_at(&timestamp, settings.deadman_timeout_secs),
            )
            .await
            {
                Some(Ok(())) => report.deadman_armed = true,
                Some(Err(error)) => {
                    report.push_incident(format!("failed to arm Cancel All After: {error}"));
                }
                None => {
                    report.push_incident("regular-domain timeout while arming Cancel All After")
                }
            }
        }
        Err(error) => {
            report.push_incident(format!("failed to format deadman timestamp: {error}"));
        }
    }
    if let Some(kickoff) = regular_kickoff.take() {
        let _ = kickoff.send(());
    }

    let verification_anchor = Instant::now();
    let verify_after = verification_anchor + settings.verification_delay;
    let mut pacer = RequestPacer::new(settings.pacing_policy.clone());
    let mut seen_regular_orders = BTreeSet::new();
    let mut unmanaged_symbols = BTreeSet::new();
    let mut last_orders: Option<Vec<RegularOrder>> = None;

    while account_started.elapsed() < settings.account_timeout {
        let orders = match enumerate_regular_pending_orders(
            client,
            clock,
            account_id,
            &mut pacer,
            &mut report,
            account_started,
            settings,
        )
        .await
        {
            Ok(orders) => orders,
            Err(error) => {
                report.push_incident(error);
                if account_started.elapsed() >= settings.account_timeout {
                    break;
                }
                sleep_bounded(
                    settings.poll_interval,
                    account_started,
                    settings.account_timeout,
                )
                .await;
                continue;
            }
        };
        if report.initial_open_orders.is_none() {
            report.initial_open_orders = Some(orders.len());
        }
        observe_orders(
            &orders,
            managed_symbols,
            &mut seen_regular_orders,
            &mut unmanaged_symbols,
            &mut report,
        );
        last_orders = Some(orders.clone());
        if orders.is_empty() {
            if Instant::now() >= verify_after {
                report.verified_zero_after_deadman = true;
                break;
            }
        } else if !cancel_pending_orders(
            client,
            clock,
            &orders,
            &mut pacer,
            &mut report,
            account_started,
            settings,
        )
        .await
        {
            break;
        }
        sleep_bounded(
            settings.poll_interval,
            account_started,
            settings.account_timeout,
        )
        .await;
    }

    report.unique_orders_seen = seen_regular_orders.len();
    report.unmanaged_symbols = unmanaged_symbols.into_iter().collect();
    report.final_open_orders = last_orders.as_ref().map(Vec::len);
    report.remaining_orders = last_orders
        .unwrap_or_default()
        .into_iter()
        .take(MAX_REMAINING_ORDER_DETAILS)
        .map(order_ref)
        .collect();
    if !report.verified_zero_after_deadman && account_started.elapsed() >= settings.account_timeout
    {
        report.push_incident("regular-domain timeout before regular orders were proven zero");
    }
    RegularCancelProgress { report }
}

async fn run_algo_cancel_domain(
    client: &dyn EmergencyAccountStopRole,
    clock: &ExchangeClock,
    account_id: &str,
    managed_symbols: &HashSet<String>,
    account_started: Instant,
    settings: &AccountCancelSettings,
) -> AlgoCancelProgress {
    let verification_anchor = Instant::now();
    let verify_after = verification_anchor + settings.verification_delay;
    let mut report = EmergencyAccountReport::new(account_id.to_string());
    let mut pacer = RequestPacer::new(settings.pacing_policy.clone());
    let mut seen_orders = BTreeSet::new();
    let mut unmanaged_symbols = BTreeSet::new();
    let mut last_orders: Option<Vec<AlgoOrder>> = None;

    while account_started.elapsed() < settings.account_timeout {
        let orders = match enumerate_algo_pending_orders(
            client,
            clock,
            account_id,
            &mut pacer,
            &mut report,
            account_started,
            settings,
        )
        .await
        {
            Ok(orders) => orders,
            Err(error) => {
                report.push_incident(error);
                if account_started.elapsed() >= settings.account_timeout {
                    break;
                }
                sleep_bounded(
                    settings.poll_interval,
                    account_started,
                    settings.account_timeout,
                )
                .await;
                continue;
            }
        };
        if report.initial_algo_orders.is_none() {
            report.initial_algo_orders = Some(orders.len());
        }
        observe_algo_orders(
            &orders,
            managed_symbols,
            &mut seen_orders,
            &mut unmanaged_symbols,
        );
        last_orders = Some(orders.clone());
        if orders.is_empty() {
            if Instant::now() >= verify_after {
                report.verified_algo_zero_after_deadman = true;
                break;
            }
        } else if !cancel_pending_algo_orders(
            client,
            clock,
            &orders,
            &mut pacer,
            &mut report,
            account_started,
            settings,
        )
        .await
        {
            break;
        }
        sleep_bounded(
            settings.poll_interval,
            account_started,
            settings.account_timeout,
        )
        .await;
    }

    report.unique_algo_orders_seen = seen_orders.len();
    report.unmanaged_symbols = unmanaged_symbols.into_iter().collect();
    report.final_algo_orders = last_orders.as_ref().map(Vec::len);
    report.remaining_algo_orders = last_orders
        .unwrap_or_default()
        .into_iter()
        .take(MAX_REMAINING_ORDER_DETAILS)
        .map(algo_order_ref)
        .collect();
    if !report.verified_algo_zero_after_deadman
        && account_started.elapsed() >= settings.account_timeout
    {
        report.push_incident("algo-domain timeout before algo orders were proven zero");
    }
    AlgoCancelProgress { report }
}

async fn run_spread_cancel_domain(
    client: &dyn EmergencyAccountStopRole,
    clock: &ExchangeClock,
    account_id: &str,
    account_started: Instant,
    settings: &AccountCancelSettings,
) -> SpreadCancelProgress {
    let mut report = EmergencyAccountReport::new(account_id.to_string());

    match clock.timestamp() {
        Ok(timestamp) => match run_bounded(
            account_started,
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
                report.push_incident("spread-domain timeout while arming spread Cancel All After");
            }
        },
        Err(error) => report.push_incident(format!(
            "failed to format spread deadman timestamp: {error}"
        )),
    }

    let verification_anchor = Instant::now();
    let verify_after = verification_anchor + settings.verification_delay;
    let mut pacer = RequestPacer::new(settings.pacing_policy.clone());
    let mut seen_orders = BTreeSet::new();
    let mut last_orders: Option<Vec<SpreadOrder>> = None;

    while account_started.elapsed() < settings.account_timeout {
        let orders = match enumerate_spread_pending_orders(
            client,
            clock,
            account_id,
            &mut pacer,
            &mut report,
            account_started,
            settings,
        )
        .await
        {
            Ok(orders) => orders,
            Err(error) => {
                report.push_incident(error);
                if account_started.elapsed() >= settings.account_timeout {
                    break;
                }
                sleep_bounded(
                    settings.poll_interval,
                    account_started,
                    settings.account_timeout,
                )
                .await;
                continue;
            }
        };
        if report.initial_spread_orders.is_none() {
            report.initial_spread_orders = Some(orders.len());
        }
        observe_spread_orders(&orders, &mut seen_orders);
        last_orders = Some(orders.clone());
        if orders.is_empty() {
            if Instant::now() >= verify_after {
                report.verified_spread_zero_after_deadman = true;
                break;
            }
        } else if !cancel_pending_spread_orders(
            client,
            clock,
            &mut pacer,
            &mut report,
            account_started,
            settings,
        )
        .await
        {
            break;
        }
        sleep_bounded(
            settings.poll_interval,
            account_started,
            settings.account_timeout,
        )
        .await;
    }

    report.unique_spread_orders_seen = seen_orders.len();
    report.final_spread_orders = last_orders.as_ref().map(Vec::len);
    report.remaining_spread_orders = last_orders
        .unwrap_or_default()
        .into_iter()
        .take(MAX_REMAINING_ORDER_DETAILS)
        .map(spread_order_ref)
        .collect();
    if !report.verified_spread_zero_after_deadman
        && account_started.elapsed() >= settings.account_timeout
    {
        report.push_incident("spread-domain timeout before spread orders were proven zero");
    }
    SpreadCancelProgress { report }
}

fn merge_domain_progress(
    report: &mut EmergencyAccountReport,
    regular: RegularCancelProgress,
    algo: AlgoCancelProgress,
    spread: SpreadCancelProgress,
) {
    let regular = regular.report;
    let algo = algo.report;
    let spread = spread.report;

    report.deadman_armed = regular.deadman_armed;
    report.spread_deadman_armed = spread.spread_deadman_armed;
    report.enumeration_attempts = regular
        .enumeration_attempts
        .saturating_add(algo.enumeration_attempts)
        .saturating_add(spread.enumeration_attempts);
    report.enumeration_failures = regular
        .enumeration_failures
        .saturating_add(algo.enumeration_failures)
        .saturating_add(spread.enumeration_failures);
    report.initial_open_orders = regular.initial_open_orders;
    report.initial_algo_orders = algo.initial_algo_orders;
    report.initial_spread_orders = spread.initial_spread_orders;
    report.unique_orders_seen = regular.unique_orders_seen;
    report.unique_algo_orders_seen = algo.unique_algo_orders_seen;
    report.unique_spread_orders_seen = spread.unique_spread_orders_seen;
    report.cancel_batches = regular.cancel_batches;
    report.cancel_batch_failures = regular.cancel_batch_failures;
    report.accepted_cancel_requests = regular.accepted_cancel_requests;
    report.rejected_cancel_requests = regular.rejected_cancel_requests;
    report.unacknowledged_cancel_requests = regular.unacknowledged_cancel_requests;
    report.algo_cancel_batches = algo.algo_cancel_batches;
    report.algo_cancel_batch_failures = algo.algo_cancel_batch_failures;
    report.accepted_algo_cancel_requests = algo.accepted_algo_cancel_requests;
    report.rejected_algo_cancel_requests = algo.rejected_algo_cancel_requests;
    report.unacknowledged_algo_cancel_requests = algo.unacknowledged_algo_cancel_requests;
    report.spread_mass_cancel_attempts = spread.spread_mass_cancel_attempts;
    report.spread_mass_cancel_failures = spread.spread_mass_cancel_failures;
    report.verified_zero_after_deadman = regular.verified_zero_after_deadman;
    report.verified_algo_zero_after_deadman = algo.verified_algo_zero_after_deadman;
    report.verified_spread_zero_after_deadman = spread.verified_spread_zero_after_deadman;
    report.final_open_orders = regular.final_open_orders;
    report.final_algo_orders = algo.final_algo_orders;
    report.final_spread_orders = spread.final_spread_orders;
    report.remaining_orders = regular.remaining_orders;
    report.remaining_algo_orders = algo.remaining_algo_orders;
    report.remaining_spread_orders = spread.remaining_spread_orders;
    report.unmanaged_symbols = regular
        .unmanaged_symbols
        .into_iter()
        .chain(algo.unmanaged_symbols)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();

    merge_domain_incidents(report, regular.incident_count, regular.incidents);
    merge_domain_incidents(report, algo.incident_count, algo.incidents);
    merge_domain_incidents(report, spread.incident_count, spread.incidents);
}

fn merge_domain_incidents(
    report: &mut EmergencyAccountReport,
    incident_count: u64,
    incidents: Vec<String>,
) {
    report.incident_count = report.incident_count.saturating_add(incident_count);
    let remaining = MAX_INCIDENTS.saturating_sub(report.incidents.len());
    report
        .incidents
        .extend(incidents.into_iter().take(remaining));
}

async fn enumerate_regular_pending_orders(
    client: &dyn EmergencyAccountStopRole,
    clock: &ExchangeClock,
    account_id: &str,
    pacer: &mut RequestPacer,
    report: &mut EmergencyAccountReport,
    started: Instant,
    settings: &AccountCancelSettings,
) -> Result<Vec<RegularOrder>, String> {
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
                    "domain timeout expired during request".to_string(),
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
    Ok(regular.into_orders())
}

async fn enumerate_algo_pending_orders(
    client: &dyn EmergencyAccountStopRole,
    clock: &ExchangeClock,
    account_id: &str,
    pacer: &mut RequestPacer,
    report: &mut EmergencyAccountReport,
    started: Instant,
    settings: &AccountCancelSettings,
) -> Result<Vec<AlgoOrder>, String> {
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
                        "domain timeout expired during request".to_string(),
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
    Ok(algo_orders)
}

async fn enumerate_spread_pending_orders(
    client: &dyn EmergencyAccountStopRole,
    clock: &ExchangeClock,
    account_id: &str,
    pacer: &mut RequestPacer,
    report: &mut EmergencyAccountReport,
    started: Instant,
    settings: &AccountCancelSettings,
) -> Result<Vec<SpreadOrder>, String> {
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
                    "domain timeout expired during request".to_string(),
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
    Ok(spread.into_orders())
}

/*
 * The three enumerators above deliberately remain separate. Their paginators,
 * pacing state, deadlines, and failure accounting must never be recombined.
 */

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

    #[derive(Debug, Clone, Copy)]
    enum UnsupportedBehavior {
        InstantEmpty,
        Hang,
    }

    struct UnsupportedIsolationState {
        calls: Vec<String>,
        regular_pages: VecDeque<RegularOrderPage>,
        regular_completed_at: Option<Instant>,
    }

    #[derive(Clone)]
    struct UnsupportedIsolationRole {
        state: Arc<Mutex<UnsupportedIsolationState>>,
        unsupported_behavior: UnsupportedBehavior,
    }

    impl UnsupportedIsolationRole {
        fn new(
            regular_pages: impl IntoIterator<Item = RegularOrderPage>,
            unsupported_behavior: UnsupportedBehavior,
        ) -> (Self, Arc<Mutex<UnsupportedIsolationState>>) {
            let state = Arc::new(Mutex::new(UnsupportedIsolationState {
                calls: Vec::new(),
                regular_pages: regular_pages.into_iter().collect(),
                regular_completed_at: None,
            }));
            (
                Self {
                    state: Arc::clone(&state),
                    unsupported_behavior,
                },
                state,
            )
        }

        fn record(&self, call: &str) {
            self.state.lock().unwrap().calls.push(call.to_string());
        }
    }

    #[async_trait]
    impl EmergencyAccountStopRole for UnsupportedIsolationRole {
        async fn server_time_ms(&self) -> RoleResult<u64> {
            self.record("server_time");
            Ok(unix_time_ms())
        }

        async fn account_identity_at(
            &self,
            _timestamp: &str,
        ) -> RoleResult<EmergencyAccountIdentity> {
            self.record("account_identity");
            Ok(EmergencyAccountIdentity {
                user_id: "7".to_string(),
                main_user_id: "6".to_string(),
            })
        }

        async fn regular_pending_orders_page_at(
            &self,
            _timestamp: &str,
            _after: Option<&str>,
        ) -> RoleResult<RegularOrderPage> {
            let mut state = self.state.lock().unwrap();
            state.calls.push("enumerate_regular".to_string());
            let page = state
                .regular_pages
                .pop_front()
                .expect("missing regular page");
            if page.orders.is_empty() && state.regular_pages.is_empty() {
                state.regular_completed_at = Some(Instant::now());
            }
            Ok(page)
        }

        async fn algo_pending_orders_page_at(
            &self,
            _timestamp: &str,
            _query: AlgoOrderQuery,
            _after: Option<&str>,
        ) -> RoleResult<AlgoOrderPage> {
            self.record("enumerate_algo");
            match self.unsupported_behavior {
                UnsupportedBehavior::InstantEmpty => Ok(empty_algo_page()),
                UnsupportedBehavior::Hang => std::future::pending().await,
            }
        }

        async fn spread_pending_orders_page_at(
            &self,
            _timestamp: &str,
            _end_id: Option<&str>,
        ) -> RoleResult<SpreadOrderPage> {
            self.record("enumerate_spread");
            match self.unsupported_behavior {
                UnsupportedBehavior::InstantEmpty => Ok(empty_spread_page()),
                UnsupportedBehavior::Hang => std::future::pending().await,
            }
        }

        async fn cancel_all_after_at(
            &self,
            _timestamp: &str,
            _timeout_secs: u64,
        ) -> RoleResult<()> {
            self.record("arm_regular_deadman");
            Ok(())
        }

        async fn spread_cancel_all_after_at(
            &self,
            _timestamp: &str,
            _timeout_secs: u64,
        ) -> RoleResult<()> {
            self.record("arm_spread_deadman");
            Ok(())
        }

        async fn cancel_batch_orders_at(
            &self,
            _timestamp: &str,
            _orders: &[CancelOrder],
        ) -> RoleResult<Vec<CancelOrderResult>> {
            self.record("cancel_regular");
            Ok(vec![accepted_regular_cancel()])
        }

        async fn cancel_algo_orders_at(
            &self,
            _timestamp: &str,
            _orders: &[CancelAlgoOrder],
        ) -> RoleResult<Vec<AlgoCancelResult>> {
            panic!("hung algo enumeration must not reach cancellation")
        }

        async fn spread_mass_cancel_at(&self, _timestamp: &str) -> RoleResult<()> {
            panic!("hung spread enumeration must not reach cancellation")
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

    fn regular_order() -> RegularOrder {
        RegularOrder {
            symbol: "BTC-USDT".to_string(),
            exchange_order_id: "regular-1".to_string(),
            client_order_id: "regular-client-1".to_string(),
        }
    }

    fn algo_order() -> AlgoOrder {
        AlgoOrder {
            algo_id: "algo-1".to_string(),
            client_order_id: "algo-client-1".to_string(),
            symbol: "BTC-USDT".to_string(),
        }
    }

    fn spread_order() -> SpreadOrder {
        SpreadOrder {
            spread_id: "BTC-USDT_BTC-USDT-SWAP".to_string(),
            exchange_order_id: "spread-1".to_string(),
            client_order_id: "spread-client-1".to_string(),
        }
    }

    fn accepted_regular_cancel() -> CancelOrderResult {
        CancelOrderResult {
            exchange_order_id: "regular-1".to_string(),
            client_order_id: "regular-client-1".to_string(),
            code: "0".to_string(),
            message: String::new(),
        }
    }

    fn accepted_algo_cancel() -> AlgoCancelResult {
        AlgoCancelResult {
            algo_id: "algo-1".to_string(),
            client_order_id: "algo-client-1".to_string(),
            code: "0".to_string(),
            message: String::new(),
        }
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum FailedDomain {
        Regular,
        Spread,
    }

    fn independent_failure_role(failed_domain: FailedDomain) -> (ScriptedRole, Arc<Mutex<Script>>) {
        let mut script = Script {
            server_times: [Ok(unix_time_ms())].into(),
            regular_deadman: [Ok(())].into(),
            spread_deadman: [Ok(())].into(),
            regular_cancels: [Ok(vec![accepted_regular_cancel()])].into(),
            algo_cancels: [Ok(vec![accepted_algo_cancel()])].into(),
            spread_cancels: [Ok(())].into(),
            ..Script::default()
        };
        if failed_domain == FailedDomain::Regular {
            script.regular_pages.extend(
                (0..64).map(|_| Err(EmergencyRoleError("regular unavailable".to_string()))),
            );
        } else {
            script.regular_pages.extend([
                Ok(RegularOrderPage {
                    orders: vec![regular_order()],
                    next_after: None,
                }),
                Ok(empty_regular_page()),
            ]);
        }
        script.algo_pages.push_back(Ok(AlgoOrderPage {
            orders: vec![algo_order()],
            next_after: None,
        }));
        script
            .algo_pages
            .extend((1..AlgoOrderQuery::ALL.len()).map(|_| Ok(empty_algo_page())));
        script
            .algo_pages
            .extend((0..AlgoOrderQuery::ALL.len()).map(|_| Ok(empty_algo_page())));
        if failed_domain == FailedDomain::Spread {
            script
                .spread_pages
                .extend((0..64).map(|_| Err(EmergencyRoleError("spread unavailable".to_string()))));
        } else {
            script.spread_pages.extend([
                Ok(SpreadOrderPage {
                    orders: vec![spread_order()],
                    next_end_id: None,
                }),
                Ok(empty_spread_page()),
            ]);
        }
        ScriptedRole::new(script)
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
    async fn independent_workflows_preserve_domain_progress_after_regular_kickoff() {
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
        assert_eq!(&calls[..2], ["server_time", "arm_regular_deadman"]);
        let position = |needle: &str| {
            calls
                .iter()
                .position(|call| call == needle)
                .unwrap_or_else(|| panic!("missing call {needle}: {calls:?}"))
        };
        let positions = |needle: &str| {
            calls
                .iter()
                .enumerate()
                .filter_map(|(index, call)| (call == needle).then_some(index))
                .collect::<Vec<_>>()
        };
        assert!(position("arm_regular_deadman") < position("arm_spread_deadman"));
        assert!(position("enumerate_regular") < position("cancel_regular"));
        assert!(position("cancel_regular") < positions("enumerate_regular")[1]);
        assert!(
            calls
                .iter()
                .position(|call| call.starts_with("enumerate_algo:"))
                .unwrap()
                < position("cancel_algo")
        );
        assert!(
            position("cancel_algo")
                < calls
                    .iter()
                    .enumerate()
                    .filter(|(_, call)| call.starts_with("enumerate_algo:"))
                    .nth(AlgoOrderQuery::ALL.len())
                    .unwrap()
                    .0
        );
        assert!(position("enumerate_spread") < position("cancel_spread"));
        assert!(position("cancel_spread") < positions("enumerate_spread")[1]);
        assert_eq!(calls.last().map(String::as_str), Some("account_identity"));
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
    async fn failed_algo_enumeration_does_not_prevent_regular_or_spread_cancel_domains() {
        let mut script = Script {
            server_times: [Ok(unix_time_ms())].into(),
            regular_pages: [
                Ok(RegularOrderPage {
                    orders: vec![regular_order()],
                    next_after: None,
                }),
                Ok(empty_regular_page()),
            ]
            .into(),
            spread_pages: [
                Ok(SpreadOrderPage {
                    orders: vec![spread_order()],
                    next_end_id: None,
                }),
                Ok(empty_spread_page()),
            ]
            .into(),
            regular_deadman: [Ok(())].into(),
            spread_deadman: [Ok(())].into(),
            regular_cancels: [Ok(vec![accepted_regular_cancel()])].into(),
            spread_cancels: [Ok(())].into(),
            ..Script::default()
        };
        script
            .algo_pages
            .extend((0..64).map(|_| Err(EmergencyRoleError("algo unavailable".to_string()))));
        let (role, recorded) = ScriptedRole::new(script);
        let mut bounded = settings();
        bounded.account_timeout = Duration::from_millis(30);
        bounded.poll_interval = Duration::from_millis(5);

        let report =
            run_account_cancel(Box::new(role), "main".to_string(), HashSet::new(), bounded).await;

        assert!(!report.all_clear);
        assert_eq!(report.initial_open_orders, Some(1));
        assert_eq!(report.final_open_orders, Some(0));
        assert!(report.verified_zero_after_deadman);
        assert_eq!(report.accepted_cancel_requests, 1);
        assert_eq!(report.initial_algo_orders, None);
        assert_eq!(report.final_algo_orders, None);
        assert!(!report.verified_algo_zero_after_deadman);
        assert_eq!(report.initial_spread_orders, Some(1));
        assert_eq!(report.final_spread_orders, Some(0));
        assert!(report.verified_spread_zero_after_deadman);
        assert_eq!(report.spread_mass_cancel_attempts, 1);
        assert!(report.incidents.iter().any(|message| {
            message.contains("algo pending-order enumeration failed: algo unavailable")
        }));
        let calls = &recorded.lock().unwrap().calls;
        assert!(calls.iter().any(|call| call == "cancel_regular"));
        assert!(calls.iter().any(|call| call == "cancel_spread"));
        assert!(calls.iter().all(|call| call != "cancel_algo"));
    }

    #[tokio::test]
    async fn regular_and_spread_enumeration_failures_are_isolated_from_other_domains() {
        for failed_domain in [FailedDomain::Regular, FailedDomain::Spread] {
            let (role, recorded) = independent_failure_role(failed_domain);
            let mut bounded = settings();
            bounded.account_timeout = Duration::from_millis(30);
            bounded.poll_interval = Duration::from_millis(5);

            let report =
                run_account_cancel(Box::new(role), "main".to_string(), HashSet::new(), bounded)
                    .await;

            assert!(!report.all_clear, "{failed_domain:?}");
            assert!(report.enumeration_failures > 0, "{failed_domain:?}");
            assert_eq!(
                report.verified_zero_after_deadman,
                failed_domain != FailedDomain::Regular,
                "{failed_domain:?}"
            );
            assert!(report.verified_algo_zero_after_deadman, "{failed_domain:?}");
            assert_eq!(
                report.verified_spread_zero_after_deadman,
                failed_domain != FailedDomain::Spread,
                "{failed_domain:?}"
            );
            assert_eq!(
                report.accepted_cancel_requests,
                u64::from(failed_domain != FailedDomain::Regular),
                "{failed_domain:?}"
            );
            assert_eq!(report.accepted_algo_cancel_requests, 1, "{failed_domain:?}");
            assert_eq!(
                report.spread_mass_cancel_attempts,
                u64::from(failed_domain != FailedDomain::Spread),
                "{failed_domain:?}"
            );

            let calls = &recorded.lock().unwrap().calls;
            assert_eq!(
                calls.iter().any(|call| call == "cancel_regular"),
                failed_domain != FailedDomain::Regular,
                "{failed_domain:?}: {calls:?}"
            );
            assert!(
                calls.iter().any(|call| call == "cancel_algo"),
                "{failed_domain:?}: {calls:?}"
            );
            assert_eq!(
                calls.iter().any(|call| call == "cancel_spread"),
                failed_domain != FailedDomain::Spread,
                "{failed_domain:?}: {calls:?}"
            );
        }
    }

    #[tokio::test]
    async fn hung_unsupported_domains_do_not_change_the_regular_mitigation_trace() {
        let regular_pages = || {
            [
                RegularOrderPage {
                    orders: vec![regular_order()],
                    next_after: None,
                },
                empty_regular_page(),
            ]
        };
        let mut bounded = settings();
        bounded.account_timeout = Duration::from_millis(200);
        bounded.poll_interval = Duration::from_millis(1);
        let pacing_quantum = bounded.pacing_policy.window;

        let (baseline_role, baseline_recorded) =
            UnsupportedIsolationRole::new(regular_pages(), UnsupportedBehavior::InstantEmpty);
        let baseline_started = Instant::now();
        let baseline_report = run_account_cancel(
            Box::new(baseline_role),
            "main".to_string(),
            HashSet::new(),
            bounded.clone(),
        )
        .await;
        let baseline_regular_elapsed = baseline_recorded
            .lock()
            .unwrap()
            .regular_completed_at
            .expect("baseline regular workflow did not complete")
            .duration_since(baseline_started);
        assert!(baseline_report.all_clear, "{:?}", baseline_report.incidents);

        let (hung_role, hung_recorded) =
            UnsupportedIsolationRole::new(regular_pages(), UnsupportedBehavior::Hang);
        let hung_started = Instant::now();
        let report = run_account_cancel(
            Box::new(hung_role),
            "main".to_string(),
            HashSet::new(),
            bounded,
        )
        .await;
        let hung_regular_elapsed = hung_recorded
            .lock()
            .unwrap()
            .regular_completed_at
            .expect("hung case regular workflow did not complete")
            .duration_since(hung_started);

        assert!(report.deadman_armed);
        assert!(report.verified_zero_after_deadman);
        assert_eq!(report.initial_open_orders, Some(1));
        assert_eq!(report.final_open_orders, Some(0));
        assert_eq!(report.accepted_cancel_requests, 1);
        assert!(!report.verified_algo_zero_after_deadman);
        assert!(!report.verified_spread_zero_after_deadman);
        assert!(!report.all_clear);

        let regular_trace = |recorded: &Arc<Mutex<UnsupportedIsolationState>>| {
            recorded
                .lock()
                .unwrap()
                .calls
                .iter()
                .filter(|call| {
                    matches!(
                        call.as_str(),
                        "arm_regular_deadman" | "enumerate_regular" | "cancel_regular"
                    )
                })
                .cloned()
                .collect::<Vec<_>>()
        };
        let baseline_trace = regular_trace(&baseline_recorded);
        let hung_trace = regular_trace(&hung_recorded);
        assert_eq!(
            baseline_trace,
            [
                "arm_regular_deadman",
                "enumerate_regular",
                "cancel_regular",
                "enumerate_regular",
            ]
        );
        assert_eq!(hung_trace, baseline_trace);
        assert!(
            hung_regular_elapsed
                <= baseline_regular_elapsed
                    .saturating_add(pacing_quantum)
                    .saturating_add(Duration::from_millis(100)),
            "hung unsupported domains delayed regular completion: baseline={baseline_regular_elapsed:?}, hung={hung_regular_elapsed:?}, pacing_quantum={pacing_quantum:?}"
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
            std::future::pending().await
        }

        async fn regular_pending_orders_page_at(
            &self,
            _timestamp: &str,
            _after: Option<&str>,
        ) -> RoleResult<RegularOrderPage> {
            std::future::pending().await
        }

        async fn algo_pending_orders_page_at(
            &self,
            _timestamp: &str,
            _query: AlgoOrderQuery,
            _after: Option<&str>,
        ) -> RoleResult<AlgoOrderPage> {
            std::future::pending().await
        }

        async fn spread_pending_orders_page_at(
            &self,
            _timestamp: &str,
            _end_id: Option<&str>,
        ) -> RoleResult<SpreadOrderPage> {
            std::future::pending().await
        }

        async fn cancel_all_after_at(
            &self,
            _timestamp: &str,
            _timeout_secs: u64,
        ) -> RoleResult<()> {
            std::future::pending().await
        }

        async fn spread_cancel_all_after_at(
            &self,
            _timestamp: &str,
            _timeout_secs: u64,
        ) -> RoleResult<()> {
            std::future::pending().await
        }

        async fn cancel_batch_orders_at(
            &self,
            _timestamp: &str,
            _orders: &[CancelOrder],
        ) -> RoleResult<Vec<CancelOrderResult>> {
            std::future::pending().await
        }

        async fn cancel_algo_orders_at(
            &self,
            _timestamp: &str,
            _orders: &[CancelAlgoOrder],
        ) -> RoleResult<Vec<AlgoCancelResult>> {
            std::future::pending().await
        }

        async fn spread_mass_cancel_at(&self, _timestamp: &str) -> RoleResult<()> {
            std::future::pending().await
        }
    }

    #[tokio::test]
    async fn clock_hang_cannot_reset_the_absolute_account_deadline() {
        let mut bounded = settings();
        bounded.account_timeout = Duration::from_millis(200);
        let account_timeout = bounded.account_timeout;
        let started = Instant::now();

        let report = run_account_cancel(
            Box::new(HangingRole),
            "main".to_string(),
            HashSet::new(),
            bounded,
        )
        .await;

        assert!(!report.all_clear);
        assert!(
            started.elapsed() <= account_timeout.saturating_add(Duration::from_millis(100)),
            "clock timeout was followed by a reset domain timeout: elapsed={:?}, account_timeout={account_timeout:?}",
            started.elapsed()
        );
        assert!(
            report.elapsed_ms
                <= duration_ms(account_timeout.saturating_add(Duration::from_millis(100)))
        );
        assert!(!report.exchange_clock_sampled);
        assert_eq!(report.exchange_clock_skew_ms, None);
        assert_eq!(report.enumeration_attempts, 0);
        assert_eq!(report.enumeration_failures, 0);
        assert_eq!(report.initial_open_orders, None);
        assert_eq!(report.initial_algo_orders, None);
        assert_eq!(report.initial_spread_orders, None);
        assert_eq!(report.final_open_orders, None);
        assert_eq!(report.final_algo_orders, None);
        assert_eq!(report.final_spread_orders, None);
        assert!(!report.verified_zero_after_deadman);
        assert!(!report.verified_algo_zero_after_deadman);
        assert!(!report.verified_spread_zero_after_deadman);
        assert!(
            report
                .incidents
                .iter()
                .any(|message| message.contains("account timeout"))
        );
        let encoded = serde_json::to_vec(&report).unwrap();
        let decoded: EmergencyAccountReport = serde_json::from_slice(&encoded).unwrap();
        assert_eq!(decoded.account_id, "main");
        assert_eq!(decoded.initial_open_orders, None);
        assert_eq!(decoded.final_spread_orders, None);
    }

    #[tokio::test]
    async fn clock_error_keeps_domain_pairs_valid_within_the_absolute_deadline() {
        let mut script = Script {
            server_times: [Err(EmergencyRoleError("clock unavailable".to_string()))].into(),
            identities: [Ok(EmergencyAccountIdentity {
                user_id: "7".to_string(),
                main_user_id: "6".to_string(),
            })]
            .into(),
            regular_pages: [Ok(empty_regular_page())].into(),
            spread_pages: [Ok(empty_spread_page())].into(),
            regular_deadman: [Ok(())].into(),
            spread_deadman: [Ok(())].into(),
            ..Script::default()
        };
        script
            .algo_pages
            .extend((0..AlgoOrderQuery::ALL.len()).map(|_| Ok(empty_algo_page())));
        let (role, _) = ScriptedRole::new(script);
        let bounded = settings();
        let account_timeout = bounded.account_timeout;

        let report =
            run_account_cancel(Box::new(role), "main".to_string(), HashSet::new(), bounded).await;

        assert!(report.all_clear, "{:?}", report.incidents);
        assert!(!report.exchange_clock_sampled);
        assert_eq!(report.exchange_clock_skew_ms, None);
        assert_eq!(report.initial_open_orders, Some(0));
        assert_eq!(report.initial_algo_orders, Some(0));
        assert_eq!(report.initial_spread_orders, Some(0));
        assert_eq!(report.final_open_orders, Some(0));
        assert_eq!(report.final_algo_orders, Some(0));
        assert_eq!(report.final_spread_orders, Some(0));
        assert!(report.verified_zero_after_deadman);
        assert!(report.verified_algo_zero_after_deadman);
        assert!(report.verified_spread_zero_after_deadman);
        assert!(report.elapsed_ms <= duration_ms(account_timeout));
        assert!(report.incidents.iter().any(|message| {
            message.contains(
                "exchange clock sampling failed; using local UTC for cancellation: clock unavailable",
            )
        }));
    }
}
