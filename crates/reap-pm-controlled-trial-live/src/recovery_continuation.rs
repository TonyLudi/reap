use std::{
    collections::HashSet,
    path::{Path, PathBuf},
};

use reap_pm_controlled_trial::{
    AuthorizationHostBinding, AuthorizationRecoveryCancelPreparedAnchorV1,
    AuthorizationRecoveryContinuationRegistryV1, AuthorizationRecoveryTerminalPlanV1,
    AuthorizationRuntimeBinding, CanonicalAuthorization, CanonicalTrialConfig,
    ConsumedAuthorizationConsumption, TerminalDisposition as AuthorizationTerminalDisposition,
};
use serde::{Deserialize, Serialize};

use crate::{
    PmTrialLiveJournalError,
    hash::{ZERO_FINGERPRINT, canonical_json, hash_domain, validate_fingerprint},
    protected::{ProtectedJournal, read_protected},
    recovery::{PmPhaseALiveOriginalTerminalStateV1, PmPhaseALiveRecoveryContinuationBasisV1},
    schema::{
        CounterpartLinkV1, MAX_JOURNAL_BYTES, MAX_JOURNAL_LINE_BYTES, MAX_JOURNAL_RECORDS,
        PmCancelDispatchClassV1, PmCancelPreparationV1, PmCancelPreparationViewV1,
        PmCancelResultKindV1, PmIntentTerminalDispositionV1, PmReconciliationOrderStateV1,
        PmTrialLiveConsumedFingerprintsV1, PmTrialLiveJournalScopeV1,
        PmTrialLivePreflightBindingV1, validate_order_id, validate_utc,
    },
};

pub const PM_PHASE_A_LIVE_CANCEL_RECOVERY_CONTINUATION_INTENT_FILE_V1: &str =
    "pm-t2-phase-a-live-cancel-recovery-continuation-intent-v1.jsonl";
pub const PM_PHASE_A_LIVE_CANCEL_RECOVERY_CONTINUATION_DISPATCH_FILE_V1: &str =
    "pm-t2-phase-a-live-cancel-recovery-continuation-dispatch-v1.jsonl";

const FAMILY: &str = "reap.pm-t2.phase-a-live-cancel-recovery-continuation";
const VERSION: u32 = 1;
const SCOPE_DOMAIN: &[u8] = b"reap.pm-t2.phase-a-live-cancel-recovery-continuation.scope.v1\0";
const INTENT_DOMAIN: &[u8] =
    b"reap.pm-t2.phase-a-live-cancel-recovery-continuation.intent-record.v1\0";
const DISPATCH_DOMAIN: &[u8] =
    b"reap.pm-t2.phase-a-live-cancel-recovery-continuation.dispatch-record.v1\0";
const PREPARATION_DOMAIN: &[u8] =
    b"reap.pm-t2.phase-a-live-cancel-recovery-continuation.preparation.v1\0";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum BarrierStateV1 {
    UnusableConservative,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum OriginalTerminalStateV1 {
    Paired,
    DispatchTerminalOnly,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ContinuationScopeV1 {
    journal_family: String,
    journal_version: u32,
    intent_file: String,
    dispatch_file: String,
    canonical_config_fingerprint: String,
    authorization_fingerprint: String,
    original_scope_fingerprint: String,
    original_intent_tail: CounterpartLinkV1,
    original_dispatch_terminal: CounterpartLinkV1,
    original_terminal_state: OriginalTerminalStateV1,
    preserved_exposure: CounterpartLinkV1,
    consumed: PmTrialLiveConsumedFingerprintsV1,
    exact_venue_order_id: String,
    minimum_cancel_l2_timestamp_seconds: u64,
    barrier_state: BarrierStateV1,
    current_release_binary_sha256: String,
    current_release_binary_length: u64,
    current_host: AuthorizationHostBinding,
    current_runtime_observed_at_utc: String,
    cleanup_not_after_utc: String,
    production_order_entry_authorized: bool,
    real_order_submission_authorized: bool,
    place_dispatch_allowance: u8,
    placement_resumption_allowed: bool,
    scope_fingerprint: String,
}

impl ContinuationScopeV1 {
    fn build(
        config: &CanonicalTrialConfig,
        authorization: &CanonicalAuthorization,
        original_scope: &PmTrialLiveJournalScopeV1,
        basis: &PmPhaseALiveRecoveryContinuationBasisV1,
        consumed: PmTrialLiveConsumedFingerprintsV1,
        current_runtime: &AuthorizationRuntimeBinding,
    ) -> Result<Self, PmTrialLiveJournalError> {
        let mut value = Self {
            journal_family: FAMILY.to_owned(),
            journal_version: VERSION,
            intent_file: PM_PHASE_A_LIVE_CANCEL_RECOVERY_CONTINUATION_INTENT_FILE_V1.to_owned(),
            dispatch_file: PM_PHASE_A_LIVE_CANCEL_RECOVERY_CONTINUATION_DISPATCH_FILE_V1.to_owned(),
            canonical_config_fingerprint: config.fingerprint().to_owned(),
            authorization_fingerprint: authorization.fingerprint().to_owned(),
            original_scope_fingerprint: original_scope.scope_fingerprint.clone(),
            original_intent_tail: basis.original_intent_tail().clone(),
            original_dispatch_terminal: basis.original_dispatch_terminal().clone(),
            original_terminal_state: match basis.original_terminal_state() {
                PmPhaseALiveOriginalTerminalStateV1::Paired => OriginalTerminalStateV1::Paired,
                PmPhaseALiveOriginalTerminalStateV1::DispatchTerminalOnly => {
                    OriginalTerminalStateV1::DispatchTerminalOnly
                }
            },
            preserved_exposure: basis.preserved_exposure().clone(),
            consumed,
            exact_venue_order_id: basis.exact_venue_order_id().to_owned(),
            minimum_cancel_l2_timestamp_seconds: basis.minimum_cancel_l2_timestamp_seconds(),
            barrier_state: BarrierStateV1::UnusableConservative,
            current_release_binary_sha256: current_runtime.release_binary_sha256.clone(),
            current_release_binary_length: current_runtime.release_binary_length,
            current_host: current_runtime.host.clone(),
            current_runtime_observed_at_utc: current_runtime.observed_at_utc.clone(),
            cleanup_not_after_utc: authorization.value().cleanup_not_after_utc.clone(),
            production_order_entry_authorized: false,
            real_order_submission_authorized: false,
            place_dispatch_allowance: 0,
            placement_resumption_allowed: false,
            scope_fingerprint: ZERO_FINGERPRINT.to_owned(),
        };
        value.scope_fingerprint = hash_domain(SCOPE_DOMAIN, &value)?;
        value.validate(config, authorization, original_scope, basis)?;
        Ok(value)
    }

    fn validate(
        &self,
        config: &CanonicalTrialConfig,
        authorization: &CanonicalAuthorization,
        original_scope: &PmTrialLiveJournalScopeV1,
        basis: &PmPhaseALiveRecoveryContinuationBasisV1,
    ) -> Result<(), PmTrialLiveJournalError> {
        self.original_intent_tail.validate()?;
        self.original_dispatch_terminal.validate()?;
        self.preserved_exposure.validate()?;
        self.consumed.validate()?;
        validate_order_id(&self.exact_venue_order_id)?;
        validate_fingerprint(&self.current_release_binary_sha256)?;
        validate_fingerprint(&self.scope_fingerprint)?;
        let runtime_at = validate_utc(&self.current_runtime_observed_at_utc)?;
        let original_at = validate_utc(&original_scope.runtime_observed_at_utc)?;
        let cleanup_at = validate_utc(&self.cleanup_not_after_utc)?;
        let authorization_value = authorization.value();
        if self.journal_family != FAMILY
            || self.journal_version != VERSION
            || self.intent_file != PM_PHASE_A_LIVE_CANCEL_RECOVERY_CONTINUATION_INTENT_FILE_V1
            || self.dispatch_file != PM_PHASE_A_LIVE_CANCEL_RECOVERY_CONTINUATION_DISPATCH_FILE_V1
            || self.canonical_config_fingerprint != config.fingerprint()
            || self.authorization_fingerprint != authorization.fingerprint()
            || self.original_scope_fingerprint != original_scope.scope_fingerprint
            || self.original_intent_tail != *basis.original_intent_tail()
            || self.original_dispatch_terminal != *basis.original_dispatch_terminal()
            || self.original_terminal_state
                != match basis.original_terminal_state() {
                    PmPhaseALiveOriginalTerminalStateV1::Paired => OriginalTerminalStateV1::Paired,
                    PmPhaseALiveOriginalTerminalStateV1::DispatchTerminalOnly => {
                        OriginalTerminalStateV1::DispatchTerminalOnly
                    }
                }
            || self.preserved_exposure != *basis.preserved_exposure()
            || self.exact_venue_order_id != basis.exact_venue_order_id()
            || self.minimum_cancel_l2_timestamp_seconds
                != basis.minimum_cancel_l2_timestamp_seconds()
            || self.barrier_state != BarrierStateV1::UnusableConservative
            || self.current_release_binary_sha256 != authorization_value.build.release_binary_sha256
            || self.current_release_binary_length != authorization_value.build.release_binary_length
            || self.current_host != authorization_value.host
            || self.current_release_binary_sha256 != original_scope.release_binary_sha256
            || self.current_release_binary_length != original_scope.release_binary_length
            || self.current_host != original_scope.host
            || self.cleanup_not_after_utc != authorization_value.cleanup_not_after_utc
            || runtime_at < original_at
            || runtime_at > cleanup_at
            || self.production_order_entry_authorized
            || self.real_order_submission_authorized
            || self.place_dispatch_allowance != 0
            || self.placement_resumption_allowed
        {
            return Err(PmTrialLiveJournalError::InvalidBinding);
        }
        let mut basis_value = self.clone();
        basis_value.scope_fingerprint = ZERO_FINGERPRINT.to_owned();
        if hash_domain(SCOPE_DOMAIN, &basis_value)? != self.scope_fingerprint {
            return Err(PmTrialLiveJournalError::InvalidBinding);
        }
        Ok(())
    }

    fn validate_current_runtime(
        &self,
        authorization: &CanonicalAuthorization,
        original_scope: &PmTrialLiveJournalScopeV1,
        current_runtime: &AuthorizationRuntimeBinding,
    ) -> Result<(), PmTrialLiveJournalError> {
        let current_at = validate_utc(&current_runtime.observed_at_utc)?;
        let created_at = validate_utc(&self.current_runtime_observed_at_utc)?;
        let cleanup_at = validate_utc(&self.cleanup_not_after_utc)?;
        let authorization_value = authorization.value();
        if current_runtime.release_binary_sha256 != authorization_value.build.release_binary_sha256
            || current_runtime.release_binary_length
                != authorization_value.build.release_binary_length
            || current_runtime.host != authorization_value.host
            || current_runtime.release_binary_sha256 != original_scope.release_binary_sha256
            || current_runtime.release_binary_length != original_scope.release_binary_length
            || current_runtime.host != original_scope.host
            || current_at < created_at
            || current_at > cleanup_at
        {
            return Err(PmTrialLiveJournalError::InvalidBinding);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ContinuationLinkV1 {
    sequence: u8,
    record_fingerprint: String,
}

impl ContinuationLinkV1 {
    fn validate(&self) -> Result<(), PmTrialLiveJournalError> {
        validate_fingerprint(&self.record_fingerprint)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "family", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum ContinuationDispatchTargetV1 {
    OriginalV1 {
        sequence: u8,
        record_fingerprint: String,
    },
    ContinuationV1 {
        sequence: u8,
        record_fingerprint: String,
    },
}

impl ContinuationDispatchTargetV1 {
    fn from_original(link: &CounterpartLinkV1) -> Self {
        Self::OriginalV1 {
            sequence: link.sequence,
            record_fingerprint: link.record_fingerprint.clone(),
        }
    }

    fn from_continuation(link: &ContinuationLinkV1) -> Self {
        Self::ContinuationV1 {
            sequence: link.sequence,
            record_fingerprint: link.record_fingerprint.clone(),
        }
    }

    fn validate(&self) -> Result<(), PmTrialLiveJournalError> {
        let fingerprint = match self {
            Self::OriginalV1 {
                record_fingerprint, ..
            }
            | Self::ContinuationV1 {
                record_fingerprint, ..
            } => record_fingerprint,
        };
        validate_fingerprint(fingerprint)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "record", rename_all = "snake_case", deny_unknown_fields)]
enum ContinuationIntentRecordV1 {
    Header {
        scope: Box<ContinuationScopeV1>,
    },
    Reconciliation {
        observed_at_utc: String,
        state: PmReconciliationOrderStateV1,
        exact_venue_order_id: Option<String>,
        dispatch: ContinuationDispatchTargetV1,
    },
    CancelIntent {
        created_at_utc: String,
        ownership_source: ContinuationLinkV1,
        exact_venue_order_id: String,
        dispatch_class: PmCancelDispatchClassV1,
    },
    CancelOutcomeBridge {
        dispatch: ContinuationLinkV1,
        outcome: PmCancelResultKindV1,
        exact_venue_order_id: String,
    },
    Terminal {
        terminal_at_utc: String,
        disposition: PmIntentTerminalDispositionV1,
        dispatch_terminal: ContinuationLinkV1,
        terminal_is_evidence_not_authority: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "record", rename_all = "snake_case", deny_unknown_fields)]
enum ContinuationDispatchRecordV1 {
    Header {
        scope_fingerprint: String,
        intent_header: ContinuationLinkV1,
    },
    CancelPrepared {
        intent: ContinuationLinkV1,
        dispatch_class: PmCancelDispatchClassV1,
        preparation: PmCancelPreparationV1,
        continuation_preparation_fingerprint: String,
    },
    CancelDispatchAuthorized {
        prepared_sequence: u8,
        prepared_record_fingerprint: String,
        dispatch_class: PmCancelDispatchClassV1,
        exact_venue_order_id: String,
        production_order_entry_authorized: bool,
        real_order_submission_authorized: bool,
        place_dispatch_allowance: u8,
    },
    CancelResult {
        dispatch_authorized_sequence: u8,
        dispatch_authorized_fingerprint: String,
        outcome: PmCancelResultKindV1,
        exact_venue_order_id: String,
    },
    Terminal {
        terminal_at_utc: String,
        disposition: PmIntentTerminalDispositionV1,
        intent: ContinuationLinkV1,
        terminal_is_evidence_not_authority: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ContinuationIntentLineV1 {
    schema_version: u32,
    sequence: u8,
    previous_record_fingerprint: String,
    scope_fingerprint: String,
    body: ContinuationIntentRecordV1,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ContinuationDispatchLineV1 {
    schema_version: u32,
    sequence: u8,
    previous_record_fingerprint: String,
    scope_fingerprint: String,
    body: ContinuationDispatchRecordV1,
}

fn intent_fingerprint(line: &ContinuationIntentLineV1) -> Result<String, PmTrialLiveJournalError> {
    hash_domain(INTENT_DOMAIN, line)
}

fn dispatch_fingerprint(
    line: &ContinuationDispatchLineV1,
) -> Result<String, PmTrialLiveJournalError> {
    hash_domain(DISPATCH_DOMAIN, line)
}

fn intent_link(
    line: &ContinuationIntentLineV1,
) -> Result<ContinuationLinkV1, PmTrialLiveJournalError> {
    Ok(ContinuationLinkV1 {
        sequence: line.sequence,
        record_fingerprint: intent_fingerprint(line)?,
    })
}

fn dispatch_link(
    line: &ContinuationDispatchLineV1,
) -> Result<ContinuationLinkV1, PmTrialLiveJournalError> {
    Ok(ContinuationLinkV1 {
        sequence: line.sequence,
        record_fingerprint: dispatch_fingerprint(line)?,
    })
}

#[derive(Clone, PartialEq, Eq)]
pub(crate) struct PmRecoveryContinuationProjectionV1 {
    scope: ContinuationScopeV1,
    original_scope: PmTrialLiveJournalScopeV1,
    preflight: PmTrialLivePreflightBindingV1,
    intent_bytes: Vec<u8>,
    dispatch_bytes: Vec<u8>,
    intent_lines: Vec<ContinuationIntentLineV1>,
    dispatch_lines: Vec<ContinuationDispatchLineV1>,
    pub(crate) latest_reconciliation: Option<(PmReconciliationOrderStateV1, Option<String>)>,
    pub(crate) latest_cancel_result: Option<PmCancelResultKindV1>,
    pub(crate) terminal: bool,
    pub(crate) terminal_prefix: bool,
}

#[derive(Clone, PartialEq, Eq)]
pub(crate) enum PmRecoveryContinuationLoadV1 {
    Absent,
    IntentZero,
    IntentHeaderDispatchAbsent,
    IntentHeaderDispatchZero,
    Complete(Box<PmRecoveryContinuationProjectionV1>),
}

impl PmRecoveryContinuationLoadV1 {
    pub(crate) const fn is_durable_attempt(&self) -> bool {
        !matches!(self, Self::Absent)
    }

    pub(crate) fn complete(&self) -> Option<&PmRecoveryContinuationProjectionV1> {
        match self {
            Self::Complete(value) => Some(value.as_ref()),
            _ => None,
        }
    }
}

impl PmRecoveryContinuationProjectionV1 {
    pub(crate) fn exact_venue_order_id(&self) -> &str {
        &self.scope.exact_venue_order_id
    }

    pub(crate) fn validate_against_consumption_registry(
        &self,
        registry: Option<&AuthorizationRecoveryContinuationRegistryV1>,
    ) -> Result<(), PmTrialLiveJournalError> {
        validate_consumption_registry(
            &self.scope,
            &self.original_scope,
            &self.preflight,
            &self.intent_lines,
            &self.dispatch_lines,
            registry,
        )
    }
}

pub(crate) fn bound_paths(config: &CanonicalTrialConfig) -> (PathBuf, PathBuf) {
    let parent = PathBuf::from(&config.value().journal.artifact_directory);
    (
        parent.join(PM_PHASE_A_LIVE_CANCEL_RECOVERY_CONTINUATION_INTENT_FILE_V1),
        parent.join(PM_PHASE_A_LIVE_CANCEL_RECOVERY_CONTINUATION_DISPATCH_FILE_V1),
    )
}

pub(crate) fn any_file_present(
    config: &CanonicalTrialConfig,
) -> Result<bool, PmTrialLiveJournalError> {
    let (intent, dispatch) = bound_paths(config);
    Ok(read_optional(&intent)?.is_some() || read_optional(&dispatch)?.is_some())
}

pub(crate) fn load_optional(
    config: &CanonicalTrialConfig,
    authorization: &CanonicalAuthorization,
    original_scope: &PmTrialLiveJournalScopeV1,
    basis: &PmPhaseALiveRecoveryContinuationBasisV1,
    consumed: &PmTrialLiveConsumedFingerprintsV1,
    preflight: &PmTrialLivePreflightBindingV1,
) -> Result<PmRecoveryContinuationLoadV1, PmTrialLiveJournalError> {
    let (intent_path, dispatch_path) = bound_paths(config);
    let intent = read_optional(&intent_path)?;
    let dispatch = read_optional(&dispatch_path)?;
    match (intent, dispatch) {
        (None, None) => Ok(PmRecoveryContinuationLoadV1::Absent),
        (Some(intent_bytes), None) if intent_bytes.is_empty() => {
            Ok(PmRecoveryContinuationLoadV1::IntentZero)
        }
        (Some(intent_bytes), None) => {
            validate_intent_header_prefix(
                config,
                authorization,
                original_scope,
                basis,
                consumed,
                &intent_bytes,
            )?;
            Ok(PmRecoveryContinuationLoadV1::IntentHeaderDispatchAbsent)
        }
        (Some(intent_bytes), Some(dispatch_bytes))
            if !intent_bytes.is_empty() && dispatch_bytes.is_empty() =>
        {
            validate_intent_header_prefix(
                config,
                authorization,
                original_scope,
                basis,
                consumed,
                &intent_bytes,
            )?;
            Ok(PmRecoveryContinuationLoadV1::IntentHeaderDispatchZero)
        }
        (Some(intent_bytes), Some(dispatch_bytes))
            if !intent_bytes.is_empty() && !dispatch_bytes.is_empty() =>
        {
            let projection = parse_and_validate(
                config,
                authorization,
                original_scope,
                basis,
                preflight,
                intent_bytes,
                dispatch_bytes,
            )?;
            if &projection.scope.consumed != consumed {
                return Err(PmTrialLiveJournalError::InvalidBinding);
            }
            Ok(PmRecoveryContinuationLoadV1::Complete(Box::new(projection)))
        }
        _ => Err(PmTrialLiveJournalError::AmbiguousTail),
    }
}

fn validate_intent_header_prefix(
    config: &CanonicalTrialConfig,
    authorization: &CanonicalAuthorization,
    original_scope: &PmTrialLiveJournalScopeV1,
    basis: &PmPhaseALiveRecoveryContinuationBasisV1,
    consumed: &PmTrialLiveConsumedFingerprintsV1,
    intent_bytes: &[u8],
) -> Result<(), PmTrialLiveJournalError> {
    let lines: Vec<ContinuationIntentLineV1> = parse_lines(intent_bytes)?;
    if lines.len() != 1 {
        return Err(PmTrialLiveJournalError::AmbiguousTail);
    }
    let ContinuationIntentRecordV1::Header { scope } = &lines[0].body else {
        return Err(PmTrialLiveJournalError::InvalidRecord);
    };
    scope.validate(config, authorization, original_scope, basis)?;
    if &scope.consumed != consumed
        || lines[0].schema_version != VERSION
        || lines[0].sequence != 0
        || lines[0].previous_record_fingerprint != ZERO_FINGERPRINT
        || lines[0].scope_fingerprint != scope.scope_fingerprint
    {
        return Err(PmTrialLiveJournalError::InvalidBinding);
    }
    intent_fingerprint(&lines[0]).map(|_| ())
}

fn read_optional(path: &Path) -> Result<Option<Vec<u8>>, PmTrialLiveJournalError> {
    match read_protected(path, MAX_JOURNAL_BYTES) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(PmTrialLiveJournalError::Absent) => Ok(None),
        Err(error) => Err(error),
    }
}

fn parse_and_validate(
    config: &CanonicalTrialConfig,
    authorization: &CanonicalAuthorization,
    original_scope: &PmTrialLiveJournalScopeV1,
    basis: &PmPhaseALiveRecoveryContinuationBasisV1,
    preflight: &PmTrialLivePreflightBindingV1,
    intent_bytes: Vec<u8>,
    dispatch_bytes: Vec<u8>,
) -> Result<PmRecoveryContinuationProjectionV1, PmTrialLiveJournalError> {
    let intent_lines: Vec<ContinuationIntentLineV1> = parse_lines(&intent_bytes)?;
    let dispatch_lines: Vec<ContinuationDispatchLineV1> = parse_lines(&dispatch_bytes)?;
    let scope = match &intent_lines
        .first()
        .ok_or(PmTrialLiveJournalError::InvalidRecord)?
        .body
    {
        ContinuationIntentRecordV1::Header { scope } => scope.as_ref().clone(),
        _ => return Err(PmTrialLiveJournalError::InvalidRecord),
    };
    scope.validate(config, authorization, original_scope, basis)?;
    validate_chains(&scope, &intent_lines, &dispatch_lines)?;
    validate_records(
        &scope,
        original_scope,
        preflight,
        &intent_lines,
        &dispatch_lines,
    )
    .map(|facts| PmRecoveryContinuationProjectionV1 {
        scope,
        original_scope: original_scope.clone(),
        preflight: preflight.clone(),
        intent_bytes,
        dispatch_bytes,
        intent_lines,
        dispatch_lines,
        latest_reconciliation: facts.latest_reconciliation,
        latest_cancel_result: facts.latest_cancel_result,
        terminal: facts.terminal,
        terminal_prefix: facts.terminal_prefix,
    })
}

struct ContinuationFactsV1 {
    latest_reconciliation: Option<(PmReconciliationOrderStateV1, Option<String>)>,
    latest_cancel_result: Option<PmCancelResultKindV1>,
    terminal: bool,
    terminal_prefix: bool,
}

struct DispatchCycleV1 {
    intent: ContinuationLinkV1,
    dispatch_class: PmCancelDispatchClassV1,
    dispatch_authorized: Option<ContinuationLinkV1>,
    result: Option<(ContinuationLinkV1, PmCancelResultKindV1)>,
}

pub(crate) struct ContinuationOutcomeResumeV1 {
    pub(crate) bridge: ContinuationAckV1,
    pub(crate) result: ContinuationAckV1,
    pub(crate) exact_venue_order_id: String,
}

struct ContinuationPreparedAnchorEvidenceV1 {
    recovery_ordinal: u8,
    intent_sequence: u8,
    intent_record_fingerprint: String,
    prepared_sequence: u8,
    dispatch_previous_record_fingerprint: String,
    prepared_record_fingerprint: String,
    prepared_record_canonical_json: String,
    exact_venue_order_id: String,
    semantic_request_commitment_sha256: String,
    l2_timestamp_seconds: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ContinuationTerminalPlanPhysicalStateV1 {
    MissingBoth,
    DispatchOnly,
    Complete,
}

struct ContinuationTerminalPlanLinesV1 {
    dispatch: ContinuationDispatchLineV1,
    intent: ContinuationIntentLineV1,
    physical_state: ContinuationTerminalPlanPhysicalStateV1,
}

fn prepared_anchor_evidence(
    dispatch: &[ContinuationDispatchLineV1],
) -> Result<Vec<ContinuationPreparedAnchorEvidenceV1>, PmTrialLiveJournalError> {
    dispatch
        .iter()
        .filter_map(|line| match &line.body {
            ContinuationDispatchRecordV1::CancelPrepared {
                intent,
                dispatch_class: PmCancelDispatchClassV1::Recovery { ordinal },
                preparation,
                ..
            } => Some((line, intent, *ordinal, preparation)),
            ContinuationDispatchRecordV1::CancelPrepared { .. } => None,
            _ => None,
        })
        .map(|(line, intent, recovery_ordinal, preparation)| {
            let view = preparation.view(PmCancelDispatchClassV1::Recovery {
                ordinal: recovery_ordinal,
            })?;
            Ok(ContinuationPreparedAnchorEvidenceV1 {
                recovery_ordinal,
                intent_sequence: intent.sequence,
                intent_record_fingerprint: intent.record_fingerprint.clone(),
                prepared_sequence: line.sequence,
                dispatch_previous_record_fingerprint: line.previous_record_fingerprint.clone(),
                prepared_record_fingerprint: dispatch_fingerprint(line)?,
                prepared_record_canonical_json: String::from_utf8(canonical_json(line)?)
                    .map_err(|_| PmTrialLiveJournalError::InvalidRecord)?,
                exact_venue_order_id: preparation.exact_venue_order_id().to_owned(),
                semantic_request_commitment_sha256: lower_hex32(
                    view.semantic_request_commitment().bytes(),
                ),
                l2_timestamp_seconds: preparation.l2_timestamp_seconds(),
            })
        })
        .collect()
}

fn authorization_terminal_disposition(
    disposition: PmIntentTerminalDispositionV1,
) -> AuthorizationTerminalDisposition {
    match disposition {
        PmIntentTerminalDispositionV1::Completed => AuthorizationTerminalDisposition::Completed,
        PmIntentTerminalDispositionV1::Stopped => AuthorizationTerminalDisposition::Stopped,
        PmIntentTerminalDispositionV1::OperatorActionRequired => {
            AuthorizationTerminalDisposition::OperatorActionRequired
        }
    }
}

fn reconstruct_terminal_plan_lines(
    scope: &ContinuationScopeV1,
    original_scope: &PmTrialLiveJournalScopeV1,
    preflight: &PmTrialLivePreflightBindingV1,
    intent: &[ContinuationIntentLineV1],
    dispatch: &[ContinuationDispatchLineV1],
    plan: &AuthorizationRecoveryTerminalPlanV1,
) -> Result<ContinuationTerminalPlanLinesV1, PmTrialLiveJournalError> {
    let dispatch_canonical = plan
        .continuation_dispatch_terminal_record_canonical_json
        .as_bytes();
    let dispatch_terminal: ContinuationDispatchLineV1 = serde_json::from_slice(dispatch_canonical)
        .map_err(|_| PmTrialLiveJournalError::InvalidRecord)?;
    let intent_canonical = plan
        .continuation_intent_terminal_record_canonical_json
        .as_bytes();
    let intent_terminal: ContinuationIntentLineV1 = serde_json::from_slice(intent_canonical)
        .map_err(|_| PmTrialLiveJournalError::InvalidRecord)?;
    if canonical_json(&dispatch_terminal)? != dispatch_canonical
        || canonical_json(&intent_terminal)? != intent_canonical
        || plan.continuation_scope_fingerprint != scope.scope_fingerprint
        || dispatch_terminal.schema_version != VERSION
        || dispatch_terminal.scope_fingerprint != scope.scope_fingerprint
        || dispatch_terminal.sequence != plan.continuation_dispatch_terminal_sequence
        || dispatch_terminal.previous_record_fingerprint
            != plan.continuation_dispatch_terminal_previous_record_fingerprint
        || dispatch_fingerprint(&dispatch_terminal)?
            != plan.continuation_dispatch_terminal_record_fingerprint
        || intent_terminal.schema_version != VERSION
        || intent_terminal.scope_fingerprint != scope.scope_fingerprint
        || intent_terminal.sequence != plan.continuation_intent_terminal_sequence
        || intent_terminal.previous_record_fingerprint
            != plan.continuation_intent_terminal_previous_record_fingerprint
        || intent_fingerprint(&intent_terminal)?
            != plan.continuation_intent_terminal_record_fingerprint
    {
        return Err(PmTrialLiveJournalError::InvalidRecord);
    }
    let intent_predecessor = intent
        .get(usize::from(plan.continuation_intent_predecessor_sequence))
        .ok_or(PmTrialLiveJournalError::InvalidRecord)?;
    let dispatch_predecessor = dispatch
        .get(usize::from(plan.continuation_dispatch_predecessor_sequence))
        .ok_or(PmTrialLiveJournalError::InvalidRecord)?;
    let intent_predecessor_link = intent_link(intent_predecessor)?;
    let dispatch_predecessor_link = dispatch_link(dispatch_predecessor)?;
    let dispatch_terminal_link = dispatch_link(&dispatch_terminal)?;
    if intent_predecessor_link.sequence != plan.continuation_intent_predecessor_sequence
        || intent_predecessor_link.record_fingerprint
            != plan.continuation_intent_predecessor_record_fingerprint
        || dispatch_predecessor_link.sequence != plan.continuation_dispatch_predecessor_sequence
        || dispatch_predecessor_link.record_fingerprint
            != plan.continuation_dispatch_predecessor_record_fingerprint
        || plan.continuation_dispatch_terminal_sequence
            != plan
                .continuation_dispatch_predecessor_sequence
                .checked_add(1)
                .ok_or(PmTrialLiveJournalError::BoundExceeded)?
        || plan.continuation_intent_terminal_sequence
            != plan
                .continuation_intent_predecessor_sequence
                .checked_add(1)
                .ok_or(PmTrialLiveJournalError::BoundExceeded)?
        || plan.continuation_dispatch_terminal_previous_record_fingerprint
            != plan.continuation_dispatch_predecessor_record_fingerprint
        || plan.continuation_intent_terminal_previous_record_fingerprint
            != plan.continuation_intent_predecessor_record_fingerprint
        || !matches!(
            &dispatch_terminal.body,
            ContinuationDispatchRecordV1::Terminal {
                terminal_at_utc,
                disposition,
                intent,
                terminal_is_evidence_not_authority: true,
            } if terminal_at_utc == &plan.terminal_at_utc
                && authorization_terminal_disposition(*disposition) == plan.disposition
                && intent == &intent_predecessor_link
        )
        || !matches!(
            &intent_terminal.body,
            ContinuationIntentRecordV1::Terminal {
                terminal_at_utc,
                disposition,
                dispatch_terminal: target,
                terminal_is_evidence_not_authority: true,
            } if terminal_at_utc == &plan.terminal_at_utc
                && authorization_terminal_disposition(*disposition) == plan.disposition
                && target == &dispatch_terminal_link
        )
    {
        return Err(PmTrialLiveJournalError::InvalidRecord);
    }
    validate_utc(&plan.terminal_at_utc)?;
    let dispatch_missing_len = usize::from(plan.continuation_dispatch_terminal_sequence);
    let intent_missing_len = usize::from(plan.continuation_intent_terminal_sequence);
    let physical_state = match (dispatch.len(), intent.len()) {
        (dispatch_len, intent_len)
            if dispatch_len == dispatch_missing_len && intent_len == intent_missing_len =>
        {
            ContinuationTerminalPlanPhysicalStateV1::MissingBoth
        }
        (dispatch_len, intent_len)
            if dispatch_len == dispatch_missing_len + 1 && intent_len == intent_missing_len =>
        {
            if dispatch.last() != Some(&dispatch_terminal) {
                return Err(PmTrialLiveJournalError::InvalidRecord);
            }
            ContinuationTerminalPlanPhysicalStateV1::DispatchOnly
        }
        (dispatch_len, intent_len)
            if dispatch_len == dispatch_missing_len + 1 && intent_len == intent_missing_len + 1 =>
        {
            if dispatch.last() != Some(&dispatch_terminal)
                || intent.last() != Some(&intent_terminal)
            {
                return Err(PmTrialLiveJournalError::InvalidRecord);
            }
            ContinuationTerminalPlanPhysicalStateV1::Complete
        }
        _ => return Err(PmTrialLiveJournalError::InvalidRecord),
    };
    let mut reconstructed_intent = intent.to_vec();
    let mut reconstructed_dispatch = dispatch.to_vec();
    if physical_state == ContinuationTerminalPlanPhysicalStateV1::MissingBoth {
        reconstructed_dispatch.push(dispatch_terminal.clone());
    }
    if physical_state != ContinuationTerminalPlanPhysicalStateV1::Complete {
        reconstructed_intent.push(intent_terminal.clone());
    }
    validate_chains(scope, &reconstructed_intent, &reconstructed_dispatch)?;
    let facts = validate_records(
        scope,
        original_scope,
        preflight,
        &reconstructed_intent,
        &reconstructed_dispatch,
    )?;
    if !facts.terminal || facts.terminal_prefix {
        return Err(PmTrialLiveJournalError::InvalidRecord);
    }
    Ok(ContinuationTerminalPlanLinesV1 {
        dispatch: dispatch_terminal,
        intent: intent_terminal,
        physical_state,
    })
}

fn validate_consumption_registry(
    scope: &ContinuationScopeV1,
    original_scope: &PmTrialLiveJournalScopeV1,
    preflight: &PmTrialLivePreflightBindingV1,
    intent: &[ContinuationIntentLineV1],
    dispatch: &[ContinuationDispatchLineV1],
    registry: Option<&AuthorizationRecoveryContinuationRegistryV1>,
) -> Result<(), PmTrialLiveJournalError> {
    let intent_header_fingerprint = intent_fingerprint(
        intent
            .first()
            .ok_or(PmTrialLiveJournalError::InvalidRecord)?,
    )?;
    let dispatch_header_fingerprint = dispatch_fingerprint(
        dispatch
            .first()
            .ok_or(PmTrialLiveJournalError::InvalidRecord)?,
    )?;
    let durable_prepared = prepared_anchor_evidence(dispatch)?;
    let Some(registry) = registry else {
        // The only pair-before-root crash prefix is the exact two-header pair.
        // No transition is admitted until its root is independently anchored.
        return (intent.len() == 1 && dispatch.len() == 1 && durable_prepared.is_empty())
            .then_some(())
            .ok_or(PmTrialLiveJournalError::InvalidRecord);
    };
    if registry.root.continuation_scope_fingerprint != scope.scope_fingerprint
        || registry.root.continuation_intent_header_fingerprint != intent_header_fingerprint
        || registry.root.continuation_dispatch_header_fingerprint != dispatch_header_fingerprint
        || durable_prepared.len() > registry.prepared.len()
        || registry.prepared.len() > durable_prepared.len().saturating_add(1)
    {
        return Err(PmTrialLiveJournalError::InvalidRecord);
    }
    // Prepared ordinals linearize in the monotonic ledger first. Therefore a
    // pair may never lead the registry; the sole recoverable mismatch is one
    // registry anchor ahead of the exact current dispatch tail.
    for (anchor, durable) in registry.prepared.iter().zip(&durable_prepared) {
        if anchor.continuation_scope_fingerprint != scope.scope_fingerprint
            || anchor.recovery_ordinal != durable.recovery_ordinal
            || anchor.continuation_intent_sequence != durable.intent_sequence
            || anchor.continuation_intent_record_fingerprint != durable.intent_record_fingerprint
            || anchor.continuation_prepared_sequence != durable.prepared_sequence
            || anchor.continuation_dispatch_previous_record_fingerprint
                != durable.dispatch_previous_record_fingerprint
            || anchor.continuation_prepared_record_fingerprint
                != durable.prepared_record_fingerprint
            || anchor.continuation_prepared_record_canonical_json
                != durable.prepared_record_canonical_json
            || anchor.exact_venue_order_id != durable.exact_venue_order_id
            || anchor.semantic_request_commitment_sha256
                != durable.semantic_request_commitment_sha256
            || anchor.l2_timestamp_seconds != durable.l2_timestamp_seconds
        {
            return Err(PmTrialLiveJournalError::InvalidRecord);
        }
    }
    if registry.prepared.len() == durable_prepared.len().saturating_add(1) {
        reconstruct_anchored_prepared_line(
            scope,
            original_scope,
            preflight,
            intent,
            dispatch,
            registry
                .prepared
                .last()
                .ok_or(PmTrialLiveJournalError::InvalidRecord)?,
        )?;
    }
    let physical_terminal = matches!(
        dispatch.last().map(|line| &line.body),
        Some(ContinuationDispatchRecordV1::Terminal { .. })
    ) || matches!(
        intent.last().map(|line| &line.body),
        Some(ContinuationIntentRecordV1::Terminal { .. })
    );
    match &registry.terminal_plan {
        None if physical_terminal => return Err(PmTrialLiveJournalError::InvalidRecord),
        None => {}
        Some(plan) => {
            if registry.prepared.len() != durable_prepared.len() {
                return Err(PmTrialLiveJournalError::InvalidRecord);
            }
            reconstruct_terminal_plan_lines(
                scope,
                original_scope,
                preflight,
                intent,
                dispatch,
                plan,
            )?;
        }
    }
    Ok(())
}

fn reconstruct_anchored_prepared_line(
    scope: &ContinuationScopeV1,
    original_scope: &PmTrialLiveJournalScopeV1,
    preflight: &PmTrialLivePreflightBindingV1,
    intent: &[ContinuationIntentLineV1],
    dispatch: &[ContinuationDispatchLineV1],
    anchor: &AuthorizationRecoveryCancelPreparedAnchorV1,
) -> Result<ContinuationDispatchLineV1, PmTrialLiveJournalError> {
    let canonical = anchor
        .continuation_prepared_record_canonical_json
        .as_bytes();
    let line: ContinuationDispatchLineV1 =
        serde_json::from_slice(canonical).map_err(|_| PmTrialLiveJournalError::InvalidRecord)?;
    if canonical_json(&line)? != canonical {
        return Err(PmTrialLiveJournalError::InvalidRecord);
    }
    let expected_sequence =
        u8::try_from(dispatch.len()).map_err(|_| PmTrialLiveJournalError::BoundExceeded)?;
    let expected_previous = dispatch
        .last()
        .map(dispatch_fingerprint)
        .transpose()?
        .ok_or(PmTrialLiveJournalError::InvalidRecord)?;
    let latest_intent = intent_link(
        intent
            .last()
            .ok_or(PmTrialLiveJournalError::InvalidRecord)?,
    )?;
    let ContinuationDispatchRecordV1::CancelPrepared {
        intent: prepared_intent,
        dispatch_class: PmCancelDispatchClassV1::Recovery { ordinal },
        preparation,
        ..
    } = &line.body
    else {
        return Err(PmTrialLiveJournalError::InvalidRecord);
    };
    let view = preparation.view(PmCancelDispatchClassV1::Recovery { ordinal: *ordinal })?;
    if line.schema_version != VERSION
        || line.scope_fingerprint != scope.scope_fingerprint
        || line.sequence != expected_sequence
        || line.sequence != anchor.continuation_prepared_sequence
        || line.previous_record_fingerprint != expected_previous
        || line.previous_record_fingerprint
            != anchor.continuation_dispatch_previous_record_fingerprint
        || dispatch_fingerprint(&line)? != anchor.continuation_prepared_record_fingerprint
        || prepared_intent != &latest_intent
        || prepared_intent.sequence != anchor.continuation_intent_sequence
        || prepared_intent.record_fingerprint != anchor.continuation_intent_record_fingerprint
        || *ordinal != anchor.recovery_ordinal
        || preparation.exact_venue_order_id() != anchor.exact_venue_order_id
        || preparation.exact_venue_order_id() != scope.exact_venue_order_id
        || lower_hex32(view.semantic_request_commitment().bytes())
            != anchor.semantic_request_commitment_sha256
        || preparation.l2_timestamp_seconds() != anchor.l2_timestamp_seconds
    {
        return Err(PmTrialLiveJournalError::InvalidRecord);
    }
    let mut reconstructed = dispatch.to_vec();
    reconstructed.push(line.clone());
    validate_chains(scope, intent, &reconstructed)?;
    validate_records(scope, original_scope, preflight, intent, &reconstructed)?;
    Ok(line)
}

fn lower_hex32(bytes: [u8; 32]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(64);
    for byte in bytes {
        output.push(char::from(HEX[usize::from(byte >> 4)]));
        output.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    output
}

fn validate_records(
    scope: &ContinuationScopeV1,
    original_scope: &PmTrialLiveJournalScopeV1,
    preflight: &PmTrialLivePreflightBindingV1,
    intent: &[ContinuationIntentLineV1],
    dispatch: &[ContinuationDispatchLineV1],
) -> Result<ContinuationFactsV1, PmTrialLiveJournalError> {
    let header_link = intent_link(&intent[0])?;
    let ContinuationDispatchRecordV1::Header {
        scope_fingerprint,
        intent_header,
    } = &dispatch[0].body
    else {
        return Err(PmTrialLiveJournalError::InvalidRecord);
    };
    if scope_fingerprint != &scope.scope_fingerprint || intent_header != &header_link {
        return Err(PmTrialLiveJournalError::InvalidRecord);
    }

    // First validate the dispatch-side attempts in their exact durable order.
    // Every CancelPrepared burns its recovery ordinal even when it has no
    // authorized child. Primary is structurally absent from this family.
    let mut cycles = Vec::<DispatchCycleV1>::new();
    let mut prepared_classes = Vec::new();
    let mut active_cycle = None;
    let mut dispatch_terminal = None;
    for (index, line) in dispatch.iter().enumerate().skip(1) {
        match &line.body {
            ContinuationDispatchRecordV1::Header { .. } => {
                return Err(PmTrialLiveJournalError::InvalidRecord);
            }
            ContinuationDispatchRecordV1::CancelPrepared {
                intent: target,
                dispatch_class,
                preparation,
                continuation_preparation_fingerprint,
            } => {
                let PmCancelDispatchClassV1::Recovery { .. } = dispatch_class else {
                    return Err(PmTrialLiveJournalError::InvalidRecord);
                };
                let intent_line = continuation_intent_line(target, intent)?;
                let ContinuationIntentRecordV1::CancelIntent {
                    exact_venue_order_id,
                    dispatch_class: intent_class,
                    ..
                } = &intent_line.body
                else {
                    return Err(PmTrialLiveJournalError::InvalidRecord);
                };
                if intent_class != dispatch_class
                    || exact_venue_order_id != preparation.exact_venue_order_id()
                    || exact_venue_order_id != &scope.exact_venue_order_id
                    || cycles.iter().any(|cycle| cycle.intent == *target)
                {
                    return Err(PmTrialLiveJournalError::InvalidRecord);
                }
                validate_next_class(original_scope, *dispatch_class, &prepared_classes)?;
                let minimum_l2 = latest_preparation_l2(dispatch, index, scope);
                preparation.validate_against_scope(
                    original_scope,
                    preflight,
                    target.sequence,
                    minimum_l2,
                )?;
                if preparation_fingerprint(
                    &scope.scope_fingerprint,
                    target,
                    *dispatch_class,
                    preparation,
                )? != *continuation_preparation_fingerprint
                {
                    return Err(PmTrialLiveJournalError::InvalidRecord);
                }
                prepared_classes.push(*dispatch_class);
                cycles.push(DispatchCycleV1 {
                    intent: target.clone(),
                    dispatch_class: *dispatch_class,
                    dispatch_authorized: None,
                    result: None,
                });
                active_cycle = Some(cycles.len() - 1);
            }
            ContinuationDispatchRecordV1::CancelDispatchAuthorized {
                prepared_sequence,
                prepared_record_fingerprint,
                dispatch_class,
                exact_venue_order_id,
                production_order_entry_authorized,
                real_order_submission_authorized,
                place_dispatch_allowance,
            } => {
                let cycle = active_cycle
                    .and_then(|cycle| cycles.get_mut(cycle))
                    .ok_or(PmTrialLiveJournalError::InvalidRecord)?;
                let prepared = dispatch
                    .get(usize::from(*prepared_sequence))
                    .ok_or(PmTrialLiveJournalError::InvalidRecord)?;
                if index == 0
                    || usize::from(*prepared_sequence) != index - 1
                    || dispatch_fingerprint(prepared)? != *prepared_record_fingerprint
                    || cycle.dispatch_authorized.is_some()
                    || *production_order_entry_authorized
                    || *real_order_submission_authorized
                    || *place_dispatch_allowance != 0
                    || *dispatch_class != cycle.dispatch_class
                    || exact_venue_order_id != &scope.exact_venue_order_id
                    || !matches!(
                        &prepared.body,
                        ContinuationDispatchRecordV1::CancelPrepared {
                            dispatch_class: prepared_class,
                            preparation,
                            ..
                        } if prepared_class == dispatch_class
                            && preparation.exact_venue_order_id() == exact_venue_order_id
                    )
                {
                    return Err(PmTrialLiveJournalError::InvalidRecord);
                }
                cycle.dispatch_authorized = Some(dispatch_link(line)?);
            }
            ContinuationDispatchRecordV1::CancelResult {
                dispatch_authorized_sequence,
                dispatch_authorized_fingerprint,
                outcome,
                exact_venue_order_id,
            } => {
                let cycle = active_cycle
                    .and_then(|cycle| cycles.get_mut(cycle))
                    .ok_or(PmTrialLiveJournalError::InvalidRecord)?;
                let authorized = dispatch
                    .get(usize::from(*dispatch_authorized_sequence))
                    .ok_or(PmTrialLiveJournalError::InvalidRecord)?;
                if index == 0
                    || usize::from(*dispatch_authorized_sequence) != index - 1
                    || dispatch_fingerprint(authorized)? != *dispatch_authorized_fingerprint
                    || cycle.result.is_some()
                    || cycle.dispatch_authorized.as_ref() != Some(&dispatch_link(authorized)?)
                    || exact_venue_order_id != &scope.exact_venue_order_id
                    || !matches!(
                        &authorized.body,
                        ContinuationDispatchRecordV1::CancelDispatchAuthorized {
                            exact_venue_order_id: dispatched_id,
                            ..
                        } if dispatched_id == exact_venue_order_id
                    )
                {
                    return Err(PmTrialLiveJournalError::InvalidRecord);
                }
                cycle.result = Some((dispatch_link(line)?, *outcome));
            }
            ContinuationDispatchRecordV1::Terminal {
                terminal_at_utc,
                disposition,
                intent: target,
                terminal_is_evidence_not_authority,
            } => {
                validate_utc(terminal_at_utc)?;
                if index + 1 != dispatch.len()
                    || !terminal_is_evidence_not_authority
                    || dispatch_terminal.is_some()
                {
                    return Err(PmTrialLiveJournalError::InvalidRecord);
                }
                dispatch_terminal = Some((
                    terminal_at_utc.clone(),
                    *disposition,
                    target.clone(),
                    dispatch_link(line)?,
                ));
            }
        }
    }

    // Replay the intent side causally. A reconciliation must target the exact
    // current exposure. Its ExactLive ownership can be consumed only by the
    // next cancel intent, and a prepared/dispatch/result chain updates that
    // exposure before any subsequent bridge or reconciliation.
    let mut expected_target =
        ContinuationDispatchTargetV1::from_original(&scope.preserved_exposure);
    let mut exposure_reconciled = false;
    let mut latest_reconciliation = None;
    let mut latest_reconciliation_link = None;
    let mut used_ownership_sources = HashSet::new();
    let mut used_cycles = HashSet::new();
    let mut bridged_results = HashSet::new();
    let mut pending_result_bridge = None;
    let mut last_cycle_index = None;
    let mut intent_terminal = None;
    for (index, line) in intent.iter().enumerate().skip(1) {
        match &line.body {
            ContinuationIntentRecordV1::Header { .. } => {
                return Err(PmTrialLiveJournalError::InvalidRecord);
            }
            ContinuationIntentRecordV1::Reconciliation {
                observed_at_utc,
                state,
                exact_venue_order_id,
                dispatch: target,
            } => {
                validate_utc(observed_at_utc)?;
                validate_reconciliation(scope, *state, exact_venue_order_id.as_deref())?;
                validate_target(scope, target, dispatch)?;
                if target != &expected_target || pending_result_bridge.is_some() {
                    return Err(PmTrialLiveJournalError::InvalidRecord);
                }
                latest_reconciliation = Some((*state, exact_venue_order_id.clone()));
                latest_reconciliation_link = Some(intent_link(line)?);
                exposure_reconciled = true;
            }
            ContinuationIntentRecordV1::CancelIntent {
                created_at_utc,
                ownership_source,
                exact_venue_order_id,
                dispatch_class,
            } => {
                validate_utc(created_at_utc)?;
                validate_order_id(exact_venue_order_id)?;
                let PmCancelDispatchClassV1::Recovery { .. } = dispatch_class else {
                    return Err(PmTrialLiveJournalError::InvalidRecord);
                };
                let source = continuation_intent_line(ownership_source, intent)?;
                if pending_result_bridge.is_some()
                    || latest_reconciliation_link.as_ref() != Some(ownership_source)
                    || !used_ownership_sources.insert(ownership_source.sequence)
                    || exact_venue_order_id != &scope.exact_venue_order_id
                    || !matches!(
                        &source.body,
                        ContinuationIntentRecordV1::Reconciliation {
                            state: PmReconciliationOrderStateV1::ExactLive,
                            exact_venue_order_id: Some(reconciled_id),
                            dispatch,
                            ..
                        } if reconciled_id == exact_venue_order_id
                            && dispatch == &expected_target
                    )
                {
                    return Err(PmTrialLiveJournalError::InvalidRecord);
                }
                let link = intent_link(line)?;
                if let Some((cycle_index, cycle)) = cycles
                    .iter()
                    .enumerate()
                    .find(|(_, cycle)| cycle.intent == link)
                {
                    if cycle.dispatch_class != *dispatch_class
                        || !used_cycles.insert(cycle_index)
                        || last_cycle_index.is_some_and(|prior| cycle_index <= prior)
                    {
                        return Err(PmTrialLiveJournalError::InvalidRecord);
                    }
                    last_cycle_index = Some(cycle_index);
                    if let Some((result, _)) = &cycle.result {
                        expected_target = ContinuationDispatchTargetV1::from_continuation(result);
                        exposure_reconciled = false;
                        pending_result_bridge = Some(result.clone());
                    } else if let Some(dispatched) = &cycle.dispatch_authorized {
                        expected_target =
                            ContinuationDispatchTargetV1::from_continuation(dispatched);
                        exposure_reconciled = false;
                    }
                }
            }
            ContinuationIntentRecordV1::CancelOutcomeBridge {
                dispatch: target,
                outcome,
                exact_venue_order_id,
            } => {
                validate_order_id(exact_venue_order_id)?;
                let target_line = continuation_dispatch_line(target, dispatch)?;
                if exact_venue_order_id != &scope.exact_venue_order_id
                    || exposure_reconciled
                    || pending_result_bridge.as_ref() != Some(target)
                    || !bridged_results.insert(target.sequence)
                    || expected_target != ContinuationDispatchTargetV1::from_continuation(target)
                    || !matches!(
                        &target_line.body,
                        ContinuationDispatchRecordV1::CancelResult {
                            outcome: recorded_outcome,
                            exact_venue_order_id: recorded_id,
                            ..
                        } if recorded_outcome == outcome
                            && recorded_id == exact_venue_order_id
                    )
                {
                    return Err(PmTrialLiveJournalError::InvalidRecord);
                }
                pending_result_bridge = None;
            }
            ContinuationIntentRecordV1::Terminal {
                terminal_at_utc,
                disposition,
                dispatch_terminal,
                terminal_is_evidence_not_authority,
            } => {
                validate_utc(terminal_at_utc)?;
                if index + 1 != intent.len()
                    || !terminal_is_evidence_not_authority
                    || pending_result_bridge.is_some()
                    || !latest_reconciliation_is_safe(
                        latest_reconciliation.as_ref(),
                        latest_reconciliation_link.as_ref(),
                        intent,
                        &expected_target,
                    )?
                {
                    return Err(PmTrialLiveJournalError::InvalidRecord);
                }
                intent_terminal = Some((
                    terminal_at_utc.clone(),
                    *disposition,
                    dispatch_terminal.clone(),
                ));
            }
        }
    }
    if used_cycles.len() != cycles.len() {
        return Err(PmTrialLiveJournalError::InvalidRecord);
    }

    let terminal_safe = latest_reconciliation_is_safe(
        latest_reconciliation.as_ref(),
        latest_reconciliation_link.as_ref(),
        intent,
        &expected_target,
    )?;
    let (terminal, terminal_prefix) = match (intent_terminal, dispatch_terminal) {
        (None, None) => (false, false),
        (
            Some((intent_at, intent_disposition, intent_target)),
            Some((dispatch_at, dispatch_disposition, predecessor, dispatch_link)),
        ) if terminal_safe
            && intent_at == dispatch_at
            && intent_disposition == dispatch_disposition
            && intent_target == dispatch_link
            && predecessor
                == intent_link(
                    intent
                        .get(intent.len().saturating_sub(2))
                        .ok_or(PmTrialLiveJournalError::InvalidRecord)?,
                )? =>
        {
            (true, false)
        }
        (None, Some((_, _, predecessor, _)))
            if terminal_safe
                && predecessor
                    == intent_link(
                        intent
                            .last()
                            .ok_or(PmTrialLiveJournalError::InvalidRecord)?,
                    )? =>
        {
            (false, true)
        }
        _ => return Err(PmTrialLiveJournalError::InvalidRecord),
    };
    let latest_cycle = cycles.last();
    let current_reconciliation = if let Some(link) = &latest_reconciliation_link {
        match &continuation_intent_line(link, intent)?.body {
            ContinuationIntentRecordV1::Reconciliation { dispatch, .. }
                if dispatch == &expected_target =>
            {
                latest_reconciliation.clone()
            }
            _ => None,
        }
    } else {
        None
    };
    Ok(ContinuationFactsV1 {
        latest_reconciliation: current_reconciliation,
        latest_cancel_result: latest_cycle
            .and_then(|cycle| cycle.result.as_ref())
            .map(|(_, outcome)| *outcome),
        terminal,
        terminal_prefix,
    })
}

fn latest_reconciliation_is_safe(
    latest: Option<&(PmReconciliationOrderStateV1, Option<String>)>,
    latest_link: Option<&ContinuationLinkV1>,
    intent: &[ContinuationIntentLineV1],
    expected_target: &ContinuationDispatchTargetV1,
) -> Result<bool, PmTrialLiveJournalError> {
    let Some((state, _)) = latest else {
        return Ok(false);
    };
    let Some(link) = latest_link else {
        return Ok(false);
    };
    let line = continuation_intent_line(link, intent)?;
    let ContinuationIntentRecordV1::Reconciliation { dispatch, .. } = &line.body else {
        return Err(PmTrialLiveJournalError::InvalidRecord);
    };
    Ok(dispatch == expected_target
        && matches!(
            state,
            PmReconciliationOrderStateV1::Absent
                | PmReconciliationOrderStateV1::ExactCanceled
                | PmReconciliationOrderStateV1::ExactFilled
        ))
}

fn validate_chains(
    scope: &ContinuationScopeV1,
    intent: &[ContinuationIntentLineV1],
    dispatch: &[ContinuationDispatchLineV1],
) -> Result<(), PmTrialLiveJournalError> {
    let mut previous = ZERO_FINGERPRINT.to_owned();
    for (index, line) in intent.iter().enumerate() {
        if line.schema_version != VERSION
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
        if line.schema_version != VERSION
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

fn validate_target(
    scope: &ContinuationScopeV1,
    target: &ContinuationDispatchTargetV1,
    dispatch: &[ContinuationDispatchLineV1],
) -> Result<(), PmTrialLiveJournalError> {
    target.validate()?;
    match target {
        ContinuationDispatchTargetV1::OriginalV1 {
            sequence,
            record_fingerprint,
        } if *sequence == scope.preserved_exposure.sequence
            && record_fingerprint == &scope.preserved_exposure.record_fingerprint =>
        {
            Ok(())
        }
        ContinuationDispatchTargetV1::ContinuationV1 {
            sequence,
            record_fingerprint,
        } => {
            let line = dispatch
                .get(usize::from(*sequence))
                .ok_or(PmTrialLiveJournalError::InvalidRecord)?;
            if dispatch_fingerprint(line)? != *record_fingerprint
                || !matches!(
                    line.body,
                    ContinuationDispatchRecordV1::CancelDispatchAuthorized { .. }
                        | ContinuationDispatchRecordV1::CancelResult { .. }
                )
            {
                return Err(PmTrialLiveJournalError::InvalidRecord);
            }
            Ok(())
        }
        _ => Err(PmTrialLiveJournalError::InvalidRecord),
    }
}

fn continuation_intent_line<'a>(
    link: &ContinuationLinkV1,
    lines: &'a [ContinuationIntentLineV1],
) -> Result<&'a ContinuationIntentLineV1, PmTrialLiveJournalError> {
    link.validate()?;
    let line = lines
        .get(usize::from(link.sequence))
        .ok_or(PmTrialLiveJournalError::InvalidRecord)?;
    if intent_fingerprint(line)? != link.record_fingerprint {
        return Err(PmTrialLiveJournalError::InvalidRecord);
    }
    Ok(line)
}

fn continuation_dispatch_line<'a>(
    link: &ContinuationLinkV1,
    lines: &'a [ContinuationDispatchLineV1],
) -> Result<&'a ContinuationDispatchLineV1, PmTrialLiveJournalError> {
    link.validate()?;
    let line = lines
        .get(usize::from(link.sequence))
        .ok_or(PmTrialLiveJournalError::InvalidRecord)?;
    if dispatch_fingerprint(line)? != link.record_fingerprint {
        return Err(PmTrialLiveJournalError::InvalidRecord);
    }
    Ok(line)
}

fn validate_reconciliation(
    scope: &ContinuationScopeV1,
    state: PmReconciliationOrderStateV1,
    exact_venue_order_id: Option<&str>,
) -> Result<(), PmTrialLiveJournalError> {
    match (state, exact_venue_order_id) {
        (
            PmReconciliationOrderStateV1::ExactLive
            | PmReconciliationOrderStateV1::ExactCanceled
            | PmReconciliationOrderStateV1::ExactFilled,
            Some(order_id),
        ) if order_id == scope.exact_venue_order_id => validate_order_id(order_id),
        (PmReconciliationOrderStateV1::Absent, None)
        | (PmReconciliationOrderStateV1::Ambiguous, None) => Ok(()),
        _ => Err(PmTrialLiveJournalError::InvalidRecord),
    }
}

fn validate_next_class(
    scope: &PmTrialLiveJournalScopeV1,
    candidate: PmCancelDispatchClassV1,
    prior: &[PmCancelDispatchClassV1],
) -> Result<(), PmTrialLiveJournalError> {
    if prior.contains(&PmCancelDispatchClassV1::Primary) {
        return Err(PmTrialLiveJournalError::InvalidRecord);
    }
    let highest_recovery = prior
        .iter()
        .filter_map(|class| match class {
            PmCancelDispatchClassV1::Recovery { ordinal } => Some(*ordinal),
            PmCancelDispatchClassV1::Primary => None,
        })
        .max()
        .unwrap_or(0);
    match candidate {
        PmCancelDispatchClassV1::Recovery { ordinal }
            if ordinal == highest_recovery.saturating_add(1)
                && ordinal > 0
                && ordinal <= scope.trial.order.recovery_cancel_dispatch_budget =>
        {
            Ok(())
        }
        _ => Err(PmTrialLiveJournalError::InvalidRecord),
    }
}

fn latest_preparation_l2(
    dispatch: &[ContinuationDispatchLineV1],
    before: usize,
    scope: &ContinuationScopeV1,
) -> u64 {
    dispatch[..before]
        .iter()
        .rev()
        .find_map(|line| match &line.body {
            ContinuationDispatchRecordV1::CancelPrepared { preparation, .. } => {
                Some(preparation.l2_timestamp_seconds())
            }
            _ => None,
        })
        .unwrap_or(scope.minimum_cancel_l2_timestamp_seconds)
}

fn preparation_fingerprint(
    scope_fingerprint: &str,
    intent: &ContinuationLinkV1,
    dispatch_class: PmCancelDispatchClassV1,
    preparation: &PmCancelPreparationV1,
) -> Result<String, PmTrialLiveJournalError> {
    #[derive(Serialize)]
    struct Basis<'a> {
        scope_fingerprint: &'a str,
        intent: &'a ContinuationLinkV1,
        dispatch_class: PmCancelDispatchClassV1,
        preparation: &'a PmCancelPreparationV1,
    }
    hash_domain(
        PREPARATION_DOMAIN,
        &Basis {
            scope_fingerprint,
            intent,
            dispatch_class,
            preparation,
        },
    )
}

fn parse_lines<T>(bytes: &[u8]) -> Result<Vec<T>, PmTrialLiveJournalError>
where
    T: serde::de::DeserializeOwned + Serialize,
{
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
        let record: T =
            serde_json::from_slice(line).map_err(|_| PmTrialLiveJournalError::InvalidRecord)?;
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

fn encode_line(value: &impl Serialize) -> Result<Vec<u8>, PmTrialLiveJournalError> {
    let mut bytes = canonical_json(value)?;
    if bytes.len() > MAX_JOURNAL_LINE_BYTES {
        return Err(PmTrialLiveJournalError::BoundExceeded);
    }
    bytes.push(b'\n');
    Ok(bytes)
}

struct IntentWriterV1 {
    file: ProtectedJournal,
    bytes: Vec<u8>,
    lines: Vec<ContinuationIntentLineV1>,
}

struct DispatchWriterV1 {
    file: ProtectedJournal,
    bytes: Vec<u8>,
    lines: Vec<ContinuationDispatchLineV1>,
}

pub(crate) struct RecoveryContinuationJournalsV1 {
    scope: ContinuationScopeV1,
    original_scope: PmTrialLiveJournalScopeV1,
    preflight: PmTrialLivePreflightBindingV1,
    intent: IntentWriterV1,
    dispatch: DispatchWriterV1,
}

#[derive(Clone)]
pub(crate) struct ContinuationAckV1 {
    pub(crate) sequence: u8,
    pub(crate) record_fingerprint: String,
}

impl ContinuationAckV1 {
    fn link(&self) -> ContinuationLinkV1 {
        ContinuationLinkV1 {
            sequence: self.sequence,
            record_fingerprint: self.record_fingerprint.clone(),
        }
    }
}

impl RecoveryContinuationJournalsV1 {
    pub(crate) fn create_or_open(
        config: &CanonicalTrialConfig,
        authorization: &CanonicalAuthorization,
        original_scope: &PmTrialLiveJournalScopeV1,
        basis: &PmPhaseALiveRecoveryContinuationBasisV1,
        consumed: PmTrialLiveConsumedFingerprintsV1,
        preflight: &PmTrialLivePreflightBindingV1,
        current_runtime: &AuthorizationRuntimeBinding,
    ) -> Result<(Self, bool), PmTrialLiveJournalError> {
        let (intent_path, dispatch_path) = bound_paths(config);
        let intent_existing = read_optional(&intent_path)?;
        let dispatch_existing = read_optional(&dispatch_path)?;
        if matches!((&intent_existing, &dispatch_existing), (None, Some(_)))
            || matches!((&intent_existing, &dispatch_existing), (Some(bytes), Some(_)) if bytes.is_empty())
        {
            return Err(PmTrialLiveJournalError::AmbiguousTail);
        }

        // A complete existing pair owns its sealed creation-time runtime. A
        // later open validates the newly observed runtime independently and
        // never reseals or compares it to the old timestamp.
        if let (Some(intent_bytes), Some(dispatch_bytes)) = (&intent_existing, &dispatch_existing)
            && !intent_bytes.is_empty()
            && !dispatch_bytes.is_empty()
        {
            let projection = parse_and_validate(
                config,
                authorization,
                original_scope,
                basis,
                preflight,
                intent_bytes.clone(),
                dispatch_bytes.clone(),
            )?;
            if projection.scope.consumed != consumed {
                return Err(PmTrialLiveJournalError::InvalidBinding);
            }
            projection.scope.validate_current_runtime(
                authorization,
                original_scope,
                current_runtime,
            )?;
            let mut intent_file = ProtectedJournal::open_existing(&intent_path, MAX_JOURNAL_BYTES)?;
            let mut dispatch_file =
                ProtectedJournal::open_existing(&dispatch_path, MAX_JOURNAL_BYTES)?;
            intent_file.validate_exact_bytes(intent_bytes)?;
            dispatch_file.validate_exact_bytes(dispatch_bytes)?;
            return Ok((
                Self {
                    scope: projection.scope,
                    original_scope: original_scope.clone(),
                    preflight: preflight.clone(),
                    intent: IntentWriterV1 {
                        file: intent_file,
                        bytes: projection.intent_bytes,
                        lines: projection.intent_lines,
                    },
                    dispatch: DispatchWriterV1 {
                        file: dispatch_file,
                        bytes: projection.dispatch_bytes,
                        lines: projection.dispatch_lines,
                    },
                },
                false,
            ));
        }

        // The only resumable creation prefixes are: no files; an exact empty
        // intent with no dispatch; or one exact canonical intent header with
        // an absent/empty dispatch. Any other partial pair is ambiguous.
        let scope = if let Some(intent_bytes) = &intent_existing
            && !intent_bytes.is_empty()
        {
            let lines: Vec<ContinuationIntentLineV1> = parse_lines(intent_bytes)?;
            if lines.len() != 1 {
                return Err(PmTrialLiveJournalError::AmbiguousTail);
            }
            let ContinuationIntentRecordV1::Header { scope } = &lines[0].body else {
                return Err(PmTrialLiveJournalError::InvalidRecord);
            };
            scope.validate(config, authorization, original_scope, basis)?;
            if scope.consumed != consumed {
                return Err(PmTrialLiveJournalError::InvalidBinding);
            }
            scope.validate_current_runtime(authorization, original_scope, current_runtime)?;
            scope.as_ref().clone()
        } else {
            ContinuationScopeV1::build(
                config,
                authorization,
                original_scope,
                basis,
                consumed,
                current_runtime,
            )?
        };
        let mut created = false;
        let mut intent_file = match &intent_existing {
            Some(_) => ProtectedJournal::open_existing(&intent_path, MAX_JOURNAL_BYTES)?,
            None => {
                created = true;
                ProtectedJournal::create_new(&intent_path, MAX_JOURNAL_BYTES)?
            }
        };
        let intent_header = ContinuationIntentLineV1 {
            schema_version: VERSION,
            sequence: 0,
            previous_record_fingerprint: ZERO_FINGERPRINT.to_owned(),
            scope_fingerprint: scope.scope_fingerprint.clone(),
            body: ContinuationIntentRecordV1::Header {
                scope: Box::new(scope.clone()),
            },
        };
        let intent_header_bytes = encode_line(&intent_header)?;
        let intent_bytes = intent_existing.unwrap_or_default();
        if intent_bytes.is_empty() {
            intent_file.append_durable(&[], &intent_header_bytes)?;
        } else if intent_bytes != intent_header_bytes {
            return Err(PmTrialLiveJournalError::InvalidBinding);
        }
        let mut dispatch_file = match &dispatch_existing {
            Some(_) => ProtectedJournal::open_existing(&dispatch_path, MAX_JOURNAL_BYTES)?,
            None => {
                created = true;
                ProtectedJournal::create_new(&dispatch_path, MAX_JOURNAL_BYTES)?
            }
        };
        let dispatch_header = ContinuationDispatchLineV1 {
            schema_version: VERSION,
            sequence: 0,
            previous_record_fingerprint: ZERO_FINGERPRINT.to_owned(),
            scope_fingerprint: scope.scope_fingerprint.clone(),
            body: ContinuationDispatchRecordV1::Header {
                scope_fingerprint: scope.scope_fingerprint.clone(),
                intent_header: intent_link(&intent_header)?,
            },
        };
        let dispatch_header_bytes = encode_line(&dispatch_header)?;
        let dispatch_bytes = dispatch_existing.unwrap_or_default();
        if dispatch_bytes.is_empty() {
            dispatch_file.append_durable(&[], &dispatch_header_bytes)?;
        } else if dispatch_bytes != dispatch_header_bytes {
            return Err(PmTrialLiveJournalError::InvalidBinding);
        }
        intent_file.refresh_parent_after_bound_create()?;
        dispatch_file.refresh_parent_after_bound_create()?;
        let intent_bytes = read_protected(&intent_path, MAX_JOURNAL_BYTES)?;
        let dispatch_bytes = read_protected(&dispatch_path, MAX_JOURNAL_BYTES)?;
        let projection = parse_and_validate(
            config,
            authorization,
            original_scope,
            basis,
            preflight,
            intent_bytes,
            dispatch_bytes,
        )?;
        if projection.scope.consumed != scope.consumed {
            return Err(PmTrialLiveJournalError::InvalidBinding);
        }
        Ok((
            Self {
                scope,
                original_scope: original_scope.clone(),
                preflight: preflight.clone(),
                intent: IntentWriterV1 {
                    file: intent_file,
                    bytes: projection.intent_bytes,
                    lines: projection.intent_lines,
                },
                dispatch: DispatchWriterV1 {
                    file: dispatch_file,
                    bytes: projection.dispatch_bytes,
                    lines: projection.dispatch_lines,
                },
            },
            created,
        ))
    }

    pub(crate) fn validate_exact(&mut self) -> Result<(), PmTrialLiveJournalError> {
        self.intent.file.validate_exact_bytes(&self.intent.bytes)?;
        self.dispatch
            .file
            .validate_exact_bytes(&self.dispatch.bytes)
    }

    pub(crate) fn validate_against_consumption_registry(
        &self,
        registry: Option<&AuthorizationRecoveryContinuationRegistryV1>,
    ) -> Result<(), PmTrialLiveJournalError> {
        validate_consumption_registry(
            &self.scope,
            &self.original_scope,
            &self.preflight,
            &self.intent.lines,
            &self.dispatch.lines,
            registry,
        )
    }

    pub(crate) fn validate_fully_anchored_against_consumption_registry(
        &self,
        registry: Option<&AuthorizationRecoveryContinuationRegistryV1>,
    ) -> Result<(), PmTrialLiveJournalError> {
        self.validate_against_consumption_registry(registry)?;
        let registry = registry.ok_or(PmTrialLiveJournalError::InvalidRecord)?;
        if registry.prepared.len() != self.prepared_anchor_evidence()?.len() {
            return Err(PmTrialLiveJournalError::InvalidRecord);
        }
        if let Some(plan) = &registry.terminal_plan
            && reconstruct_terminal_plan_lines(
                &self.scope,
                &self.original_scope,
                &self.preflight,
                &self.intent.lines,
                &self.dispatch.lines,
                plan,
            )?
            .physical_state
                != ContinuationTerminalPlanPhysicalStateV1::Complete
        {
            return Err(PmTrialLiveJournalError::InvalidRecord);
        }
        Ok(())
    }

    pub(crate) fn complete_consumption_registry(
        &mut self,
        authorization: &mut ConsumedAuthorizationConsumption,
    ) -> Result<(), PmTrialLiveJournalError> {
        let intent_header = intent_fingerprint(
            self.intent
                .lines
                .first()
                .ok_or(PmTrialLiveJournalError::InvalidRecord)?,
        )?;
        let dispatch_header = dispatch_fingerprint(
            self.dispatch
                .lines
                .first()
                .ok_or(PmTrialLiveJournalError::InvalidRecord)?,
        )?;
        authorization
            .anchor_recovery_continuation_root(
                &self.scope.scope_fingerprint,
                &intent_header,
                &dispatch_header,
            )
            .map_err(|_| PmTrialLiveJournalError::Protection)?;
        let registry = authorization
            .recovery_continuation_registry()
            .cloned()
            .ok_or(PmTrialLiveJournalError::InvalidRecord)?;
        self.complete_one_anchored_prepared(&registry)?;
        self.complete_anchored_terminal_plan(&registry)?;
        Ok(())
    }

    pub(crate) fn refresh_after_bound_create(&mut self) -> Result<(), PmTrialLiveJournalError> {
        self.intent.file.refresh_parent_after_bound_create()?;
        self.dispatch.file.refresh_parent_after_bound_create()?;
        self.validate_exact()
    }

    pub(crate) fn first_target(&self) -> ContinuationDispatchTargetV1 {
        ContinuationDispatchTargetV1::from_original(&self.scope.preserved_exposure)
    }

    pub(crate) fn current_reconciliation_target(
        &self,
    ) -> Result<ContinuationDispatchTargetV1, PmTrialLiveJournalError> {
        self.latest_exposure_target()
    }

    pub(crate) fn next_recovery_ordinal(&self) -> Result<u8, PmTrialLiveJournalError> {
        let highest = self
            .dispatch
            .lines
            .iter()
            .filter_map(|line| match &line.body {
                ContinuationDispatchRecordV1::CancelPrepared {
                    dispatch_class: PmCancelDispatchClassV1::Recovery { ordinal },
                    ..
                } => Some(*ordinal),
                _ => None,
            })
            .max()
            .unwrap_or(0);
        let next = highest
            .checked_add(1)
            .ok_or(PmTrialLiveJournalError::BoundExceeded)?;
        if next == 0
            || next
                > self
                    .original_scope
                    .trial
                    .order
                    .recovery_cancel_dispatch_budget
        {
            return Err(PmTrialLiveJournalError::BoundExceeded);
        }
        Ok(next)
    }

    pub(crate) fn record_reconciliation(
        &mut self,
        target: ContinuationDispatchTargetV1,
        observed_at_utc: String,
        state: PmReconciliationOrderStateV1,
        exact_venue_order_id: Option<String>,
    ) -> Result<ContinuationAckV1, PmTrialLiveJournalError> {
        self.validate_exact()?;
        self.require_not_terminal()?;
        validate_utc(&observed_at_utc)?;
        validate_reconciliation(&self.scope, state, exact_venue_order_id.as_deref())?;
        validate_target(&self.scope, &target, &self.dispatch.lines)?;
        if self.pending_result_bridge()?.is_some() {
            return Err(PmTrialLiveJournalError::InvalidTransition);
        }
        let expected_target = self.latest_exposure_target()?;
        if target != expected_target {
            return Err(PmTrialLiveJournalError::InvalidTransition);
        }
        self.append_intent(ContinuationIntentRecordV1::Reconciliation {
            observed_at_utc,
            state,
            exact_venue_order_id,
            dispatch: target,
        })
    }

    pub(crate) fn record_cancel_intent(
        &mut self,
        reconciliation: &ContinuationAckV1,
        dispatch_class: PmCancelDispatchClassV1,
        created_at_utc: String,
    ) -> Result<ContinuationAckV1, PmTrialLiveJournalError> {
        self.validate_exact()?;
        self.require_not_terminal()?;
        validate_utc(&created_at_utc)?;
        let source = self.require_intent_ack(reconciliation)?;
        let ContinuationIntentRecordV1::Reconciliation {
            state: PmReconciliationOrderStateV1::ExactLive,
            exact_venue_order_id: Some(order_id),
            ..
        } = &source.body
        else {
            return Err(PmTrialLiveJournalError::InvalidTransition);
        };
        if order_id != &self.scope.exact_venue_order_id {
            return Err(PmTrialLiveJournalError::InvalidBinding);
        }
        let PmCancelDispatchClassV1::Recovery { ordinal } = dispatch_class else {
            return Err(PmTrialLiveJournalError::InvalidTransition);
        };
        if ordinal != self.next_recovery_ordinal()? {
            return Err(PmTrialLiveJournalError::InvalidTransition);
        }
        self.append_intent(ContinuationIntentRecordV1::CancelIntent {
            created_at_utc,
            ownership_source: reconciliation.link(),
            exact_venue_order_id: order_id.clone(),
            dispatch_class,
        })
    }

    pub(crate) fn record_cancel_prepared_ledger_first(
        &mut self,
        authorization: &mut ConsumedAuthorizationConsumption,
        intent: &ContinuationAckV1,
        dispatch_class: PmCancelDispatchClassV1,
        preparation: PmCancelPreparationV1,
    ) -> Result<(ContinuationAckV1, PmCancelPreparationViewV1), PmTrialLiveJournalError> {
        self.validate_exact()?;
        self.require_not_terminal()?;
        let intent_line = self.require_intent_ack(intent)?;
        let ContinuationIntentRecordV1::CancelIntent {
            exact_venue_order_id,
            dispatch_class: intent_class,
            ..
        } = &intent_line.body
        else {
            return Err(PmTrialLiveJournalError::InvalidTransition);
        };
        if *intent_class != dispatch_class
            || exact_venue_order_id != &self.scope.exact_venue_order_id
        {
            return Err(PmTrialLiveJournalError::InvalidBinding);
        }
        let prior_classes = self.prepared_classes();
        validate_next_class(&self.original_scope, dispatch_class, &prior_classes)?;
        let minimum_l2 =
            latest_preparation_l2(&self.dispatch.lines, self.dispatch.lines.len(), &self.scope);
        let preparation = preparation.bind_request(
            &self.original_scope,
            &self.preflight,
            intent.sequence,
            minimum_l2,
        )?;
        if preparation.exact_venue_order_id() != exact_venue_order_id {
            return Err(PmTrialLiveJournalError::InvalidBinding);
        }
        let view = preparation.view(dispatch_class)?;
        let continuation_preparation_fingerprint = preparation_fingerprint(
            &self.scope.scope_fingerprint,
            &intent.link(),
            dispatch_class,
            &preparation,
        )?;
        let line = self.next_dispatch_line(ContinuationDispatchRecordV1::CancelPrepared {
            intent: intent.link(),
            dispatch_class,
            preparation,
            continuation_preparation_fingerprint,
        })?;
        let anchored = prepared_anchor_evidence(std::slice::from_ref(&line))?
            .pop()
            .ok_or(PmTrialLiveJournalError::InvalidRecord)?;
        authorization
            .anchor_recovery_cancel_prepared(
                &self.scope.scope_fingerprint,
                anchored.recovery_ordinal,
                anchored.intent_sequence,
                &anchored.intent_record_fingerprint,
                anchored.prepared_sequence,
                &anchored.dispatch_previous_record_fingerprint,
                &anchored.prepared_record_fingerprint,
                &anchored.prepared_record_canonical_json,
                &anchored.exact_venue_order_id,
                &anchored.semantic_request_commitment_sha256,
                anchored.l2_timestamp_seconds,
            )
            .map_err(|_| PmTrialLiveJournalError::Protection)?;
        authorization
            .revalidate_held_consumption_evidence()
            .map_err(|_| PmTrialLiveJournalError::Protection)?;
        let registry = authorization
            .recovery_continuation_registry()
            .cloned()
            .ok_or(PmTrialLiveJournalError::InvalidRecord)?;
        let (ack, reconstructed_view) = self
            .complete_one_anchored_prepared(&registry)?
            .ok_or(PmTrialLiveJournalError::InvalidRecord)?;
        if reconstructed_view != view {
            return Err(PmTrialLiveJournalError::InvalidRecord);
        }
        Ok((ack, view))
    }

    pub(crate) fn record_cancel_dispatch_authorized(
        &mut self,
        prepared: &ContinuationAckV1,
        dispatch_class: PmCancelDispatchClassV1,
        exact_venue_order_id: &str,
    ) -> Result<ContinuationAckV1, PmTrialLiveJournalError> {
        self.validate_exact()?;
        self.require_not_terminal()?;
        let line = self.require_dispatch_ack(prepared)?;
        let ContinuationDispatchRecordV1::CancelPrepared {
            dispatch_class: prepared_class,
            preparation,
            ..
        } = &line.body
        else {
            return Err(PmTrialLiveJournalError::InvalidTransition);
        };
        if *prepared_class != dispatch_class
            || preparation.exact_venue_order_id() != exact_venue_order_id
            || exact_venue_order_id != self.scope.exact_venue_order_id
        {
            return Err(PmTrialLiveJournalError::InvalidBinding);
        }
        self.append_dispatch(ContinuationDispatchRecordV1::CancelDispatchAuthorized {
            prepared_sequence: prepared.sequence,
            prepared_record_fingerprint: prepared.record_fingerprint.clone(),
            dispatch_class,
            exact_venue_order_id: exact_venue_order_id.to_owned(),
            production_order_entry_authorized: false,
            real_order_submission_authorized: false,
            place_dispatch_allowance: 0,
        })
    }

    pub(crate) fn record_cancel_result(
        &mut self,
        dispatch: &ContinuationAckV1,
        outcome: PmCancelResultKindV1,
        exact_venue_order_id: &str,
    ) -> Result<ContinuationAckV1, PmTrialLiveJournalError> {
        self.validate_exact()?;
        self.require_not_terminal()?;
        let line = self.require_dispatch_ack(dispatch)?;
        if !matches!(
            &line.body,
            ContinuationDispatchRecordV1::CancelDispatchAuthorized {
                exact_venue_order_id: dispatched_id,
                ..
            } if dispatched_id == exact_venue_order_id
                && dispatched_id == &self.scope.exact_venue_order_id
        ) {
            return Err(PmTrialLiveJournalError::InvalidTransition);
        }
        self.append_dispatch(ContinuationDispatchRecordV1::CancelResult {
            dispatch_authorized_sequence: dispatch.sequence,
            dispatch_authorized_fingerprint: dispatch.record_fingerprint.clone(),
            outcome,
            exact_venue_order_id: exact_venue_order_id.to_owned(),
        })
    }

    pub(crate) fn record_cancel_outcome_bridge(
        &mut self,
        result: &ContinuationAckV1,
        outcome: PmCancelResultKindV1,
        exact_venue_order_id: &str,
    ) -> Result<ContinuationAckV1, PmTrialLiveJournalError> {
        self.validate_exact()?;
        self.require_not_terminal()?;
        let line = self.require_dispatch_ack(result)?;
        if !matches!(
            &line.body,
            ContinuationDispatchRecordV1::CancelResult {
                outcome: recorded,
                exact_venue_order_id: recorded_id,
                ..
            } if *recorded == outcome && recorded_id == exact_venue_order_id
        ) {
            return Err(PmTrialLiveJournalError::InvalidTransition);
        }
        self.append_intent(ContinuationIntentRecordV1::CancelOutcomeBridge {
            dispatch: result.link(),
            outcome,
            exact_venue_order_id: exact_venue_order_id.to_owned(),
        })
    }

    /// Reconstructs only the exact durable bridge custody needed after a
    /// crash between a cancel result and its intent-side outcome bridge. If
    /// the bridge was already durable, it must still be the current intent
    /// tail. No caller-supplied result facts participate in this recovery
    /// seam.
    pub(crate) fn resume_cancel_outcome_bridge(
        &mut self,
    ) -> Result<ContinuationOutcomeResumeV1, PmTrialLiveJournalError> {
        self.validate_exact()?;
        self.require_not_terminal()?;
        let (result, outcome, exact_venue_order_id) = {
            let result_line = self
                .dispatch
                .lines
                .last()
                .ok_or(PmTrialLiveJournalError::InvalidTransition)?;
            let ContinuationDispatchRecordV1::CancelResult {
                outcome,
                exact_venue_order_id,
                ..
            } = &result_line.body
            else {
                return Err(PmTrialLiveJournalError::InvalidTransition);
            };
            (
                ContinuationAckV1 {
                    sequence: result_line.sequence,
                    record_fingerprint: dispatch_fingerprint(result_line)?,
                },
                *outcome,
                exact_venue_order_id.clone(),
            )
        };
        let existing_bridge = self.intent.lines.last().and_then(|line| match &line.body {
            ContinuationIntentRecordV1::CancelOutcomeBridge {
                dispatch,
                outcome: recorded,
                exact_venue_order_id: recorded_id,
            } if dispatch == &result.link()
                && *recorded == outcome
                && recorded_id == &exact_venue_order_id =>
            {
                Some(line)
            }
            _ => None,
        });
        let bridge = if let Some(line) = existing_bridge {
            ContinuationAckV1 {
                sequence: line.sequence,
                record_fingerprint: intent_fingerprint(line)?,
            }
        } else {
            if self.pending_result_bridge()?.as_ref() != Some(&result.link()) {
                return Err(PmTrialLiveJournalError::InvalidTransition);
            }
            self.record_cancel_outcome_bridge(&result, outcome, &exact_venue_order_id)?
        };
        Ok(ContinuationOutcomeResumeV1 {
            bridge,
            result,
            exact_venue_order_id,
        })
    }

    pub(crate) fn record_terminal(
        &mut self,
        authorization: &mut ConsumedAuthorizationConsumption,
        terminal_at_utc: String,
        disposition: PmIntentTerminalDispositionV1,
    ) -> Result<ContinuationAckV1, PmTrialLiveJournalError> {
        self.validate_exact()?;
        validate_utc(&terminal_at_utc)?;
        if !self.latest_reconciliation_is_safe()? {
            return Err(PmTrialLiveJournalError::InvalidTransition);
        }
        if matches!(
            self.intent.lines.last().map(|line| &line.body),
            Some(ContinuationIntentRecordV1::Terminal { .. })
        ) {
            return Err(PmTrialLiveJournalError::InvalidTransition);
        }
        if matches!(
            self.dispatch.lines.last().map(|line| &line.body),
            Some(ContinuationDispatchRecordV1::Terminal { .. })
        ) {
            return Err(PmTrialLiveJournalError::InvalidTransition);
        }
        self.validate_fully_anchored_against_consumption_registry(
            authorization.recovery_continuation_registry(),
        )?;
        let latest_intent = intent_link(
            self.intent
                .lines
                .last()
                .ok_or(PmTrialLiveJournalError::InvalidTransition)?,
        )?;
        let latest_dispatch = dispatch_link(
            self.dispatch
                .lines
                .last()
                .ok_or(PmTrialLiveJournalError::InvalidTransition)?,
        )?;
        let dispatch_terminal =
            self.next_dispatch_line(ContinuationDispatchRecordV1::Terminal {
                terminal_at_utc: terminal_at_utc.clone(),
                disposition,
                intent: latest_intent.clone(),
                terminal_is_evidence_not_authority: true,
            })?;
        let dispatch_terminal_link = dispatch_link(&dispatch_terminal)?;
        let intent_terminal = self.next_intent_line(ContinuationIntentRecordV1::Terminal {
            terminal_at_utc: terminal_at_utc.clone(),
            disposition,
            dispatch_terminal: dispatch_terminal_link,
            terminal_is_evidence_not_authority: true,
        })?;
        let dispatch_terminal_fingerprint = dispatch_fingerprint(&dispatch_terminal)?;
        let dispatch_terminal_canonical_json =
            String::from_utf8(canonical_json(&dispatch_terminal)?)
                .map_err(|_| PmTrialLiveJournalError::InvalidRecord)?;
        let intent_terminal_fingerprint = intent_fingerprint(&intent_terminal)?;
        let intent_terminal_canonical_json = String::from_utf8(canonical_json(&intent_terminal)?)
            .map_err(|_| PmTrialLiveJournalError::InvalidRecord)?;
        authorization
            .anchor_recovery_terminal_plan(
                &self.scope.scope_fingerprint,
                latest_intent.sequence,
                &latest_intent.record_fingerprint,
                latest_dispatch.sequence,
                &latest_dispatch.record_fingerprint,
                &terminal_at_utc,
                authorization_terminal_disposition(disposition),
                dispatch_terminal.sequence,
                &dispatch_terminal.previous_record_fingerprint,
                &dispatch_terminal_fingerprint,
                &dispatch_terminal_canonical_json,
                intent_terminal.sequence,
                &intent_terminal.previous_record_fingerprint,
                &intent_terminal_fingerprint,
                &intent_terminal_canonical_json,
            )
            .map_err(|_| PmTrialLiveJournalError::Protection)?;
        authorization
            .revalidate_held_consumption_evidence()
            .map_err(|_| PmTrialLiveJournalError::Protection)?;
        let registry = authorization
            .recovery_continuation_registry()
            .cloned()
            .ok_or(PmTrialLiveJournalError::InvalidRecord)?;
        self.complete_anchored_terminal_plan(&registry)?
            .ok_or(PmTrialLiveJournalError::InvalidRecord)
    }

    pub(crate) fn has_complete_anchored_terminal_plan(
        &self,
        registry: Option<&AuthorizationRecoveryContinuationRegistryV1>,
    ) -> Result<bool, PmTrialLiveJournalError> {
        let Some(plan) = registry.and_then(|registry| registry.terminal_plan.as_ref()) else {
            return Ok(false);
        };
        Ok(reconstruct_terminal_plan_lines(
            &self.scope,
            &self.original_scope,
            &self.preflight,
            &self.intent.lines,
            &self.dispatch.lines,
            plan,
        )?
        .physical_state
            == ContinuationTerminalPlanPhysicalStateV1::Complete)
    }

    pub(crate) fn target_for_dispatch_ack(
        &self,
        ack: &ContinuationAckV1,
    ) -> Result<ContinuationDispatchTargetV1, PmTrialLiveJournalError> {
        self.require_dispatch_ack(ack)?;
        Ok(ContinuationDispatchTargetV1::from_continuation(&ack.link()))
    }

    pub(crate) fn validate_intent_ack(
        &self,
        ack: &ContinuationAckV1,
    ) -> Result<(), PmTrialLiveJournalError> {
        self.require_intent_ack(ack).map(|_| ())
    }

    pub(crate) fn terminal_safe(&self) -> Result<bool, PmTrialLiveJournalError> {
        self.latest_reconciliation_is_safe()
    }

    fn require_intent_ack(
        &self,
        ack: &ContinuationAckV1,
    ) -> Result<&ContinuationIntentLineV1, PmTrialLiveJournalError> {
        let line = self
            .intent
            .lines
            .get(usize::from(ack.sequence))
            .ok_or(PmTrialLiveJournalError::ForeignAcknowledgement)?;
        if intent_fingerprint(line)? != ack.record_fingerprint
            || self.intent.lines.last() != Some(line)
        {
            return Err(PmTrialLiveJournalError::ForeignAcknowledgement);
        }
        Ok(line)
    }

    fn require_not_terminal(&self) -> Result<(), PmTrialLiveJournalError> {
        if matches!(
            self.intent.lines.last().map(|line| &line.body),
            Some(ContinuationIntentRecordV1::Terminal { .. })
        ) || matches!(
            self.dispatch.lines.last().map(|line| &line.body),
            Some(ContinuationDispatchRecordV1::Terminal { .. })
        ) {
            return Err(PmTrialLiveJournalError::InvalidTransition);
        }
        Ok(())
    }

    fn require_dispatch_ack(
        &self,
        ack: &ContinuationAckV1,
    ) -> Result<&ContinuationDispatchLineV1, PmTrialLiveJournalError> {
        let line = self
            .dispatch
            .lines
            .get(usize::from(ack.sequence))
            .ok_or(PmTrialLiveJournalError::ForeignAcknowledgement)?;
        if dispatch_fingerprint(line)? != ack.record_fingerprint
            || self.dispatch.lines.last() != Some(line)
        {
            return Err(PmTrialLiveJournalError::ForeignAcknowledgement);
        }
        Ok(line)
    }

    fn prepared_classes(&self) -> Vec<PmCancelDispatchClassV1> {
        self.dispatch
            .lines
            .iter()
            .filter_map(|line| match &line.body {
                ContinuationDispatchRecordV1::CancelPrepared { dispatch_class, .. } => {
                    Some(*dispatch_class)
                }
                _ => None,
            })
            .collect()
    }

    fn prepared_anchor_evidence(
        &self,
    ) -> Result<Vec<ContinuationPreparedAnchorEvidenceV1>, PmTrialLiveJournalError> {
        prepared_anchor_evidence(&self.dispatch.lines)
    }

    fn latest_exposure_target(
        &self,
    ) -> Result<ContinuationDispatchTargetV1, PmTrialLiveJournalError> {
        self.dispatch
            .lines
            .iter()
            .rev()
            .find(|line| {
                matches!(
                    line.body,
                    ContinuationDispatchRecordV1::CancelDispatchAuthorized { .. }
                        | ContinuationDispatchRecordV1::CancelResult { .. }
                )
            })
            .map(dispatch_link)
            .transpose()?
            .map(|link| ContinuationDispatchTargetV1::from_continuation(&link))
            .map_or_else(|| Ok(self.first_target()), Ok)
    }

    fn pending_result_bridge(&self) -> Result<Option<ContinuationLinkV1>, PmTrialLiveJournalError> {
        let Some(result_line) =
            self.dispatch.lines.iter().rev().find(|line| {
                matches!(line.body, ContinuationDispatchRecordV1::CancelResult { .. })
            })
        else {
            return Ok(None);
        };
        let result = dispatch_link(result_line)?;
        if self.intent.lines.iter().any(|line| {
            matches!(
                &line.body,
                ContinuationIntentRecordV1::CancelOutcomeBridge { dispatch, .. }
                    if dispatch == &result
            )
        }) {
            return Ok(None);
        }
        Ok(Some(result))
    }

    fn latest_reconciliation_is_safe(&self) -> Result<bool, PmTrialLiveJournalError> {
        let expected_target = self.latest_exposure_target()?;
        Ok(self
            .intent
            .lines
            .iter()
            .rev()
            .find_map(|line| match &line.body {
                ContinuationIntentRecordV1::Reconciliation {
                    state, dispatch, ..
                } => Some((state, dispatch)),
                _ => None,
            })
            .is_some_and(|(state, target)| {
                target == &expected_target
                    && matches!(
                        state,
                        PmReconciliationOrderStateV1::Absent
                            | PmReconciliationOrderStateV1::ExactCanceled
                            | PmReconciliationOrderStateV1::ExactFilled
                    )
            }))
    }

    fn complete_one_anchored_prepared(
        &mut self,
        registry: &AuthorizationRecoveryContinuationRegistryV1,
    ) -> Result<Option<(ContinuationAckV1, PmCancelPreparationViewV1)>, PmTrialLiveJournalError>
    {
        self.validate_exact()?;
        self.validate_against_consumption_registry(Some(registry))?;
        let durable_count = self.prepared_anchor_evidence()?.len();
        if registry.prepared.len() == durable_count {
            return Ok(None);
        }
        self.require_not_terminal()?;
        if registry.prepared.len() != durable_count.saturating_add(1) {
            return Err(PmTrialLiveJournalError::InvalidRecord);
        }
        let line = reconstruct_anchored_prepared_line(
            &self.scope,
            &self.original_scope,
            &self.preflight,
            &self.intent.lines,
            &self.dispatch.lines,
            registry
                .prepared
                .last()
                .ok_or(PmTrialLiveJournalError::InvalidRecord)?,
        )?;
        let ContinuationDispatchRecordV1::CancelPrepared {
            dispatch_class,
            preparation,
            ..
        } = &line.body
        else {
            return Err(PmTrialLiveJournalError::InvalidRecord);
        };
        let view = preparation.view(*dispatch_class)?;
        let encoded = encode_line(&line)?;
        self.dispatch
            .file
            .append_durable(&self.dispatch.bytes, &encoded)?;
        self.dispatch.bytes.extend_from_slice(&encoded);
        let ack = ContinuationAckV1 {
            sequence: line.sequence,
            record_fingerprint: dispatch_fingerprint(&line)?,
        };
        self.dispatch.lines.push(line);
        self.validate_fully_anchored_against_consumption_registry(Some(registry))?;
        Ok(Some((ack, view)))
    }

    fn complete_anchored_terminal_plan(
        &mut self,
        registry: &AuthorizationRecoveryContinuationRegistryV1,
    ) -> Result<Option<ContinuationAckV1>, PmTrialLiveJournalError> {
        self.validate_exact()?;
        self.validate_against_consumption_registry(Some(registry))?;
        let Some(plan) = &registry.terminal_plan else {
            return Ok(None);
        };
        let mut reconstructed = reconstruct_terminal_plan_lines(
            &self.scope,
            &self.original_scope,
            &self.preflight,
            &self.intent.lines,
            &self.dispatch.lines,
            plan,
        )?;
        if reconstructed.physical_state == ContinuationTerminalPlanPhysicalStateV1::MissingBoth {
            self.append_exact_dispatch_line(reconstructed.dispatch.clone())?;
            reconstructed.physical_state = ContinuationTerminalPlanPhysicalStateV1::DispatchOnly;
            self.validate_against_consumption_registry(Some(registry))?;
        }
        if reconstructed.physical_state == ContinuationTerminalPlanPhysicalStateV1::DispatchOnly {
            self.append_exact_intent_line(reconstructed.intent.clone())?;
        }
        self.validate_exact()?;
        self.validate_fully_anchored_against_consumption_registry(Some(registry))?;
        Ok(Some(ContinuationAckV1 {
            sequence: reconstructed.intent.sequence,
            record_fingerprint: intent_fingerprint(&reconstructed.intent)?,
        }))
    }

    fn append_intent(
        &mut self,
        body: ContinuationIntentRecordV1,
    ) -> Result<ContinuationAckV1, PmTrialLiveJournalError> {
        let sequence = u8::try_from(self.intent.lines.len())
            .map_err(|_| PmTrialLiveJournalError::BoundExceeded)?;
        let previous = self
            .intent
            .lines
            .last()
            .map(intent_fingerprint)
            .transpose()?
            .ok_or(PmTrialLiveJournalError::InvalidTransition)?;
        let line = ContinuationIntentLineV1 {
            schema_version: VERSION,
            sequence,
            previous_record_fingerprint: previous,
            scope_fingerprint: self.scope.scope_fingerprint.clone(),
            body,
        };
        let encoded = encode_line(&line)?;
        self.intent
            .file
            .append_durable(&self.intent.bytes, &encoded)?;
        self.intent.bytes.extend_from_slice(&encoded);
        let record_fingerprint = intent_fingerprint(&line)?;
        self.intent.lines.push(line);
        Ok(ContinuationAckV1 {
            sequence,
            record_fingerprint,
        })
    }

    fn append_exact_intent_line(
        &mut self,
        line: ContinuationIntentLineV1,
    ) -> Result<(), PmTrialLiveJournalError> {
        if line != self.next_intent_line(line.body.clone())? {
            return Err(PmTrialLiveJournalError::InvalidRecord);
        }
        let encoded = encode_line(&line)?;
        self.intent
            .file
            .append_durable(&self.intent.bytes, &encoded)?;
        self.intent.bytes.extend_from_slice(&encoded);
        self.intent.lines.push(line);
        Ok(())
    }

    fn append_dispatch(
        &mut self,
        body: ContinuationDispatchRecordV1,
    ) -> Result<ContinuationAckV1, PmTrialLiveJournalError> {
        let line = self.next_dispatch_line(body)?;
        let encoded = encode_line(&line)?;
        self.dispatch
            .file
            .append_durable(&self.dispatch.bytes, &encoded)?;
        self.dispatch.bytes.extend_from_slice(&encoded);
        let sequence = line.sequence;
        let record_fingerprint = dispatch_fingerprint(&line)?;
        self.dispatch.lines.push(line);
        Ok(ContinuationAckV1 {
            sequence,
            record_fingerprint,
        })
    }

    fn append_exact_dispatch_line(
        &mut self,
        line: ContinuationDispatchLineV1,
    ) -> Result<(), PmTrialLiveJournalError> {
        if line != self.next_dispatch_line(line.body.clone())? {
            return Err(PmTrialLiveJournalError::InvalidRecord);
        }
        let encoded = encode_line(&line)?;
        self.dispatch
            .file
            .append_durable(&self.dispatch.bytes, &encoded)?;
        self.dispatch.bytes.extend_from_slice(&encoded);
        self.dispatch.lines.push(line);
        Ok(())
    }

    fn next_intent_line(
        &self,
        body: ContinuationIntentRecordV1,
    ) -> Result<ContinuationIntentLineV1, PmTrialLiveJournalError> {
        let sequence = u8::try_from(self.intent.lines.len())
            .map_err(|_| PmTrialLiveJournalError::BoundExceeded)?;
        let previous = self
            .intent
            .lines
            .last()
            .map(intent_fingerprint)
            .transpose()?
            .ok_or(PmTrialLiveJournalError::InvalidTransition)?;
        Ok(ContinuationIntentLineV1 {
            schema_version: VERSION,
            sequence,
            previous_record_fingerprint: previous,
            scope_fingerprint: self.scope.scope_fingerprint.clone(),
            body,
        })
    }

    fn next_dispatch_line(
        &self,
        body: ContinuationDispatchRecordV1,
    ) -> Result<ContinuationDispatchLineV1, PmTrialLiveJournalError> {
        let sequence = u8::try_from(self.dispatch.lines.len())
            .map_err(|_| PmTrialLiveJournalError::BoundExceeded)?;
        let previous = self
            .dispatch
            .lines
            .last()
            .map(dispatch_fingerprint)
            .transpose()?
            .ok_or(PmTrialLiveJournalError::InvalidTransition)?;
        Ok(ContinuationDispatchLineV1 {
            schema_version: VERSION,
            sequence,
            previous_record_fingerprint: previous,
            scope_fingerprint: self.scope.scope_fingerprint.clone(),
            body,
        })
    }
}
