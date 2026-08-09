use std::{collections::HashSet, fs, io, path::Path};

use reap_pm_controlled_trial::{
    AuthorizationConsumptionEvidence, AuthorizationConsumptionState,
    AuthorizationConsumptionVerification, AuthorizationRuntimeBinding, CanonicalAuthorization,
    CanonicalTrialConfig, OfflineAuthorizationState, verify_authorization_consumption,
};

use crate::{
    PmTrialLiveJournalError,
    hash::{ZERO_FINGERPRINT, canonical_json, hash_domain, validate_fingerprint},
    journal::{bound_paths, build_scope},
    protected::read_protected,
    schema::{
        CounterpartLinkV1, DispatchLineV1, DispatchRecordV1, IntentLineV1, IntentRecordV1,
        MAX_JOURNAL_BYTES, MAX_JOURNAL_LINE_BYTES, MAX_JOURNAL_RECORDS,
        PM_TRIAL_LIVE_JOURNAL_VERSION, PmCancelDispatchClassV1, PmCancelResultKindV1,
        PmPlaceResultKindV1, PmReconciliationOrderStateV1, PmTrialLiveJournalScopeV1,
        PmTrialLivePreflightBindingV1, dispatch_fingerprint, intent_fingerprint, validate_order_id,
        validate_utc,
    },
};

const MAX_CONSUMPTION_BYTES: usize = 128 * 1_024;
const CONSUMPTION_RECORD_FINGERPRINT_DOMAIN: &[u8] =
    b"reap.pm-t2.authorization-consumption.record.v1\0";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PmTrialLiveRecoveryClassificationV1 {
    PrePreparedDefinitelyUnsent,
    PreparedWithoutClaimDefinitelyUnsent,
    AuthorizationBurnedNoPlace,
    PlaceMayHaveBeenSentNoResend,
    RecoveryCancelOnly {
        exact_venue_order_id: String,
    },
    ReconcileBeforeRecoveryCancel {
        exact_venue_order_id: Option<String>,
    },
    TerminalEvidenceOnly,
}

/// Move-only, secret-free projection of two exact journal snapshots and the
/// separately bound take-once consumption state. It is evidence, not a place
/// retry permit or network-send grant.
pub struct PmTrialLiveRecoveryProjectionV1 {
    pub(crate) classification: PmTrialLiveRecoveryClassificationV1,
    pub(crate) scope: PmTrialLiveJournalScopeV1,
    pub(crate) preflight: Option<PmTrialLivePreflightBindingV1>,
    pub(crate) intent_bytes: Vec<u8>,
    pub(crate) dispatch_bytes: Vec<u8>,
    pub(crate) intent_lines: Vec<IntentLineV1>,
    pub(crate) dispatch_lines: Vec<DispatchLineV1>,
    pub(crate) consumption: ConsumptionSnapshot,
    pub(crate) reconciliation_target: CounterpartLinkV1,
}

impl PmTrialLiveRecoveryProjectionV1 {
    #[must_use]
    pub const fn classification(&self) -> &PmTrialLiveRecoveryClassificationV1 {
        &self.classification
    }

    #[must_use]
    pub fn scope_fingerprint(&self) -> &str {
        &self.scope.scope_fingerprint
    }

    pub fn latest_intent_record_fingerprint(&self) -> Result<String, PmTrialLiveJournalError> {
        intent_fingerprint(
            self.intent_lines
                .last()
                .ok_or(PmTrialLiveJournalError::InvalidRecord)?,
        )
    }

    pub fn latest_dispatch_record_fingerprint(&self) -> Result<String, PmTrialLiveJournalError> {
        dispatch_fingerprint(
            self.dispatch_lines
                .last()
                .ok_or(PmTrialLiveJournalError::InvalidRecord)?,
        )
    }

    #[must_use]
    pub fn intent_record_count(&self) -> usize {
        self.intent_lines.len()
    }

    #[must_use]
    pub fn dispatch_record_count(&self) -> usize {
        self.dispatch_lines.len()
    }

    #[must_use]
    pub const fn production_order_entry_authorized(&self) -> bool {
        false
    }

    #[must_use]
    pub const fn real_order_submission_authorized(&self) -> bool {
        false
    }

    #[must_use]
    pub const fn place_dispatch_allowance(&self) -> u8 {
        0
    }

    #[must_use]
    pub const fn placement_resumption_allowed(&self) -> bool {
        false
    }
}

impl std::fmt::Debug for PmTrialLiveRecoveryProjectionV1 {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PmTrialLiveRecoveryProjectionV1")
            .field("classification", &self.classification)
            .field("scope_fingerprint", &self.scope.scope_fingerprint)
            .field("intent_record_count", &self.intent_lines.len())
            .field("dispatch_record_count", &self.dispatch_lines.len())
            .field("production_order_entry_authorized", &false)
            .field("real_order_submission_authorized", &false)
            .field("place_dispatch_allowance", &0_u8)
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub(crate) enum ConsumptionSnapshot {
    Absent,
    Prepared {
        binding_fingerprint: String,
        prepared_record_fingerprint: String,
        latest_record_fingerprint: String,
    },
    Burned {
        binding_fingerprint: String,
        prepared_record_fingerprint: String,
        atomic_claim_fingerprint: String,
        consumed_record_fingerprint: Option<String>,
        ledger_record_count: u8,
        latest_record_fingerprint: String,
    },
}

pub fn verify_controlled_trial_live_recovery(
    config: &CanonicalTrialConfig,
    authorization: &CanonicalAuthorization,
) -> Result<PmTrialLiveRecoveryProjectionV1, PmTrialLiveJournalError> {
    let (intent_path, dispatch_path) = bound_paths(config);
    let intent_bytes = read_protected(&intent_path, MAX_JOURNAL_BYTES)?;
    let intent_lines = parse_intent(&intent_bytes)?;
    let dispatch_bytes = match read_protected(&dispatch_path, MAX_JOURNAL_BYTES) {
        Ok(bytes) => bytes,
        Err(PmTrialLiveJournalError::Absent) => {
            return verify_intent_header_without_dispatch_records(
                config,
                authorization,
                intent_bytes,
                intent_lines,
                Vec::new(),
            );
        }
        Err(error) => return Err(error),
    };
    // A crash after create-new and parent fsync but before the first dispatch
    // header append leaves an exact protected zero-byte dispatch file. Since
    // no preparation can exist at that boundary, it is definitely unsent.
    if dispatch_bytes.is_empty() {
        return verify_intent_header_without_dispatch_records(
            config,
            authorization,
            intent_bytes,
            intent_lines,
            dispatch_bytes,
        );
    }
    let dispatch_lines = parse_dispatch(&dispatch_bytes)?;
    let scope = validate_headers_and_scope(config, authorization, &intent_lines, &dispatch_lines)?;
    let consumption = load_consumption(config, authorization)?;
    validate_consumption_scope(&scope, &consumption)?;
    let facts = validate_records(&scope, &intent_lines, &dispatch_lines, &consumption)?;
    let classification = classify(&facts, &consumption);
    Ok(PmTrialLiveRecoveryProjectionV1 {
        classification,
        scope,
        preflight: facts.preflight.clone(),
        intent_bytes,
        dispatch_bytes,
        intent_lines,
        dispatch_lines,
        consumption,
        reconciliation_target: facts.reconciliation_target,
    })
}

fn verify_intent_header_without_dispatch_records(
    config: &CanonicalTrialConfig,
    authorization: &CanonicalAuthorization,
    intent_bytes: Vec<u8>,
    intent_lines: Vec<IntentLineV1>,
    dispatch_bytes: Vec<u8>,
) -> Result<PmTrialLiveRecoveryProjectionV1, PmTrialLiveJournalError> {
    if intent_lines.len() != 1 {
        return Err(PmTrialLiveJournalError::InvalidRecord);
    }
    let scope = match &intent_lines[0].body {
        IntentRecordV1::Header { scope } => scope.as_ref().clone(),
        _ => return Err(PmTrialLiveJournalError::InvalidRecord),
    };
    scope.validate()?;
    validate_chains(&scope, &intent_lines, &[])?;
    let runtime = AuthorizationRuntimeBinding {
        release_binary_sha256: scope.release_binary_sha256.clone(),
        release_binary_length: scope.release_binary_length,
        host: scope.host.clone(),
        observed_at_utc: scope.runtime_observed_at_utc.clone(),
    };
    if build_scope(
        config,
        authorization,
        &runtime,
        scope.owner_process_identity.clone(),
        scope.artifact_directory_lease_fingerprint.clone(),
        scope
            .expected_consumption
            .prepared_record_fingerprint
            .clone(),
    )? != scope
    {
        return Err(PmTrialLiveJournalError::InvalidBinding);
    }
    let consumption = load_consumption(config, authorization)?;
    validate_consumption_scope(&scope, &consumption)?;
    let classification = match &consumption {
        ConsumptionSnapshot::Burned { .. } => {
            PmTrialLiveRecoveryClassificationV1::AuthorizationBurnedNoPlace
        }
        ConsumptionSnapshot::Absent | ConsumptionSnapshot::Prepared { .. } => {
            PmTrialLiveRecoveryClassificationV1::PrePreparedDefinitelyUnsent
        }
    };
    Ok(PmTrialLiveRecoveryProjectionV1 {
        classification,
        scope,
        preflight: None,
        intent_bytes,
        dispatch_bytes,
        intent_lines,
        dispatch_lines: Vec::new(),
        consumption,
        reconciliation_target: CounterpartLinkV1 {
            sequence: 0,
            record_fingerprint: ZERO_FINGERPRINT.to_owned(),
        },
    })
}

fn validate_headers_and_scope(
    config: &CanonicalTrialConfig,
    authorization: &CanonicalAuthorization,
    intent: &[IntentLineV1],
    dispatch: &[DispatchLineV1],
) -> Result<PmTrialLiveJournalScopeV1, PmTrialLiveJournalError> {
    let scope = match &intent
        .first()
        .ok_or(PmTrialLiveJournalError::InvalidRecord)?
        .body
    {
        IntentRecordV1::Header { scope } => scope.as_ref().clone(),
        _ => return Err(PmTrialLiveJournalError::InvalidRecord),
    };
    let dispatch_scope = match &dispatch
        .first()
        .ok_or(PmTrialLiveJournalError::InvalidRecord)?
        .body
    {
        DispatchRecordV1::Header { scope } => scope.as_ref(),
        _ => return Err(PmTrialLiveJournalError::InvalidRecord),
    };
    scope.validate()?;
    if dispatch_scope != &scope {
        return Err(PmTrialLiveJournalError::InvalidRecord);
    }
    let runtime = AuthorizationRuntimeBinding {
        release_binary_sha256: scope.release_binary_sha256.clone(),
        release_binary_length: scope.release_binary_length,
        host: scope.host.clone(),
        observed_at_utc: scope.runtime_observed_at_utc.clone(),
    };
    let rebuilt = build_scope(
        config,
        authorization,
        &runtime,
        scope.owner_process_identity.clone(),
        scope.artifact_directory_lease_fingerprint.clone(),
        scope
            .expected_consumption
            .prepared_record_fingerprint
            .clone(),
    )?;
    if rebuilt != scope {
        return Err(PmTrialLiveJournalError::InvalidBinding);
    }
    Ok(scope)
}

fn parse_intent(bytes: &[u8]) -> Result<Vec<IntentLineV1>, PmTrialLiveJournalError> {
    parse_lines(bytes, |line| {
        serde_json::from_slice(line).map_err(|_| PmTrialLiveJournalError::InvalidRecord)
    })
}

fn parse_dispatch(bytes: &[u8]) -> Result<Vec<DispatchLineV1>, PmTrialLiveJournalError> {
    parse_lines(bytes, |line| {
        serde_json::from_slice(line).map_err(|_| PmTrialLiveJournalError::InvalidRecord)
    })
}

fn parse_lines<T: serde::de::DeserializeOwned + serde::Serialize>(
    bytes: &[u8],
    parse: impl Fn(&[u8]) -> Result<T, PmTrialLiveJournalError>,
) -> Result<Vec<T>, PmTrialLiveJournalError> {
    if bytes.is_empty() || !bytes.ends_with(b"\n") {
        return Err(PmTrialLiveJournalError::AmbiguousTail);
    }
    let mut records = Vec::new();
    let mut split = bytes.split(|byte| *byte == b'\n').peekable();
    while let Some(line) = split.next() {
        if line.is_empty() {
            if split.peek().is_none() {
                break;
            }
            return Err(PmTrialLiveJournalError::InvalidRecord);
        }
        if line.len() > MAX_JOURNAL_LINE_BYTES || records.len() == MAX_JOURNAL_RECORDS {
            return Err(PmTrialLiveJournalError::BoundExceeded);
        }
        let record = parse(line)?;
        if canonical_json(&record)? != line {
            return Err(PmTrialLiveJournalError::InvalidRecord);
        }
        records.push(record);
    }
    if records.is_empty() {
        return Err(PmTrialLiveJournalError::InvalidRecord);
    }
    Ok(records)
}

pub(crate) fn load_consumption(
    config: &CanonicalTrialConfig,
    authorization: &CanonicalAuthorization,
) -> Result<ConsumptionSnapshot, PmTrialLiveJournalError> {
    let parent = Path::new(&config.value().journal.artifact_directory);
    let ledger = parent.join(&config.value().journal.authorization_consumption_ledger_file);
    let claim = parent.join(&config.value().journal.authorization_consumption_claim_file);
    let ledger_exists = exists_exact(&ledger)?;
    let claim_exists = exists_exact(&claim)?;
    if !ledger_exists && !claim_exists {
        return Ok(ConsumptionSnapshot::Absent);
    }
    if !ledger_exists {
        return Err(PmTrialLiveJournalError::AmbiguousTail);
    }
    let verification = verify_authorization_consumption(config, authorization)
        .map_err(|_| PmTrialLiveJournalError::InvalidRecord)?;
    validate_consumption_verification(&verification)?;
    let bytes = read_protected(&ledger, MAX_CONSUMPTION_BYTES)?;
    let records: Vec<AuthorizationConsumptionEvidence> = parse_lines(&bytes, |line| {
        serde_json::from_slice(line).map_err(|_| PmTrialLiveJournalError::InvalidRecord)
    })?;
    if records.is_empty() || records.len() > 3 {
        return Err(PmTrialLiveJournalError::InvalidRecord);
    }
    let prepared = &records[0];
    if prepared.sequence != 0
        || prepared.previous_record_fingerprint != ZERO_FINGERPRINT
        || !matches!(
            prepared.consumption,
            AuthorizationConsumptionState::Prepared { .. }
        )
    {
        return Err(PmTrialLiveJournalError::InvalidRecord);
    }
    let prepared_record_fingerprint = hash_domain(CONSUMPTION_RECORD_FINGERPRINT_DOMAIN, prepared)?;
    if !verification.atomic_consumption_claim_durable {
        if claim_exists
            || records.len() != 1
            || !matches!(
                verification.state,
                AuthorizationConsumptionState::Prepared { .. }
            )
        {
            return Err(PmTrialLiveJournalError::InvalidRecord);
        }
        return Ok(ConsumptionSnapshot::Prepared {
            binding_fingerprint: verification.binding_fingerprint,
            prepared_record_fingerprint,
            latest_record_fingerprint: verification.latest_record_fingerprint,
        });
    }
    let atomic_claim_fingerprint = verification
        .claim_fingerprint
        .ok_or(PmTrialLiveJournalError::InvalidRecord)?;
    let consumed_record_fingerprint = records
        .get(1)
        .map(|record| {
            if record.sequence != 1
                || !matches!(
                    record.consumption,
                    AuthorizationConsumptionState::Consumed {
                        burned_before_dispatch_authority: true,
                        crash_allows_recovery_cancel_only: true,
                        placement_can_never_resume: true,
                        ..
                    }
                )
            {
                return Err(PmTrialLiveJournalError::InvalidRecord);
            }
            hash_domain(CONSUMPTION_RECORD_FINGERPRINT_DOMAIN, record)
        })
        .transpose()?;
    Ok(ConsumptionSnapshot::Burned {
        binding_fingerprint: verification.binding_fingerprint,
        prepared_record_fingerprint,
        atomic_claim_fingerprint,
        consumed_record_fingerprint,
        ledger_record_count: verification.ledger_record_count,
        latest_record_fingerprint: verification.latest_record_fingerprint,
    })
}

pub(crate) fn revalidate_projection(
    config: &CanonicalTrialConfig,
    authorization: &CanonicalAuthorization,
    expected: &PmTrialLiveRecoveryProjectionV1,
) -> Result<(), PmTrialLiveJournalError> {
    let current = verify_controlled_trial_live_recovery(config, authorization)?;
    if current.classification != expected.classification
        || current.scope != expected.scope
        || current.preflight != expected.preflight
        || current.intent_bytes != expected.intent_bytes
        || current.dispatch_bytes != expected.dispatch_bytes
        || current.intent_lines != expected.intent_lines
        || current.dispatch_lines != expected.dispatch_lines
        || current.consumption != expected.consumption
        || current.reconciliation_target != expected.reconciliation_target
    {
        return Err(PmTrialLiveJournalError::AmbiguousTail);
    }
    Ok(())
}

fn exists_exact(path: &Path) -> Result<bool, PmTrialLiveJournalError> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(_) => Err(PmTrialLiveJournalError::Protection),
    }
}

fn validate_consumption_verification(
    verification: &AuthorizationConsumptionVerification,
) -> Result<(), PmTrialLiveJournalError> {
    if verification.schema_version != 1
        || verification.ledger_record_count == 0
        || verification.ledger_record_count > 3
        || verification.ambiguous_tail
        || !verification.exact_bindings_structurally_valid
        || verification.authorization != OfflineAuthorizationState::DENIED
    {
        return Err(PmTrialLiveJournalError::InvalidRecord);
    }
    validate_fingerprint(&verification.latest_record_fingerprint)?;
    validate_fingerprint(&verification.binding_fingerprint)?;
    if let Some(claim) = &verification.claim_fingerprint {
        validate_fingerprint(claim)?;
    }
    Ok(())
}

fn validate_consumption_scope(
    scope: &PmTrialLiveJournalScopeV1,
    consumption: &ConsumptionSnapshot,
) -> Result<(), PmTrialLiveJournalError> {
    let (binding, prepared_record_fingerprint) = match consumption {
        ConsumptionSnapshot::Absent => return Err(PmTrialLiveJournalError::InvalidRecord),
        ConsumptionSnapshot::Prepared {
            binding_fingerprint,
            prepared_record_fingerprint,
            ..
        }
        | ConsumptionSnapshot::Burned {
            binding_fingerprint,
            prepared_record_fingerprint,
            ..
        } => (binding_fingerprint, prepared_record_fingerprint),
    };
    if binding != &scope.expected_consumption.binding_fingerprint
        || prepared_record_fingerprint != &scope.expected_consumption.prepared_record_fingerprint
    {
        return Err(PmTrialLiveJournalError::InvalidBinding);
    }
    Ok(())
}

struct JournalFacts {
    preflight: Option<PmTrialLivePreflightBindingV1>,
    place_prepared: bool,
    place_dispatch: Option<CounterpartLinkV1>,
    place_result: Option<(PmPlaceResultKindV1, Option<String>)>,
    latest_cancel_dispatch: Option<(CounterpartLinkV1, String)>,
    latest_cancel_result: Option<PmCancelResultKindV1>,
    latest_reconciliation: Option<(PmReconciliationOrderStateV1, Option<String>, u8)>,
    dispatch_terminal: bool,
    reconciliation_target: CounterpartLinkV1,
}

fn validate_records(
    scope: &PmTrialLiveJournalScopeV1,
    intent: &[IntentLineV1],
    dispatch: &[DispatchLineV1],
    consumption: &ConsumptionSnapshot,
) -> Result<JournalFacts, PmTrialLiveJournalError> {
    validate_chains(scope, intent, dispatch)?;
    let mut place_intent = None;
    let mut intent_preflight = None;
    let mut place_bridge_targets = HashSet::new();
    let mut cancel_bridge_targets = HashSet::new();
    let mut cancel_ownership_sources = HashSet::new();
    let mut latest_cross_dispatch_sequence = 0_u8;
    let mut latest_reconciliation = None;
    let mut intent_terminal = None;

    for (index, line) in intent.iter().enumerate().skip(1) {
        match &line.body {
            IntentRecordV1::Header { .. } => return Err(PmTrialLiveJournalError::InvalidRecord),
            IntentRecordV1::PreflightBound { preflight } => {
                preflight.validate()?;
                if index != 1 || intent_preflight.replace(preflight.clone()).is_some() {
                    return Err(PmTrialLiveJournalError::InvalidRecord);
                }
            }
            IntentRecordV1::PlaceIntent { created_at_utc } => {
                validate_utc(created_at_utc)?;
                if index != 2
                    || intent_preflight.is_none()
                    || place_intent.replace(line.sequence).is_some()
                {
                    return Err(PmTrialLiveJournalError::InvalidRecord);
                }
            }
            IntentRecordV1::PlaceOutcomeBridge {
                dispatch: target,
                outcome,
                observed_order_id,
            } => {
                validate_monotonic_cross_link(
                    target,
                    dispatch,
                    &mut latest_cross_dispatch_sequence,
                )?;
                let DispatchRecordV1::PlaceResult {
                    outcome: dispatch_outcome,
                    observed_order_id: dispatch_order_id,
                    ..
                } = dispatch_body(target, dispatch)?
                else {
                    return Err(PmTrialLiveJournalError::InvalidRecord);
                };
                if outcome != dispatch_outcome
                    || observed_order_id != dispatch_order_id
                    || !place_bridge_targets.insert(target.sequence)
                {
                    return Err(PmTrialLiveJournalError::InvalidRecord);
                }
            }
            IntentRecordV1::CancelIntent {
                created_at_utc,
                ownership_source,
                exact_venue_order_id,
                dispatch_class,
            } => {
                validate_utc(created_at_utc)?;
                validate_order_id(exact_venue_order_id)?;
                let source = intent_body_before(ownership_source, intent, line.sequence)?;
                let source_order_id = match source {
                    IntentRecordV1::PlaceOutcomeBridge {
                        outcome: PmPlaceResultKindV1::Accepted,
                        observed_order_id: Some(order_id),
                        ..
                    }
                    | IntentRecordV1::Reconciliation {
                        state: PmReconciliationOrderStateV1::ExactLive,
                        exact_venue_order_id: Some(order_id),
                        ..
                    } => order_id,
                    _ => return Err(PmTrialLiveJournalError::InvalidRecord),
                };
                if source_order_id != exact_venue_order_id
                    || !cancel_ownership_sources.insert(ownership_source.sequence)
                {
                    return Err(PmTrialLiveJournalError::InvalidRecord);
                }
                validate_cancel_class_value(scope, *dispatch_class)?;
            }
            IntentRecordV1::CancelOutcomeBridge {
                dispatch: target,
                outcome,
                exact_venue_order_id,
            } => {
                validate_order_id(exact_venue_order_id)?;
                validate_monotonic_cross_link(
                    target,
                    dispatch,
                    &mut latest_cross_dispatch_sequence,
                )?;
                let DispatchRecordV1::CancelResult {
                    outcome: dispatch_outcome,
                    exact_venue_order_id: dispatch_order_id,
                    ..
                } = dispatch_body(target, dispatch)?
                else {
                    return Err(PmTrialLiveJournalError::InvalidRecord);
                };
                if outcome != dispatch_outcome
                    || exact_venue_order_id != dispatch_order_id
                    || !cancel_bridge_targets.insert(target.sequence)
                {
                    return Err(PmTrialLiveJournalError::InvalidRecord);
                }
            }
            IntentRecordV1::Reconciliation {
                observed_at_utc,
                state,
                exact_venue_order_id,
                dispatch: target,
            } => {
                validate_utc(observed_at_utc)?;
                validate_reconciliation(scope, *state, exact_venue_order_id.as_deref())?;
                validate_monotonic_cross_link(
                    target,
                    dispatch,
                    &mut latest_cross_dispatch_sequence,
                )?;
                match dispatch_body(target, dispatch)? {
                    DispatchRecordV1::PlaceDispatchAuthorized { .. }
                    | DispatchRecordV1::PlaceResult { .. }
                    | DispatchRecordV1::CancelDispatchAuthorized { .. }
                    | DispatchRecordV1::CancelResult { .. } => {}
                    _ => return Err(PmTrialLiveJournalError::InvalidRecord),
                }
                latest_reconciliation =
                    Some((*state, exact_venue_order_id.clone(), target.sequence));
            }
            IntentRecordV1::Terminal {
                terminal_at_utc,
                dispatch_terminal,
                terminal_is_evidence_not_authority,
                ..
            } => {
                validate_utc(terminal_at_utc)?;
                if index + 1 != intent.len() || !terminal_is_evidence_not_authority {
                    return Err(PmTrialLiveJournalError::InvalidRecord);
                }
                let DispatchRecordV1::Terminal {
                    terminal_at_utc: dispatch_at,
                    terminal_is_evidence_not_authority: true,
                    ..
                } = dispatch_body(dispatch_terminal, dispatch)?
                else {
                    return Err(PmTrialLiveJournalError::InvalidRecord);
                };
                if dispatch_at != terminal_at_utc {
                    return Err(PmTrialLiveJournalError::InvalidRecord);
                }
                intent_terminal = Some(dispatch_terminal.clone());
            }
        }
    }

    let place_intent = place_intent;
    let mut stage = DispatchStage::Header;
    let mut bound_preflight = None;
    let mut place_prepared = false;
    let mut expected_order_id = None;
    let mut place_dispatch = None;
    let mut place_result = None;
    let mut latest_cancel_dispatch = None;
    let mut latest_cancel_result = None;
    let mut primary_seen = false;
    let mut next_recovery = 1_u8;
    let mut dispatch_terminal = false;
    let mut reconciliation_target = None;

    for (index, line) in dispatch.iter().enumerate().skip(1) {
        match &line.body {
            DispatchRecordV1::Header { .. } => return Err(PmTrialLiveJournalError::InvalidRecord),
            DispatchRecordV1::PreflightBound {
                preflight,
                intent_preflight: target,
            } => {
                if index != 1 || stage != DispatchStage::Header {
                    return Err(PmTrialLiveJournalError::InvalidRecord);
                }
                let IntentRecordV1::PreflightBound {
                    preflight: intent_value,
                } = intent_body(target, intent)?
                else {
                    return Err(PmTrialLiveJournalError::InvalidRecord);
                };
                preflight.validate()?;
                if preflight != intent_value || intent_preflight.as_ref() != Some(preflight) {
                    return Err(PmTrialLiveJournalError::InvalidRecord);
                }
                let created = validate_utc(&scope.runtime_observed_at_utc)?;
                let validated = validate_utc(preflight.validated_at_utc())?;
                let expires = validate_utc(
                    &scope
                        .expected_consumption
                        .binding
                        .authorization_expires_at_utc,
                )?;
                if validated < created || validated >= expires {
                    return Err(PmTrialLiveJournalError::InvalidRecord);
                }
                bound_preflight = Some(preflight.clone());
                stage = DispatchStage::Preflight;
            }
            DispatchRecordV1::PlacePrepared {
                intent: target,
                preparation,
            } => {
                if index != 2 || stage != DispatchStage::Preflight || place_intent.is_none() {
                    return Err(PmTrialLiveJournalError::InvalidRecord);
                }
                let IntentRecordV1::PlaceIntent { .. } = intent_body(target, intent)? else {
                    return Err(PmTrialLiveJournalError::InvalidRecord);
                };
                preparation.validate_against_scope(
                    scope,
                    bound_preflight
                        .as_ref()
                        .ok_or(PmTrialLiveJournalError::InvalidRecord)?,
                    target.sequence,
                )?;
                expected_order_id = Some(preparation.expected_order_id().to_owned());
                place_prepared = true;
                stage = DispatchStage::PlacePrepared;
            }
            DispatchRecordV1::PlaceDispatchAuthorized {
                prepared_sequence,
                prepared_record_fingerprint,
                consumption: recorded_consumption,
                production_order_entry_authorized,
                real_order_submission_authorized,
                place_dispatch_allowance,
            } => {
                if stage != DispatchStage::PlacePrepared
                    || *production_order_entry_authorized
                    || *real_order_submission_authorized
                    || *place_dispatch_allowance != 0
                {
                    return Err(PmTrialLiveJournalError::InvalidRecord);
                }
                let prepared = dispatch
                    .get(usize::from(*prepared_sequence))
                    .ok_or(PmTrialLiveJournalError::InvalidRecord)?;
                if !matches!(prepared.body, DispatchRecordV1::PlacePrepared { .. })
                    || dispatch_fingerprint(prepared)? != *prepared_record_fingerprint
                {
                    return Err(PmTrialLiveJournalError::InvalidRecord);
                }
                validate_recorded_consumption(recorded_consumption, consumption)?;
                let link = dispatch_link(line)?;
                reconciliation_target = Some(link.clone());
                place_dispatch = Some(link);
                stage = DispatchStage::PlaceDispatch;
            }
            DispatchRecordV1::PlaceResult {
                dispatch_authorized_sequence,
                dispatch_authorized_fingerprint,
                outcome,
                observed_order_id,
            } => {
                if stage != DispatchStage::PlaceDispatch {
                    return Err(PmTrialLiveJournalError::InvalidRecord);
                }
                validate_dispatch_reference(
                    dispatch,
                    *dispatch_authorized_sequence,
                    dispatch_authorized_fingerprint,
                    |record| matches!(record, DispatchRecordV1::PlaceDispatchAuthorized { .. }),
                )?;
                validate_place_result(
                    *outcome,
                    observed_order_id.as_deref(),
                    expected_order_id
                        .as_deref()
                        .ok_or(PmTrialLiveJournalError::InvalidRecord)?,
                )?;
                reconciliation_target = Some(dispatch_link(line)?);
                place_result = Some((*outcome, observed_order_id.clone()));
                stage = DispatchStage::PlaceResult;
            }
            DispatchRecordV1::CancelPrepared {
                intent: target,
                dispatch_class,
                preparation,
            } => {
                if !matches!(
                    stage,
                    DispatchStage::PlaceDispatch
                        | DispatchStage::PlaceResult
                        | DispatchStage::CancelDispatch
                        | DispatchStage::CancelResult
                ) {
                    return Err(PmTrialLiveJournalError::InvalidRecord);
                }
                let IntentRecordV1::CancelIntent {
                    exact_venue_order_id,
                    dispatch_class: intent_class,
                    ..
                } = intent_body(target, intent)?
                else {
                    return Err(PmTrialLiveJournalError::InvalidRecord);
                };
                if intent_class != dispatch_class
                    || exact_venue_order_id != preparation.exact_venue_order_id()
                {
                    return Err(PmTrialLiveJournalError::InvalidRecord);
                }
                validate_cancel_ordinal(
                    scope,
                    *dispatch_class,
                    &mut primary_seen,
                    &mut next_recovery,
                )?;
                let minimum_l2 = latest_preparation_l2(dispatch, index)?;
                preparation.validate_against_scope(
                    scope,
                    bound_preflight
                        .as_ref()
                        .ok_or(PmTrialLiveJournalError::InvalidRecord)?,
                    target.sequence,
                    minimum_l2,
                )?;
                stage = DispatchStage::CancelPrepared;
            }
            DispatchRecordV1::CancelDispatchAuthorized {
                prepared_sequence,
                prepared_record_fingerprint,
                dispatch_class,
                exact_venue_order_id,
                production_order_entry_authorized,
                real_order_submission_authorized,
                place_dispatch_allowance,
            } => {
                if stage != DispatchStage::CancelPrepared
                    || *production_order_entry_authorized
                    || *real_order_submission_authorized
                    || *place_dispatch_allowance != 0
                {
                    return Err(PmTrialLiveJournalError::InvalidRecord);
                }
                let prepared = dispatch
                    .get(usize::from(*prepared_sequence))
                    .ok_or(PmTrialLiveJournalError::InvalidRecord)?;
                let DispatchRecordV1::CancelPrepared {
                    dispatch_class: prepared_class,
                    preparation,
                    ..
                } = &prepared.body
                else {
                    return Err(PmTrialLiveJournalError::InvalidRecord);
                };
                if prepared_class != dispatch_class
                    || preparation.exact_venue_order_id() != exact_venue_order_id
                    || dispatch_fingerprint(prepared)? != *prepared_record_fingerprint
                {
                    return Err(PmTrialLiveJournalError::InvalidRecord);
                }
                let link = dispatch_link(line)?;
                reconciliation_target = Some(link.clone());
                latest_cancel_dispatch = Some((link, exact_venue_order_id.clone()));
                latest_cancel_result = None;
                stage = DispatchStage::CancelDispatch;
            }
            DispatchRecordV1::CancelResult {
                dispatch_authorized_sequence,
                dispatch_authorized_fingerprint,
                outcome,
                exact_venue_order_id,
            } => {
                if stage != DispatchStage::CancelDispatch {
                    return Err(PmTrialLiveJournalError::InvalidRecord);
                }
                validate_order_id(exact_venue_order_id)?;
                let record = validate_dispatch_reference(
                    dispatch,
                    *dispatch_authorized_sequence,
                    dispatch_authorized_fingerprint,
                    |record| matches!(record, DispatchRecordV1::CancelDispatchAuthorized { .. }),
                )?;
                let DispatchRecordV1::CancelDispatchAuthorized {
                    exact_venue_order_id: dispatched_id,
                    ..
                } = record
                else {
                    unreachable!();
                };
                if dispatched_id != exact_venue_order_id {
                    return Err(PmTrialLiveJournalError::InvalidRecord);
                }
                reconciliation_target = Some(dispatch_link(line)?);
                latest_cancel_result = Some(*outcome);
                stage = DispatchStage::CancelResult;
            }
            DispatchRecordV1::Terminal {
                terminal_at_utc,
                intent: target,
                terminal_is_evidence_not_authority,
            } => {
                validate_utc(terminal_at_utc)?;
                if index + 1 != dispatch.len() || index == 1 || !terminal_is_evidence_not_authority
                {
                    return Err(PmTrialLiveJournalError::InvalidRecord);
                }
                let target_line = intent_line(target, intent)?;
                if matches!(target_line.body, IntentRecordV1::Terminal { .. }) {
                    return Err(PmTrialLiveJournalError::InvalidRecord);
                }
                dispatch_terminal = true;
                stage = DispatchStage::Terminal;
            }
        }
    }
    if stage == DispatchStage::CancelPrepared {
        // Prepared cancel bytes are definitely unsent, but recovery must first
        // reconcile instead of recreating or auto-dispatching them.
        reconciliation_target = dispatch
            .iter()
            .rev()
            .skip(1)
            .find(|line| {
                matches!(
                    line.body,
                    DispatchRecordV1::PlaceDispatchAuthorized { .. }
                        | DispatchRecordV1::PlaceResult { .. }
                        | DispatchRecordV1::CancelDispatchAuthorized { .. }
                        | DispatchRecordV1::CancelResult { .. }
                )
            })
            .map(dispatch_link)
            .transpose()?;
    }
    if let Some(intent_terminal_link) = intent_terminal {
        if !dispatch_terminal
            || dispatch.last().map(|line| line.sequence) != Some(intent_terminal_link.sequence)
        {
            return Err(PmTrialLiveJournalError::InvalidRecord);
        }
    } else if dispatch_terminal {
        let DispatchRecordV1::Terminal { intent: target, .. } = &dispatch
            .last()
            .ok_or(PmTrialLiveJournalError::InvalidRecord)?
            .body
        else {
            return Err(PmTrialLiveJournalError::InvalidRecord);
        };
        if target.sequence != intent.last().map(|line| line.sequence).unwrap_or_default() {
            return Err(PmTrialLiveJournalError::InvalidRecord);
        }
    }
    if place_intent.is_some() && bound_preflight.is_none() {
        return Err(PmTrialLiveJournalError::InvalidRecord);
    }
    let reconciliation_target = reconciliation_target.unwrap_or_else(|| CounterpartLinkV1 {
        sequence: 0,
        record_fingerprint: dispatch_fingerprint(&dispatch[0])
            .unwrap_or_else(|_| ZERO_FINGERPRINT.to_owned()),
    });
    Ok(JournalFacts {
        preflight: bound_preflight,
        place_prepared,
        place_dispatch,
        place_result,
        latest_cancel_dispatch,
        latest_cancel_result,
        latest_reconciliation,
        dispatch_terminal,
        reconciliation_target,
    })
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum DispatchStage {
    Header,
    Preflight,
    PlacePrepared,
    PlaceDispatch,
    PlaceResult,
    CancelPrepared,
    CancelDispatch,
    CancelResult,
    Terminal,
}

fn validate_chains(
    scope: &PmTrialLiveJournalScopeV1,
    intent: &[IntentLineV1],
    dispatch: &[DispatchLineV1],
) -> Result<(), PmTrialLiveJournalError> {
    let mut previous = ZERO_FINGERPRINT.to_owned();
    for (index, line) in intent.iter().enumerate() {
        if line.schema_version != PM_TRIAL_LIVE_JOURNAL_VERSION
            || usize::from(line.sequence) != index
            || line.previous_record_fingerprint != previous
            || line.scope_fingerprint != scope.scope_fingerprint
        {
            return Err(PmTrialLiveJournalError::InvalidRecord);
        }
        previous = intent_fingerprint(line)?;
    }
    previous = ZERO_FINGERPRINT.to_owned();
    for (index, line) in dispatch.iter().enumerate() {
        if line.schema_version != PM_TRIAL_LIVE_JOURNAL_VERSION
            || usize::from(line.sequence) != index
            || line.previous_record_fingerprint != previous
            || line.scope_fingerprint != scope.scope_fingerprint
        {
            return Err(PmTrialLiveJournalError::InvalidRecord);
        }
        previous = dispatch_fingerprint(line)?;
    }
    Ok(())
}

fn validate_recorded_consumption(
    recorded: &crate::schema::PmTrialLiveConsumedFingerprintsV1,
    actual: &ConsumptionSnapshot,
) -> Result<(), PmTrialLiveJournalError> {
    recorded.validate()?;
    let ConsumptionSnapshot::Burned {
        binding_fingerprint,
        prepared_record_fingerprint,
        atomic_claim_fingerprint,
        consumed_record_fingerprint: Some(consumed_record_fingerprint),
        ..
    } = actual
    else {
        return Err(PmTrialLiveJournalError::InvalidRecord);
    };
    if &recorded.binding_fingerprint != binding_fingerprint
        || &recorded.prepared_record_fingerprint != prepared_record_fingerprint
        || &recorded.atomic_claim_fingerprint != atomic_claim_fingerprint
        || &recorded.consumed_record_fingerprint != consumed_record_fingerprint
    {
        return Err(PmTrialLiveJournalError::InvalidRecord);
    }
    Ok(())
}

fn classify(
    facts: &JournalFacts,
    consumption: &ConsumptionSnapshot,
) -> PmTrialLiveRecoveryClassificationV1 {
    if facts.dispatch_terminal {
        return PmTrialLiveRecoveryClassificationV1::TerminalEvidenceOnly;
    }
    if facts.place_dispatch.is_none() {
        return match consumption {
            ConsumptionSnapshot::Burned { .. } => {
                PmTrialLiveRecoveryClassificationV1::AuthorizationBurnedNoPlace
            }
            ConsumptionSnapshot::Absent | ConsumptionSnapshot::Prepared { .. }
                if facts.place_prepared =>
            {
                PmTrialLiveRecoveryClassificationV1::PreparedWithoutClaimDefinitelyUnsent
            }
            ConsumptionSnapshot::Absent | ConsumptionSnapshot::Prepared { .. } => {
                PmTrialLiveRecoveryClassificationV1::PrePreparedDefinitelyUnsent
            }
        };
    }
    if let Some((state, order_id, target_sequence)) = &facts.latest_reconciliation
        && *target_sequence >= facts.reconciliation_target.sequence
    {
        return match (state, order_id) {
            (PmReconciliationOrderStateV1::ExactLive, Some(exact_venue_order_id)) => {
                PmTrialLiveRecoveryClassificationV1::RecoveryCancelOnly {
                    exact_venue_order_id: exact_venue_order_id.clone(),
                }
            }
            (PmReconciliationOrderStateV1::Ambiguous, _) => {
                PmTrialLiveRecoveryClassificationV1::ReconcileBeforeRecoveryCancel {
                    exact_venue_order_id: None,
                }
            }
            _ => PmTrialLiveRecoveryClassificationV1::PlaceMayHaveBeenSentNoResend,
        };
    }
    if let Some((_, exact_venue_order_id)) = &facts.latest_cancel_dispatch {
        if facts.latest_cancel_result != Some(PmCancelResultKindV1::Canceled) {
            return PmTrialLiveRecoveryClassificationV1::ReconcileBeforeRecoveryCancel {
                exact_venue_order_id: Some(exact_venue_order_id.clone()),
            };
        }
        return PmTrialLiveRecoveryClassificationV1::PlaceMayHaveBeenSentNoResend;
    }
    if let Some((PmPlaceResultKindV1::Accepted, Some(exact_venue_order_id))) = &facts.place_result {
        return PmTrialLiveRecoveryClassificationV1::RecoveryCancelOnly {
            exact_venue_order_id: exact_venue_order_id.clone(),
        };
    }
    PmTrialLiveRecoveryClassificationV1::PlaceMayHaveBeenSentNoResend
}

fn intent_line<'a>(
    link: &CounterpartLinkV1,
    lines: &'a [IntentLineV1],
) -> Result<&'a IntentLineV1, PmTrialLiveJournalError> {
    link.validate()?;
    let line = lines
        .get(usize::from(link.sequence))
        .ok_or(PmTrialLiveJournalError::InvalidRecord)?;
    if intent_fingerprint(line)? != link.record_fingerprint {
        return Err(PmTrialLiveJournalError::InvalidRecord);
    }
    Ok(line)
}

fn intent_body<'a>(
    link: &CounterpartLinkV1,
    lines: &'a [IntentLineV1],
) -> Result<&'a IntentRecordV1, PmTrialLiveJournalError> {
    Ok(&intent_line(link, lines)?.body)
}

fn intent_body_before<'a>(
    link: &CounterpartLinkV1,
    lines: &'a [IntentLineV1],
    before: u8,
) -> Result<&'a IntentRecordV1, PmTrialLiveJournalError> {
    if link.sequence >= before {
        return Err(PmTrialLiveJournalError::InvalidRecord);
    }
    intent_body(link, lines)
}

fn dispatch_body<'a>(
    link: &CounterpartLinkV1,
    lines: &'a [DispatchLineV1],
) -> Result<&'a DispatchRecordV1, PmTrialLiveJournalError> {
    link.validate()?;
    let line = lines
        .get(usize::from(link.sequence))
        .ok_or(PmTrialLiveJournalError::InvalidRecord)?;
    if dispatch_fingerprint(line)? != link.record_fingerprint {
        return Err(PmTrialLiveJournalError::InvalidRecord);
    }
    Ok(&line.body)
}

fn dispatch_link(line: &DispatchLineV1) -> Result<CounterpartLinkV1, PmTrialLiveJournalError> {
    Ok(CounterpartLinkV1 {
        sequence: line.sequence,
        record_fingerprint: dispatch_fingerprint(line)?,
    })
}

fn validate_monotonic_cross_link(
    link: &CounterpartLinkV1,
    dispatch: &[DispatchLineV1],
    latest: &mut u8,
) -> Result<(), PmTrialLiveJournalError> {
    dispatch_body(link, dispatch)?;
    if link.sequence < *latest {
        return Err(PmTrialLiveJournalError::InvalidRecord);
    }
    *latest = link.sequence;
    Ok(())
}

fn validate_dispatch_reference<'a>(
    dispatch: &'a [DispatchLineV1],
    sequence: u8,
    fingerprint: &str,
    expected: impl Fn(&DispatchRecordV1) -> bool,
) -> Result<&'a DispatchRecordV1, PmTrialLiveJournalError> {
    validate_fingerprint(fingerprint)?;
    let line = dispatch
        .get(usize::from(sequence))
        .ok_or(PmTrialLiveJournalError::InvalidRecord)?;
    if dispatch_fingerprint(line)? != fingerprint || !expected(&line.body) {
        return Err(PmTrialLiveJournalError::InvalidRecord);
    }
    Ok(&line.body)
}

fn latest_preparation_l2(
    dispatch: &[DispatchLineV1],
    before: usize,
) -> Result<u64, PmTrialLiveJournalError> {
    dispatch[..before]
        .iter()
        .rev()
        .find_map(|line| match &line.body {
            DispatchRecordV1::PlacePrepared { preparation, .. } => {
                Some(preparation.l2_timestamp_seconds())
            }
            DispatchRecordV1::CancelPrepared { preparation, .. } => {
                Some(preparation.l2_timestamp_seconds())
            }
            _ => None,
        })
        .ok_or(PmTrialLiveJournalError::InvalidRecord)
}

fn validate_cancel_ordinal(
    scope: &PmTrialLiveJournalScopeV1,
    class: PmCancelDispatchClassV1,
    primary_seen: &mut bool,
    next_recovery: &mut u8,
) -> Result<(), PmTrialLiveJournalError> {
    match class {
        PmCancelDispatchClassV1::Primary => {
            if *primary_seen
                || *next_recovery != 1
                || scope.trial.order.primary_cancel_dispatch_budget != 1
            {
                return Err(PmTrialLiveJournalError::InvalidRecord);
            }
            *primary_seen = true;
        }
        PmCancelDispatchClassV1::Recovery { ordinal } => {
            if ordinal != *next_recovery
                || ordinal == 0
                || ordinal > scope.trial.order.recovery_cancel_dispatch_budget
            {
                return Err(PmTrialLiveJournalError::InvalidRecord);
            }
            *next_recovery = next_recovery
                .checked_add(1)
                .ok_or(PmTrialLiveJournalError::BoundExceeded)?;
        }
    }
    Ok(())
}

fn validate_cancel_class_value(
    scope: &PmTrialLiveJournalScopeV1,
    class: PmCancelDispatchClassV1,
) -> Result<(), PmTrialLiveJournalError> {
    match class {
        PmCancelDispatchClassV1::Primary
            if scope.trial.order.primary_cancel_dispatch_budget == 1 =>
        {
            Ok(())
        }
        PmCancelDispatchClassV1::Recovery { ordinal }
            if ordinal > 0 && ordinal <= scope.trial.order.recovery_cancel_dispatch_budget =>
        {
            Ok(())
        }
        _ => Err(PmTrialLiveJournalError::InvalidRecord),
    }
}

fn validate_place_result(
    outcome: PmPlaceResultKindV1,
    observed_order_id: Option<&str>,
    expected_order_id: &str,
) -> Result<(), PmTrialLiveJournalError> {
    if let Some(order_id) = observed_order_id {
        validate_order_id(order_id)?;
    }
    match (outcome, observed_order_id) {
        (PmPlaceResultKindV1::Accepted, Some(order_id))
            if order_id.strip_prefix("0x") == Some(expected_order_id) =>
        {
            Ok(())
        }
        (PmPlaceResultKindV1::Accepted, _) => Err(PmTrialLiveJournalError::InvalidRecord),
        (_, None) => Ok(()),
        (_, Some(_)) => Err(PmTrialLiveJournalError::InvalidRecord),
    }
}

fn validate_reconciliation(
    scope: &PmTrialLiveJournalScopeV1,
    state: PmReconciliationOrderStateV1,
    exact_venue_order_id: Option<&str>,
) -> Result<(), PmTrialLiveJournalError> {
    match (state, exact_venue_order_id) {
        (
            PmReconciliationOrderStateV1::ExactLive
            | PmReconciliationOrderStateV1::ExactCanceled
            | PmReconciliationOrderStateV1::ExactFilled,
            Some(order_id),
        ) => {
            validate_order_id(order_id)?;
            if order_id.strip_prefix("0x") != Some(scope.expected_order_id.as_str()) {
                return Err(PmTrialLiveJournalError::InvalidRecord);
            }
            Ok(())
        }
        (PmReconciliationOrderStateV1::Absent, None)
        | (PmReconciliationOrderStateV1::Ambiguous, None) => Ok(()),
        _ => Err(PmTrialLiveJournalError::InvalidRecord),
    }
}
