use std::collections::{BTreeMap, BTreeSet, VecDeque};

use reap_core::{
    AccountUpdate, NewOrder, NormalizedEvent, OrderEvent, OrderIntent, OrderStatus, OrderUpdate,
    PINNED_JAVA_REVISION, SystemEvent, TimeMs, Venue,
};
use reap_engine::{ChaosEngineOutput, TradingEngine};
use reap_risk::{InstrumentOrderLimits, InstrumentRiskModel, RiskDecision, RiskGate, RiskLimits};
use reap_strategy::{ChaosConfig, ChaosExecutionPurpose, ChaosStrategy};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[path = "decision_replay/initialization_validation.rs"]
mod initialization_validation;

pub const INITIALIZATION_SCHEMA_VERSION: u32 = 1;
pub const REPLAY_SCHEMA_VERSION: u32 = 1;
pub const PROJECTION_SCHEMA_VERSION: u32 = 1;

const RISK_LIMIT_KEYS: [&str; 20] = [
    "forced_repayment_indicator_limit",
    "max_abs_position_notional_usd",
    "max_drawdown_usd",
    "max_feed_age_ms",
    "max_live_order_count",
    "max_live_order_count_per_symbol",
    "max_live_order_notional_usd",
    "max_order_notional_usd",
    "max_private_age_ms",
    "max_turnover_usd",
    "order_reject_count_limit",
    "order_reject_count_per_symbol_limit",
    "order_reject_window_ms",
    "require_feed_health",
    "require_private_health",
    "stablecoin_breach_debounce_ms",
    "stablecoin_guards",
    "stablecoin_max_age_ms",
    "unfilled_ioc_cancel_count_per_symbol_limit",
    "unfilled_ioc_cancel_window_ms",
];

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InitializationArtifactV1 {
    pub schema_version: u32,
    pub reap_commit: String,
    pub java_revision: String,
    pub strategy: ChaosConfig,
    pub risk_limits: RiskLimits,
    pub instruments: Vec<InstrumentInitialization>,
    pub accounts: Vec<AccountInitialization>,
    pub declared_state: DeclaredDecisionState,
    pub live: LiveInitialization,
    pub seed_events: Vec<SeedEvent>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstrumentInitialization {
    pub account_id: String,
    pub symbol: String,
    pub instrument_type: String,
    pub trade_mode: String,
    pub risk_model: InstrumentRiskModel,
    pub order_limits: InstrumentOrderLimits,
    pub tick_size: f64,
    pub lot_size: f64,
    pub min_size: f64,
    pub contract_value: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AccountInitialization {
    pub id: String,
    pub id_prefix: String,
    pub node_id: u16,
    pub expected_account_level: String,
    pub expected_position_mode: String,
    pub trade_modes: BTreeMap<String, String>,
    pub bootstrap_update: AccountUpdate,
    pub baseline_fill_ids: Vec<String>,
    pub quote_stp_verified: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeclaredDecisionState {
    pub kill_switch_reason: Option<String>,
    pub halted_symbols: Vec<ReasonedSymbol>,
    pub feed_health: Vec<FeedHealthState>,
    pub private_health: Vec<PrivateHealthState>,
    pub marks: Vec<NumericSymbolState>,
    pub positions: Vec<NumericSymbolState>,
    pub live_orders: Vec<OrderUpdate>,
    pub order_rejections: Vec<OrderWindowState>,
    pub rejected_order_ids: Vec<String>,
    pub last_order_rejection_ms: TimeMs,
    pub unfilled_ioc_cancellations: Vec<OrderWindowState>,
    pub unfilled_ioc_cancelled_order_ids: Vec<String>,
    pub last_unfilled_ioc_cancel_ms: TimeMs,
    pub turnover_usd: f64,
    pub equity_usd: f64,
    pub equity_by_account: Vec<AccountEquityState>,
    pub peak_equity_usd: f64,
    pub seen_fills: Vec<SeenFillState>,
    pub stablecoin_rates: Vec<StablecoinRateState>,
    pub stablecoin_missing_symbols: Vec<String>,
    pub stablecoin_breach_since: Vec<StablecoinBreachState>,
    pub strategy_references: Vec<StrategyReferenceState>,
    pub source_clock: SourceClockState,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReasonedSymbol {
    pub symbol: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FeedHealthState {
    pub venue: Venue,
    pub symbol: String,
    pub last_ready_ms: TimeMs,
    pub stale: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PrivateHealthState {
    pub venue: Venue,
    pub account_id: Option<String>,
    pub last_ready_ms: TimeMs,
    pub stale: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NumericSymbolState {
    pub symbol: String,
    pub value: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OrderWindowState {
    pub ts_ms: TimeMs,
    pub order_id: String,
    pub symbol: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AccountEquityState {
    pub account_id: Option<String>,
    pub equity_usd: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SeenFillState {
    pub order_id: String,
    pub ts_ms: TimeMs,
    pub qty_bits: u64,
    pub price_bits: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StablecoinRateState {
    pub symbol: String,
    pub ts_ms: TimeMs,
    pub price: f64,
    pub conflict: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StablecoinBreachState {
    pub symbol: String,
    pub since_ms: TimeMs,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StrategyReferenceState {
    pub kind: String,
    pub symbol: String,
    pub source_ts_ms: TimeMs,
    pub observed_now_ms: TimeMs,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceClockState {
    pub seed_now_ms: TimeMs,
    pub seed_arrival_ns: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LiveInitialization {
    pub session_id: String,
    pub storage_ready: bool,
    pub public_connectivity_ready: bool,
    pub reconciled_accounts: Vec<String>,
    pub forbidden_zero_accounts: Vec<String>,
    pub order_transport_ready_accounts: Vec<String>,
    pub gateway_action_accounts: Vec<String>,
    pub order_entry_enabled: bool,
    pub halted_accounts: Vec<ReasonedAccount>,
    pub decision_sequence: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReasonedAccount {
    pub account_id: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SeedEvent {
    pub sequence: u64,
    pub arrival_ns: u64,
    pub observed_now_ms: TimeMs,
    pub route: SeedRoute,
    pub event: NormalizedEvent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SeedRoute {
    Normalized,
    PrivateAccount,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReplayEnvelope {
    pub schema_version: u32,
    pub case: String,
    pub sequence: u64,
    pub input: ReplayInput,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ReplayInput {
    Normalized {
        receipt_ns: Option<u64>,
        observed_now_ms: TimeMs,
        event: NormalizedEvent,
    },
    PendingFeedback {
        event: OrderUpdate,
    },
    DueTradeReprice {
        now_ns: u64,
        observed_now_ms: TimeMs,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EngineProjectionRow {
    pub schema_version: u32,
    pub case: String,
    pub sequence: u64,
    pub input: ReplayInput,
    pub typed_intents: Vec<TypedIntentProjection>,
    pub rejections: Vec<RiskDecision>,
    pub system_events: Vec<SystemEvent>,
    pub safety_cancel_candidates: Vec<SafetyCancelProjection>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TypedIntentProjection {
    pub purpose: String,
    pub legacy: OrderIntent,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SafetyCancelProjection {
    pub order_id: String,
    pub reason: String,
}

pub fn parse_initialization(bytes: &[u8]) -> Result<InitializationArtifactV1, String> {
    let raw: Value =
        serde_json::from_slice(bytes).map_err(|error| format!("initialization JSON: {error}"))?;
    expect_exact_keys(
        &raw,
        "initialization",
        &[
            "accounts",
            "declared_state",
            "instruments",
            "java_revision",
            "live",
            "reap_commit",
            "risk_limits",
            "schema_version",
            "seed_events",
            "strategy",
        ],
    )?;
    let artifact: InitializationArtifactV1 = serde_json::from_value(raw.clone())
        .map_err(|error| format!("initialization schema: {error}"))?;
    require_same_shape(
        &raw,
        &serde_json::to_value(&artifact)
            .map_err(|error| format!("serialize complete initialization: {error}"))?,
        "initialization",
    )?;
    require_same_shape(
        raw.get("strategy")
            .ok_or("initialization.strategy is required")?,
        &serde_json::to_value(&artifact.strategy)
            .map_err(|error| format!("serialize effective strategy: {error}"))?,
        "initialization.strategy",
    )?;
    require_same_shape(
        raw.get("risk_limits")
            .ok_or("initialization.risk_limits is required")?,
        &serde_json::to_value(&artifact.risk_limits)
            .map_err(|error| format!("serialize effective risk limits: {error}"))?,
        "initialization.risk_limits",
    )?;
    validate_initialization(&artifact)?;
    Ok(artifact)
}

pub fn parse_replay_jsonl(bytes: &[u8]) -> Result<Vec<ReplayEnvelope>, String> {
    let text = std::str::from_utf8(bytes).map_err(|error| format!("replay UTF-8: {error}"))?;
    let mut events = Vec::new();
    let mut last_by_case = BTreeMap::<String, u64>::new();
    let mut last_case = None::<String>;
    for (line_index, line) in text.lines().enumerate() {
        if line.trim().is_empty() || line.starts_with('#') {
            continue;
        }
        let raw: Value = serde_json::from_str(line)
            .map_err(|error| format!("replay line {} JSON: {error}", line_index + 1))?;
        let event: ReplayEnvelope = serde_json::from_value(raw.clone())
            .map_err(|error| format!("replay line {} schema: {error}", line_index + 1))?;
        require_same_shape(
            &raw,
            &serde_json::to_value(&event)
                .map_err(|error| format!("serialize replay line {}: {error}", line_index + 1))?,
            &format!("replay line {}", line_index + 1),
        )?;
        if event.schema_version != REPLAY_SCHEMA_VERSION {
            return Err(format!(
                "replay line {} schema_version {}, expected {}",
                line_index + 1,
                event.schema_version,
                REPLAY_SCHEMA_VERSION
            ));
        }
        if event.case.trim().is_empty() {
            return Err(format!("replay line {} case is empty", line_index + 1));
        }
        if last_case.as_deref() != Some(event.case.as_str()) {
            if last_case
                .as_ref()
                .is_some_and(|previous| previous >= &event.case)
            {
                return Err(format!(
                    "replay line {} case {} does not follow canonical case block {:?}",
                    line_index + 1,
                    event.case,
                    last_case
                ));
            }
            last_case = Some(event.case.clone());
        }
        let previous = last_by_case.entry(event.case.clone()).or_default();
        if event.sequence != previous.saturating_add(1) {
            return Err(format!(
                "replay case {} sequence {} does not follow {}",
                event.case, event.sequence, *previous
            ));
        }
        *previous = event.sequence;
        events.push(event);
    }
    if events.is_empty() {
        return Err("replay contains no events".to_string());
    }
    Ok(events)
}

pub fn build_engine(
    artifact: &InitializationArtifactV1,
) -> Result<TradingEngine<ChaosStrategy>, String> {
    let strategy =
        ChaosStrategy::new(artifact.strategy.clone()).map_err(|error| error.to_string())?;
    let mut risk = RiskGate::new(artifact.risk_limits.clone());
    for instrument in &artifact.instruments {
        if !risk.set_instrument_model(instrument.symbol.clone(), instrument.risk_model) {
            return Err(format!("invalid risk model for {}", instrument.symbol));
        }
        if !risk.set_instrument_order_limits(instrument.symbol.clone(), instrument.order_limits) {
            return Err(format!("invalid order limits for {}", instrument.symbol));
        }
    }
    let mut engine = TradingEngine::new(strategy, risk);
    for seed in &artifact.seed_events {
        let _ = if matches!(
            seed.event,
            NormalizedEvent::Market(reap_core::MarketEvent::Depth(_))
        ) {
            engine.on_chaos_event_with_strategy_clock(
                seed.event.clone(),
                false,
                || (seed.arrival_ns, seed.observed_now_ms),
                || seed.observed_now_ms,
            )
        } else {
            engine.on_chaos_event_at(
                seed.event.clone(),
                seed.arrival_ns,
                seed.observed_now_ms,
                false,
            )
        };
    }
    Ok(engine)
}

pub fn replay_engine(
    artifact: &InitializationArtifactV1,
    replay: &[ReplayEnvelope],
) -> Result<Vec<EngineProjectionRow>, String> {
    let mut grouped = BTreeMap::<&str, Vec<&ReplayEnvelope>>::new();
    for event in replay {
        grouped.entry(event.case.as_str()).or_default().push(event);
    }
    let mut rows = Vec::new();
    for (case, events) in grouped {
        let mut engine = build_engine(artifact)?;
        let mut expected_pending = VecDeque::<ExpectedPending>::new();
        let mut next_client_ordinal = 1_u64;
        for envelope in events {
            match &envelope.input {
                ReplayInput::PendingFeedback { event } => {
                    let expected = expected_pending.pop_front().ok_or_else(|| {
                        format!(
                            "{case}/{} declares PendingFeedback without a preceding allowed submit",
                            envelope.sequence
                        )
                    })?;
                    validate_pending_feedback(
                        case,
                        envelope.sequence,
                        event,
                        &expected,
                        next_client_ordinal,
                    )?;
                    next_client_ordinal = next_client_ordinal.saturating_add(1);
                }
                ReplayInput::Normalized { .. } | ReplayInput::DueTradeReprice { .. } => {
                    if !expected_pending.is_empty() {
                        return Err(format!(
                            "{case}/{} reached a new input before {} same-turn PendingFeedback rows",
                            envelope.sequence,
                            expected_pending.len()
                        ));
                    }
                }
            }
            let output = match &envelope.input {
                ReplayInput::Normalized {
                    receipt_ns,
                    observed_now_ms,
                    event,
                } => match receipt_ns {
                    Some(receipt_ns) => {
                        engine.on_chaos_event_at(event.clone(), *receipt_ns, *observed_now_ms, true)
                    }
                    None => engine.on_chaos_event(event.clone()),
                },
                ReplayInput::PendingFeedback { event } => {
                    engine.on_chaos_event(NormalizedEvent::Order(event.clone()))
                }
                ReplayInput::DueTradeReprice {
                    now_ns,
                    observed_now_ms,
                } => {
                    let due = engine.next_chaos_trade_reprice_due_ns().ok_or_else(|| {
                        format!("{case}/{} has no pending trade reprice", envelope.sequence)
                    })?;
                    if due != *now_ns {
                        return Err(format!(
                            "{case}/{} due {due}, fixture declares {now_ns}",
                            envelope.sequence
                        ));
                    }
                    engine.service_one_due_chaos_trade_reprice(*now_ns, *observed_now_ms, true)
                }
            };
            let projected = project_engine_output(envelope, &output);
            let local_send_ms = replay_local_send_ms(&envelope.input);
            let reservation_ms = replay_reservation_ms(&envelope.input);
            for intent in output.intents {
                if matches!(
                    intent.purpose(),
                    ChaosExecutionPurpose::Quote | ChaosExecutionPurpose::Hedge
                ) {
                    let OrderIntent::NewOrder(order) = intent.to_order_intent() else {
                        return Err(format!(
                            "{case}/{} executable submit purpose did not project to NewOrder",
                            envelope.sequence
                        ));
                    };
                    engine
                        .with_locally_sent_chaos_intent(
                            intent,
                            || local_send_ms,
                            |_| Ok::<(), String>(()),
                        )
                        .map_err(|error| {
                            format!(
                                "{case}/{} local-send transition failed: {error}",
                                envelope.sequence
                            )
                        })?;
                    expected_pending.push_back(ExpectedPending {
                        order,
                        ts_ms: reservation_ms,
                    });
                }
            }
            rows.push(projected);
        }
        if !expected_pending.is_empty() {
            return Err(format!(
                "{case} ended before {} allowed submits received same-turn PendingFeedback",
                expected_pending.len()
            ));
        }
    }
    Ok(rows)
}

pub fn project_engine_output(
    envelope: &ReplayEnvelope,
    output: &ChaosEngineOutput,
) -> EngineProjectionRow {
    EngineProjectionRow {
        schema_version: PROJECTION_SCHEMA_VERSION,
        case: envelope.case.clone(),
        sequence: envelope.sequence,
        input: envelope.input.clone(),
        typed_intents: output
            .intents
            .iter()
            .map(|intent| TypedIntentProjection {
                purpose: intent.purpose().as_str().to_string(),
                legacy: intent.to_order_intent(),
            })
            .collect(),
        rejections: output.rejected.clone(),
        system_events: output.system_events.clone(),
        safety_cancel_candidates: output
            .safety_cancel_candidates
            .iter()
            .map(|candidate| SafetyCancelProjection {
                order_id: candidate.order_id().to_string(),
                reason: candidate.reason().to_string(),
            })
            .collect(),
    }
}

struct ExpectedPending {
    order: NewOrder,
    ts_ms: TimeMs,
}

fn replay_local_send_ms(input: &ReplayInput) -> TimeMs {
    match input {
        ReplayInput::Normalized {
            observed_now_ms, ..
        }
        | ReplayInput::DueTradeReprice {
            observed_now_ms, ..
        } => *observed_now_ms,
        ReplayInput::PendingFeedback { event } => event.ts_ms,
    }
}

fn replay_reservation_ms(input: &ReplayInput) -> TimeMs {
    match input {
        ReplayInput::Normalized { event, .. } => event.ts_ms(),
        ReplayInput::DueTradeReprice {
            observed_now_ms, ..
        } => *observed_now_ms,
        ReplayInput::PendingFeedback { event } => event.ts_ms,
    }
}

fn validate_pending_feedback(
    case: &str,
    sequence: u64,
    actual: &OrderUpdate,
    expected: &ExpectedPending,
    client_ordinal: u64,
) -> Result<(), String> {
    let expected_id = format!("client#{client_ordinal}");
    let expected_reason = if expected.order.reason.is_empty() {
        "pending_new".to_string()
    } else {
        format!("{}:pending_new", expected.order.reason)
    };
    let exact = actual.ts_ms == expected.ts_ms
        && actual.order_id == expected_id
        && actual.symbol == expected.order.symbol
        && actual.side == expected.order.side
        && actual.event == OrderEvent::PendingNew
        && actual.status == OrderStatus::PendingNew
        && actual.price.to_bits() == expected.order.price.to_bits()
        && actual.time_in_force == Some(expected.order.time_in_force)
        && actual.qty.to_bits() == expected.order.qty.to_bits()
        && actual.open_qty.to_bits() == expected.order.qty.to_bits()
        && actual.filled_qty.to_bits() == 0.0_f64.to_bits()
        && actual.avg_fill_price.to_bits() == 0.0_f64.to_bits()
        && actual.last_fill_qty.to_bits() == 0.0_f64.to_bits()
        && actual.last_fill_price.to_bits() == 0.0_f64.to_bits()
        && actual.last_fill_liquidity.is_none()
        && actual.last_fill_fee.is_none()
        && actual.reason == expected_reason;
    if exact {
        Ok(())
    } else {
        Err(format!(
            "{case}/{sequence} PendingFeedback is not the exact same-turn reservation update; expected id={expected_id} ts_ms={} order={:?} reason={expected_reason}, actual={actual:?}",
            expected.ts_ms, expected.order
        ))
    }
}

pub fn canonical_jsonl<T: Serialize>(rows: &[T]) -> Result<String, String> {
    let mut output = String::new();
    for row in rows {
        output.push_str(
            &serde_json::to_string(row)
                .map_err(|error| format!("serialize canonical JSONL row: {error}"))?,
        );
        output.push('\n');
    }
    Ok(output)
}

fn validate_initialization(artifact: &InitializationArtifactV1) -> Result<(), String> {
    if artifact.schema_version != INITIALIZATION_SCHEMA_VERSION {
        return Err(format!(
            "initialization schema_version {}, expected {}",
            artifact.schema_version, INITIALIZATION_SCHEMA_VERSION
        ));
    }
    if artifact.java_revision != PINNED_JAVA_REVISION {
        return Err(format!(
            "initialization Java revision {}, expected {}",
            artifact.java_revision, PINNED_JAVA_REVISION
        ));
    }
    if artifact.reap_commit.len() != 40
        || !artifact
            .reap_commit
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err("initialization reap_commit must be a full lowercase SHA".to_string());
    }
    let validation = artifact.strategy.validate();
    if !validation.valid {
        return Err(format!(
            "initialization strategy is invalid: {}",
            validation.errors.join("; ")
        ));
    }
    let declared_strategy = serde_json::to_vec(&artifact.strategy)
        .map_err(|error| format!("serialize declared strategy: {error}"))?;
    let effective_strategy = serde_json::to_vec(&artifact.strategy.effective())
        .map_err(|error| format!("serialize effective strategy: {error}"))?;
    if declared_strategy != effective_strategy {
        return Err(
            "initialization strategy must already contain its complete effective configuration"
                .to_string(),
        );
    }
    if let Some(error) = artifact.risk_limits.validation_error() {
        return Err(format!("initialization risk limits are invalid: {error}"));
    }
    validate_finite_state(artifact)?;

    let strategy_symbols = artifact
        .strategy
        .instruments
        .iter()
        .map(|instrument| instrument.symbol.as_str())
        .collect::<BTreeSet<_>>();
    let mut initialized_symbols = BTreeSet::new();
    for instrument in &artifact.instruments {
        if !initialized_symbols.insert(instrument.symbol.as_str()) {
            return Err(format!(
                "duplicate initialized instrument {}",
                instrument.symbol
            ));
        }
        if !strategy_symbols.contains(instrument.symbol.as_str()) {
            return Err(format!(
                "initialized instrument {} is not executable",
                instrument.symbol
            ));
        }
        if !instrument.risk_model.is_valid() || !instrument.order_limits.is_valid() {
            return Err(format!(
                "initialized instrument {} has invalid risk metadata",
                instrument.symbol
            ));
        }
        if instrument.account_id.trim().is_empty()
            || instrument.instrument_type.trim().is_empty()
            || instrument.trade_mode.trim().is_empty()
            || !positive_finite(instrument.tick_size)
            || !positive_finite(instrument.lot_size)
            || !positive_finite(instrument.min_size)
            || instrument
                .contract_value
                .is_some_and(|value| !positive_finite(value))
        {
            return Err(format!(
                "initialized instrument {} has incomplete live metadata",
                instrument.symbol
            ));
        }
    }
    if initialized_symbols != strategy_symbols {
        let missing = strategy_symbols
            .difference(&initialized_symbols)
            .copied()
            .collect::<Vec<_>>();
        return Err(format!(
            "missing instrument model/order-limit rows: {}",
            missing.join(",")
        ));
    }

    let mut account_ids = BTreeSet::new();
    for account in &artifact.accounts {
        if account.id.trim().is_empty() || !account_ids.insert(account.id.as_str()) {
            return Err(format!(
                "duplicate or empty initialized account {}",
                account.id
            ));
        }
        if account.id_prefix.trim().is_empty()
            || account.node_id == 0
            || account.expected_account_level.trim().is_empty()
            || account.expected_position_mode.trim().is_empty()
        {
            return Err(format!(
                "initialized account {} has incomplete live identity",
                account.id
            ));
        }
        let instrument_symbols = artifact
            .instruments
            .iter()
            .filter(|instrument| instrument.account_id == account.id)
            .map(|instrument| instrument.symbol.as_str())
            .collect::<BTreeSet<_>>();
        let trade_mode_symbols = account
            .trade_modes
            .keys()
            .map(String::as_str)
            .collect::<BTreeSet<_>>();
        if instrument_symbols != trade_mode_symbols {
            return Err(format!(
                "account {} trade-mode coverage differs from instruments",
                account.id
            ));
        }
    }
    if account_ids.is_empty() {
        return Err("initialization has no accounts".to_string());
    }
    for instrument in &artifact.instruments {
        if !account_ids.contains(instrument.account_id.as_str()) {
            return Err(format!(
                "instrument {} references unknown account {}",
                instrument.symbol, instrument.account_id
            ));
        }
    }

    require_unique_symbols(
        "feed_health",
        artifact
            .declared_state
            .feed_health
            .iter()
            .map(|row| row.symbol.as_str()),
    )?;
    let feed_symbols = artifact
        .declared_state
        .feed_health
        .iter()
        .map(|row| row.symbol.as_str())
        .collect::<BTreeSet<_>>();
    if feed_symbols != strategy_symbols {
        return Err("feed-health coverage must equal executable symbols".to_string());
    }
    require_unique_symbols(
        "marks",
        artifact
            .declared_state
            .marks
            .iter()
            .map(|row| row.symbol.as_str()),
    )?;
    let mark_symbols = artifact
        .declared_state
        .marks
        .iter()
        .map(|row| row.symbol.as_str())
        .collect::<BTreeSet<_>>();
    if mark_symbols != strategy_symbols {
        return Err("risk-mark coverage must equal executable symbols".to_string());
    }
    require_unique_symbols(
        "positions",
        artifact
            .declared_state
            .positions
            .iter()
            .map(|row| row.symbol.as_str()),
    )?;
    let position_symbols = artifact
        .declared_state
        .positions
        .iter()
        .map(|row| row.symbol.as_str())
        .collect::<BTreeSet<_>>();
    if position_symbols != strategy_symbols {
        return Err("position coverage must equal executable symbols".to_string());
    }

    let private_accounts = artifact
        .declared_state
        .private_health
        .iter()
        .filter_map(|row| row.account_id.as_deref())
        .collect::<BTreeSet<_>>();
    if private_accounts != account_ids {
        return Err("private-health coverage must equal initialized accounts".to_string());
    }
    let equity_accounts = artifact
        .declared_state
        .equity_by_account
        .iter()
        .filter_map(|row| row.account_id.as_deref())
        .collect::<BTreeSet<_>>();
    if equity_accounts != account_ids {
        return Err("account-equity coverage must equal initialized accounts".to_string());
    }

    let guarded_symbols = artifact
        .risk_limits
        .stablecoin_guards
        .iter()
        .map(|guard| guard.symbol.as_str())
        .collect::<BTreeSet<_>>();
    let present_stablecoins = artifact
        .declared_state
        .stablecoin_rates
        .iter()
        .map(|rate| rate.symbol.as_str())
        .collect::<BTreeSet<_>>();
    let missing_stablecoins = artifact
        .declared_state
        .stablecoin_missing_symbols
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    require_unique_symbols(
        "stablecoin_missing_symbols",
        artifact
            .declared_state
            .stablecoin_missing_symbols
            .iter()
            .map(String::as_str),
    )?;
    if !present_stablecoins.is_disjoint(&missing_stablecoins)
        || present_stablecoins
            .union(&missing_stablecoins)
            .copied()
            .collect::<BTreeSet<_>>()
            != guarded_symbols
    {
        return Err("stablecoin present/missing rows must partition configured guards".to_string());
    }

    if artifact.live.session_id.trim().is_empty() {
        return Err("live session_id is empty".to_string());
    }
    for (name, rows) in [
        (
            "reconciled_accounts",
            artifact.live.reconciled_accounts.as_slice(),
        ),
        (
            "forbidden_zero_accounts",
            artifact.live.forbidden_zero_accounts.as_slice(),
        ),
        (
            "order_transport_ready_accounts",
            artifact.live.order_transport_ready_accounts.as_slice(),
        ),
        (
            "gateway_action_accounts",
            artifact.live.gateway_action_accounts.as_slice(),
        ),
    ] {
        let row_set = rows.iter().map(String::as_str).collect::<BTreeSet<_>>();
        if row_set != account_ids || row_set.len() != rows.len() {
            return Err(format!(
                "live {name} must contain every account exactly once"
            ));
        }
    }
    if !artifact.live.storage_ready
        || !artifact.live.public_connectivity_ready
        || !artifact.live.order_entry_enabled
        || artifact.live.decision_sequence != 0
        || !artifact.live.halted_accounts.is_empty()
    {
        return Err("live readiness must declare a clean enabled genesis".to_string());
    }

    let mut previous = 0;
    for seed in &artifact.seed_events {
        if seed.sequence != previous + 1 {
            return Err(format!(
                "seed sequence {} does not follow {}",
                seed.sequence, previous
            ));
        }
        previous = seed.sequence;
    }
    if artifact.seed_events.is_empty() {
        return Err("initialization has no seed events".to_string());
    }
    if artifact.declared_state.source_clock.seed_arrival_ns
        < artifact
            .seed_events
            .last()
            .expect("checked nonempty seed events")
            .arrival_ns
    {
        return Err("declared seed arrival clock precedes the final seed".to_string());
    }
    initialization_validation::validate_seeded_clean_profile(artifact)?;
    Ok(())
}

fn validate_finite_state(artifact: &InitializationArtifactV1) -> Result<(), String> {
    for (name, value) in [
        ("turnover_usd", artifact.declared_state.turnover_usd),
        ("equity_usd", artifact.declared_state.equity_usd),
        ("peak_equity_usd", artifact.declared_state.peak_equity_usd),
    ] {
        if !value.is_finite() {
            return Err(format!("declared state {name} is non-finite"));
        }
    }
    for row in artifact
        .declared_state
        .marks
        .iter()
        .chain(&artifact.declared_state.positions)
    {
        if !row.value.is_finite() {
            return Err(format!("declared state {} is non-finite", row.symbol));
        }
    }
    for row in &artifact.declared_state.equity_by_account {
        if !row.equity_usd.is_finite() {
            return Err("declared account equity is non-finite".to_string());
        }
    }
    for row in &artifact.declared_state.stablecoin_rates {
        if !row.price.is_finite() {
            return Err(format!("declared stablecoin {} is non-finite", row.symbol));
        }
    }
    Ok(())
}

fn expect_exact_keys(value: &Value, path: &str, expected: &[&str]) -> Result<(), String> {
    let object = value
        .as_object()
        .ok_or_else(|| format!("{path} must be an object"))?;
    let actual = object.keys().map(String::as_str).collect::<BTreeSet<_>>();
    let expected = expected.iter().copied().collect::<BTreeSet<_>>();
    if actual != expected {
        let missing = expected.difference(&actual).copied().collect::<Vec<_>>();
        let unknown = actual.difference(&expected).copied().collect::<Vec<_>>();
        return Err(format!(
            "{path} keys differ; missing=[{}] unknown=[{}]",
            missing.join(","),
            unknown.join(",")
        ));
    }
    Ok(())
}

fn require_same_shape(actual: &Value, complete: &Value, path: &str) -> Result<(), String> {
    match (actual, complete) {
        (Value::Object(actual), Value::Object(complete)) => {
            let actual_keys = actual.keys().map(String::as_str).collect::<BTreeSet<_>>();
            let complete_keys = complete.keys().map(String::as_str).collect::<BTreeSet<_>>();
            if actual_keys != complete_keys {
                let missing = complete_keys
                    .difference(&actual_keys)
                    .copied()
                    .collect::<Vec<_>>();
                let unknown = actual_keys
                    .difference(&complete_keys)
                    .copied()
                    .collect::<Vec<_>>();
                return Err(format!(
                    "{path} shape differs; missing=[{}] unknown=[{}]",
                    missing.join(","),
                    unknown.join(",")
                ));
            }
            for key in complete_keys {
                require_same_shape(
                    actual.get(key).expect("checked key"),
                    complete.get(key).expect("checked key"),
                    &format!("{path}.{key}"),
                )?;
            }
            Ok(())
        }
        (Value::Array(actual), Value::Array(complete)) => {
            if actual.len() != complete.len() {
                return Err(format!(
                    "{path} array length {}, expected {}",
                    actual.len(),
                    complete.len()
                ));
            }
            for (index, (actual, complete)) in actual.iter().zip(complete).enumerate() {
                require_same_shape(actual, complete, &format!("{path}[{index}]"))?;
            }
            Ok(())
        }
        (Value::Null, Value::Null)
        | (Value::Bool(_), Value::Bool(_))
        | (Value::Number(_), Value::Number(_))
        | (Value::String(_), Value::String(_)) => Ok(()),
        _ => Err(format!("{path} value kind differs from complete schema")),
    }
}

fn require_unique_symbols<'a>(
    name: &str,
    symbols: impl Iterator<Item = &'a str>,
) -> Result<(), String> {
    let mut seen = BTreeSet::new();
    for symbol in symbols {
        if symbol.trim().is_empty() || !seen.insert(symbol) {
            return Err(format!(
                "{name} contains duplicate or empty symbol {symbol}"
            ));
        }
    }
    Ok(())
}

fn positive_finite(value: f64) -> bool {
    value.is_finite() && value > 0.0
}

pub fn risk_limit_keys() -> &'static [&'static str] {
    &RISK_LIMIT_KEYS
}
