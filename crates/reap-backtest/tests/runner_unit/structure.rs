use std::path::Path;

fn collect_runner_module_sources(directory: &Path, sources: &mut Vec<(String, String)>) {
    if !directory.exists() {
        return;
    }
    let mut entries = std::fs::read_dir(directory)
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", directory.display()))
        .collect::<Result<Vec<_>, _>>()
        .unwrap_or_else(|error| panic!("failed to enumerate {}: {error}", directory.display()));
    entries.sort_by_key(|entry| entry.path());
    for entry in entries {
        let path = entry.path();
        if path.is_dir() {
            collect_runner_module_sources(&path, sources);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            let source = std::fs::read_to_string(&path)
                .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()));
            sources.push((path.display().to_string(), source));
        }
    }
}

#[test]
fn production_runner_keeps_single_owner_responsibility_state() {
    const TEST_MODULE_MARKER: &str =
        "#[cfg(test)]\n#[path = \"../tests/runner_unit/mod.rs\"]\nmod tests";
    const ROOT_FIELDS: [&str; 67] = [
        "strategy_config: ChaosConfig",
        "strategy: ChaosStrategy",
        "matchers: BTreeMap<Symbol, MatchingEngine>",
        "portfolio: Portfolio",
        "initial_portfolio: BacktestInitialPortfolioConfig",
        "initial_account_snapshot_delivered: bool",
        "execution: BacktestExecutionConfig",
        "latency_sampler: BacktestLatencySampler",
        "time_basis: BacktestTimeBasis",
        "raw_replay_boundary: Option<RawReplayBoundary>",
        "carry_source_boundary: Option<RawReplayBoundary>",
        "scheduled: BTreeMap<(u64, u64), ScheduledAction>",
        "next_action_seq: u64",
        "pending_cancels: HashSet<String>",
        "pending_fill_account_updates: usize",
        "last_account_publish_ns: Option<u64>",
        "periodic_account_refreshes: u64",
        "depth_marks: HashMap<Symbol, f64>",
        "exchange_marks: HashMap<Symbol, f64>",
        "currency_by_index_symbol: HashMap<Symbol, String>",
        "currency_rate_observations: HashMap<String, CurrencyRateObservation>",
        "realized_funding_rates: HashMap<(Symbol, u64), f64>",
        "scheduled_funding: HashSet<(Symbol, u64)>",
        "settled_funding: HashSet<(Symbol, u64)>",
        "last_settled_funding_time_ms: BTreeMap<Symbol, u64>",
        "now_ns: u64",
        "first_arrival_ns: Option<u64>",
        "last_arrival_ns: Option<u64>",
        "input_events: u64",
        "input_clock_regressions: u64",
        "max_input_clock_regression_ns: u64",
        "order_entry_ready_at_ns: Option<u64>",
        "new_orders_blocked_not_ready: usize",
        "orders_sent: usize",
        "cancel_requests: usize",
        "deduplicated_cancel_requests: usize",
        "ignored_cancel_requests: usize",
        "exchange_activations: usize",
        "cancelled_orders: usize",
        "rejected_orders: usize",
        "fills: usize",
        "maker_fills: usize",
        "taker_fills: usize",
        "funding_rate_events: u64",
        "funding_settlements: u64",
        "late_funding_rate_events: u64",
        "invalid_funding_rate_events: u64",
        "missed_funding_settlements: u64",
        "funding_settlement_failures: u64",
        "currency_rate_events: u64",
        "invalid_currency_rate_events: u64",
        "opening_equity_usd: Option<f64>",
        "opening_valuation_at_ns: Option<u64>",
        "peak_equity_usd: f64",
        "max_drawdown_usd: f64",
        "max_abs_delta_usd: f64",
        "max_abs_pending_delta_usd: f64",
        "max_gross_exposure_usd: f64",
        "max_active_orders: usize",
        "max_active_order_notional_usd: f64",
        "abs_delta_time_integral: f64",
        "inventory_open_duration_ns: u64",
        "metric_clock_ns: Option<u64>",
        "current_abs_delta_usd: f64",
        "current_inventory_open: bool",
        "risk_metric_samples: u64",
        "invalid_risk_metric_samples: u64",
    ];

    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let root_path = manifest.join("src/lib.rs");
    let root_source = std::fs::read_to_string(&root_path)
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", root_path.display()));
    let root_path_string = root_path.display().to_string();
    let (production_root, _) = root_source
        .split_once(TEST_MODULE_MARKER)
        .expect("backtest runner test module marker");
    let mut sources = vec![(root_path_string.clone(), production_root.to_string())];
    collect_runner_module_sources(&manifest.join("src/runner"), &mut sources);

    assert_eq!(
        sources
            .iter()
            .map(|(_, source)| source.matches("pub struct BacktestRunner").count())
            .sum::<usize>(),
        1,
        "production runner sources must define exactly one BacktestRunner owner",
    );

    let (_, after_owner) = production_root
        .split_once("pub struct BacktestRunner {\n")
        .expect("BacktestRunner owner");
    let (field_body, _) = after_owner
        .split_once("\n}\n")
        .expect("BacktestRunner field body");
    assert_eq!(
        field_body
            .lines()
            .filter(|line| !line.trim().is_empty())
            .count(),
        ROOT_FIELDS.len(),
        "test-only extraction must retain the exact 67-field root owner",
    );
    for field in ROOT_FIELDS {
        assert!(
            field_body.contains(field),
            "sole-owner state `{field}` must remain on the root BacktestRunner",
        );
        for (path, source) in sources.iter().filter(|(path, _)| path != &root_path_string) {
            assert!(
                !source.contains(field),
                "{path} must not redeclare sole-owner state `{field}`",
            );
        }
    }

    for (path, source) in sources {
        let compact = source
            .chars()
            .filter(|character| !character.is_whitespace())
            .collect::<String>();
        assert!(
            !compact.contains("Arc<Mutex"),
            "{path} must not put runner ownership behind Arc<Mutex<_>>",
        );
        assert!(
            !source.contains("use super::*;"),
            "{path} must declare explicit production dependencies",
        );
    }

    let manifest_source =
        std::fs::read_to_string(manifest.join("Cargo.toml")).expect("backtest Cargo.toml");
    let (_, normal_dependencies) = manifest_source
        .split_once("[dependencies]\n")
        .expect("normal dependency table");
    let normal_dependencies = normal_dependencies
        .split_once("\n[")
        .map_or(normal_dependencies, |(dependencies, _)| dependencies);
    assert!(
        normal_dependencies.lines().all(|line| {
            line.split(['.', '='])
                .next()
                .is_none_or(|dependency| dependency.trim() != "reap-live")
        }),
        "reap-backtest must not add a normal dependency on reap-live",
    );
}
