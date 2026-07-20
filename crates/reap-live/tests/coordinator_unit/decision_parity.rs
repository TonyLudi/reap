#[allow(dead_code)]
#[path = "../../../reap-engine/tests/support/decision_replay.rs"]
mod decision_replay_support;

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::path::PathBuf;

use decision_replay_support::{
    EngineProjectionRow, InitializationArtifactV1, ReplayEnvelope, ReplayInput,
    SafetyCancelProjection, SeedRoute, TypedIntentProjection, canonical_jsonl,
    parse_initialization, parse_replay_jsonl, replay_engine,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use super::*;

const LIVE_PROJECTION_SCHEMA_VERSION: u32 = 1;
const LIVE_PROJECTION_NAME: &str = "live_reduction_v1";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct LiveReductionManifest {
    schema_version: u32,
    projection: String,
    rows: usize,
    bytes: usize,
    sha256: String,
}

#[derive(Debug, Serialize)]
struct LiveReductionProjection {
    schema_version: u32,
    case: String,
    sequence: u64,
    input: ReplayInput,
    delivery: &'static str,
    origin_sequence: Option<u64>,
    decision: EngineProjectionRow,
    records: Vec<Value>,
    actions: Vec<Value>,
    same_turn_pending: Vec<Value>,
    implicit_pending: Option<Value>,
}

struct DecisionParts {
    typed_intents: Vec<(reap_strategy::ChaosExecutionPurpose, OrderIntent)>,
    rejections: Vec<RiskDecision>,
    system_events: Vec<SystemEvent>,
    safety_cancel_candidates: Vec<(String, String)>,
}

struct QueuedImplicitPending {
    origin_sequence: u64,
    decision: DecisionParts,
    update: OrderUpdate,
    evidence: Value,
}

#[derive(Default)]
struct AlphaClientIds {
    actual_to_alpha: BTreeMap<String, String>,
    alpha_to_actual: BTreeMap<String, String>,
}

impl AlphaClientIds {
    fn bind(&mut self, actual: &str) -> String {
        if let Some(alpha) = self.actual_to_alpha.get(actual) {
            return alpha.clone();
        }
        let alpha = format!("client#{}", self.actual_to_alpha.len() + 1);
        assert!(
            self.alpha_to_actual
                .insert(alpha.clone(), actual.to_string())
                .is_none(),
            "fresh alpha client id must be unique"
        );
        assert!(
            self.actual_to_alpha
                .insert(actual.to_string(), alpha.clone())
                .is_none(),
            "fresh actual client id must be unique"
        );
        alpha
    }

    fn resolve(&self, logical: &str) -> String {
        self.alpha_to_actual
            .get(logical)
            .cloned()
            .unwrap_or_else(|| logical.to_string())
    }

    fn normalize<T: Serialize>(&self, value: &T) -> Result<Value, String> {
        let mut value = serde_json::to_value(value)
            .map_err(|error| format!("serialize projection: {error}"))?;
        self.normalize_value(&mut value);
        Ok(value)
    }

    fn normalize_value(&self, value: &mut Value) {
        match value {
            Value::String(string) => {
                if let Some(alpha) = self.actual_to_alpha.get(string) {
                    *string = alpha.clone();
                }
            }
            Value::Array(values) => {
                for value in values {
                    self.normalize_value(value);
                }
            }
            Value::Object(values) => {
                for value in values.values_mut() {
                    self.normalize_value(value);
                }
            }
            Value::Null | Value::Bool(_) | Value::Number(_) => {}
        }
    }
}

fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/decision_parity")
        .join(name)
}

fn fixture_bytes(name: &str) -> Vec<u8> {
    std::fs::read(fixture_path(name))
        .unwrap_or_else(|error| panic!("read decision parity fixture {name}: {error}"))
}

fn parse_enum<T>(value: &str, field: &str) -> Result<T, String>
where
    T: serde::de::DeserializeOwned,
{
    serde_json::from_value(Value::String(value.to_string()))
        .map_err(|error| format!("{field} value {value}: {error}"))
}

fn build_live_coordinator(artifact: &InitializationArtifactV1) -> Result<LiveCoordinator, String> {
    let accounts = artifact
        .accounts
        .iter()
        .map(|account| {
            let trade_modes = account
                .trade_modes
                .iter()
                .map(|(symbol, mode)| {
                    Ok((
                        symbol.clone(),
                        parse_enum::<OkxTradeModeConfig>(mode, "trade_mode")?,
                    ))
                })
                .collect::<Result<HashMap<_, _>, String>>()?;
            Ok(LiveAccountConfig {
                id: account.id.clone(),
                api_key_env: format!("DECISION_PARITY_{}_KEY", account.id),
                secret_key_env: format!("DECISION_PARITY_{}_SECRET", account.id),
                passphrase_env: format!("DECISION_PARITY_{}_PASSPHRASE", account.id),
                expected_account_level: parse_enum(
                    &account.expected_account_level,
                    "expected_account_level",
                )?,
                expected_position_mode: parse_enum(
                    &account.expected_position_mode,
                    "expected_position_mode",
                )?,
                api_key_policy: crate::OkxApiKeyPolicyConfig::default(),
                id_prefix: account.id_prefix.clone(),
                node_id: account.node_id,
                trade_modes,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    let config = LiveConfig {
        strategy: artifact.strategy.clone(),
        risk: artifact.risk_limits.clone(),
        venue: OkxVenueConfig::default(),
        runtime: RuntimeConfig::default(),
        storage: LiveStorageConfig::default(),
        operator: crate::OperatorConfig::default(),
        alerts: crate::AlertConfig::default(),
        host_guard: crate::HostGuardConfig::default(),
        accounts,
    };

    let instruments = artifact
        .instruments
        .iter()
        .map(|instrument| {
            let instrument_type = match instrument.instrument_type.as_str() {
                "spot" => OkxInstrumentType::Spot,
                "margin" => OkxInstrumentType::Margin,
                "swap" => OkxInstrumentType::Swap,
                "futures" => OkxInstrumentType::Futures,
                "option" => OkxInstrumentType::Option,
                other => {
                    return Err(format!(
                        "instrument {} has unsupported type {other}",
                        instrument.symbol
                    ));
                }
            };
            let tick_size = instrument.tick_size.to_string();
            let lot_size = instrument.lot_size.to_string();
            let min_size = instrument.min_size.to_string();
            let regular_order_rules =
                reap_venue::okx::OkxRegularOrderRules::from_exchange_decimals(
                    &tick_size, &lot_size, &min_size,
                )
                .map_err(|error| {
                    format!(
                        "instrument {} has invalid exact order rules: {error}",
                        instrument.symbol
                    )
                })?;
            Ok((
                instrument.symbol.clone(),
                VerifiedInstrument::new(
                    instrument.account_id.clone(),
                    instrument.symbol.clone(),
                    instrument_type,
                    parse_enum(&instrument.trade_mode, "instrument.trade_mode")?,
                    instrument.risk_model,
                    instrument.order_limits,
                    instrument.tick_size,
                    instrument.lot_size,
                    instrument.min_size,
                    instrument.contract_value,
                    regular_order_rules,
                ),
            ))
        })
        .collect::<Result<HashMap<_, _>, String>>()?;
    let account_updates = artifact
        .accounts
        .iter()
        .map(|account| (account.id.clone(), account.bootstrap_update.clone()))
        .collect();
    let mut baseline_fill_ids = HashMap::new();
    for account in &artifact.accounts {
        if !account.baseline_fill_ids.is_empty() {
            return Err(format!(
                "decision replay account {} requires structured baseline fill keys",
                account.id
            ));
        }
        baseline_fill_ids.insert(account.id.clone(), HashSet::new());
    }
    let verified = VerifiedBootstrap {
        instruments,
        account_updates,
        baseline_fill_ids,
        quote_stp_verified_accounts: artifact
            .accounts
            .iter()
            .filter(|account| account.quote_stp_verified)
            .map(|account| account.id.clone())
            .collect(),
    };
    let scopes = approval_scopes(
        artifact
            .live
            .gateway_action_accounts
            .iter()
            .map(String::as_str),
    );
    let mut coordinator =
        LiveCoordinator::new(config, verified, scopes, artifact.live.session_id.clone())
            .map_err(|error| error.to_string())?;
    coordinator.set_order_entry_enabled(false);

    for seed in &artifact.seed_events {
        match seed.route {
            SeedRoute::Normalized => {
                coordinator
                    .process_feed_arrived_at(
                        FeedOutput::Event(seed.event.clone()),
                        seed.observed_now_ms,
                        seed.arrival_ns,
                    )
                    .map_err(|error| error.to_string())?;
            }
            SeedRoute::PrivateAccount => {
                let NormalizedEvent::Account(update) = &seed.event else {
                    return Err(format!(
                        "private-account seed {} is not an account event",
                        seed.sequence
                    ));
                };
                let account_ids = update
                    .balances
                    .iter()
                    .filter_map(|balance| balance.account_id.as_deref())
                    .chain(
                        update
                            .margins
                            .iter()
                            .filter_map(|margin| margin.account_id.as_deref()),
                    )
                    .collect::<HashSet<_>>();
                if account_ids.len() != 1 {
                    return Err(format!(
                        "private-account seed {} does not identify exactly one account",
                        seed.sequence
                    ));
                }
                coordinator
                    .apply_authoritative_account_snapshot(
                        account_ids.into_iter().next().expect("checked one account"),
                        update.clone(),
                    )
                    .map_err(|error| error.to_string())?;
            }
        }
    }

    coordinator.mark_storage_ready(
        artifact.live.storage_ready,
        "decision parity initialization artifact",
    );
    coordinator.mark_public_connectivity(
        artifact.live.public_connectivity_ready,
        "decision parity initialization artifact",
    );
    for account_id in &artifact.live.reconciled_accounts {
        coordinator
            .on_reconciliation(ReconciliationResult {
                account_id: account_id.clone(),
                ts_ms: artifact.declared_state.source_clock.seed_now_ms,
                clean: true,
                local_live_orders: 0,
                remote_live_orders: 0,
                remote_recent_fills: 0,
                reason: "decision parity initialized clean reconciliation".to_string(),
            })
            .map_err(|error| error.to_string())?;
    }
    for account_id in &artifact.live.forbidden_zero_accounts {
        coordinator
            .startup
            .mark_forbidden_order_proof(
                account_id,
                true,
                "decision parity initialized complete zero proof",
            )
            .map_err(|error| error.to_string())?;
    }
    coordinator.set_order_entry_enabled(artifact.live.order_entry_enabled);
    if coordinator.decision_sequence != artifact.live.decision_sequence {
        return Err(format!(
            "live decision sequence {} differs from declared {}",
            coordinator.decision_sequence, artifact.live.decision_sequence
        ));
    }
    if !coordinator.readiness().is_ready() {
        return Err(format!(
            "live initialization is not ready: {:?}",
            coordinator.readiness()
        ));
    }
    coordinator.chaos_decision_trace.clear();
    coordinator.chaos_intent_trace.clear();
    Ok(coordinator)
}

fn resolve_event(event: &NormalizedEvent, ids: &AlphaClientIds) -> NormalizedEvent {
    let mut event = event.clone();
    if let NormalizedEvent::Order(update) = &mut event {
        update.order_id = ids.resolve(&update.order_id);
    }
    event
}

fn take_decision_parts(coordinator: &mut LiveCoordinator) -> Vec<DecisionParts> {
    std::mem::take(&mut coordinator.chaos_decision_trace)
        .into_iter()
        .map(|batch| DecisionParts {
            typed_intents: batch.typed_intents,
            rejections: batch.rejected,
            system_events: batch.system_events,
            safety_cancel_candidates: batch.safety_cancel_candidates,
        })
        .collect()
}

fn project_decision(
    envelope: &ReplayEnvelope,
    parts: DecisionParts,
    ids: &AlphaClientIds,
) -> Result<EngineProjectionRow, String> {
    let row = EngineProjectionRow {
        schema_version: decision_replay_support::PROJECTION_SCHEMA_VERSION,
        case: envelope.case.clone(),
        sequence: envelope.sequence,
        input: envelope.input.clone(),
        typed_intents: parts
            .typed_intents
            .into_iter()
            .map(|(purpose, legacy)| TypedIntentProjection {
                purpose: purpose.as_str().to_string(),
                legacy,
            })
            .collect(),
        rejections: parts.rejections,
        system_events: parts.system_events,
        safety_cancel_candidates: parts
            .safety_cancel_candidates
            .into_iter()
            .map(|(order_id, reason)| SafetyCancelProjection { order_id, reason })
            .collect(),
    };
    serde_json::from_value(ids.normalize(&row)?)
        .map_err(|error| format!("deserialize normalized decision row: {error}"))
}

fn bind_submit_ids(output: &CoordinatorOutput, ids: &mut AlphaClientIds) {
    for action in &output.actions {
        if let LiveAction::Submit(submit) = action {
            ids.bind(submit.client_order_id());
        }
    }
}

fn project_actions(output: &CoordinatorOutput, ids: &AlphaClientIds) -> Result<Vec<Value>, String> {
    output
        .actions
        .iter()
        .map(|action| {
            let value = match action {
                LiveAction::Submit(submit) => json!({
                    "kind": "submit",
                    "ts_ms": submit.ts_ms(),
                    "account_id": submit.account_id(),
                    "idempotency_key": submit.idempotency_key(),
                    "client_order_id": submit.client_order_id(),
                    "order": submit.order(),
                }),
                LiveAction::Cancel(cancel) => json!({
                    "kind": "cancel",
                    "ts_ms": cancel.ts_ms(),
                    "account_id": cancel.account_id(),
                    "symbol": cancel.symbol(),
                    "client_order_id": cancel.client_order_id(),
                    "reason": cancel.reason(),
                }),
                LiveAction::RecoverBook(request) => json!({
                    "kind": "recover_book",
                    "stream": {
                        "venue": request.stream.venue,
                        "channel": request.stream.channel,
                        "symbol": request.stream.symbol,
                    },
                    "source_conn_id": request.source_conn_id,
                    "expected_prev": request.expected_prev,
                    "received_prev": request.received_prev,
                    "received_seq": request.received_seq,
                }),
                LiveAction::Reconcile(reconcile) => json!({
                    "kind": "reconcile",
                    "ts_ms": reconcile.ts_ms,
                    "account_id": reconcile.account_id,
                    "reason": reconcile.reason,
                }),
            };
            let mut value = value;
            ids.normalize_value(&mut value);
            Ok(value)
        })
        .collect()
}

fn project_records(output: &CoordinatorOutput, ids: &AlphaClientIds) -> Result<Vec<Value>, String> {
    output
        .records
        .iter()
        .map(|record| ids.normalize(record))
        .collect()
}

fn same_turn_pending(
    coordinator: &LiveCoordinator,
    output: &CoordinatorOutput,
    ids: &AlphaClientIds,
    origin_sequence: u64,
) -> Result<Vec<(OrderUpdate, Value)>, String> {
    output
        .actions
        .iter()
        .enumerate()
        .filter_map(|(action_index, action)| match action {
            LiveAction::Submit(submit) => Some((action_index, submit)),
            LiveAction::Cancel(_) | LiveAction::RecoverBook(_) | LiveAction::Reconcile(_) => None,
        })
        .map(|(action_index, submit)| {
            let private_order = coordinator
                .private_state(submit.account_id())
                .and_then(|state| state.order_reducer().get(submit.client_order_id()))
                .cloned()
                .ok_or_else(|| {
                    format!(
                        "submit {} is absent from canonical private state",
                        submit.client_order_id()
                    )
                })?;
            if private_order.status != OrderStatus::PendingNew {
                return Err(format!(
                    "submit {} has same-turn status {:?}, expected PendingNew",
                    submit.client_order_id(),
                    private_order.status
                ));
            }
            let (record_index, update) = output
                .records
                .iter()
                .enumerate()
                .find_map(|(record_index, record)| match record {
                    StorageRecord::Order {
                        account_id: Some(account_id),
                        update,
                    } if account_id == submit.account_id()
                        && update.order_id == submit.client_order_id()
                        && update.status == OrderStatus::PendingNew =>
                    {
                        Some((record_index, update.clone()))
                    }
                    _ => None,
                })
                .ok_or_else(|| {
                    format!(
                        "submit {} has no same-turn PendingNew storage record",
                        submit.client_order_id()
                    )
                })?;
            let evidence = json!({
                "origin_sequence": origin_sequence,
                "action_index": action_index,
                "record_index": record_index,
                "account_id": submit.account_id(),
                "client_order_id": submit.client_order_id(),
                "update": update,
                "private_order": private_order,
            });
            let mut evidence = evidence;
            ids.normalize_value(&mut evidence);
            Ok((update, evidence))
        })
        .collect()
}

fn run_live_replay(
    artifact: &InitializationArtifactV1,
    replay: &[ReplayEnvelope],
) -> Result<(Vec<EngineProjectionRow>, Vec<LiveReductionProjection>), String> {
    let mut grouped = BTreeMap::<&str, Vec<&ReplayEnvelope>>::new();
    for envelope in replay {
        grouped
            .entry(envelope.case.as_str())
            .or_default()
            .push(envelope);
    }

    let mut decisions = Vec::new();
    let mut reductions = Vec::new();
    for (case, envelopes) in grouped {
        let mut coordinator = build_live_coordinator(artifact)?;
        let mut ids = AlphaClientIds::default();
        let mut implicit = VecDeque::<QueuedImplicitPending>::new();

        for envelope in envelopes {
            match &envelope.input {
                ReplayInput::PendingFeedback { event } => {
                    let queued = implicit.pop_front().ok_or_else(|| {
                        format!(
                            "{case}/{} has no queued same-turn PendingNew batch",
                            envelope.sequence
                        )
                    })?;
                    let normalized_update = ids.normalize(&queued.update)?;
                    let declared_update = serde_json::to_value(event)
                        .map_err(|error| format!("serialize pending feedback: {error}"))?;
                    if normalized_update != declared_update {
                        return Err(format!(
                            "{case}/{} PendingFeedback differs from the genuine same-turn update; actual={} declared={}",
                            envelope.sequence, normalized_update, declared_update
                        ));
                    }
                    let decision = project_decision(envelope, queued.decision, &ids)?;
                    decisions.push(decision.clone());
                    reductions.push(LiveReductionProjection {
                        schema_version: LIVE_PROJECTION_SCHEMA_VERSION,
                        case: envelope.case.clone(),
                        sequence: envelope.sequence,
                        input: envelope.input.clone(),
                        delivery: "implicit_same_turn",
                        origin_sequence: Some(queued.origin_sequence),
                        decision,
                        records: Vec::new(),
                        actions: Vec::new(),
                        same_turn_pending: Vec::new(),
                        implicit_pending: Some(queued.evidence),
                    });
                }
                ReplayInput::Normalized {
                    receipt_ns,
                    observed_now_ms,
                    event,
                } => {
                    if !implicit.is_empty() {
                        return Err(format!(
                            "{case}/{} reached a new input before consuming every PendingFeedback",
                            envelope.sequence
                        ));
                    }
                    let event = resolve_event(event, &ids);
                    let output = match receipt_ns {
                        Some(receipt_ns) => coordinator
                            .process_feed_arrived_at(
                                FeedOutput::Event(event),
                                *observed_now_ms,
                                *receipt_ns,
                            )
                            .map_err(|error| error.to_string())?,
                        None => coordinator.process_event(event),
                    };
                    project_actual_reduction(
                        &mut coordinator,
                        envelope,
                        output,
                        &mut ids,
                        &mut implicit,
                        &mut decisions,
                        &mut reductions,
                    )?;
                }
                ReplayInput::DueTradeReprice {
                    now_ns,
                    observed_now_ms,
                } => {
                    if !implicit.is_empty() {
                        return Err(format!(
                            "{case}/{} reached a due action before consuming every PendingFeedback",
                            envelope.sequence
                        ));
                    }
                    let due = coordinator.next_trade_reprice_due_ns().ok_or_else(|| {
                        format!("{case}/{} has no pending trade reprice", envelope.sequence)
                    })?;
                    if due != *now_ns {
                        return Err(format!(
                            "{case}/{} live due {due}, fixture declares {now_ns}",
                            envelope.sequence
                        ));
                    }
                    let output =
                        coordinator.service_one_due_trade_reprice(*now_ns, *observed_now_ms);
                    project_actual_reduction(
                        &mut coordinator,
                        envelope,
                        output,
                        &mut ids,
                        &mut implicit,
                        &mut decisions,
                        &mut reductions,
                    )?;
                }
            }
        }
        if !implicit.is_empty() {
            return Err(format!(
                "{case} ended with {} unconsumed same-turn PendingFeedback batches",
                implicit.len()
            ));
        }
    }
    Ok((decisions, reductions))
}

#[allow(clippy::too_many_arguments)]
fn project_actual_reduction(
    coordinator: &mut LiveCoordinator,
    envelope: &ReplayEnvelope,
    output: CoordinatorOutput,
    ids: &mut AlphaClientIds,
    implicit: &mut VecDeque<QueuedImplicitPending>,
    decisions: &mut Vec<EngineProjectionRow>,
    reductions: &mut Vec<LiveReductionProjection>,
) -> Result<(), String> {
    bind_submit_ids(&output, ids);
    let pending = same_turn_pending(coordinator, &output, ids, envelope.sequence)?;
    let mut traces = take_decision_parts(coordinator).into_iter();
    let outer = traces.next().ok_or_else(|| {
        format!(
            "{}/{} produced no coordinator decision trace",
            envelope.case, envelope.sequence
        )
    })?;
    let nested = traces.collect::<Vec<_>>();
    if nested.len() != pending.len() {
        return Err(format!(
            "{}/{} produced {} implicit decision batches for {} genuine reservations",
            envelope.case,
            envelope.sequence,
            nested.len(),
            pending.len()
        ));
    }
    for (decision, (update, evidence)) in nested.into_iter().zip(pending.iter()) {
        implicit.push_back(QueuedImplicitPending {
            origin_sequence: envelope.sequence,
            decision,
            update: update.clone(),
            evidence: evidence.clone(),
        });
    }

    let decision = project_decision(envelope, outer, ids)?;
    decisions.push(decision.clone());
    reductions.push(LiveReductionProjection {
        schema_version: LIVE_PROJECTION_SCHEMA_VERSION,
        case: envelope.case.clone(),
        sequence: envelope.sequence,
        input: envelope.input.clone(),
        delivery: "coordinator_reduction",
        origin_sequence: None,
        decision,
        records: project_records(&output, ids)?,
        actions: project_actions(&output, ids)?,
        same_turn_pending: pending.into_iter().map(|(_, evidence)| evidence).collect(),
        implicit_pending: None,
    });
    coordinator.chaos_intent_trace.clear();
    Ok(())
}

#[test]
fn initialized_live_reduction_matches_engine_decisions_and_is_byte_stable() {
    let initialization = fixture_bytes("risk_initialization_v1.json");
    let replay_bytes = fixture_bytes("replay_events_v1.jsonl");
    let expected_engine = String::from_utf8(fixture_bytes("expected_engine_v1.jsonl")).unwrap();
    let expected_live: LiveReductionManifest =
        serde_json::from_slice(&fixture_bytes("expected_live_reduction_v1.json"))
            .expect("strict live reduction manifest");
    let artifact = parse_initialization(&initialization).expect("strict initialization fixture");
    let replay = parse_replay_jsonl(&replay_bytes).expect("strict replay fixture");

    let (first_decisions, first_reductions) =
        run_live_replay(&artifact, &replay).expect("first live decision replay");
    let (second_decisions, second_reductions) =
        run_live_replay(&artifact, &replay).expect("second live decision replay");
    let first_decision_bytes = canonical_jsonl(&first_decisions).unwrap();
    let second_decision_bytes = canonical_jsonl(&second_decisions).unwrap();
    let first_live_bytes = canonical_jsonl(&first_reductions).unwrap();
    let second_live_bytes = canonical_jsonl(&second_reductions).unwrap();

    assert_eq!(first_decision_bytes, second_decision_bytes);
    assert_eq!(first_decision_bytes, expected_engine);
    assert_eq!(first_live_bytes, second_live_bytes);
    assert_eq!(expected_live.schema_version, LIVE_PROJECTION_SCHEMA_VERSION);
    assert_eq!(expected_live.projection, LIVE_PROJECTION_NAME);
    assert_eq!(first_reductions.len(), expected_live.rows);
    assert_eq!(first_live_bytes.lines().count(), expected_live.rows);
    assert_eq!(first_live_bytes.len(), expected_live.bytes);
    assert_eq!(
        format!("{:x}", Sha256::digest(first_live_bytes.as_bytes())),
        expected_live.sha256
    );
}

#[test]
#[ignore = "one-shot fixture authoring helper"]
fn print_live_reduction_projection_fixture() {
    let artifact = parse_initialization(&fixture_bytes("risk_initialization_v1.json")).unwrap();
    let replay = parse_replay_jsonl(&fixture_bytes("replay_events_v1.jsonl")).unwrap();
    let (live_decisions, live_reductions) = run_live_replay(&artifact, &replay).unwrap();

    assert_eq!(
        canonical_jsonl(&live_decisions).unwrap(),
        canonical_jsonl(&replay_engine(&artifact, &replay).unwrap()).unwrap(),
    );
    print!("{}", canonical_jsonl(&live_reductions).unwrap());
}
