use chrono::{DateTime, SecondsFormat, Utc};
use reap_pm_controlled_trial::{
    AuthorizationConsumptionBindingEvidence, AuthorizationHostBinding, CanonicalTrialConfig,
    CanonicalTrialPreflight, ExpectedOrderId, FixedOrderId, OfflineAuthorizationState,
    OwnedCancelSemanticRequestCommitment, PM_T2_JOURNAL_FAMILY_V1, PM_T2_JOURNAL_VERSION_V1,
    PM_T2_LIVE_DISPATCH_JOURNAL_FILE_V1, PM_T2_LIVE_INTENT_JOURNAL_FILE_V1,
    PlacePublicRequestIdentity, PlaceSemanticRequestCommitment, TrialConfig,
    derive_owned_cancel_semantic_request_commitment,
};
use serde::{Deserialize, Serialize};

use crate::{
    PmTrialLiveJournalError,
    hash::{ZERO_FINGERPRINT, hash_bytes, hash_domain, validate_fingerprint},
};

pub const PM_TRIAL_LIVE_INTENT_FILE_V1: &str = PM_T2_LIVE_INTENT_JOURNAL_FILE_V1;
pub const PM_TRIAL_LIVE_DISPATCH_FILE_V1: &str = PM_T2_LIVE_DISPATCH_JOURNAL_FILE_V1;
pub(crate) const PM_TRIAL_LIVE_JOURNAL_FAMILY: &str = PM_T2_JOURNAL_FAMILY_V1;
pub(crate) const PM_TRIAL_LIVE_JOURNAL_VERSION: u32 = PM_T2_JOURNAL_VERSION_V1;
pub(crate) const MAX_JOURNAL_BYTES: usize = 512 * 1_024;
pub(crate) const MAX_JOURNAL_LINE_BYTES: usize = 64 * 1_024;
pub(crate) const MAX_JOURNAL_RECORDS: usize = 32;

const SCOPE_FINGERPRINT_DOMAIN: &[u8] = b"reap.pm-t2.controlled-trial-live.scope.v1\0";
const INTENT_RECORD_FINGERPRINT_DOMAIN: &[u8] =
    b"reap.pm-t2.controlled-trial-live.intent-record.v1\0";
const DISPATCH_RECORD_FINGERPRINT_DOMAIN: &[u8] =
    b"reap.pm-t2.controlled-trial-live.dispatch-record.v1\0";
const PREPARED_REQUEST_FINGERPRINT_DOMAIN: &[u8] =
    b"reap.pm-t2.controlled-trial-live.prepared-request.v1\0";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PmTrialLivePreflightBindingV1 {
    fingerprint: String,
    canonical_sha256: String,
    canonical_length: u64,
    validated_at_utc: String,
    dispatch_deadline_at_utc: String,
}

impl PmTrialLivePreflightBindingV1 {
    pub(crate) fn from_canonical(
        value: &CanonicalTrialPreflight,
    ) -> Result<Self, PmTrialLiveJournalError> {
        let binding = Self {
            fingerprint: value.fingerprint().to_owned(),
            canonical_sha256: hash_bytes(&[], value.canonical_bytes()),
            canonical_length: u64::try_from(value.canonical_bytes().len())
                .map_err(|_| PmTrialLiveJournalError::BoundExceeded)?,
            validated_at_utc: value.value().window.validated_at_utc.clone(),
            dispatch_deadline_at_utc: value.value().window.dispatch_deadline_at_utc.clone(),
        };
        binding.validate()?;
        Ok(binding)
    }

    #[must_use]
    pub fn fingerprint(&self) -> &str {
        &self.fingerprint
    }

    #[must_use]
    pub fn canonical_sha256(&self) -> &str {
        &self.canonical_sha256
    }

    #[must_use]
    pub fn validated_at_utc(&self) -> &str {
        &self.validated_at_utc
    }

    #[must_use]
    pub fn dispatch_deadline_at_utc(&self) -> &str {
        &self.dispatch_deadline_at_utc
    }

    pub(crate) fn validate(&self) -> Result<(), PmTrialLiveJournalError> {
        validate_fingerprint(&self.fingerprint)?;
        validate_fingerprint(&self.canonical_sha256)?;
        if self.canonical_length == 0 {
            return Err(PmTrialLiveJournalError::InvalidBinding);
        }
        let validated = validate_utc(&self.validated_at_utc)?;
        let deadline = validate_utc(&self.dispatch_deadline_at_utc)?;
        if deadline < validated {
            return Err(PmTrialLiveJournalError::InvalidBinding);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PmTrialLiveExpectedConsumptionV1 {
    pub(crate) binding: AuthorizationConsumptionBindingEvidence,
    pub(crate) binding_fingerprint: String,
    pub(crate) prepared_record_fingerprint: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PmTrialLiveConsumedFingerprintsV1 {
    pub(crate) binding_fingerprint: String,
    pub(crate) prepared_record_fingerprint: String,
    pub(crate) atomic_claim_fingerprint: String,
    pub(crate) consumed_record_fingerprint: String,
}

impl PmTrialLiveConsumedFingerprintsV1 {
    pub(crate) fn validate(&self) -> Result<(), PmTrialLiveJournalError> {
        validate_fingerprint(&self.binding_fingerprint)?;
        validate_fingerprint(&self.prepared_record_fingerprint)?;
        validate_fingerprint(&self.atomic_claim_fingerprint)?;
        validate_fingerprint(&self.consumed_record_fingerprint)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PmTrialLiveJournalScopeV1 {
    pub(crate) journal_family: String,
    pub(crate) journal_version: u32,
    pub(crate) intent_file: String,
    pub(crate) dispatch_file: String,
    pub(crate) canonical_config_sha256: String,
    pub(crate) canonical_config_length: u64,
    pub(crate) canonical_config_fingerprint: String,
    pub(crate) trial_plan_fingerprint: String,
    pub(crate) authorization_id: String,
    pub(crate) authorization_fingerprint: String,
    pub(crate) authorization_cleanup_not_after_utc: String,
    pub(crate) source_pin_manifest_sha256: String,
    pub(crate) release_binary_sha256: String,
    pub(crate) release_binary_length: u64,
    pub(crate) runtime_observed_at_utc: String,
    pub(crate) host: AuthorizationHostBinding,
    pub(crate) credential_slot_id: String,
    pub(crate) credential_slot_nonsecret_fingerprint_sha256: String,
    pub(crate) expected_order_id: String,
    pub(crate) place_semantic_request_commitment: String,
    pub(crate) owner_process_identity: String,
    pub(crate) artifact_directory_lease_fingerprint: String,
    pub(crate) trial: TrialConfig,
    pub(crate) expected_consumption: PmTrialLiveExpectedConsumptionV1,
    pub(crate) authorization: OfflineAuthorizationState,
    pub(crate) scope_fingerprint: String,
}

impl PmTrialLiveJournalScopeV1 {
    pub(crate) fn seal(mut self) -> Result<Self, PmTrialLiveJournalError> {
        self.scope_fingerprint = ZERO_FINGERPRINT.to_owned();
        self.scope_fingerprint = hash_domain(SCOPE_FINGERPRINT_DOMAIN, &self)?;
        self.validate()?;
        Ok(self)
    }

    pub(crate) fn validate(&self) -> Result<(), PmTrialLiveJournalError> {
        if self.journal_family != PM_TRIAL_LIVE_JOURNAL_FAMILY
            || self.journal_version != PM_TRIAL_LIVE_JOURNAL_VERSION
            || self.intent_file != PM_TRIAL_LIVE_INTENT_FILE_V1
            || self.dispatch_file != PM_TRIAL_LIVE_DISPATCH_FILE_V1
            || self.canonical_config_length == 0
            || self.release_binary_length == 0
            || self.authorization != OfflineAuthorizationState::DENIED
        {
            return Err(PmTrialLiveJournalError::InvalidBinding);
        }
        for fingerprint in [
            &self.canonical_config_sha256,
            &self.canonical_config_fingerprint,
            &self.trial_plan_fingerprint,
            &self.authorization_fingerprint,
            &self.source_pin_manifest_sha256,
            &self.release_binary_sha256,
            &self.credential_slot_nonsecret_fingerprint_sha256,
            &self.expected_order_id,
            &self.place_semantic_request_commitment,
            &self.artifact_directory_lease_fingerprint,
            &self.expected_consumption.binding_fingerprint,
            &self.expected_consumption.prepared_record_fingerprint,
            &self.scope_fingerprint,
        ] {
            validate_fingerprint(fingerprint)?;
        }
        if self.owner_process_identity.is_empty() || self.owner_process_identity.len() > 256 {
            return Err(PmTrialLiveJournalError::InvalidBinding);
        }
        validate_utc(&self.runtime_observed_at_utc)?;
        validate_utc(&self.authorization_cleanup_not_after_utc)?;
        self.trial
            .validate()
            .map_err(|_| PmTrialLiveJournalError::InvalidBinding)?;
        let mut basis = self.clone();
        basis.scope_fingerprint = ZERO_FINGERPRINT.to_owned();
        if hash_domain(SCOPE_FINGERPRINT_DOMAIN, &basis)? != self.scope_fingerprint {
            return Err(PmTrialLiveJournalError::InvalidBinding);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PmPlacePreparationV1 {
    canonical_config_fingerprint: String,
    trial_plan_fingerprint: String,
    preflight_fingerprint: String,
    preflight_canonical_sha256: String,
    semantic_request_commitment: String,
    request_commitment: String,
    expected_order_id: String,
    l2_timestamp_seconds: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PmPlacePreparationViewV1 {
    semantic_request_commitment: PlaceSemanticRequestCommitment,
    request_commitment: String,
    expected_order_id: ExpectedOrderId,
    l2_timestamp_seconds: u64,
}

impl PmPlacePreparationViewV1 {
    #[must_use]
    pub const fn semantic_request_commitment(&self) -> PlaceSemanticRequestCommitment {
        self.semantic_request_commitment
    }

    #[must_use]
    pub fn request_commitment(&self) -> &str {
        &self.request_commitment
    }

    #[must_use]
    pub const fn expected_order_id(&self) -> ExpectedOrderId {
        self.expected_order_id
    }

    #[must_use]
    pub const fn l2_timestamp_seconds(&self) -> u64 {
        self.l2_timestamp_seconds
    }

    #[must_use]
    pub const fn mutation_authority(&self) -> bool {
        false
    }
}

impl PmPlacePreparationV1 {
    pub fn for_scoped_plan(
        config: &CanonicalTrialConfig,
        preflight: &PmTrialLivePreflightBindingV1,
        identity: PlacePublicRequestIdentity,
        l2_timestamp_seconds: u64,
    ) -> Result<Self, PmTrialLiveJournalError> {
        if identity != config.exact_place_public_request_identity() {
            return Err(PmTrialLiveJournalError::InvalidBinding);
        }
        let value = Self {
            canonical_config_fingerprint: config.fingerprint().to_owned(),
            trial_plan_fingerprint: config.plan_fingerprint().to_owned(),
            preflight_fingerprint: preflight.fingerprint().to_owned(),
            preflight_canonical_sha256: preflight.canonical_sha256().to_owned(),
            semantic_request_commitment: lower_hex32(
                identity.semantic_request_commitment().bytes(),
            ),
            request_commitment: ZERO_FINGERPRINT.to_owned(),
            expected_order_id: lower_hex32(identity.expected_order_id().bytes()),
            l2_timestamp_seconds,
        };
        value.validate()?;
        Ok(value)
    }

    pub(crate) fn validate(&self) -> Result<(), PmTrialLiveJournalError> {
        validate_fingerprint(&self.canonical_config_fingerprint)?;
        validate_fingerprint(&self.trial_plan_fingerprint)?;
        validate_fingerprint(&self.preflight_fingerprint)?;
        validate_fingerprint(&self.preflight_canonical_sha256)?;
        validate_fingerprint(&self.semantic_request_commitment)?;
        validate_fingerprint(&self.expected_order_id)?;
        validate_l2_time(self.l2_timestamp_seconds)
    }

    pub(crate) fn expected_order_id(&self) -> &str {
        &self.expected_order_id
    }

    pub(crate) const fn l2_timestamp_seconds(&self) -> u64 {
        self.l2_timestamp_seconds
    }

    pub(crate) fn validate_against_scope(
        &self,
        scope: &PmTrialLiveJournalScopeV1,
        preflight: &PmTrialLivePreflightBindingV1,
        prior_intent_sequence: u8,
    ) -> Result<(), PmTrialLiveJournalError> {
        self.validate()?;
        if self.canonical_config_fingerprint != scope.canonical_config_fingerprint
            || self.trial_plan_fingerprint != scope.trial_plan_fingerprint
            || self.preflight_fingerprint != preflight.fingerprint
            || self.preflight_canonical_sha256 != preflight.canonical_sha256
            || self.expected_order_id != scope.expected_order_id
            || self.semantic_request_commitment != scope.place_semantic_request_commitment
        {
            return Err(PmTrialLiveJournalError::InvalidBinding);
        }
        validate_place_l2_time(scope, preflight, self.l2_timestamp_seconds)?;
        if self.request_commitment
            != self.calculate_request_commitment(scope, preflight, prior_intent_sequence)?
        {
            return Err(PmTrialLiveJournalError::InvalidBinding);
        }
        Ok(())
    }

    pub(crate) fn bind_request(
        mut self,
        scope: &PmTrialLiveJournalScopeV1,
        preflight: &PmTrialLivePreflightBindingV1,
        prior_intent_sequence: u8,
    ) -> Result<Self, PmTrialLiveJournalError> {
        self.request_commitment =
            self.calculate_request_commitment(scope, preflight, prior_intent_sequence)?;
        self.validate_against_scope(scope, preflight, prior_intent_sequence)?;
        Ok(self)
    }

    fn calculate_request_commitment(
        &self,
        scope: &PmTrialLiveJournalScopeV1,
        preflight: &PmTrialLivePreflightBindingV1,
        prior_intent_sequence: u8,
    ) -> Result<String, PmTrialLiveJournalError> {
        #[derive(Serialize)]
        struct Basis<'a> {
            scope_fingerprint: &'a str,
            preflight_fingerprint: &'a str,
            method: &'static str,
            route: &'static str,
            prior_intent_sequence: u8,
            semantic_request_commitment: &'a str,
            expected_order_id: &'a str,
            l2_timestamp_seconds: u64,
        }
        hash_domain(
            PREPARED_REQUEST_FINGERPRINT_DOMAIN,
            &Basis {
                scope_fingerprint: &scope.scope_fingerprint,
                preflight_fingerprint: &preflight.fingerprint,
                method: "POST",
                route: "/order",
                prior_intent_sequence,
                semantic_request_commitment: &self.semantic_request_commitment,
                expected_order_id: &self.expected_order_id,
                l2_timestamp_seconds: self.l2_timestamp_seconds,
            },
        )
    }

    pub(crate) fn view(
        &self,
        identity: PlacePublicRequestIdentity,
    ) -> Result<PmPlacePreparationViewV1, PmTrialLiveJournalError> {
        if self.semantic_request_commitment
            != lower_hex32(identity.semantic_request_commitment().bytes())
            || self.expected_order_id != lower_hex32(identity.expected_order_id().bytes())
        {
            return Err(PmTrialLiveJournalError::InvalidBinding);
        }
        Ok(PmPlacePreparationViewV1 {
            semantic_request_commitment: identity.semantic_request_commitment(),
            request_commitment: self.request_commitment.clone(),
            expected_order_id: identity.expected_order_id(),
            l2_timestamp_seconds: self.l2_timestamp_seconds,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PmCancelPreparationV1 {
    canonical_config_fingerprint: String,
    trial_plan_fingerprint: String,
    preflight_fingerprint: String,
    preflight_canonical_sha256: String,
    exact_venue_order_id: String,
    semantic_request_commitment: String,
    request_commitment: String,
    l2_timestamp_seconds: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PmCancelPreparationViewV1 {
    semantic_request_commitment: OwnedCancelSemanticRequestCommitment,
    request_commitment: String,
    exact_venue_order_id: FixedOrderId,
    l2_timestamp_seconds: u64,
    dispatch_class: PmCancelDispatchClassV1,
}

impl PmCancelPreparationViewV1 {
    #[must_use]
    pub const fn semantic_request_commitment(&self) -> OwnedCancelSemanticRequestCommitment {
        self.semantic_request_commitment
    }

    #[must_use]
    pub fn request_commitment(&self) -> &str {
        &self.request_commitment
    }

    #[must_use]
    pub const fn exact_venue_order_id(&self) -> FixedOrderId {
        self.exact_venue_order_id
    }

    #[must_use]
    pub const fn l2_timestamp_seconds(&self) -> u64 {
        self.l2_timestamp_seconds
    }

    #[must_use]
    pub const fn dispatch_class(&self) -> PmCancelDispatchClassV1 {
        self.dispatch_class
    }

    #[must_use]
    pub const fn mutation_authority(&self) -> bool {
        false
    }
}

impl PmCancelPreparationV1 {
    pub fn for_scoped_plan(
        config: &CanonicalTrialConfig,
        preflight: &PmTrialLivePreflightBindingV1,
        exact_venue_order_id: FixedOrderId,
        l2_timestamp_seconds: u64,
    ) -> Result<Self, PmTrialLiveJournalError> {
        let semantic_request_commitment = lower_hex32(
            derive_owned_cancel_semantic_request_commitment(exact_venue_order_id).bytes(),
        );
        let value = Self {
            canonical_config_fingerprint: config.fingerprint().to_owned(),
            trial_plan_fingerprint: config.plan_fingerprint().to_owned(),
            preflight_fingerprint: preflight.fingerprint().to_owned(),
            preflight_canonical_sha256: preflight.canonical_sha256().to_owned(),
            exact_venue_order_id: exact_venue_order_id.to_string(),
            semantic_request_commitment,
            request_commitment: ZERO_FINGERPRINT.to_owned(),
            l2_timestamp_seconds,
        };
        value.validate()?;
        Ok(value)
    }

    pub(crate) fn validate(&self) -> Result<(), PmTrialLiveJournalError> {
        validate_fingerprint(&self.canonical_config_fingerprint)?;
        validate_fingerprint(&self.trial_plan_fingerprint)?;
        validate_fingerprint(&self.preflight_fingerprint)?;
        validate_fingerprint(&self.preflight_canonical_sha256)?;
        validate_order_id(&self.exact_venue_order_id)?;
        validate_fingerprint(&self.semantic_request_commitment)?;
        validate_l2_time(self.l2_timestamp_seconds)
    }

    pub(crate) fn exact_venue_order_id(&self) -> &str {
        &self.exact_venue_order_id
    }

    pub(crate) const fn l2_timestamp_seconds(&self) -> u64 {
        self.l2_timestamp_seconds
    }

    pub(crate) fn validate_against_scope(
        &self,
        scope: &PmTrialLiveJournalScopeV1,
        preflight: &PmTrialLivePreflightBindingV1,
        prior_cancel_intent_sequence: u8,
        minimum_l2_timestamp_seconds: u64,
    ) -> Result<(), PmTrialLiveJournalError> {
        self.validate()?;
        if self.canonical_config_fingerprint != scope.canonical_config_fingerprint
            || self.trial_plan_fingerprint != scope.trial_plan_fingerprint
            || self.preflight_fingerprint != preflight.fingerprint
            || self.preflight_canonical_sha256 != preflight.canonical_sha256
        {
            return Err(PmTrialLiveJournalError::InvalidBinding);
        }
        let fixed = FixedOrderId::parse(&self.exact_venue_order_id)
            .map_err(|_| PmTrialLiveJournalError::InvalidBinding)?;
        if self.semantic_request_commitment
            != lower_hex32(derive_owned_cancel_semantic_request_commitment(fixed).bytes())
        {
            return Err(PmTrialLiveJournalError::InvalidBinding);
        }
        validate_cancel_l2_time(
            scope,
            self.l2_timestamp_seconds,
            minimum_l2_timestamp_seconds,
        )?;
        if self.request_commitment
            != self.calculate_request_commitment(scope, preflight, prior_cancel_intent_sequence)?
        {
            return Err(PmTrialLiveJournalError::InvalidBinding);
        }
        Ok(())
    }

    pub(crate) fn bind_request(
        mut self,
        scope: &PmTrialLiveJournalScopeV1,
        preflight: &PmTrialLivePreflightBindingV1,
        prior_cancel_intent_sequence: u8,
        minimum_l2_timestamp_seconds: u64,
    ) -> Result<Self, PmTrialLiveJournalError> {
        self.request_commitment =
            self.calculate_request_commitment(scope, preflight, prior_cancel_intent_sequence)?;
        self.validate_against_scope(
            scope,
            preflight,
            prior_cancel_intent_sequence,
            minimum_l2_timestamp_seconds,
        )?;
        Ok(self)
    }

    fn calculate_request_commitment(
        &self,
        scope: &PmTrialLiveJournalScopeV1,
        preflight: &PmTrialLivePreflightBindingV1,
        prior_cancel_intent_sequence: u8,
    ) -> Result<String, PmTrialLiveJournalError> {
        #[derive(Serialize)]
        struct Basis<'a> {
            scope_fingerprint: &'a str,
            preflight_fingerprint: &'a str,
            method: &'static str,
            route: &'static str,
            prior_cancel_intent_sequence: u8,
            semantic_request_commitment: &'a str,
            exact_venue_order_id: &'a str,
            l2_timestamp_seconds: u64,
        }
        hash_domain(
            PREPARED_REQUEST_FINGERPRINT_DOMAIN,
            &Basis {
                scope_fingerprint: &scope.scope_fingerprint,
                preflight_fingerprint: &preflight.fingerprint,
                method: "DELETE",
                route: "/order",
                prior_cancel_intent_sequence,
                semantic_request_commitment: &self.semantic_request_commitment,
                exact_venue_order_id: &self.exact_venue_order_id,
                l2_timestamp_seconds: self.l2_timestamp_seconds,
            },
        )
    }

    pub(crate) fn view(
        &self,
        dispatch_class: PmCancelDispatchClassV1,
    ) -> Result<PmCancelPreparationViewV1, PmTrialLiveJournalError> {
        let exact_venue_order_id = FixedOrderId::parse(&self.exact_venue_order_id)
            .map_err(|_| PmTrialLiveJournalError::InvalidBinding)?;
        let semantic_request_commitment =
            derive_owned_cancel_semantic_request_commitment(exact_venue_order_id);
        if self.semantic_request_commitment != lower_hex32(semantic_request_commitment.bytes()) {
            return Err(PmTrialLiveJournalError::InvalidBinding);
        }
        Ok(PmCancelPreparationViewV1 {
            semantic_request_commitment,
            request_commitment: self.request_commitment.clone(),
            exact_venue_order_id,
            l2_timestamp_seconds: self.l2_timestamp_seconds,
            dispatch_class,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PmPlaceResultKindV1 {
    Accepted,
    Rejected,
    OutOfProfile,
    AcknowledgementUnknown,
    DefinitelyNotDispatched,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PmCancelResultKindV1 {
    Canceled,
    NotCanceled,
    OutOfProfile,
    AcknowledgementUnknown,
    DefinitelyNotDispatched,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "class", rename_all = "snake_case", deny_unknown_fields)]
pub enum PmCancelDispatchClassV1 {
    Primary,
    Recovery { ordinal: u8 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PmReconciliationOrderStateV1 {
    Absent,
    ExactLive,
    ExactCanceled,
    ExactFilled,
    Ambiguous,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PmIntentTerminalDispositionV1 {
    Completed,
    Stopped,
    OperatorActionRequired,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CounterpartLinkV1 {
    pub(crate) sequence: u8,
    pub(crate) record_fingerprint: String,
}

impl CounterpartLinkV1 {
    pub(crate) fn validate(&self) -> Result<(), PmTrialLiveJournalError> {
        validate_fingerprint(&self.record_fingerprint)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "record", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum IntentRecordV1 {
    Header {
        scope: Box<PmTrialLiveJournalScopeV1>,
    },
    PreflightBound {
        preflight: PmTrialLivePreflightBindingV1,
    },
    PlaceIntent {
        created_at_utc: String,
    },
    PlaceOutcomeBridge {
        dispatch: CounterpartLinkV1,
        outcome: PmPlaceResultKindV1,
        observed_order_id: Option<String>,
    },
    CancelIntent {
        created_at_utc: String,
        ownership_source: CounterpartLinkV1,
        exact_venue_order_id: String,
        dispatch_class: PmCancelDispatchClassV1,
    },
    CancelOutcomeBridge {
        dispatch: CounterpartLinkV1,
        outcome: PmCancelResultKindV1,
        exact_venue_order_id: String,
    },
    Reconciliation {
        observed_at_utc: String,
        state: PmReconciliationOrderStateV1,
        exact_venue_order_id: Option<String>,
        dispatch: CounterpartLinkV1,
    },
    Terminal {
        terminal_at_utc: String,
        disposition: PmIntentTerminalDispositionV1,
        dispatch_terminal: CounterpartLinkV1,
        terminal_is_evidence_not_authority: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "record", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum DispatchRecordV1 {
    Header {
        scope: Box<PmTrialLiveJournalScopeV1>,
    },
    PreflightBound {
        preflight: PmTrialLivePreflightBindingV1,
        intent_preflight: CounterpartLinkV1,
    },
    PlacePrepared {
        intent: CounterpartLinkV1,
        preparation: PmPlacePreparationV1,
    },
    PlaceDispatchAuthorized {
        prepared_sequence: u8,
        prepared_record_fingerprint: String,
        consumption: PmTrialLiveConsumedFingerprintsV1,
        production_order_entry_authorized: bool,
        real_order_submission_authorized: bool,
        place_dispatch_allowance: u8,
    },
    PlaceResult {
        dispatch_authorized_sequence: u8,
        dispatch_authorized_fingerprint: String,
        outcome: PmPlaceResultKindV1,
        observed_order_id: Option<String>,
    },
    CancelPrepared {
        intent: CounterpartLinkV1,
        dispatch_class: PmCancelDispatchClassV1,
        preparation: PmCancelPreparationV1,
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
        intent: CounterpartLinkV1,
        terminal_is_evidence_not_authority: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct IntentLineV1 {
    pub(crate) schema_version: u32,
    pub(crate) sequence: u8,
    pub(crate) previous_record_fingerprint: String,
    pub(crate) scope_fingerprint: String,
    pub(crate) body: IntentRecordV1,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DispatchLineV1 {
    pub(crate) schema_version: u32,
    pub(crate) sequence: u8,
    pub(crate) previous_record_fingerprint: String,
    pub(crate) scope_fingerprint: String,
    pub(crate) body: DispatchRecordV1,
}

pub(crate) fn intent_fingerprint(line: &IntentLineV1) -> Result<String, PmTrialLiveJournalError> {
    hash_domain(INTENT_RECORD_FINGERPRINT_DOMAIN, line)
}

pub(crate) fn dispatch_fingerprint(
    line: &DispatchLineV1,
) -> Result<String, PmTrialLiveJournalError> {
    hash_domain(DISPATCH_RECORD_FINGERPRINT_DOMAIN, line)
}

pub(crate) fn validate_utc(value: &str) -> Result<DateTime<Utc>, PmTrialLiveJournalError> {
    let parsed = DateTime::parse_from_rfc3339(value)
        .map_err(|_| PmTrialLiveJournalError::InvalidBinding)?
        .with_timezone(&Utc);
    if parsed.to_rfc3339_opts(SecondsFormat::Secs, true) != value {
        return Err(PmTrialLiveJournalError::InvalidBinding);
    }
    Ok(parsed)
}

pub(crate) fn validate_order_id(value: &str) -> Result<(), PmTrialLiveJournalError> {
    let Some(hex) = value.strip_prefix("0x") else {
        return Err(PmTrialLiveJournalError::InvalidBinding);
    };
    if hex.len() != 64
        || hex
            .bytes()
            .any(|byte| !byte.is_ascii_digit() && !(b'a'..=b'f').contains(&byte))
    {
        return Err(PmTrialLiveJournalError::InvalidBinding);
    }
    Ok(())
}

fn validate_l2_time(value: u64) -> Result<(), PmTrialLiveJournalError> {
    if !(1_000_000_000..=9_999_999_999).contains(&value) {
        return Err(PmTrialLiveJournalError::InvalidBinding);
    }
    Ok(())
}

fn validate_place_l2_time(
    scope: &PmTrialLiveJournalScopeV1,
    preflight: &PmTrialLivePreflightBindingV1,
    value: u64,
) -> Result<(), PmTrialLiveJournalError> {
    let l2 = l2_utc(value)?;
    let validated = validate_utc(&preflight.validated_at_utc)?;
    let deadline = validate_utc(&preflight.dispatch_deadline_at_utc)?;
    let expires = validate_utc(
        &scope
            .expected_consumption
            .binding
            .authorization_expires_at_utc,
    )?;
    if l2 < validated || l2 > deadline || l2 >= expires {
        return Err(PmTrialLiveJournalError::InvalidBinding);
    }
    Ok(())
}

fn validate_cancel_l2_time(
    scope: &PmTrialLiveJournalScopeV1,
    value: u64,
    minimum: u64,
) -> Result<(), PmTrialLiveJournalError> {
    let l2 = l2_utc(value)?;
    let cleanup = validate_utc(&scope.authorization_cleanup_not_after_utc)?;
    if value < minimum || l2 > cleanup {
        return Err(PmTrialLiveJournalError::InvalidBinding);
    }
    Ok(())
}

fn l2_utc(value: u64) -> Result<DateTime<Utc>, PmTrialLiveJournalError> {
    validate_l2_time(value)?;
    let seconds = i64::try_from(value).map_err(|_| PmTrialLiveJournalError::InvalidBinding)?;
    DateTime::from_timestamp(seconds, 0).ok_or(PmTrialLiveJournalError::InvalidBinding)
}

fn lower_hex32(bytes: [u8; 32]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::PmPlacePreparationV1;

    #[test]
    fn place_preparation_v1_canonical_golden_bytes_are_frozen() {
        let value = PmPlacePreparationV1 {
            canonical_config_fingerprint:
                "1111111111111111111111111111111111111111111111111111111111111111".into(),
            trial_plan_fingerprint:
                "2222222222222222222222222222222222222222222222222222222222222222".into(),
            preflight_fingerprint:
                "3333333333333333333333333333333333333333333333333333333333333333".into(),
            preflight_canonical_sha256:
                "4444444444444444444444444444444444444444444444444444444444444444".into(),
            semantic_request_commitment:
                "5555555555555555555555555555555555555555555555555555555555555555".into(),
            request_commitment: "6666666666666666666666666666666666666666666666666666666666666666"
                .into(),
            expected_order_id: "7777777777777777777777777777777777777777777777777777777777777777"
                .into(),
            l2_timestamp_seconds: 1_786_277_104,
        };
        let bytes = serde_json::to_vec(&value).expect("serialize golden value");
        assert_eq!(
            bytes,
            br#"{"canonical_config_fingerprint":"1111111111111111111111111111111111111111111111111111111111111111","trial_plan_fingerprint":"2222222222222222222222222222222222222222222222222222222222222222","preflight_fingerprint":"3333333333333333333333333333333333333333333333333333333333333333","preflight_canonical_sha256":"4444444444444444444444444444444444444444444444444444444444444444","semantic_request_commitment":"5555555555555555555555555555555555555555555555555555555555555555","request_commitment":"6666666666666666666666666666666666666666666666666666666666666666","expected_order_id":"7777777777777777777777777777777777777777777777777777777777777777","l2_timestamp_seconds":1786277104}"#
        );
    }
}
