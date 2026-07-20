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

fn struct_field_body<'a>(source: &'a str, visibility: &str, name: &str) -> &'a str {
    let declaration = format!("{visibility} struct {name} {{\n");
    let (_, after_declaration) = source
        .split_once(&declaration)
        .unwrap_or_else(|| panic!("missing `{declaration}`"));
    after_declaration
        .split_once("\n}\n")
        .unwrap_or_else(|| panic!("unterminated `{declaration}`"))
        .0
}

fn state_field_signatures<'a>(source: &'a str, state: &str) -> Vec<&'a str> {
    struct_field_body(source, "pub(super)", state)
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with("//"))
        .map(|line| {
            line.strip_prefix("pub(super) ")
                .and_then(|field| field.strip_suffix(','))
                .unwrap_or_else(|| panic!("invalid state field declaration `{line}`"))
        })
        .collect()
}

fn assert_exact_state_fields(source: &str, state: &str, expected_fields: &[&str]) {
    assert_eq!(
        state_field_signatures(source, state),
        expected_fields,
        "{state} must remain an exact behavior-free state bag",
    );
}

#[test]
fn production_runner_keeps_single_owner_responsibility_state() {
    const TEST_MODULE_MARKER: &str =
        "#[cfg(test)]\n#[path = \"../tests/runner_unit/mod.rs\"]\nmod tests";
    const ROOT_FIELDS: [&str; 11] = [
        "strategy_config: ChaosConfig",
        "strategy: ChaosStrategy",
        "execution: BacktestExecutionConfig",
        "latency_sampler: BacktestLatencySampler",
        "replay: ReplayState",
        "schedule: ScheduleState",
        "orders: OrderLifecycleState",
        "valuation: ValuationState",
        "funding: FundingState",
        "accounting: AccountingState",
        "metrics: MetricState",
    ];
    const DIRECT_LEAF_FIELDS: [&str; 4] = [
        "strategy_config: ChaosConfig",
        "strategy: ChaosStrategy",
        "execution: BacktestExecutionConfig",
        "latency_sampler: BacktestLatencySampler",
    ];
    const GROUP_FIELDS: [&str; 7] = [
        "replay: ReplayState",
        "schedule: ScheduleState",
        "orders: OrderLifecycleState",
        "valuation: ValuationState",
        "funding: FundingState",
        "accounting: AccountingState",
        "metrics: MetricState",
    ];
    const REPLAY_FIELDS: [&str; 10] = [
        "time_basis: BacktestTimeBasis",
        "raw_replay_boundary: Option<RawReplayBoundary>",
        "carry_source_boundary: Option<RawReplayBoundary>",
        "now_ns: u64",
        "first_arrival_ns: Option<u64>",
        "last_arrival_ns: Option<u64>",
        "input_events: u64",
        "input_clock_regressions: u64",
        "max_input_clock_regression_ns: u64",
        "trade_reprice_active: bool",
    ];
    const SCHEDULE_FIELDS: [&str; 2] = [
        "scheduled: BTreeMap<(u64, u64), ScheduledAction>",
        "next_action_seq: u64",
    ];
    const ORDER_FIELDS: [&str; 18] = [
        "matchers: BTreeMap<Symbol, MatchingEngine>",
        "initial_account_snapshot_delivered: bool",
        "pending_cancels: HashSet<String>",
        "pending_fill_account_updates: usize",
        "last_account_publish_ns: Option<u64>",
        "periodic_account_refreshes: u64",
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
    ];
    const VALUATION_FIELDS: [&str; 8] = [
        "depth_marks: HashMap<Symbol, f64>",
        "exchange_marks: HashMap<Symbol, f64>",
        "currency_by_index_symbol: HashMap<Symbol, String>",
        "currency_rate_observations: HashMap<String, CurrencyRateObservation>",
        "currency_rate_events: u64",
        "invalid_currency_rate_events: u64",
        "opening_equity_usd: Option<f64>",
        "opening_valuation_at_ns: Option<u64>",
    ];
    const FUNDING_FIELDS: [&str; 4] = [
        "realized_funding_rates: HashMap<(Symbol, u64), f64>",
        "scheduled_funding: HashSet<(Symbol, u64)>",
        "settled_funding: HashSet<(Symbol, u64)>",
        "last_settled_funding_time_ms: BTreeMap<Symbol, u64>",
    ];
    const ACCOUNTING_FIELDS: [&str; 8] = [
        "portfolio: Portfolio",
        "initial_portfolio: BacktestInitialPortfolioConfig",
        "funding_rate_events: u64",
        "funding_settlements: u64",
        "late_funding_rate_events: u64",
        "invalid_funding_rate_events: u64",
        "missed_funding_settlements: u64",
        "funding_settlement_failures: u64",
    ];
    const METRIC_FIELDS: [&str; 14] = [
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

    let field_body = struct_field_body(production_root, "pub", "BacktestRunner");
    let actual_root_fields = field_body
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(|line| {
            line.strip_suffix(',')
                .unwrap_or_else(|| panic!("invalid root field declaration `{line}`"))
        })
        .collect::<Vec<_>>();
    assert_eq!(
        actual_root_fields, ROOT_FIELDS,
        "BacktestRunner must retain the exact grouped root owner",
    );
    assert_eq!(
        &actual_root_fields[..DIRECT_LEAF_FIELDS.len()],
        DIRECT_LEAF_FIELDS,
        "runner dependencies must remain direct leaves",
    );
    assert_eq!(
        &actual_root_fields[DIRECT_LEAF_FIELDS.len()..],
        GROUP_FIELDS,
        "state groups must remain held by value on BacktestRunner",
    );
    for field in ROOT_FIELDS {
        for (path, source) in sources.iter().filter(|(path, _)| path != &root_path_string) {
            assert!(
                !source.lines().any(|line| line == format!("    {field},")),
                "{path} must not redeclare sole-owner state `{field}`",
            );
        }
    }

    let state_path = manifest.join("src/runner/state.rs");
    let state_source = std::fs::read_to_string(&state_path)
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", state_path.display()));
    let state_groups = [
        ("ReplayState", REPLAY_FIELDS.as_slice()),
        ("ScheduleState", SCHEDULE_FIELDS.as_slice()),
        ("OrderLifecycleState", ORDER_FIELDS.as_slice()),
        ("ValuationState", VALUATION_FIELDS.as_slice()),
        ("FundingState", FUNDING_FIELDS.as_slice()),
        ("AccountingState", ACCOUNTING_FIELDS.as_slice()),
        ("MetricState", METRIC_FIELDS.as_slice()),
    ];
    for (state, fields) in state_groups {
        assert_exact_state_fields(&state_source, state, fields);
    }
    let grouped_leaf_count = REPLAY_FIELDS.len()
        + SCHEDULE_FIELDS.len()
        + ORDER_FIELDS.len()
        + VALUATION_FIELDS.len()
        + FUNDING_FIELDS.len()
        + ACCOUNTING_FIELDS.len()
        + METRIC_FIELDS.len();
    assert_eq!(grouped_leaf_count, 64, "grouped runner leaf count");
    assert_eq!(
        4 + grouped_leaf_count,
        68,
        "four direct dependencies plus grouped state must preserve all 68 leaves",
    );

    let mut actual_leaf_fields = actual_root_fields[..DIRECT_LEAF_FIELDS.len()].to_vec();
    for (state, _) in state_groups {
        actual_leaf_fields.extend(state_field_signatures(&state_source, state));
    }
    let expected_leaf_fields = DIRECT_LEAF_FIELDS
        .iter()
        .chain(REPLAY_FIELDS.iter())
        .chain(SCHEDULE_FIELDS.iter())
        .chain(ORDER_FIELDS.iter())
        .chain(VALUATION_FIELDS.iter())
        .chain(FUNDING_FIELDS.iter())
        .chain(ACCOUNTING_FIELDS.iter())
        .chain(METRIC_FIELDS.iter())
        .copied()
        .collect::<Vec<_>>();
    assert_eq!(expected_leaf_fields.len(), 68, "enumerated leaf fields");
    assert_eq!(actual_leaf_fields.len(), 68, "declared leaf fields");
    for field in expected_leaf_fields {
        assert_eq!(
            actual_leaf_fields
                .iter()
                .filter(|actual| **actual == field)
                .count(),
            1,
            "leaf field signature `{field}` must be declared exactly once",
        );
    }

    for (path, source) in sources {
        assert!(
            source.lines().count() < 1_500,
            "{path} must stay below the 1,500-line module ceiling",
        );
        for forbidden_identifier in [
            "Arc",
            "Mutex",
            "RwLock",
            "Sender",
            "Receiver",
            "channel",
            "sync_channel",
            "spawn",
        ] {
            assert!(
                !source
                    .split(|character: char| {
                        !(character.is_ascii_alphanumeric() || character == '_')
                    })
                    .any(|identifier| identifier == forbidden_identifier),
                "{path} must not introduce `{forbidden_identifier}` ownership",
            );
        }
        let compact = source
            .chars()
            .filter(|character| !character.is_whitespace())
            .collect::<String>();
        for forbidden in [
            "Arc<",
            "Mutex<",
            "RwLock<",
            "Box<dyn",
            "Arc<dyn",
            "&dyn",
            "Sender<",
            "Receiver<",
            "channel(",
            "mpsc::",
            "spawn(",
        ] {
            assert!(
                !compact.contains(forbidden),
                "{path} must not introduce concurrent or indirect ownership via `{forbidden}`",
            );
        }
        assert!(
            !source.contains("dyn "),
            "{path} must not introduce trait-object ownership",
        );
        for state in [
            "ReplayState",
            "ScheduleState",
            "OrderLifecycleState",
            "ValuationState",
            "FundingState",
            "AccountingState",
            "MetricState",
        ] {
            assert!(
                !source.lines().any(|line| {
                    let line = line.trim_start();
                    line.starts_with("impl ") && line.contains(state)
                }),
                "{path} must not attach behavior to the `{state}` state bag",
            );
        }
        assert!(
            !source.contains("::*;"),
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
