use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use reap_backtest::{
    BacktestLatencyClass, BacktestLatencyProfile, BacktestLatencyRule,
    LATENCY_CALIBRATION_SCHEMA_VERSION, LatencyCalibrationArtifact, LatencyCalibrationSeries,
    LatencySourceReport, MAX_LATENCY_CALIBRATION_ARTIFACT_BYTES,
    MAX_LATENCY_CALIBRATION_RETAINED_INPUT_SAMPLES, MAX_LATENCY_CALIBRATION_SOURCE_REPORTS,
};
use reap_core::PINNED_JAVA_REVISION;
use reap_live::{
    LIVE_LATENCY_EVIDENCE_SCHEMA_VERSION, LIVE_LATENCY_RESERVOIR_CAPACITY,
    LIVE_RUN_REPORT_SCHEMA_VERSION, LiveConfig, LiveLatencySemantics, LiveLatencySeries, LiveMode,
    LiveRunReport, LiveStopReason, MAX_LIVE_FAILURE_CODE_BYTES, MAX_LIVE_FAILURE_MESSAGE_BYTES,
    MAX_LIVE_LATENCY_SERIES, MAX_LIVE_LATENCY_US, verify_live_run_paths,
};
use serde::Serialize;
use sha2::{Digest, Sha256};

const MERGED_PROFILE_SAMPLE_CAPACITY: usize = 8_192;

#[derive(Debug, Clone, Copy)]
pub(crate) struct LatencyCalibrationOptions {
    pub seed: u64,
    pub minimum_samples_per_series: u64,
    pub accept_matching_upper_bounds: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct SeriesKey {
    class: BacktestLatencyClass,
    symbol: String,
    semantics: LiveLatencySemantics,
}

struct LoadedReport {
    provenance: LatencySourceReport,
    report: LiveRunReport,
}

pub(crate) fn build_latency_calibration(
    config_path: &Path,
    report_paths: &[PathBuf],
    options: LatencyCalibrationOptions,
) -> Result<LatencyCalibrationArtifact> {
    if report_paths.is_empty() {
        bail!("at least one --report is required");
    }
    if report_paths.len() > MAX_LATENCY_CALIBRATION_SOURCE_REPORTS {
        bail!("at most {MAX_LATENCY_CALIBRATION_SOURCE_REPORTS} --report inputs are supported");
    }
    if options.minimum_samples_per_series == 0 {
        bail!("--minimum-samples-per-series must be positive");
    }
    let (config, config_source) = LiveConfig::load_with_evidence(config_path)
        .with_context(|| format!("failed to load live config {}", config_path.display()))?;
    let config_fingerprint = config.fingerprint()?;
    let evidence_config_fingerprint = config.evidence_fingerprint()?;
    let config_sha256 = config_source.sha256;
    let mut failures = Vec::new();
    if !config.host_guard.enabled || !config.host_guard.require_clock_synchronized {
        failures.push(
            "latency calibration requires an enabled synchronized-clock host guard".to_string(),
        );
    }
    let mut loaded = load_reports(
        config_path,
        report_paths,
        config_source.bytes,
        &config_sha256,
        &config_fingerprint,
        &evidence_config_fingerprint,
        &mut failures,
    )?;
    loaded.sort_by(|left, right| left.provenance.path.cmp(&right.provenance.path));
    let first_report = &loaded[0].report;
    let reap_version = first_report.reap_version.clone();
    let live_executable_sha256 = first_report.executable_sha256.clone();
    let source_host_identity_sha256 = first_report.host_identity_sha256.clone();
    let host_identity_sha256 = source_host_identity_sha256.clone().unwrap_or_default();
    let account_identity_sha256s = first_report.account_identity_sha256s.clone();
    let expected_accounts = config
        .accounts
        .iter()
        .map(|account| account.id.clone())
        .collect::<BTreeSet<_>>();
    if account_identity_sha256s
        .keys()
        .cloned()
        .collect::<BTreeSet<_>>()
        != expected_accounts
    {
        failures.push(
            "source live reports do not identify every configured account exactly once".to_string(),
        );
    }
    for source in &loaded {
        if source.report.reap_version != reap_version
            || source.report.executable_sha256 != live_executable_sha256
            || source.report.host_identity_sha256 != source_host_identity_sha256
            || source.report.account_identity_sha256s != account_identity_sha256s
        {
            failures.push(format!(
                "{} came from a different Reap build, host, or exchange account",
                source.provenance.path.display()
            ));
        }
    }

    let expected = expected_series(&config);
    let mut observed = BTreeMap::<SeriesKey, Vec<(usize, LiveLatencySeries)>>::new();
    for (report_index, source) in loaded.iter().enumerate() {
        for series in &source.report.latency_evidence.series {
            let key = SeriesKey {
                class: series.class,
                symbol: series.symbol.clone(),
                semantics: series.semantics,
            };
            observed
                .entry(key)
                .or_default()
                .push((report_index, series.clone()));
        }
    }

    for key in observed.keys() {
        if !expected.contains(key) {
            failures.push(format!(
                "unexpected latency series {:?}/{}/{} for the supplied live config",
                key.class,
                key.symbol,
                semantics_name(key.semantics)
            ));
        }
    }

    let mut calibration_series = Vec::new();
    let mut rules = Vec::new();
    for key in expected {
        let candidates = observed.get(&key).cloned().unwrap_or_default();
        let mut series_failures = Vec::new();
        let private_class = matches!(
            key.class,
            BacktestLatencyClass::MatchingNew
                | BacktestLatencyClass::MatchingCancel
                | BacktestLatencyClass::OrderUpdate
                | BacktestLatencyClass::OrderFill
        );
        let eligible = candidates
            .into_iter()
            .filter(|(report_index, _)| {
                !private_class || loaded[*report_index].report.mode == LiveMode::Demo
            })
            .collect::<Vec<_>>();
        if eligible.is_empty() {
            series_failures.push(if private_class {
                "no clean demo report supplied this private-path series".to_string()
            } else {
                "no clean live report supplied this series".to_string()
            });
        }
        if key.semantics.depends_on_exchange_clock()
            && eligible
                .iter()
                .any(|(index, _)| !loaded[*index].provenance.clock_guarded)
        {
            series_failures.push(
                "exchange-timestamp samples require synchronized host-guard evidence".to_string(),
            );
        }
        if key.semantics.is_matching_upper_bound() && !options.accept_matching_upper_bounds {
            series_failures.push(
                "exchange order acknowledgement is only an upper bound; rerun with --accept-matching-upper-bounds after reviewing that limitation"
                    .to_string(),
            );
        }

        let mut total_valid_observations = 0_u64;
        let mut total_operation_failures = 0_u64;
        let mut retained_input_samples = 0_usize;
        let mut source_hashes = BTreeSet::new();
        let mut sample_sets = Vec::new();
        for (report_index, series) in &eligible {
            validate_series(series, &mut series_failures);
            total_valid_observations =
                total_valid_observations.saturating_add(series.valid_observations);
            total_operation_failures =
                total_operation_failures.saturating_add(series.operation_failures);
            retained_input_samples =
                retained_input_samples.saturating_add(series.retained_samples_us.len());
            source_hashes.insert(loaded[*report_index].provenance.sha256.clone());
            if !series.retained_samples_us.is_empty() {
                sample_sets.push(series);
            }
        }
        if total_valid_observations < options.minimum_samples_per_series {
            series_failures.push(format!(
                "{} valid observations is below required {}",
                total_valid_observations, options.minimum_samples_per_series
            ));
        }
        let profile_samples_ms = merge_profile_samples(&sample_sets);
        if profile_samples_ms.is_empty() {
            series_failures.push("no retained valid samples are available".to_string());
        } else {
            rules.push(BacktestLatencyRule {
                class: key.class,
                symbol: Some(key.symbol.clone()),
                samples_ms: profile_samples_ms.clone(),
            });
        }
        series_failures.sort();
        series_failures.dedup();
        let passed = series_failures.is_empty();
        failures.extend(
            series_failures
                .iter()
                .map(|failure| format!("{:?}/{}: {failure}", key.class, key.symbol)),
        );
        calibration_series.push(LatencyCalibrationSeries {
            class: key.class,
            symbol: key.symbol,
            semantics: semantics_name(key.semantics).to_string(),
            source_report_sha256s: source_hashes.into_iter().collect(),
            total_valid_observations,
            total_operation_failures,
            retained_input_samples,
            profile_samples_ms,
            passed,
            failures: series_failures,
        });
    }

    let profile = BacktestLatencyProfile {
        seed: options.seed,
        rules,
    };
    if let Err(error) = profile.validate() {
        failures.push(format!("generated latency profile is invalid: {error}"));
    }
    failures.sort();
    failures.dedup();
    let artifact = LatencyCalibrationArtifact {
        schema_version: LATENCY_CALIBRATION_SCHEMA_VERSION,
        java_reference_revision: PINNED_JAVA_REVISION.to_string(),
        reap_version,
        live_executable_sha256,
        host_identity_sha256,
        account_identity_sha256s,
        live_config_sha256: config_sha256,
        live_config_fingerprint: config_fingerprint,
        live_config_evidence_fingerprint: evidence_config_fingerprint,
        profile_seed: options.seed,
        minimum_samples_per_series: options.minimum_samples_per_series,
        matching_latency_is_upper_bound: true,
        matching_upper_bounds_accepted: options.accept_matching_upper_bounds,
        source_reports: loaded.into_iter().map(|source| source.provenance).collect(),
        series: calibration_series,
        profile,
        passed: failures.is_empty(),
        failures,
    };
    if artifact.passed {
        artifact
            .validate_integrity()
            .context("generated latency calibration failed its integrity check")?;
    }
    Ok(artifact)
}

pub(crate) fn profile_toml(profile: &BacktestLatencyProfile) -> Result<String> {
    #[derive(Serialize)]
    struct Root<'a> {
        backtest: Backtest<'a>,
    }
    #[derive(Serialize)]
    struct Backtest<'a> {
        latency_profile: &'a BacktestLatencyProfile,
    }
    toml::to_string_pretty(&Root {
        backtest: Backtest {
            latency_profile: profile,
        },
    })
    .context("failed to serialize generated latency profile")
}

fn load_reports(
    config_path: &Path,
    paths: &[PathBuf],
    expected_config_bytes: u64,
    expected_config_sha256: &str,
    expected_fingerprint: &str,
    expected_evidence_fingerprint: &str,
    failures: &mut Vec<String>,
) -> Result<Vec<LoadedReport>> {
    let mut reports = Vec::new();
    let mut canonical_paths = HashSet::new();
    let mut session_ids = HashSet::new();
    let mut total_retained_input_samples = 0_usize;
    for path in paths {
        let verification = verify_live_run_paths(config_path, path, None)
            .with_context(|| format!("failed to verify live report {}", path.display()))?;
        if !verification.acceptance_passed {
            failures.push(format!(
                "{} failed independent live-run verification",
                path.display()
            ));
        }
        let canonical = fs::canonicalize(path)
            .with_context(|| format!("failed to resolve live report {}", path.display()))?;
        if !canonical_paths.insert(canonical.clone()) {
            bail!(
                "live report {} was supplied more than once",
                canonical.display()
            );
        }
        let report_size = fs::metadata(&canonical)
            .with_context(|| format!("failed to inspect live report {}", canonical.display()))?
            .len();
        if report_size > MAX_LATENCY_CALIBRATION_ARTIFACT_BYTES {
            bail!(
                "live report {} is {report_size} bytes, maximum is {}",
                canonical.display(),
                MAX_LATENCY_CALIBRATION_ARTIFACT_BYTES
            );
        }
        let bytes = fs::read(&canonical)
            .with_context(|| format!("failed to read live report {}", canonical.display()))?;
        let report: LiveRunReport = serde_json::from_slice(&bytes)
            .with_context(|| format!("failed to parse live report {}", canonical.display()))?;
        if report.latency_evidence.series.len() > MAX_LIVE_LATENCY_SERIES {
            bail!(
                "live report {} exceeds the latency-series bound",
                canonical.display()
            );
        }
        for series in &report.latency_evidence.series {
            total_retained_input_samples = total_retained_input_samples
                .checked_add(series.retained_samples_us.len())
                .context("latency source sample count overflow")?;
        }
        if total_retained_input_samples > MAX_LATENCY_CALIBRATION_RETAINED_INPUT_SAMPLES {
            bail!(
                "source reports retain more than {MAX_LATENCY_CALIBRATION_RETAINED_INPUT_SAMPLES} latency samples"
            );
        }
        let sha256 = sha256_bytes(&bytes);
        if verification.run_report.source_path != canonical
            || verification.run_report.bytes != bytes.len() as u64
            || verification.run_report.sha256 != sha256
        {
            bail!(
                "live report {} changed while it was being verified",
                canonical.display()
            );
        }
        if report.schema_version != LIVE_RUN_REPORT_SCHEMA_VERSION {
            failures.push(format!(
                "{} has live report schema {}, expected {}",
                canonical.display(),
                report.schema_version,
                LIVE_RUN_REPORT_SCHEMA_VERSION
            ));
        }
        if !report.config_source.as_ref().is_some_and(|source| {
            source.bytes == expected_config_bytes && source.sha256 == expected_config_sha256
        }) {
            failures.push(format!(
                "{} exact source config bytes do not match the supplied live config",
                canonical.display()
            ));
        }
        if report.latency_evidence.schema_version != LIVE_LATENCY_EVIDENCE_SCHEMA_VERSION {
            failures.push(format!(
                "{} has latency evidence schema {}, expected {}",
                canonical.display(),
                report.latency_evidence.schema_version,
                LIVE_LATENCY_EVIDENCE_SCHEMA_VERSION
            ));
        }
        if report.latency_evidence.reservoir_capacity_per_series != LIVE_LATENCY_RESERVOIR_CAPACITY
            || report.latency_evidence.maximum_latency_us != MAX_LIVE_LATENCY_US
        {
            failures.push(format!(
                "{} has unsupported latency collector bounds",
                canonical.display()
            ));
        }
        if report.java_reference_revision != PINNED_JAVA_REVISION {
            failures.push(format!(
                "{} references Java revision {}, expected {}",
                canonical.display(),
                report.java_reference_revision,
                PINNED_JAVA_REVISION
            ));
        }
        if report.reap_version.is_empty()
            || !is_lower_sha256(&report.executable_sha256)
            || !report
                .host_identity_sha256
                .as_deref()
                .is_some_and(is_lower_sha256)
            || report.account_identity_sha256s.is_empty()
            || report
                .account_identity_sha256s
                .iter()
                .any(|(account_id, hash)| account_id.is_empty() || !is_lower_sha256(hash))
        {
            failures.push(format!(
                "{} lacks valid Reap build, host, or exchange-account identity",
                canonical.display()
            ));
        }
        if report.config_fingerprint != expected_fingerprint {
            failures.push(format!(
                "{} config fingerprint does not match the supplied live config",
                canonical.display()
            ));
        }
        if report.evidence_config_fingerprint != expected_evidence_fingerprint {
            failures.push(format!(
                "{} full evidence config fingerprint does not match the supplied live config",
                canonical.display()
            ));
        }
        if !report.clean_soak || !report.reached_ready {
            failures.push(format!(
                "{} is not a clean ready bounded soak",
                canonical.display()
            ));
        }
        if report.stop_reason != LiveStopReason::DurationElapsed {
            failures.push(format!(
                "{} did not stop after its configured bounded duration",
                canonical.display()
            ));
        }
        if let Some(failure) = &report.failure {
            let well_formed = !failure.code.is_empty()
                && failure.code.len() <= MAX_LIVE_FAILURE_CODE_BYTES
                && failure
                    .code
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte == b'_')
                && !failure.message.is_empty()
                && failure.message.len() <= MAX_LIVE_FAILURE_MESSAGE_BYTES;
            if well_formed {
                failures.push(format!(
                    "{} records runtime failure {}",
                    canonical.display(),
                    failure.code
                ));
            } else {
                failures.push(format!(
                    "{} has malformed runtime failure evidence",
                    canonical.display()
                ));
            }
        }
        if (report.stop_reason == LiveStopReason::RuntimeFailure) != report.failure.is_some() {
            failures.push(format!(
                "{} has inconsistent runtime failure evidence",
                canonical.display()
            ));
        }
        if report.connection_disconnect_events
            != report
                .public_connection_disconnect_events
                .saturating_add(report.private_connection_disconnect_events)
                .saturating_add(report.order_transport_disconnect_events)
        {
            failures.push(format!(
                "{} has inconsistent public/private/order-transport disconnect evidence",
                canonical.display()
            ));
        }
        if report.mode == LiveMode::Validate {
            failures.push(format!(
                "{} is a validation report, not a bounded live run",
                canonical.display()
            ));
        }
        if report.latency_evidence.dropped_observations > 0 {
            failures.push(format!(
                "{} dropped or censored {} observations at a bounded collector or authoritative-recovery boundary",
                canonical.display(),
                report.latency_evidence.dropped_observations
            ));
        }
        let session_id = report.session_id.clone().unwrap_or_default();
        if session_id.is_empty() {
            failures.push(format!("{} has no live session id", canonical.display()));
        } else if !session_ids.insert(session_id.clone()) {
            failures.push(format!(
                "live session {session_id} appears in multiple reports"
            ));
        }
        let clock_guarded = report
            .host_preflight
            .as_ref()
            .is_some_and(|snapshot| snapshot.clock_synchronized)
            && report.host_checks > 0
            && report
                .host_last_snapshot
                .as_ref()
                .is_some_and(|snapshot| snapshot.clock_synchronized);
        if !clock_guarded {
            failures.push(format!(
                "{} lacks synchronized host-guard evidence",
                canonical.display()
            ));
        }
        let mut series_identities = HashSet::new();
        for series in &report.latency_evidence.series {
            if !series_identities.insert((series.class, series.symbol.as_str(), series.semantics)) {
                failures.push(format!(
                    "{} repeats latency series {:?}/{}/{}",
                    canonical.display(),
                    series.class,
                    series.symbol,
                    semantics_name(series.semantics)
                ));
            }
        }
        reports.push(LoadedReport {
            provenance: LatencySourceReport {
                path: canonical,
                sha256,
                session_id,
                mode: mode_name(report.mode).to_string(),
                reap_version: report.reap_version.clone(),
                executable_sha256: report.executable_sha256.clone(),
                host_identity_sha256: report.host_identity_sha256.clone().unwrap_or_default(),
                account_identity_sha256s: report.account_identity_sha256s.clone(),
                config_fingerprint: report.config_fingerprint.clone(),
                evidence_config_fingerprint: report.evidence_config_fingerprint.clone(),
                clean_soak: report.clean_soak,
                reached_ready: report.reached_ready,
                clock_guarded,
            },
            report,
        });
    }
    Ok(reports)
}

fn expected_series(config: &LiveConfig) -> BTreeSet<SeriesKey> {
    let mut expected = BTreeSet::new();
    for instrument in &config.strategy.instruments {
        for (class, semantics) in [
            (
                BacktestLatencyClass::MarketDepth,
                LiveLatencySemantics::HostReceiveToStrategyVisibility,
            ),
            (
                BacktestLatencyClass::HistoricalTrade,
                LiveLatencySemantics::HostReceiveToStrategyVisibility,
            ),
            (
                BacktestLatencyClass::MatchingNew,
                LiveLatencySemantics::StrategyDispatchToOrderAckUpperBound,
            ),
            (
                BacktestLatencyClass::MatchingCancel,
                LiveLatencySemantics::StrategyDispatchToOrderAckUpperBound,
            ),
            (
                BacktestLatencyClass::OrderUpdate,
                LiveLatencySemantics::ExchangeTimestampToStrategyVisibility,
            ),
            (
                BacktestLatencyClass::OrderFill,
                LiveLatencySemantics::FillToAccountStateVisibility,
            ),
        ] {
            expected.insert(SeriesKey {
                class,
                symbol: instrument.symbol.clone(),
                semantics,
            });
        }
        if instrument.kind.is_derivative() {
            expected.insert(reference_series(instrument.symbol.clone()));
        }
        if let Some(index_symbol) = &instrument.index_symbol {
            expected.insert(reference_series(index_symbol.clone()));
        }
    }
    for guard in &config.risk.stablecoin_guards {
        expected.insert(reference_series(guard.symbol.clone()));
    }
    expected
}

fn reference_series(symbol: String) -> SeriesKey {
    SeriesKey {
        class: BacktestLatencyClass::ReferenceData,
        symbol,
        semantics: LiveLatencySemantics::HostReceiveToStrategyVisibility,
    }
}

fn validate_series(series: &LiveLatencySeries, failures: &mut Vec<String>) {
    let classified_observations = series
        .valid_observations
        .checked_add(series.negative_clock_observations)
        .and_then(|value| value.checked_add(series.above_limit_observations));
    if classified_observations != Some(series.observations) {
        failures
            .push("total observations do not equal valid plus rejected observations".to_string());
    }
    if u64::try_from(series.retained_samples_us.len()).unwrap_or(u64::MAX)
        > series.valid_observations
    {
        failures.push("retained sample count exceeds valid observations".to_string());
    }
    let expected_retained = series
        .valid_observations
        .min(LIVE_LATENCY_RESERVOIR_CAPACITY as u64) as usize;
    if series.retained_samples_us.len() != expected_retained {
        failures.push("retained sample count does not match collector policy".to_string());
    }
    if series.retained_samples_us.len() > LIVE_LATENCY_RESERVOIR_CAPACITY {
        failures.push("retained sample count exceeds collector capacity".to_string());
    }
    if series.operation_failures > 0 {
        failures.push(format!(
            "{} exchange operations failed without a latency sample",
            series.operation_failures
        ));
    }
    if series.negative_clock_observations > 0 {
        failures.push(format!(
            "{} negative/invalid clock observations were rejected",
            series.negative_clock_observations
        ));
    }
    if series.above_limit_observations > 0 {
        failures.push(format!(
            "{} observations exceeded the live latency bound",
            series.above_limit_observations
        ));
    }
    if series.valid_observations > 0
        && (series.minimum_latency_us.is_none()
            || series.maximum_latency_us.is_none()
            || series.mean_latency_us.is_none()
            || series.retained_samples_us.is_empty())
    {
        failures.push("nonempty series has incomplete summary statistics".to_string());
    }
    if series.valid_observations == 0
        && (series.total_latency_us != 0
            || series.minimum_latency_us.is_some()
            || series.maximum_latency_us.is_some()
            || series.mean_latency_us.is_some()
            || !series.retained_samples_us.is_empty())
    {
        failures.push("empty series has nonempty summary statistics".to_string());
    }
    if let (Some(minimum), Some(maximum), Some(mean)) = (
        series.minimum_latency_us,
        series.maximum_latency_us,
        series.mean_latency_us,
    ) {
        if minimum > maximum || maximum > MAX_LIVE_LATENCY_US {
            failures.push("latency summary bounds are invalid".to_string());
        }
        let total = u128::from(series.total_latency_us);
        let count = u128::from(series.valid_observations);
        if total < u128::from(minimum) * count || total > u128::from(maximum) * count {
            failures.push("latency total lies outside summary bounds".to_string());
        }
        let expected_mean = series.total_latency_us as f64 / series.valid_observations as f64;
        if !mean.is_finite() || (mean - expected_mean).abs() > f64::EPSILON * expected_mean.max(1.0)
        {
            failures.push("latency mean does not match total and count".to_string());
        }
        if series
            .retained_samples_us
            .iter()
            .any(|sample| *sample < minimum || *sample > maximum)
        {
            failures.push("retained sample lies outside summary bounds".to_string());
        }
    }
    if !series
        .retained_samples_us
        .windows(2)
        .all(|pair| pair[0] <= pair[1])
    {
        failures.push("retained samples are not sorted".to_string());
    }
}

fn merge_profile_samples(series: &[&LiveLatencySeries]) -> Vec<u64> {
    if series.is_empty() {
        return Vec::new();
    }
    let retained = series
        .iter()
        .map(|series| series.retained_samples_us.len())
        .fold(0_usize, usize::saturating_add);
    if retained <= 65_536
        && series.iter().all(|series| {
            usize::try_from(series.valid_observations).ok()
                == Some(series.retained_samples_us.len())
        })
    {
        let mut samples = series
            .iter()
            .flat_map(|series| series.retained_samples_us.iter().copied())
            .map(ceil_us_to_ms)
            .collect::<Vec<_>>();
        samples.sort_unstable();
        return samples;
    }

    let mut weighted = Vec::with_capacity(retained);
    for source in series {
        if source.retained_samples_us.is_empty() {
            continue;
        }
        let weight = source.valid_observations as f64 / source.retained_samples_us.len() as f64;
        weighted.extend(
            source
                .retained_samples_us
                .iter()
                .copied()
                .map(|sample| (sample, weight)),
        );
    }
    weighted.sort_by_key(|(sample, _)| *sample);
    let output_count = MERGED_PROFILE_SAMPLE_CAPACITY.min(
        series
            .iter()
            .map(|series| series.valid_observations)
            .fold(0_u64, u64::saturating_add)
            .min(usize::MAX as u64) as usize,
    );
    if output_count == 0 {
        return Vec::new();
    }
    let total_weight = weighted.iter().map(|(_, weight)| *weight).sum::<f64>();
    let mut output = Vec::with_capacity(output_count);
    let mut index = 0_usize;
    let mut cumulative = weighted.first().map_or(0.0, |(_, weight)| *weight);
    for ordinal in 0..output_count {
        let target = (ordinal as f64 + 0.5) * total_weight / output_count as f64;
        while index + 1 < weighted.len() && cumulative < target {
            index += 1;
            cumulative += weighted[index].1;
        }
        output.push(ceil_us_to_ms(weighted[index].0));
    }
    output
}

fn ceil_us_to_ms(value: u64) -> u64 {
    value.saturating_add(999) / 1_000
}

fn mode_name(mode: LiveMode) -> &'static str {
    match mode {
        LiveMode::Validate => "validate",
        LiveMode::Observe => "observe",
        LiveMode::Demo => "demo",
    }
}

fn semantics_name(semantics: LiveLatencySemantics) -> &'static str {
    match semantics {
        LiveLatencySemantics::HostReceiveToStrategyVisibility => {
            "host_receive_to_strategy_visibility"
        }
        LiveLatencySemantics::ExchangeTimestampToStrategyVisibility => {
            "exchange_timestamp_to_strategy_visibility"
        }
        LiveLatencySemantics::StrategyDispatchToOrderAckUpperBound => {
            "strategy_dispatch_to_order_ack_upper_bound"
        }
        LiveLatencySemantics::FillToAccountStateVisibility => "fill_to_account_state_visibility",
    }
}

fn sha256_bytes(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn is_lower_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ready_snapshot() -> reap_live::ReadinessSnapshot {
        reap_live::ReadinessSnapshot {
            phase: reap_live::LivePhase::Ready,
            metadata_verified: true,
            storage_ready: true,
            public_connectivity_ready: true,
            missing_reconciliation: Vec::new(),
            missing_account_snapshots: Vec::new(),
            missing_books: Vec::new(),
            missing_private_streams: Vec::new(),
            missing_order_transports: Vec::new(),
            missing_stablecoin_rates: Vec::new(),
            faults: BTreeMap::new(),
        }
    }

    fn series(values: &[u64], population: u64) -> LiveLatencySeries {
        LiveLatencySeries {
            class: BacktestLatencyClass::MarketDepth,
            symbol: "BTC-USDT".to_string(),
            semantics: LiveLatencySemantics::HostReceiveToStrategyVisibility,
            observations: population,
            valid_observations: population,
            operation_failures: 0,
            negative_clock_observations: 0,
            above_limit_observations: 0,
            total_latency_us: values.iter().sum(),
            minimum_latency_us: values.iter().min().copied(),
            maximum_latency_us: values.iter().max().copied(),
            mean_latency_us: Some(values.iter().sum::<u64>() as f64 / values.len() as f64),
            retained_samples_us: values.to_vec(),
        }
    }

    #[test]
    fn exact_samples_are_preserved_and_rounded_up_to_milliseconds() {
        let first = series(&[0, 1, 999, 1_000], 4);
        assert_eq!(merge_profile_samples(&[&first]), vec![0, 1, 1, 1]);
    }

    #[test]
    fn weighted_merge_respects_source_populations_and_is_bounded() {
        let small = series(&[100], 10_000);
        let large = series(&[10_000], 90_000);
        let merged = merge_profile_samples(&[&small, &large]);
        assert_eq!(merged.len(), MERGED_PROFILE_SAMPLE_CAPACITY);
        assert!(merged.iter().filter(|sample| **sample == 10).count() > 7_000);
        assert!(merged.iter().filter(|sample| **sample == 1).count() < 1_000);
    }

    #[test]
    fn profile_fragment_uses_backtest_latency_profile_shape() {
        let profile = BacktestLatencyProfile {
            seed: 42,
            rules: vec![BacktestLatencyRule {
                class: BacktestLatencyClass::MarketDepth,
                symbol: Some("BTC-USDT".to_string()),
                samples_ms: vec![1, 2],
            }],
        };
        let output = profile_toml(&profile).unwrap();
        assert!(output.contains("[backtest.latency_profile]"));
        assert!(output.contains("[[backtest.latency_profile.rules]]"));
    }

    #[test]
    fn calibration_builds_a_complete_profile_from_bound_demo_evidence() {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "reap-latency-calibration-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir(&directory).unwrap();
        let config_path = directory.join("live.toml");
        let report_path = directory.join("live.json");

        let mut config =
            LiveConfig::from_toml(include_str!("../../../examples/live-okx-demo.toml")).unwrap();
        config.host_guard.enabled = true;
        let config_bytes = toml::to_string_pretty(&config).unwrap().into_bytes();
        fs::write(&config_path, &config_bytes).unwrap();
        let fingerprint = config.fingerprint().unwrap();
        let evidence_fingerprint = config.evidence_fingerprint().unwrap();
        let identity = BTreeMap::from([("main".to_string(), "3".repeat(64))]);
        let series = expected_series(&config)
            .into_iter()
            .map(|key| LiveLatencySeries {
                class: key.class,
                symbol: key.symbol,
                semantics: key.semantics,
                observations: 1,
                valid_observations: 1,
                operation_failures: 0,
                negative_clock_observations: 0,
                above_limit_observations: 0,
                total_latency_us: 500,
                minimum_latency_us: Some(500),
                maximum_latency_us: Some(500),
                mean_latency_us: Some(500.0),
                retained_samples_us: vec![500],
            })
            .collect::<Vec<_>>();
        let host = reap_live::HostHealthSnapshot {
            checked_at_ms: 1,
            disk_available_bytes: u64::MAX,
            memory_available_bytes: u64::MAX,
            clock_synchronized: true,
        };
        let ready = ready_snapshot();
        let report = LiveRunReport {
            schema_version: LIVE_RUN_REPORT_SCHEMA_VERSION,
            session_id: Some("session-1".to_string()),
            session_started_at_ms: 1,
            config_source: Some(reap_live::LiveConfigFileEvidence {
                source_path: fs::canonicalize(&config_path).unwrap(),
                bytes: config_bytes.len() as u64,
                sha256: sha256_bytes(&config_bytes),
            }),
            config_fingerprint: fingerprint,
            evidence_config_fingerprint: evidence_fingerprint,
            java_reference_revision: PINNED_JAVA_REVISION.to_string(),
            reap_version: env!("CARGO_PKG_VERSION").to_string(),
            executable_sha256: "1".repeat(64),
            host_identity_sha256: Some("2".repeat(64)),
            account_identity_sha256s: identity,
            mode: LiveMode::Demo,
            stop_reason: reap_live::LiveStopReason::DurationElapsed,
            failure: None,
            elapsed_ms: 1_000,
            reached_ready: true,
            time_to_ready_ms: Some(1),
            readiness_loss_count: 0,
            max_readiness_outage_ms: 0,
            reconciliation_drift_events: 0,
            book_recovery_events: 0,
            stream_stale_events: 0,
            connection_disconnect_events: 0,
            public_connection_disconnect_events: 0,
            private_connection_disconnect_events: 0,
            order_transport_disconnect_events: 0,
            order_transport_stale_events: 0,
            ambiguous_submit_events: 0,
            ambiguous_cancel_events: 0,
            partial_fill_events: 0,
            fill_convergence_timeout_events: 0,
            order_convergence_timeout_events: 0,
            restored_safety_latches: 0,
            operator_commands: 0,
            operator_mutations: 0,
            max_storage_queue_depth: 1,
            alerts_delivered: 0,
            alert_delivery_failures: 0,
            alert_failure_notifications_dropped: 0,
            max_alert_queue_depth: 0,
            host_preflight: Some(host.clone()),
            host_checks: 1,
            host_last_snapshot: Some(host),
            readiness_at_stop: ready.clone(),
            readiness: ready,
            dropped_storage_records: 0,
            active_orders_after_shutdown: 0,
            latency_evidence: reap_live::LiveLatencyEvidence {
                schema_version: LIVE_LATENCY_EVIDENCE_SCHEMA_VERSION,
                reservoir_capacity_per_series: LIVE_LATENCY_RESERVOIR_CAPACITY,
                maximum_latency_us: MAX_LIVE_LATENCY_US,
                dropped_observations: 0,
                series,
            },
            clean_soak: true,
        };
        fs::write(&report_path, serde_json::to_vec(&report).unwrap()).unwrap();
        let verification =
            reap_live::verify_live_run_paths(&config_path, &report_path, Some(LiveMode::Demo))
                .unwrap();
        assert!(verification.acceptance_passed, "{verification:#?}");

        let artifact = build_latency_calibration(
            &config_path,
            std::slice::from_ref(&report_path),
            LatencyCalibrationOptions {
                seed: 42,
                minimum_samples_per_series: 1,
                accept_matching_upper_bounds: true,
            },
        )
        .unwrap();

        assert!(artifact.passed, "{:?}", artifact.failures);
        assert_eq!(artifact.profile.rules.len(), artifact.series.len());
        assert!(
            artifact
                .profile
                .rules
                .iter()
                .all(|rule| rule.samples_ms == [1])
        );
        assert_eq!(artifact.live_executable_sha256, "1".repeat(64));
        artifact.validate_integrity().unwrap();

        let forged_clean_report_path = directory.join("forged-clean-live.json");
        let mut forged_clean_report = report.clone();
        forged_clean_report.session_id = Some("session-forged-clean".to_string());
        forged_clean_report.operator_mutations = 1;
        fs::write(
            &forged_clean_report_path,
            serde_json::to_vec(&forged_clean_report).unwrap(),
        )
        .unwrap();
        let forged_clean_artifact = build_latency_calibration(
            &config_path,
            &[forged_clean_report_path],
            LatencyCalibrationOptions {
                seed: 42,
                minimum_samples_per_series: 1,
                accept_matching_upper_bounds: true,
            },
        )
        .unwrap();
        assert!(!forged_clean_artifact.passed);
        assert!(
            forged_clean_artifact
                .failures
                .iter()
                .any(|failure| failure.contains("failed independent live-run verification"))
        );

        let failed_report_path = directory.join("failed-live.json");
        let mut failed_report = report;
        failed_report.session_id = Some("session-2".to_string());
        failed_report.stop_reason = LiveStopReason::RuntimeFailure;
        failed_report.failure = Some(reap_live::LiveFailureEvidence {
            code: "gateway_task".to_string(),
            message: "injected failure".to_string(),
        });
        failed_report.clean_soak = false;
        fs::write(
            &failed_report_path,
            serde_json::to_vec(&failed_report).unwrap(),
        )
        .unwrap();
        let failed_artifact = build_latency_calibration(
            &config_path,
            &[failed_report_path],
            LatencyCalibrationOptions {
                seed: 42,
                minimum_samples_per_series: 1,
                accept_matching_upper_bounds: true,
            },
        )
        .unwrap();
        assert!(!failed_artifact.passed);
        assert!(
            failed_artifact
                .failures
                .iter()
                .any(|failure| failure.contains("records runtime failure gateway_task"))
        );

        let malformed_report_path = directory.join("malformed-failure-live.json");
        failed_report.session_id = Some("session-3".to_string());
        failed_report.failure.as_mut().unwrap().code = "gateway_task\nforged".to_string();
        fs::write(
            &malformed_report_path,
            serde_json::to_vec(&failed_report).unwrap(),
        )
        .unwrap();
        let malformed_artifact = build_latency_calibration(
            &config_path,
            &[malformed_report_path],
            LatencyCalibrationOptions {
                seed: 42,
                minimum_samples_per_series: 1,
                accept_matching_upper_bounds: true,
            },
        )
        .unwrap();
        assert!(
            malformed_artifact
                .failures
                .iter()
                .any(|failure| failure.contains("malformed runtime failure evidence"))
        );
        assert!(
            malformed_artifact
                .failures
                .iter()
                .all(|failure| !failure.contains("forged"))
        );

        let mut tampered_config = fs::read(&config_path).unwrap();
        tampered_config.extend_from_slice(b"\n# formatting-only tamper\n");
        fs::write(&config_path, tampered_config).unwrap();
        let config_tamper_artifact = build_latency_calibration(
            &config_path,
            std::slice::from_ref(&report_path),
            LatencyCalibrationOptions {
                seed: 42,
                minimum_samples_per_series: 1,
                accept_matching_upper_bounds: true,
            },
        )
        .unwrap();
        assert!(
            config_tamper_artifact
                .failures
                .iter()
                .any(|failure| failure.contains("exact source config bytes"))
        );
        fs::remove_dir_all(directory).unwrap();
    }
}
