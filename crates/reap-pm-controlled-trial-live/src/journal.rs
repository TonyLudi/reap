use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use reap_pm_controlled_trial::{
    AuthorizationConsumptionBindingEvidence, AuthorizationConsumptionState,
    AuthorizationConsumptionVerification, AuthorizationRuntimeBinding, CanonicalAuthorization,
    CanonicalTrialConfig, CanonicalTrialPreflight, ConsumedAuthorizationConsumption,
    OfflineAuthorizationState, PlacePublicRequestIdentity, TrialAuthorizationConsumptionLeaseState,
    TrialJournalLeaseEvidence, verify_authorization, verify_authorization_consumption,
};

use crate::{
    PmTrialLiveJournalError,
    hash::{ZERO_FINGERPRINT, canonical_json, hash_domain, validate_fingerprint},
    protected::{ProtectedArtifactLease, ProtectedJournal},
    recovery::{
        PmTrialLiveRecoveryClassificationV1, PmTrialLiveRecoveryProjectionV1, revalidate_projection,
    },
    schema::{
        CounterpartLinkV1, DispatchLineV1, DispatchRecordV1, IntentLineV1, IntentRecordV1,
        MAX_JOURNAL_BYTES, MAX_JOURNAL_LINE_BYTES, PM_TRIAL_LIVE_DISPATCH_FILE_V1,
        PM_TRIAL_LIVE_INTENT_FILE_V1, PM_TRIAL_LIVE_JOURNAL_FAMILY, PM_TRIAL_LIVE_JOURNAL_VERSION,
        PmCancelDispatchClassV1, PmCancelPreparationV1, PmCancelPreparationViewV1,
        PmCancelResultKindV1, PmIntentTerminalDispositionV1, PmPlacePreparationV1,
        PmPlacePreparationViewV1, PmPlaceResultKindV1, PmReconciliationOrderStateV1,
        PmTrialLiveConsumedFingerprintsV1, PmTrialLiveExpectedConsumptionV1,
        PmTrialLiveJournalScopeV1, PmTrialLivePreflightBindingV1, dispatch_fingerprint,
        intent_fingerprint, validate_order_id, validate_utc,
    },
};

const CONSUMPTION_BINDING_FINGERPRINT_DOMAIN: &[u8] =
    b"reap.pm-t2.authorization-consumption.binding.v1\0";
const CONSUMPTION_RECORD_FINGERPRINT_DOMAIN: &[u8] =
    b"reap.pm-t2.authorization-consumption.record.v1\0";

struct RuntimeIdentity;

struct DurableAckCore {
    runtime: Arc<RuntimeIdentity>,
    sequence: u8,
    record_fingerprint: String,
}

impl DurableAckCore {
    fn link(&self) -> CounterpartLinkV1 {
        CounterpartLinkV1 {
            sequence: self.sequence,
            record_fingerprint: self.record_fingerprint.clone(),
        }
    }

    fn require_runtime(
        &self,
        expected: &Arc<RuntimeIdentity>,
    ) -> Result<(), PmTrialLiveJournalError> {
        if !Arc::ptr_eq(&self.runtime, expected) {
            return Err(PmTrialLiveJournalError::ForeignAcknowledgement);
        }
        Ok(())
    }
}

macro_rules! simple_ack {
    ($name:ident) => {
        pub struct $name {
            core: DurableAckCore,
        }

        impl $name {
            #[must_use]
            pub const fn sequence(&self) -> u8 {
                self.core.sequence
            }

            #[must_use]
            pub fn record_fingerprint(&self) -> &str {
                &self.core.record_fingerprint
            }
        }

        impl std::fmt::Debug for $name {
            fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter
                    .debug_struct(stringify!($name))
                    .field("sequence", &self.core.sequence)
                    .field("record_fingerprint", &self.core.record_fingerprint)
                    .finish()
            }
        }
    };
}

simple_ack!(PmDurablePlaceIntentAckV1);
simple_ack!(PmDurableIntentTerminalAckV1);
simple_ack!(PmDurableReconciliationAckV1);

pub struct PmDurablePlacePreparedAckV1 {
    core: DurableAckCore,
    preparation: PmPlacePreparationViewV1,
}

impl PmDurablePlacePreparedAckV1 {
    #[must_use]
    pub const fn sequence(&self) -> u8 {
        self.core.sequence
    }

    #[must_use]
    pub fn record_fingerprint(&self) -> &str {
        &self.core.record_fingerprint
    }

    #[must_use]
    pub const fn preparation(&self) -> &PmPlacePreparationViewV1 {
        &self.preparation
    }
}

pub struct PmDurablePlaceDispatchAckV1 {
    core: DurableAckCore,
    preparation: PmPlacePreparationViewV1,
}

impl PmDurablePlaceDispatchAckV1 {
    #[must_use]
    pub const fn sequence(&self) -> u8 {
        self.core.sequence
    }

    #[must_use]
    pub fn record_fingerprint(&self) -> &str {
        &self.core.record_fingerprint
    }

    #[must_use]
    pub const fn preparation(&self) -> &PmPlacePreparationViewV1 {
        &self.preparation
    }
}

impl std::fmt::Debug for PmDurablePlaceDispatchAckV1 {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PmDurablePlaceDispatchAckV1")
            .field("sequence", &self.core.sequence)
            .field("record_fingerprint", &self.core.record_fingerprint)
            .field("network_send_authority", &false)
            .finish()
    }
}

pub struct PmDurablePlaceResultAckV1 {
    core: DurableAckCore,
    outcome: PmPlaceResultKindV1,
    observed_order_id: Option<String>,
}

impl PmDurablePlaceResultAckV1 {
    #[must_use]
    pub const fn outcome(&self) -> PmPlaceResultKindV1 {
        self.outcome
    }
}

pub struct PmDurablePlaceOutcomeBridgeAckV1 {
    core: DurableAckCore,
    dispatch: CounterpartLinkV1,
}

pub struct PmJournalOwnedVenueOrderV1 {
    runtime: Arc<RuntimeIdentity>,
    exact_venue_order_id: String,
}

impl std::fmt::Debug for PmJournalOwnedVenueOrderV1 {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PmJournalOwnedVenueOrderV1")
            .field("exact_venue_order_id", &self.exact_venue_order_id)
            .field("network_send_authority", &false)
            .finish()
    }
}

impl PmJournalOwnedVenueOrderV1 {
    pub fn exact_venue_order_id(
        &self,
    ) -> Result<reap_pm_controlled_trial::FixedOrderId, PmTrialLiveJournalError> {
        reap_pm_controlled_trial::FixedOrderId::parse(&self.exact_venue_order_id)
            .map_err(|_| PmTrialLiveJournalError::InvalidRecord)
    }
}

pub struct PmDurableCancelDispatchAckV1 {
    core: DurableAckCore,
    dispatch_class: PmCancelDispatchClassV1,
    exact_venue_order_id: String,
    preparation: PmCancelPreparationViewV1,
}

pub struct PmDurableCancelIntentAckV1 {
    core: DurableAckCore,
    dispatch_class: PmCancelDispatchClassV1,
    exact_venue_order_id: String,
}

pub struct PmDurableCancelPreparedAckV1 {
    core: DurableAckCore,
    dispatch_class: PmCancelDispatchClassV1,
    exact_venue_order_id: String,
    preparation: PmCancelPreparationViewV1,
}

impl PmDurableCancelDispatchAckV1 {
    #[must_use]
    pub const fn sequence(&self) -> u8 {
        self.core.sequence
    }

    #[must_use]
    pub const fn dispatch_class(&self) -> PmCancelDispatchClassV1 {
        self.dispatch_class
    }

    #[must_use]
    pub const fn preparation(&self) -> &PmCancelPreparationViewV1 {
        &self.preparation
    }
}

pub struct PmDurableCancelResultAckV1 {
    core: DurableAckCore,
    outcome: PmCancelResultKindV1,
    exact_venue_order_id: String,
}

pub struct PmDurableCancelOutcomeBridgeAckV1 {
    core: DurableAckCore,
    dispatch: CounterpartLinkV1,
}

/// Move-only proof that one exact durable PlacePrepared acknowledgement was
/// followed by the exact take-once authorization consumption. It has no
/// request bytes, credential, transport, or network-send operation.
pub struct PmPreparedConsumedAuthorizationProofV1 {
    prepared: PmDurablePlacePreparedAckV1,
    owner: ConsumedAuthorizationConsumption,
    consumption: PmTrialLiveConsumedFingerprintsV1,
}

impl std::fmt::Debug for PmPreparedConsumedAuthorizationProofV1 {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PmPreparedConsumedAuthorizationProofV1")
            .field("prepared_sequence", &self.prepared.core.sequence)
            .field("consumption", &self.consumption)
            .field("network_send_authority", &false)
            .finish()
    }
}

struct IntentWriter {
    file: ProtectedJournal,
    bytes: Vec<u8>,
    lines: Vec<IntentLineV1>,
}

struct DispatchWriter {
    file: ProtectedJournal,
    bytes: Vec<u8>,
    lines: Vec<DispatchLineV1>,
}

pub struct PmControlledTrialLiveJournals {
    scope: PmTrialLiveJournalScopeV1,
    preflight: PmTrialLivePreflightBindingV1,
    place_identity: PlacePublicRequestIdentity,
    runtime: Arc<RuntimeIdentity>,
    artifact_lease: ProtectedArtifactLease,
    intent: IntentWriter,
    dispatch: DispatchWriter,
}

/// Move-only owner of the exact fixed files and their continuous exclusive
/// leases while the network-free preflight evidence is collected.
pub struct PmPendingTrialLiveJournalsV1 {
    scope: PmTrialLiveJournalScopeV1,
    place_identity: PlacePublicRequestIdentity,
    runtime: Arc<RuntimeIdentity>,
    artifact_lease: ProtectedArtifactLease,
    intent: IntentWriter,
    dispatch: DispatchWriter,
    lease_evidence: TrialJournalLeaseEvidence,
}

impl PmPendingTrialLiveJournalsV1 {
    #[must_use]
    pub const fn lease_evidence(&self) -> &TrialJournalLeaseEvidence {
        &self.lease_evidence
    }

    #[must_use]
    pub fn scope_fingerprint(&self) -> &str {
        &self.scope.scope_fingerprint
    }

    pub fn bind_preflight(
        self,
        canonical: CanonicalTrialPreflight,
    ) -> Result<PmControlledTrialLiveJournals, PmTrialLiveJournalError> {
        validate_bound_preflight(&self.scope, &self.lease_evidence, &canonical)?;
        self.artifact_lease.validate()?;
        let preflight = PmTrialLivePreflightBindingV1::from_canonical(&canonical)?;
        let mut journals = PmControlledTrialLiveJournals {
            scope: self.scope,
            preflight: preflight.clone(),
            place_identity: self.place_identity,
            runtime: self.runtime,
            artifact_lease: self.artifact_lease,
            intent: self.intent,
            dispatch: self.dispatch,
        };
        let intent_preflight = journals.append_intent(IntentRecordV1::PreflightBound {
            preflight: preflight.clone(),
        })?;
        journals.append_dispatch(DispatchRecordV1::PreflightBound {
            preflight,
            intent_preflight: intent_preflight.link(),
        })?;
        journals.artifact_lease.validate()?;
        Ok(journals)
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
}

impl PmControlledTrialLiveJournals {
    pub fn create_pending_preflight(
        config: &CanonicalTrialConfig,
        authorization: &CanonicalAuthorization,
        runtime: &AuthorizationRuntimeBinding,
    ) -> Result<PmPendingTrialLiveJournalsV1, PmTrialLiveJournalError> {
        let artifact_directory = Path::new(&config.value().journal.artifact_directory);
        let mut artifact_lease = ProtectedArtifactLease::acquire(artifact_directory)?;
        let owner_process_identity = format!(
            "pid:{}:boot:{}",
            std::process::id(),
            runtime.host.boot_identity
        );
        let consumption = verify_prepared_consumption(config, authorization)?;
        let scope = build_scope(
            config,
            authorization,
            runtime,
            owner_process_identity.clone(),
            artifact_lease.fingerprint().to_owned(),
            consumption.latest_record_fingerprint.clone(),
        )?;
        let place_identity = config.exact_place_public_request_identity();
        if consumption.binding_fingerprint != scope.expected_consumption.binding_fingerprint {
            return Err(PmTrialLiveJournalError::InvalidBinding);
        }
        let (intent_path, dispatch_path) = bound_paths(config);
        let mut intent_file = ProtectedJournal::create_new(&intent_path, MAX_JOURNAL_BYTES)?;
        artifact_lease.refresh_after_bound_create()?;
        let intent_header = IntentLineV1 {
            schema_version: PM_TRIAL_LIVE_JOURNAL_VERSION,
            sequence: 0,
            previous_record_fingerprint: ZERO_FINGERPRINT.to_owned(),
            scope_fingerprint: scope.scope_fingerprint.clone(),
            body: IntentRecordV1::Header {
                scope: Box::new(scope.clone()),
            },
        };
        let intent_bytes = encode_line(&intent_header)?;
        intent_file.append_durable(&[], &intent_bytes)?;

        let mut dispatch_file = ProtectedJournal::create_new(&dispatch_path, MAX_JOURNAL_BYTES)?;
        artifact_lease.refresh_after_bound_create()?;
        intent_file.refresh_parent_after_bound_create()?;
        let dispatch_header = DispatchLineV1 {
            schema_version: PM_TRIAL_LIVE_JOURNAL_VERSION,
            sequence: 0,
            previous_record_fingerprint: ZERO_FINGERPRINT.to_owned(),
            scope_fingerprint: scope.scope_fingerprint.clone(),
            body: DispatchRecordV1::Header {
                scope: Box::new(scope.clone()),
            },
        };
        let dispatch_bytes = encode_line(&dispatch_header)?;
        dispatch_file.append_durable(&[], &dispatch_bytes)?;
        artifact_lease.validate()?;

        let lease_evidence = TrialJournalLeaseEvidence {
            owner_process_identity,
            owner_process_count: 1,
            artifact_directory: config.value().journal.artifact_directory.clone(),
            artifact_directory_lease_fingerprint: scope
                .artifact_directory_lease_fingerprint
                .clone(),
            artifact_directory_exclusive: true,
            product_journal_path: intent_path
                .to_str()
                .ok_or(PmTrialLiveJournalError::InvalidBinding)?
                .to_owned(),
            product_journal_schema_version: PM_TRIAL_LIVE_JOURNAL_VERSION,
            product_journal_scope_fingerprint: scope.scope_fingerprint.clone(),
            product_journal_exclusive: true,
            authenticated_journal_path: dispatch_path
                .to_str()
                .ok_or(PmTrialLiveJournalError::InvalidBinding)?
                .to_owned(),
            authenticated_journal_schema_version: PM_TRIAL_LIVE_JOURNAL_VERSION,
            authenticated_journal_scope_fingerprint: scope.scope_fingerprint.clone(),
            authenticated_journal_exclusive: true,
            leases_held_continuously: true,
            recovery_state_unambiguous: true,
            authorization_consumption_state:
                TrialAuthorizationConsumptionLeaseState::PreparedUnconsumed,
            authorization_consumption_binding_fingerprint: consumption.binding_fingerprint,
            authorization_consumption_ledger_record_count: 1,
            authorization_consumption_claim_absent: true,
        };
        Ok(PmPendingTrialLiveJournalsV1 {
            scope,
            place_identity,
            runtime: Arc::new(RuntimeIdentity),
            artifact_lease,
            intent: IntentWriter {
                file: intent_file,
                bytes: intent_bytes,
                lines: vec![intent_header],
            },
            dispatch: DispatchWriter {
                file: dispatch_file,
                bytes: dispatch_bytes,
                lines: vec![dispatch_header],
            },
            lease_evidence,
        })
    }

    #[must_use]
    pub const fn preflight_binding(&self) -> &PmTrialLivePreflightBindingV1 {
        &self.preflight
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

    pub fn record_place_intent(
        &mut self,
        created_at_utc: String,
    ) -> Result<PmDurablePlaceIntentAckV1, PmTrialLiveJournalError> {
        validate_utc(&created_at_utc)?;
        if self.intent.lines.len() != 2
            || self.dispatch.lines.len() != 2
            || !matches!(
                self.intent.lines.last().map(|line| &line.body),
                Some(IntentRecordV1::PreflightBound { .. })
            )
            || !matches!(
                self.dispatch.lines.last().map(|line| &line.body),
                Some(DispatchRecordV1::PreflightBound { .. })
            )
        {
            return Err(PmTrialLiveJournalError::InvalidTransition);
        }
        let core = self.append_intent(IntentRecordV1::PlaceIntent { created_at_utc })?;
        Ok(PmDurablePlaceIntentAckV1 { core })
    }

    pub fn record_place_prepared(
        &mut self,
        intent: PmDurablePlaceIntentAckV1,
        preparation: PmPlacePreparationV1,
    ) -> Result<PmDurablePlacePreparedAckV1, PmTrialLiveJournalError> {
        intent.core.require_runtime(&self.runtime)?;
        let preparation =
            preparation.bind_request(&self.scope, &self.preflight, intent.core.sequence)?;
        let preparation_view = preparation.view(self.place_identity)?;
        if !matches!(
            self.intent.lines.last().map(|line| &line.body),
            Some(IntentRecordV1::PlaceIntent { .. })
        ) || self.dispatch.lines.len() != 2
        {
            return Err(PmTrialLiveJournalError::InvalidTransition);
        }
        let core = self.append_dispatch(DispatchRecordV1::PlacePrepared {
            intent: intent.core.link(),
            preparation,
        })?;
        Ok(PmDurablePlacePreparedAckV1 {
            core,
            preparation: preparation_view,
        })
    }

    pub fn bind_consumed_authorization(
        &mut self,
        prepared: PmDurablePlacePreparedAckV1,
        owner: ConsumedAuthorizationConsumption,
        verification: &AuthorizationConsumptionVerification,
    ) -> Result<PmPreparedConsumedAuthorizationProofV1, PmTrialLiveJournalError> {
        prepared.core.require_runtime(&self.runtime)?;
        let consumption = validate_consumed_authorization(&self.scope, &owner, verification)?;
        // The fixed take-once claim is the only expected directory-entry
        // creation after both journals were pinned. Re-resolve all three held
        // directory descriptors only after the exact durable claim and
        // Consumed evidence have been validated against this scope.
        self.artifact_lease.refresh_after_bound_create()?;
        self.intent.file.refresh_parent_after_bound_create()?;
        self.dispatch.file.refresh_parent_after_bound_create()?;
        self.artifact_lease.validate()?;
        Ok(PmPreparedConsumedAuthorizationProofV1 {
            prepared,
            owner,
            consumption,
        })
    }

    pub fn record_place_dispatch_authorized(
        &mut self,
        proof: PmPreparedConsumedAuthorizationProofV1,
    ) -> Result<
        (
            PmDurablePlaceDispatchAckV1,
            ConsumedAuthorizationConsumption,
        ),
        PmTrialLiveJournalError,
    > {
        proof.prepared.core.require_runtime(&self.runtime)?;
        match self.dispatch.lines.last().map(|line| &line.body) {
            Some(DispatchRecordV1::PlacePrepared { .. }) => {}
            _ => return Err(PmTrialLiveJournalError::InvalidTransition),
        }
        let prepared_sequence = proof.prepared.core.sequence;
        let prepared_record_fingerprint = proof.prepared.core.record_fingerprint.clone();
        let preparation = proof.prepared.preparation;
        let core = self.append_dispatch(DispatchRecordV1::PlaceDispatchAuthorized {
            prepared_sequence,
            prepared_record_fingerprint,
            consumption: proof.consumption,
            production_order_entry_authorized: false,
            real_order_submission_authorized: false,
            place_dispatch_allowance: 0,
        })?;
        Ok((
            PmDurablePlaceDispatchAckV1 { core, preparation },
            proof.owner,
        ))
    }

    pub fn record_place_result(
        &mut self,
        dispatch: PmDurablePlaceDispatchAckV1,
        outcome: PmPlaceResultKindV1,
        observed_order_id: Option<String>,
    ) -> Result<PmDurablePlaceResultAckV1, PmTrialLiveJournalError> {
        dispatch.core.require_runtime(&self.runtime)?;
        validate_place_result(
            outcome,
            observed_order_id.as_deref(),
            &hex32(dispatch.preparation.expected_order_id().bytes()),
        )?;
        if !matches!(
            self.dispatch.lines.last().map(|line| &line.body),
            Some(DispatchRecordV1::PlaceDispatchAuthorized { .. })
        ) {
            return Err(PmTrialLiveJournalError::InvalidTransition);
        }
        let core = self.append_dispatch(DispatchRecordV1::PlaceResult {
            dispatch_authorized_sequence: dispatch.core.sequence,
            dispatch_authorized_fingerprint: dispatch.core.record_fingerprint,
            outcome,
            observed_order_id: observed_order_id.clone(),
        })?;
        Ok(PmDurablePlaceResultAckV1 {
            core,
            outcome,
            observed_order_id,
        })
    }

    pub fn record_place_outcome_bridge(
        &mut self,
        result: PmDurablePlaceResultAckV1,
    ) -> Result<
        (
            PmDurablePlaceOutcomeBridgeAckV1,
            Option<PmJournalOwnedVenueOrderV1>,
        ),
        PmTrialLiveJournalError,
    > {
        result.core.require_runtime(&self.runtime)?;
        let dispatch = result.core.link();
        let outcome = result.outcome;
        let observed_order_id = result.observed_order_id;
        let core = self.append_intent(IntentRecordV1::PlaceOutcomeBridge {
            dispatch: dispatch.clone(),
            outcome,
            observed_order_id: observed_order_id.clone(),
        })?;
        let owned = match (outcome, observed_order_id) {
            (PmPlaceResultKindV1::Accepted, Some(exact_venue_order_id)) => {
                Some(PmJournalOwnedVenueOrderV1 {
                    runtime: Arc::clone(&self.runtime),
                    exact_venue_order_id,
                })
            }
            _ => None,
        };
        Ok((PmDurablePlaceOutcomeBridgeAckV1 { core, dispatch }, owned))
    }

    fn append_intent(
        &mut self,
        body: IntentRecordV1,
    ) -> Result<DurableAckCore, PmTrialLiveJournalError> {
        self.artifact_lease.validate()?;
        let sequence = u8::try_from(self.intent.lines.len())
            .map_err(|_| PmTrialLiveJournalError::BoundExceeded)?;
        let previous = intent_fingerprint(
            self.intent
                .lines
                .last()
                .ok_or(PmTrialLiveJournalError::InvalidTransition)?,
        )?;
        let line = IntentLineV1 {
            schema_version: PM_TRIAL_LIVE_JOURNAL_VERSION,
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
        self.artifact_lease.validate()?;
        Ok(DurableAckCore {
            runtime: Arc::clone(&self.runtime),
            sequence,
            record_fingerprint,
        })
    }

    fn append_dispatch(
        &mut self,
        body: DispatchRecordV1,
    ) -> Result<DurableAckCore, PmTrialLiveJournalError> {
        self.artifact_lease.validate()?;
        let sequence = u8::try_from(self.dispatch.lines.len())
            .map_err(|_| PmTrialLiveJournalError::BoundExceeded)?;
        let previous = dispatch_fingerprint(
            self.dispatch
                .lines
                .last()
                .ok_or(PmTrialLiveJournalError::InvalidTransition)?,
        )?;
        let line = DispatchLineV1 {
            schema_version: PM_TRIAL_LIVE_JOURNAL_VERSION,
            sequence,
            previous_record_fingerprint: previous,
            scope_fingerprint: self.scope.scope_fingerprint.clone(),
            body,
        };
        let encoded = encode_line(&line)?;
        self.dispatch
            .file
            .append_durable(&self.dispatch.bytes, &encoded)?;
        self.dispatch.bytes.extend_from_slice(&encoded);
        let record_fingerprint = dispatch_fingerprint(&line)?;
        self.dispatch.lines.push(line);
        self.artifact_lease.validate()?;
        Ok(DurableAckCore {
            runtime: Arc::clone(&self.runtime),
            sequence,
            record_fingerprint,
        })
    }
}

impl PmControlledTrialLiveJournals {
    pub fn record_primary_cancel_intent(
        &mut self,
        place_bridge: PmDurablePlaceOutcomeBridgeAckV1,
        owned: PmJournalOwnedVenueOrderV1,
        created_at_utc: String,
    ) -> Result<PmDurableCancelIntentAckV1, PmTrialLiveJournalError> {
        place_bridge.core.require_runtime(&self.runtime)?;
        self.record_cancel_intent_inner(
            place_bridge.core,
            owned,
            PmCancelDispatchClassV1::Primary,
            created_at_utc,
        )
    }

    pub fn record_recovery_cancel_intent(
        &mut self,
        reconciliation: PmDurableReconciliationAckV1,
        owned: PmJournalOwnedVenueOrderV1,
        ordinal: u8,
        created_at_utc: String,
    ) -> Result<PmDurableCancelIntentAckV1, PmTrialLiveJournalError> {
        reconciliation.core.require_runtime(&self.runtime)?;
        self.record_cancel_intent_inner(
            reconciliation.core,
            owned,
            PmCancelDispatchClassV1::Recovery { ordinal },
            created_at_utc,
        )
    }

    fn record_cancel_intent_inner(
        &mut self,
        predecessor: DurableAckCore,
        owned: PmJournalOwnedVenueOrderV1,
        dispatch_class: PmCancelDispatchClassV1,
        created_at_utc: String,
    ) -> Result<PmDurableCancelIntentAckV1, PmTrialLiveJournalError> {
        predecessor.require_runtime(&self.runtime)?;
        if !Arc::ptr_eq(&owned.runtime, &self.runtime) {
            return Err(PmTrialLiveJournalError::ForeignAcknowledgement);
        }
        validate_utc(&created_at_utc)?;
        validate_cancel_class(&self.scope, &self.dispatch.lines, dispatch_class)?;
        validate_order_id(&owned.exact_venue_order_id)?;
        let exact_venue_order_id = owned.exact_venue_order_id;
        let core = self.append_intent(IntentRecordV1::CancelIntent {
            created_at_utc,
            ownership_source: predecessor.link(),
            exact_venue_order_id: exact_venue_order_id.clone(),
            dispatch_class,
        })?;
        Ok(PmDurableCancelIntentAckV1 {
            core,
            dispatch_class,
            exact_venue_order_id,
        })
    }

    pub fn record_cancel_prepared(
        &mut self,
        intent: PmDurableCancelIntentAckV1,
        preparation: PmCancelPreparationV1,
    ) -> Result<PmDurableCancelPreparedAckV1, PmTrialLiveJournalError> {
        intent.core.require_runtime(&self.runtime)?;
        let preparation = preparation.bind_request(
            &self.scope,
            &self.preflight,
            intent.core.sequence,
            latest_l2_timestamp(&self.dispatch.lines)?,
        )?;
        let preparation_view = preparation.view(intent.dispatch_class)?;
        if preparation.exact_venue_order_id() != intent.exact_venue_order_id
            || !matches!(
                self.intent.lines.last().map(|line| &line.body),
                Some(IntentRecordV1::CancelIntent { .. })
            )
        {
            return Err(PmTrialLiveJournalError::InvalidTransition);
        }
        let core = self.append_dispatch(DispatchRecordV1::CancelPrepared {
            intent: intent.core.link(),
            dispatch_class: intent.dispatch_class,
            preparation,
        })?;
        Ok(PmDurableCancelPreparedAckV1 {
            core,
            dispatch_class: intent.dispatch_class,
            exact_venue_order_id: intent.exact_venue_order_id,
            preparation: preparation_view,
        })
    }

    pub fn record_cancel_dispatch_authorized(
        &mut self,
        prepared: PmDurableCancelPreparedAckV1,
    ) -> Result<PmDurableCancelDispatchAckV1, PmTrialLiveJournalError> {
        prepared.core.require_runtime(&self.runtime)?;
        if !matches!(
            self.dispatch.lines.last().map(|line| &line.body),
            Some(DispatchRecordV1::CancelPrepared { .. })
        ) {
            return Err(PmTrialLiveJournalError::InvalidTransition);
        }
        let core = self.append_dispatch(DispatchRecordV1::CancelDispatchAuthorized {
            prepared_sequence: prepared.core.sequence,
            prepared_record_fingerprint: prepared.core.record_fingerprint,
            dispatch_class: prepared.dispatch_class,
            exact_venue_order_id: prepared.exact_venue_order_id.clone(),
            production_order_entry_authorized: false,
            real_order_submission_authorized: false,
            place_dispatch_allowance: 0,
        })?;
        let preparation = prepared.preparation;
        Ok(PmDurableCancelDispatchAckV1 {
            core,
            dispatch_class: prepared.dispatch_class,
            exact_venue_order_id: prepared.exact_venue_order_id,
            preparation,
        })
    }

    pub fn record_cancel_result(
        &mut self,
        dispatch: PmDurableCancelDispatchAckV1,
        outcome: PmCancelResultKindV1,
    ) -> Result<PmDurableCancelResultAckV1, PmTrialLiveJournalError> {
        dispatch.core.require_runtime(&self.runtime)?;
        if !matches!(
            self.dispatch.lines.last().map(|line| &line.body),
            Some(DispatchRecordV1::CancelDispatchAuthorized { .. })
        ) {
            return Err(PmTrialLiveJournalError::InvalidTransition);
        }
        let core = self.append_dispatch(DispatchRecordV1::CancelResult {
            dispatch_authorized_sequence: dispatch.core.sequence,
            dispatch_authorized_fingerprint: dispatch.core.record_fingerprint,
            outcome,
            exact_venue_order_id: dispatch.exact_venue_order_id.clone(),
        })?;
        Ok(PmDurableCancelResultAckV1 {
            core,
            outcome,
            exact_venue_order_id: dispatch.exact_venue_order_id,
        })
    }

    pub fn record_cancel_outcome_bridge(
        &mut self,
        result: PmDurableCancelResultAckV1,
    ) -> Result<PmDurableCancelOutcomeBridgeAckV1, PmTrialLiveJournalError> {
        result.core.require_runtime(&self.runtime)?;
        let dispatch = result.core.link();
        let core = self.append_intent(IntentRecordV1::CancelOutcomeBridge {
            dispatch: dispatch.clone(),
            outcome: result.outcome,
            exact_venue_order_id: result.exact_venue_order_id,
        })?;
        Ok(PmDurableCancelOutcomeBridgeAckV1 { core, dispatch })
    }

    pub fn record_place_reconciliation(
        &mut self,
        place_bridge: PmDurablePlaceOutcomeBridgeAckV1,
        observed_at_utc: String,
        state: PmReconciliationOrderStateV1,
        exact_venue_order_id: Option<String>,
    ) -> Result<
        (
            PmDurableReconciliationAckV1,
            Option<PmJournalOwnedVenueOrderV1>,
        ),
        PmTrialLiveJournalError,
    > {
        place_bridge.core.require_runtime(&self.runtime)?;
        self.record_reconciliation_inner(
            place_bridge.dispatch,
            observed_at_utc,
            state,
            exact_venue_order_id,
        )
    }

    pub fn record_cancel_reconciliation(
        &mut self,
        cancel_bridge: PmDurableCancelOutcomeBridgeAckV1,
        observed_at_utc: String,
        state: PmReconciliationOrderStateV1,
        exact_venue_order_id: Option<String>,
    ) -> Result<
        (
            PmDurableReconciliationAckV1,
            Option<PmJournalOwnedVenueOrderV1>,
        ),
        PmTrialLiveJournalError,
    > {
        cancel_bridge.core.require_runtime(&self.runtime)?;
        self.record_reconciliation_inner(
            cancel_bridge.dispatch,
            observed_at_utc,
            state,
            exact_venue_order_id,
        )
    }

    fn record_reconciliation_inner(
        &mut self,
        dispatch: CounterpartLinkV1,
        observed_at_utc: String,
        state: PmReconciliationOrderStateV1,
        exact_venue_order_id: Option<String>,
    ) -> Result<
        (
            PmDurableReconciliationAckV1,
            Option<PmJournalOwnedVenueOrderV1>,
        ),
        PmTrialLiveJournalError,
    > {
        validate_utc(&observed_at_utc)?;
        validate_reconciliation(&self.scope, state, exact_venue_order_id.as_deref())?;
        let core = self.append_intent(IntentRecordV1::Reconciliation {
            observed_at_utc,
            state,
            exact_venue_order_id: exact_venue_order_id.clone(),
            dispatch,
        })?;
        let owned = if state == PmReconciliationOrderStateV1::ExactLive {
            Some(PmJournalOwnedVenueOrderV1 {
                runtime: Arc::clone(&self.runtime),
                exact_venue_order_id: exact_venue_order_id
                    .ok_or(PmTrialLiveJournalError::InvalidTransition)?,
            })
        } else {
            None
        };
        Ok((PmDurableReconciliationAckV1 { core }, owned))
    }

    pub fn record_terminal(
        &mut self,
        terminal_at_utc: String,
        disposition: PmIntentTerminalDispositionV1,
    ) -> Result<PmDurableIntentTerminalAckV1, PmTrialLiveJournalError> {
        validate_utc(&terminal_at_utc)?;
        if self.intent.lines.len() <= 1
            || self.dispatch.lines.len() <= 1
            || matches!(
                self.intent.lines.last().map(|line| &line.body),
                Some(IntentRecordV1::Terminal { .. })
            )
            || matches!(
                self.dispatch.lines.last().map(|line| &line.body),
                Some(DispatchRecordV1::Terminal { .. })
            )
        {
            return Err(PmTrialLiveJournalError::InvalidTransition);
        }
        let latest_intent = self.latest_intent_link()?;
        let dispatch_terminal = self.append_dispatch(DispatchRecordV1::Terminal {
            terminal_at_utc: terminal_at_utc.clone(),
            intent: latest_intent,
            terminal_is_evidence_not_authority: true,
        })?;
        let core = self.append_intent(IntentRecordV1::Terminal {
            terminal_at_utc,
            disposition,
            dispatch_terminal: dispatch_terminal.link(),
            terminal_is_evidence_not_authority: true,
        })?;
        Ok(PmDurableIntentTerminalAckV1 { core })
    }

    fn latest_intent_link(&self) -> Result<CounterpartLinkV1, PmTrialLiveJournalError> {
        let line = self
            .intent
            .lines
            .last()
            .ok_or(PmTrialLiveJournalError::InvalidTransition)?;
        Ok(CounterpartLinkV1 {
            sequence: line.sequence,
            record_fingerprint: intent_fingerprint(line)?,
        })
    }
}

fn validate_cancel_class(
    scope: &PmTrialLiveJournalScopeV1,
    dispatch: &[DispatchLineV1],
    candidate: PmCancelDispatchClassV1,
) -> Result<(), PmTrialLiveJournalError> {
    let mut primary_seen = false;
    let mut highest_recovery = 0_u8;
    for line in dispatch {
        let class = match &line.body {
            DispatchRecordV1::CancelPrepared { dispatch_class, .. }
            | DispatchRecordV1::CancelDispatchAuthorized { dispatch_class, .. } => {
                Some(*dispatch_class)
            }
            _ => None,
        };
        match class {
            Some(PmCancelDispatchClassV1::Primary) => primary_seen = true,
            Some(PmCancelDispatchClassV1::Recovery { ordinal }) => {
                highest_recovery = highest_recovery.max(ordinal);
            }
            None => {}
        }
    }
    match candidate {
        PmCancelDispatchClassV1::Primary => {
            if primary_seen || scope.trial.order.primary_cancel_dispatch_budget != 1 {
                return Err(PmTrialLiveJournalError::InvalidTransition);
            }
        }
        PmCancelDispatchClassV1::Recovery { ordinal } => {
            if ordinal == 0
                || ordinal != highest_recovery.saturating_add(1)
                || ordinal > scope.trial.order.recovery_cancel_dispatch_budget
            {
                return Err(PmTrialLiveJournalError::InvalidTransition);
            }
        }
    }
    Ok(())
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
                return Err(PmTrialLiveJournalError::InvalidTransition);
            }
            Ok(())
        }
        (PmReconciliationOrderStateV1::Absent, None)
        | (PmReconciliationOrderStateV1::Ambiguous, None) => Ok(()),
        _ => Err(PmTrialLiveJournalError::InvalidTransition),
    }
}

pub(crate) fn build_scope(
    config: &CanonicalTrialConfig,
    authorization: &CanonicalAuthorization,
    runtime: &AuthorizationRuntimeBinding,
    owner_process_identity: String,
    artifact_directory_lease_fingerprint: String,
    prepared_record_fingerprint: String,
) -> Result<PmTrialLiveJournalScopeV1, PmTrialLiveJournalError> {
    let runtime_time = validate_utc(&runtime.observed_at_utc)?;
    verify_authorization(config, authorization, runtime_time)
        .map_err(|_| PmTrialLiveJournalError::InvalidBinding)?;
    let authorization_value = authorization.value();
    if runtime.release_binary_sha256 != authorization_value.build.release_binary_sha256
        || runtime.release_binary_length != authorization_value.build.release_binary_length
        || runtime.host != authorization_value.host
    {
        return Err(PmTrialLiveJournalError::InvalidBinding);
    }

    let expected_binding = expected_consumption_binding(config, authorization, runtime);
    let expected_consumption = PmTrialLiveExpectedConsumptionV1 {
        binding_fingerprint: hash_domain(
            CONSUMPTION_BINDING_FINGERPRINT_DOMAIN,
            &expected_binding,
        )?,
        binding: expected_binding,
        prepared_record_fingerprint,
    };
    let place_identity = config.exact_place_public_request_identity();
    let consumption_files = [
        config
            .value()
            .journal
            .authorization_consumption_ledger_file
            .as_str(),
        config
            .value()
            .journal
            .authorization_consumption_claim_file
            .as_str(),
    ];
    if consumption_files.contains(&PM_TRIAL_LIVE_INTENT_FILE_V1)
        || consumption_files.contains(&PM_TRIAL_LIVE_DISPATCH_FILE_V1)
    {
        return Err(PmTrialLiveJournalError::InvalidBinding);
    }

    PmTrialLiveJournalScopeV1 {
        journal_family: PM_TRIAL_LIVE_JOURNAL_FAMILY.to_owned(),
        journal_version: PM_TRIAL_LIVE_JOURNAL_VERSION,
        intent_file: PM_TRIAL_LIVE_INTENT_FILE_V1.to_owned(),
        dispatch_file: PM_TRIAL_LIVE_DISPATCH_FILE_V1.to_owned(),
        canonical_config_sha256: config.canonical_sha256().to_owned(),
        canonical_config_length: config.canonical_length(),
        canonical_config_fingerprint: config.fingerprint().to_owned(),
        trial_plan_fingerprint: config.plan_fingerprint().to_owned(),
        authorization_id: authorization_value.authorization_id.clone(),
        authorization_fingerprint: authorization.fingerprint().to_owned(),
        authorization_cleanup_not_after_utc: authorization_value.cleanup_not_after_utc.clone(),
        source_pin_manifest_sha256: config.value().source_pin_manifest_sha256.clone(),
        release_binary_sha256: runtime.release_binary_sha256.clone(),
        release_binary_length: runtime.release_binary_length,
        runtime_observed_at_utc: runtime.observed_at_utc.clone(),
        host: runtime.host.clone(),
        credential_slot_id: config.value().credential_slot.slot_id.clone(),
        credential_slot_nonsecret_fingerprint_sha256: config
            .value()
            .credential_slot
            .nonsecret_fingerprint_sha256
            .clone(),
        expected_order_id: hex32(place_identity.expected_order_id().bytes()),
        place_semantic_request_commitment: hex32(
            place_identity.semantic_request_commitment().bytes(),
        ),
        owner_process_identity,
        artifact_directory_lease_fingerprint,
        trial: config.value().clone(),
        expected_consumption,
        authorization: OfflineAuthorizationState::DENIED,
        scope_fingerprint: ZERO_FINGERPRINT.to_owned(),
    }
    .seal()
}

fn verify_prepared_consumption(
    config: &CanonicalTrialConfig,
    authorization: &CanonicalAuthorization,
) -> Result<AuthorizationConsumptionVerification, PmTrialLiveJournalError> {
    let verification = verify_authorization_consumption(config, authorization)
        .map_err(|_| PmTrialLiveJournalError::InvalidBinding)?;
    if verification.schema_version != 1
        || !matches!(
            verification.state,
            AuthorizationConsumptionState::Prepared { .. }
        )
        || verification.ledger_record_count != 1
        || verification.atomic_consumption_claim_durable
        || verification.consumed_ledger_record_durable
        || verification.claim_fingerprint.is_some()
        || verification.ambiguous_tail
        || !verification.exact_bindings_structurally_valid
        || verification.authorization != OfflineAuthorizationState::DENIED
    {
        return Err(PmTrialLiveJournalError::InvalidBinding);
    }
    validate_fingerprint(&verification.latest_record_fingerprint)?;
    Ok(verification)
}

fn validate_bound_preflight(
    scope: &PmTrialLiveJournalScopeV1,
    lease: &TrialJournalLeaseEvidence,
    canonical: &CanonicalTrialPreflight,
) -> Result<(), PmTrialLiveJournalError> {
    let value = canonical.value();
    let binding = &value.binding;
    if binding.canonical_config_sha256 != scope.canonical_config_sha256
        || binding.canonical_config_length != scope.canonical_config_length
        || binding.canonical_config_fingerprint != scope.canonical_config_fingerprint
        || binding.trial_plan_fingerprint != scope.trial_plan_fingerprint
        || binding.authorization_id != scope.authorization_id
        || binding.authorization_fingerprint != scope.authorization_fingerprint
        || binding.source_pin_manifest_sha256 != scope.source_pin_manifest_sha256
        || binding.release_binary_sha256 != scope.release_binary_sha256
        || binding.release_binary_length != scope.release_binary_length
        || binding.host != scope.host
        || binding.credential_slot_id != scope.credential_slot_id
        || binding.credential_slot_nonsecret_fingerprint_sha256
            != scope.credential_slot_nonsecret_fingerprint_sha256
        || binding.journal != scope.trial.journal
        || &binding.leases != lease
        || binding.leases.owner_process_identity != scope.owner_process_identity
        || binding.leases.artifact_directory_lease_fingerprint
            != scope.artifact_directory_lease_fingerprint
        || binding.leases.product_journal_scope_fingerprint != scope.scope_fingerprint
        || binding.leases.authenticated_journal_scope_fingerprint != scope.scope_fingerprint
        || value.authorization != OfflineAuthorizationState::DENIED
    {
        return Err(PmTrialLiveJournalError::InvalidBinding);
    }
    let created = validate_utc(&scope.runtime_observed_at_utc)?;
    let validated = validate_utc(&value.window.validated_at_utc)?;
    let deadline = validate_utc(&value.window.dispatch_deadline_at_utc)?;
    let expires = validate_utc(
        &scope
            .expected_consumption
            .binding
            .authorization_expires_at_utc,
    )?;
    if validated < created || validated >= expires || deadline < validated {
        return Err(PmTrialLiveJournalError::InvalidBinding);
    }
    Ok(())
}

fn expected_consumption_binding(
    config: &CanonicalTrialConfig,
    authorization: &CanonicalAuthorization,
    runtime: &AuthorizationRuntimeBinding,
) -> AuthorizationConsumptionBindingEvidence {
    let record = authorization.value();
    AuthorizationConsumptionBindingEvidence {
        authorization_id: record.authorization_id.clone(),
        phase: record.phase,
        authorization_fingerprint: authorization.fingerprint().to_owned(),
        canonical_config_sha256: config.canonical_sha256().to_owned(),
        canonical_config_length: config.canonical_length(),
        canonical_config_fingerprint: config.fingerprint().to_owned(),
        trial_plan_fingerprint: config.plan_fingerprint().to_owned(),
        release_binary_sha256: runtime.release_binary_sha256.clone(),
        release_binary_length: runtime.release_binary_length,
        host: runtime.host.clone(),
        authorization_not_before_utc: record.not_before_utc.clone(),
        authorization_expires_at_utc: record.expires_at_utc.clone(),
        artifact_directory: config.value().journal.artifact_directory.clone(),
        journal_family: config.value().journal.journal_family.clone(),
        journal_version: config.value().journal.journal_version,
        credential_slot_id: config.value().credential_slot.slot_id.clone(),
        credential_slot_nonsecret_fingerprint_sha256: config
            .value()
            .credential_slot
            .nonsecret_fingerprint_sha256
            .clone(),
        ledger_file: config
            .value()
            .journal
            .authorization_consumption_ledger_file
            .clone(),
        consume_claim_file: config
            .value()
            .journal
            .authorization_consumption_claim_file
            .clone(),
    }
}

fn validate_consumed_authorization(
    scope: &PmTrialLiveJournalScopeV1,
    owner: &ConsumedAuthorizationConsumption,
    verification: &AuthorizationConsumptionVerification,
) -> Result<PmTrialLiveConsumedFingerprintsV1, PmTrialLiveJournalError> {
    let evidence = owner.evidence();
    if evidence.sequence != 1
        || evidence.binding != scope.expected_consumption.binding
        || evidence.binding_fingerprint != scope.expected_consumption.binding_fingerprint
        || evidence.authorization != OfflineAuthorizationState::DENIED
        || verification.schema_version != 1
        || verification.ledger_record_count != 2
        || !verification.atomic_consumption_claim_durable
        || !verification.consumed_ledger_record_durable
        || verification.ambiguous_tail
        || verification.authorization != OfflineAuthorizationState::DENIED
        || verification.binding_fingerprint != scope.expected_consumption.binding_fingerprint
        || evidence.previous_record_fingerprint
            != scope.expected_consumption.prepared_record_fingerprint
    {
        return Err(PmTrialLiveJournalError::InvalidBinding);
    }
    if !matches!(
        &evidence.consumption,
        AuthorizationConsumptionState::Consumed {
            burned_before_dispatch_authority: true,
            crash_allows_recovery_cancel_only: true,
            placement_can_never_resume: true,
            ..
        }
    ) || !matches!(
        &verification.state,
        AuthorizationConsumptionState::Consumed {
            burned_before_dispatch_authority: true,
            crash_allows_recovery_cancel_only: true,
            placement_can_never_resume: true,
            ..
        }
    ) {
        return Err(PmTrialLiveJournalError::InvalidBinding);
    }
    let consumed_record_fingerprint = hash_domain(CONSUMPTION_RECORD_FINGERPRINT_DOMAIN, evidence)?;
    if consumed_record_fingerprint != verification.latest_record_fingerprint {
        return Err(PmTrialLiveJournalError::InvalidBinding);
    }
    let claim = verification
        .claim_fingerprint
        .clone()
        .ok_or(PmTrialLiveJournalError::InvalidBinding)?;
    let fingerprints = PmTrialLiveConsumedFingerprintsV1 {
        binding_fingerprint: evidence.binding_fingerprint.clone(),
        prepared_record_fingerprint: evidence.previous_record_fingerprint.clone(),
        atomic_claim_fingerprint: claim,
        consumed_record_fingerprint,
    };
    fingerprints.validate()?;
    Ok(fingerprints)
}

fn validate_place_result(
    outcome: PmPlaceResultKindV1,
    observed_order_id: Option<&str>,
    expected_order_id: &str,
) -> Result<(), PmTrialLiveJournalError> {
    if let Some(observed) = observed_order_id {
        validate_order_id(observed)?;
    }
    match outcome {
        PmPlaceResultKindV1::Accepted => {
            let observed = observed_order_id.ok_or(PmTrialLiveJournalError::InvalidTransition)?;
            if observed.strip_prefix("0x") != Some(expected_order_id) {
                return Err(PmTrialLiveJournalError::InvalidTransition);
            }
        }
        PmPlaceResultKindV1::Rejected
        | PmPlaceResultKindV1::OutOfProfile
        | PmPlaceResultKindV1::AcknowledgementUnknown
            if observed_order_id.is_some() =>
        {
            return Err(PmTrialLiveJournalError::InvalidTransition);
        }
        _ => {}
    }
    Ok(())
}

fn latest_l2_timestamp(dispatch: &[DispatchLineV1]) -> Result<u64, PmTrialLiveJournalError> {
    dispatch
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
        .ok_or(PmTrialLiveJournalError::InvalidTransition)
}

fn hex32(bytes: [u8; 32]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn encode_line(value: &impl serde::Serialize) -> Result<Vec<u8>, PmTrialLiveJournalError> {
    let mut bytes = canonical_json(value)?;
    if bytes.len() > MAX_JOURNAL_LINE_BYTES {
        return Err(PmTrialLiveJournalError::BoundExceeded);
    }
    bytes.push(b'\n');
    Ok(bytes)
}

pub(crate) fn bound_paths(config: &CanonicalTrialConfig) -> (PathBuf, PathBuf) {
    let parent = PathBuf::from(&config.value().journal.artifact_directory);
    (
        parent.join(PM_TRIAL_LIVE_INTENT_FILE_V1),
        parent.join(PM_TRIAL_LIVE_DISPATCH_FILE_V1),
    )
}

// Recovery methods are implemented after the verifier has produced an exact
// move-only projection; this type exposes no place path.
pub struct PmControlledTrialLiveRecoveryJournals {
    pub(crate) inner: PmControlledTrialLiveJournals,
    pub(crate) projection: PmTrialLiveRecoveryProjectionV1,
    initial_reconciliation_available: bool,
}

impl PmControlledTrialLiveRecoveryJournals {
    pub fn open(
        config: &CanonicalTrialConfig,
        authorization: &CanonicalAuthorization,
        projection: PmTrialLiveRecoveryProjectionV1,
    ) -> Result<Self, PmTrialLiveJournalError> {
        if !matches!(
            projection.classification,
            PmTrialLiveRecoveryClassificationV1::PlaceMayHaveBeenSentNoResend
                | PmTrialLiveRecoveryClassificationV1::RecoveryCancelOnly { .. }
                | PmTrialLiveRecoveryClassificationV1::ReconcileBeforeRecoveryCancel { .. }
        ) {
            return Err(PmTrialLiveJournalError::RecoveryOperationForbidden);
        }
        let preflight = projection
            .preflight
            .clone()
            .ok_or(PmTrialLiveJournalError::RecoveryOperationForbidden)?;
        let artifact_directory = Path::new(&config.value().journal.artifact_directory);
        let artifact_lease = ProtectedArtifactLease::acquire(artifact_directory)?;
        if artifact_lease.fingerprint() != projection.scope.artifact_directory_lease_fingerprint {
            return Err(PmTrialLiveJournalError::Protection);
        }
        let (intent_path, dispatch_path) = bound_paths(config);
        let intent_file = ProtectedJournal::open_existing(&intent_path, MAX_JOURNAL_BYTES)?;
        let dispatch_file = ProtectedJournal::open_existing(&dispatch_path, MAX_JOURNAL_BYTES)?;
        artifact_lease.validate()?;
        revalidate_projection(config, authorization, &projection)?;
        let inner = PmControlledTrialLiveJournals {
            scope: projection.scope.clone(),
            preflight,
            place_identity: config.exact_place_public_request_identity(),
            runtime: Arc::new(RuntimeIdentity),
            artifact_lease,
            intent: IntentWriter {
                file: intent_file,
                bytes: projection.intent_bytes.clone(),
                lines: projection.intent_lines.clone(),
            },
            dispatch: DispatchWriter {
                file: dispatch_file,
                bytes: projection.dispatch_bytes.clone(),
                lines: projection.dispatch_lines.clone(),
            },
        };
        Ok(Self {
            inner,
            projection,
            initial_reconciliation_available: true,
        })
    }

    #[must_use]
    pub const fn preflight_binding(&self) -> &PmTrialLivePreflightBindingV1 {
        &self.inner.preflight
    }

    pub fn record_reconciliation(
        &mut self,
        observed_at_utc: String,
        state: PmReconciliationOrderStateV1,
        exact_venue_order_id: Option<String>,
    ) -> Result<
        (
            PmDurableReconciliationAckV1,
            Option<PmJournalOwnedVenueOrderV1>,
        ),
        PmTrialLiveJournalError,
    > {
        if !self.initial_reconciliation_available {
            return Err(PmTrialLiveJournalError::RecoveryOperationForbidden);
        }
        if let PmTrialLiveRecoveryClassificationV1::RecoveryCancelOnly {
            exact_venue_order_id: expected,
        } = &self.projection.classification
            && (state != PmReconciliationOrderStateV1::ExactLive
                || exact_venue_order_id.as_deref() != Some(expected))
        {
            return Err(PmTrialLiveJournalError::InvalidBinding);
        }
        let result = self.inner.record_reconciliation_inner(
            self.projection.reconciliation_target.clone(),
            observed_at_utc,
            state,
            exact_venue_order_id,
        )?;
        self.initial_reconciliation_available = false;
        Ok(result)
    }

    pub fn record_recovery_cancel_intent(
        &mut self,
        reconciliation: PmDurableReconciliationAckV1,
        owned: PmJournalOwnedVenueOrderV1,
        created_at_utc: String,
    ) -> Result<PmDurableCancelIntentAckV1, PmTrialLiveJournalError> {
        let ordinal = next_recovery_ordinal(&self.inner.dispatch.lines)?;
        self.inner
            .record_recovery_cancel_intent(reconciliation, owned, ordinal, created_at_utc)
    }

    pub fn record_cancel_prepared(
        &mut self,
        intent: PmDurableCancelIntentAckV1,
        preparation: PmCancelPreparationV1,
    ) -> Result<PmDurableCancelPreparedAckV1, PmTrialLiveJournalError> {
        self.inner.record_cancel_prepared(intent, preparation)
    }

    pub fn record_cancel_dispatch_authorized(
        &mut self,
        prepared: PmDurableCancelPreparedAckV1,
    ) -> Result<PmDurableCancelDispatchAckV1, PmTrialLiveJournalError> {
        self.inner.record_cancel_dispatch_authorized(prepared)
    }

    pub fn record_cancel_result(
        &mut self,
        dispatch: PmDurableCancelDispatchAckV1,
        outcome: PmCancelResultKindV1,
    ) -> Result<PmDurableCancelResultAckV1, PmTrialLiveJournalError> {
        self.inner.record_cancel_result(dispatch, outcome)
    }

    pub fn record_cancel_outcome_bridge(
        &mut self,
        result: PmDurableCancelResultAckV1,
    ) -> Result<PmDurableCancelOutcomeBridgeAckV1, PmTrialLiveJournalError> {
        self.inner.record_cancel_outcome_bridge(result)
    }

    pub fn record_cancel_reconciliation(
        &mut self,
        bridge: PmDurableCancelOutcomeBridgeAckV1,
        observed_at_utc: String,
        state: PmReconciliationOrderStateV1,
        exact_venue_order_id: Option<String>,
    ) -> Result<
        (
            PmDurableReconciliationAckV1,
            Option<PmJournalOwnedVenueOrderV1>,
        ),
        PmTrialLiveJournalError,
    > {
        self.inner.record_cancel_reconciliation(
            bridge,
            observed_at_utc,
            state,
            exact_venue_order_id,
        )
    }

    pub fn record_terminal(
        &mut self,
        terminal_at_utc: String,
        disposition: PmIntentTerminalDispositionV1,
    ) -> Result<PmDurableIntentTerminalAckV1, PmTrialLiveJournalError> {
        self.inner.record_terminal(terminal_at_utc, disposition)
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
}

fn next_recovery_ordinal(dispatch: &[DispatchLineV1]) -> Result<u8, PmTrialLiveJournalError> {
    let highest = dispatch
        .iter()
        .filter_map(|line| match &line.body {
            DispatchRecordV1::CancelPrepared {
                dispatch_class: PmCancelDispatchClassV1::Recovery { ordinal },
                ..
            } => Some(*ordinal),
            _ => None,
        })
        .max()
        .unwrap_or(0);
    highest
        .checked_add(1)
        .ok_or(PmTrialLiveJournalError::BoundExceeded)
}
