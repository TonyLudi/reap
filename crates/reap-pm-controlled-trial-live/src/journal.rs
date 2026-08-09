use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use reap_pm_controlled_trial::{
    AuthorizationConsumptionBindingEvidence, AuthorizationConsumptionState,
    AuthorizationConsumptionVerification, AuthorizationRuntimeBinding, CanonicalAuthorization,
    CanonicalTrialConfig, CanonicalTrialPreflight, ConsumedAuthorizationConsumption, FixedOrderId,
    OfflineAuthorizationState, OwnedCancelSemanticRequestCommitment, PlacePublicRequestIdentity,
    TrialAuthorizationConsumptionLeaseState, TrialJournalLeaseEvidence,
    reopen_consumed_authorization_consumption, verify_authorization,
    verify_authorization_consumption,
};

use crate::{
    PmTrialLiveJournalError,
    hash::{ZERO_FINGERPRINT, canonical_json, hash_domain, validate_fingerprint},
    live_dispatch::{
        PM_PHASE_A_LIVE_DISPATCH_BARRIER_FILE_V1, PmPhaseAPlaceDispatchBarrierWitnessV1,
        PmPhaseAPlaceDispatchGrantV1, PmPhaseAPlaceDispatchMintEvidenceV1,
        PmPhaseAPlaceLiveDispatchContextV1, PmPhaseAPlaceLiveDispatchProfileV1,
        PmRevalidatedPhaseAPlaceDispatchGrantV1, grant_matches_v1_dispatch,
        prepare_phase_a_place_dispatch_grant, reopen_phase_a_place_dispatch_barrier_witness,
    },
    protected::{ProtectedArtifactLease, ProtectedJournal},
    recovery::{
        PmPhaseALiveCancelRecoveryRequiredActionV1, PmTrialLiveRecoveryClassificationV1,
        PmTrialLiveRecoveryProjectionV1, revalidate_projection,
    },
    recovery_continuation::{
        ContinuationAckV1, ContinuationDispatchTargetV1,
        PM_PHASE_A_LIVE_CANCEL_RECOVERY_CONTINUATION_DISPATCH_FILE_V1,
        PM_PHASE_A_LIVE_CANCEL_RECOVERY_CONTINUATION_INTENT_FILE_V1,
        RecoveryContinuationJournalsV1,
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

fn continuation_ack(core: &DurableAckCore) -> ContinuationAckV1 {
    ContinuationAckV1 {
        sequence: core.sequence,
        record_fingerprint: core.record_fingerprint.clone(),
    }
}

fn continuation_core(runtime: &Arc<RuntimeIdentity>, ack: ContinuationAckV1) -> DurableAckCore {
    DurableAckCore {
        runtime: Arc::clone(runtime),
        sequence: ack.sequence,
        record_fingerprint: ack.record_fingerprint,
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
    outcome: PmPlaceResultKindV1,
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

/// Positive place-provenance cancel intent. Its private inner acknowledgement
/// cannot be downgraded into the legacy evidence-only cancel path.
pub struct PmDurablePhaseALiveCancelIntentAckV1 {
    intent: PmDurableCancelIntentAckV1,
}

/// Positive place-provenance cancel preparation. Only the live intent wrapper
/// can mint this value.
pub struct PmDurablePhaseALiveCancelPreparedAckV1 {
    prepared: PmDurableCancelPreparedAckV1,
}

/// Positive place-provenance cancel dispatch evidence. A future live cancel
/// authority join must require this type, never `PmDurableCancelDispatchAckV1`.
pub struct PmDurablePhaseALiveCancelDispatchAckV1 {
    dispatch: PmDurableCancelDispatchAckV1,
}

impl PmDurablePhaseALiveCancelDispatchAckV1 {
    #[must_use]
    pub const fn sequence(&self) -> u8 {
        self.dispatch.core.sequence
    }

    #[must_use]
    pub const fn dispatch_class(&self) -> PmCancelDispatchClassV1 {
        self.dispatch.dispatch_class
    }

    #[must_use]
    pub const fn preparation(&self) -> &PmCancelPreparationViewV1 {
        &self.dispatch.preparation
    }
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

struct PmPhaseALiveDispatchEpoch;

/// Sole move-only owner of the positive Phase-A place path before the runner
/// barrier recheck. The V1 acknowledgement, positive grant, consumed
/// authorization, and live journal epoch cannot be split by callers.
///
/// ```compile_fail
/// use reap_pm_controlled_trial_live::PmPhaseAPlaceLiveDispatchOwnerV1;
/// fn split(owner: PmPhaseAPlaceLiveDispatchOwnerV1) {
///     let PmPhaseAPlaceLiveDispatchOwnerV1 { dispatch, .. } = owner;
///     drop(dispatch);
/// }
/// ```
pub struct PmPhaseAPlaceLiveDispatchOwnerV1 {
    dispatch: PmDurablePlaceDispatchAckV1,
    grant: PmPhaseAPlaceDispatchGrantV1,
    authorization: ConsumedAuthorizationConsumption,
    epoch: Arc<PmPhaseALiveDispatchEpoch>,
}

impl PmPhaseAPlaceLiveDispatchOwnerV1 {
    /// Consume the entire combined owner and revalidate the durable barrier for
    /// runner-side preparation. This is not the final network-boundary check.
    pub fn revalidate_for_runner(
        self,
    ) -> Result<PmRevalidatedPhaseAPlaceLiveDispatchOwnerV1, PmTrialLiveJournalError> {
        let grant = self.grant.revalidate_for_runner()?;
        if !grant_matches_v1_dispatch(
            &grant,
            self.dispatch.core.sequence,
            &self.dispatch.core.record_fingerprint,
        ) {
            return Err(PmTrialLiveJournalError::ForeignAcknowledgement);
        }
        Ok(PmRevalidatedPhaseAPlaceLiveDispatchOwnerV1 {
            dispatch: self.dispatch,
            grant,
            authorization: self.authorization,
            epoch: self.epoch,
        })
    }
}

impl std::fmt::Debug for PmPhaseAPlaceLiveDispatchOwnerV1 {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PmPhaseAPlaceLiveDispatchOwnerV1")
            .field("dispatch", &self.dispatch)
            .field("grant", &self.grant)
            .field("token_split_allowed", &false)
            .finish()
    }
}

/// Combined owner used for signer, HMAC, current-time, and other pre-send
/// checks. It can be returned unchanged by every proven pre-send failure.
pub struct PmRevalidatedPhaseAPlaceLiveDispatchOwnerV1 {
    dispatch: PmDurablePlaceDispatchAckV1,
    grant: PmRevalidatedPhaseAPlaceDispatchGrantV1,
    authorization: ConsumedAuthorizationConsumption,
    epoch: Arc<PmPhaseALiveDispatchEpoch>,
}

impl PmRevalidatedPhaseAPlaceLiveDispatchOwnerV1 {
    #[must_use]
    pub const fn profile(&self) -> &PmPhaseAPlaceLiveDispatchProfileV1 {
        self.grant.profile()
    }

    /// Destroy all send authority for a proven pre-send failure. The returned
    /// token can only enter the durable DND result exchange.
    #[must_use]
    pub fn into_definitely_not_dispatched(self) -> PmPhaseAPlaceDefinitelyNotDispatchedV1 {
        PmPhaseAPlaceDefinitelyNotDispatchedV1 {
            dispatch: self.dispatch,
            grant: self.grant,
            authorization: self.authorization,
            epoch: self.epoch,
        }
    }
}

impl std::fmt::Debug for PmRevalidatedPhaseAPlaceLiveDispatchOwnerV1 {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PmRevalidatedPhaseAPlaceLiveDispatchOwnerV1")
            .field("profile", self.profile())
            .field("final_network_boundary_revalidated", &false)
            .field("token_split_allowed", &false)
            .finish()
    }
}

/// Final move-only typestate returned by the live journal immediately before
/// the future network transport. It has passed a second exact barrier check and
/// an in-memory outstanding-epoch join while the journal leases remain held.
///
/// The final owner borrows the journals, so no terminal/result transition can
/// race between this check and the transport observation:
///
/// ```compile_fail
/// use reap_pm_controlled_trial_live::{
///     PmControlledTrialLiveJournals, PmIntentTerminalDispositionV1,
///     PmRevalidatedPhaseAPlaceLiveDispatchOwnerV1,
/// };
/// fn cannot_advance_while_final_owner_is_live(
///     journals: &mut PmControlledTrialLiveJournals,
///     owner: PmRevalidatedPhaseAPlaceLiveDispatchOwnerV1,
/// ) {
///     let final_owner = journals
///         .revalidate_phase_a_place_for_network_dispatch(owner)
///         .unwrap();
///     let _ = journals.record_terminal(
///         "2026-08-09T12:05:05Z".into(),
///         PmIntentTerminalDispositionV1::OperatorActionRequired,
///     );
///     let _ = final_owner.profile();
/// }
/// ```
pub struct PmPhaseAPlaceNetworkDispatchOwnerV1<'journal> {
    journals: &'journal mut PmControlledTrialLiveJournals,
    dispatch: PmDurablePlaceDispatchAckV1,
    grant: PmRevalidatedPhaseAPlaceDispatchGrantV1,
    authorization: ConsumedAuthorizationConsumption,
    epoch: Arc<PmPhaseALiveDispatchEpoch>,
}

impl PmPhaseAPlaceNetworkDispatchOwnerV1<'_> {
    #[must_use]
    pub const fn profile(&self) -> &PmPhaseAPlaceLiveDispatchProfileV1 {
        self.grant.profile()
    }

    /// Consume the sole network-boundary owner once transport may have sent.
    /// The observation contains no grant, making a later send impossible.
    pub fn into_may_have_been_dispatched(
        mut self,
    ) -> Result<PmPhaseAPlaceMayHaveBeenDispatchedV1, PmTrialLiveJournalError> {
        self.journals.require_phase_a_live_epoch(&self.epoch)?;
        self.dispatch.core.require_runtime(&self.journals.runtime)?;
        self.journals
            .require_phase_a_live_dispatch_tail(&self.dispatch)?;
        self.journals
            .validate_phase_a_live_durable_set(&mut self.grant, &mut self.authorization)?;
        Ok(PmPhaseAPlaceMayHaveBeenDispatchedV1 {
            dispatch: self.dispatch,
            barrier: self.grant.into_barrier_witness(),
            authorization: self.authorization,
            epoch: self.epoch,
        })
    }

    /// A transport that proves no send occurred returns the combined owner to
    /// the same DND-only exchange instead of producing an observation.
    pub fn into_definitely_not_dispatched(
        mut self,
    ) -> Result<PmPhaseAPlaceDefinitelyNotDispatchedV1, PmTrialLiveJournalError> {
        self.journals.require_phase_a_live_epoch(&self.epoch)?;
        self.dispatch.core.require_runtime(&self.journals.runtime)?;
        self.journals
            .require_phase_a_live_dispatch_tail(&self.dispatch)?;
        self.journals
            .validate_phase_a_live_durable_set(&mut self.grant, &mut self.authorization)?;
        Ok(PmPhaseAPlaceDefinitelyNotDispatchedV1 {
            dispatch: self.dispatch,
            grant: self.grant,
            authorization: self.authorization,
            epoch: self.epoch,
        })
    }
}

impl std::fmt::Debug for PmPhaseAPlaceNetworkDispatchOwnerV1<'_> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PmPhaseAPlaceNetworkDispatchOwnerV1")
            .field("profile", self.profile())
            .field("final_network_boundary_revalidated", &true)
            .field("token_split_allowed", &false)
            .finish()
    }
}

/// Move-only observation produced only by consuming the combined final owner
/// at the future transport boundary. It contains no dispatch grant.
pub struct PmPhaseAPlaceMayHaveBeenDispatchedV1 {
    dispatch: PmDurablePlaceDispatchAckV1,
    barrier: PmPhaseAPlaceDispatchBarrierWitnessV1,
    authorization: ConsumedAuthorizationConsumption,
    epoch: Arc<PmPhaseALiveDispatchEpoch>,
}

impl std::fmt::Debug for PmPhaseAPlaceMayHaveBeenDispatchedV1 {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PmPhaseAPlaceMayHaveBeenDispatchedV1")
            .field("dispatch_sequence", &self.dispatch.core.sequence)
            .field("dispatch_grant_present", &false)
            .finish()
    }
}

/// Move-only proof that the combined owner was destroyed without crossing a
/// may-have-sent boundary. It can only record the DND terminal place result.
pub struct PmPhaseAPlaceDefinitelyNotDispatchedV1 {
    dispatch: PmDurablePlaceDispatchAckV1,
    grant: PmRevalidatedPhaseAPlaceDispatchGrantV1,
    authorization: ConsumedAuthorizationConsumption,
    epoch: Arc<PmPhaseALiveDispatchEpoch>,
}

impl std::fmt::Debug for PmPhaseAPlaceDefinitelyNotDispatchedV1 {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PmPhaseAPlaceDefinitelyNotDispatchedV1")
            .field("dispatch_sequence", &self.dispatch.core.sequence)
            .field("dispatch_grant_present", &true)
            .field("network_dispatch_observed", &false)
            .finish()
    }
}

/// Positive-path result acknowledgement emitted only after the combined grant
/// was consumed by DND or by a typed may-have-been-dispatched observation.
pub struct PmDurablePhaseAPlaceLiveResultAckV1 {
    result: PmDurablePlaceResultAckV1,
}

impl PmDurablePhaseAPlaceLiveResultAckV1 {
    #[must_use]
    pub const fn outcome(&self) -> PmPlaceResultKindV1 {
        self.result.outcome
    }
}

/// Positive-path place bridge, kept distinct from legacy V1 evidence.
pub struct PmPhaseAPlaceLiveOutcomeBridgeAckV1 {
    bridge: PmDurablePlaceOutcomeBridgeAckV1,
}

/// Exact venue-order custody minted only from the positive barrier plus a
/// consumed may-have-sent transport observation and Accepted result.
pub struct PmPhaseAPlaceLiveOwnedVenueOrderV1 {
    owned: PmJournalOwnedVenueOrderV1,
}

impl PmPhaseAPlaceLiveOwnedVenueOrderV1 {
    pub fn exact_venue_order_id(
        &self,
    ) -> Result<reap_pm_controlled_trial::FixedOrderId, PmTrialLiveJournalError> {
        self.owned.exact_venue_order_id()
    }
}

impl std::fmt::Debug for PmPhaseAPlaceLiveOwnedVenueOrderV1 {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PmPhaseAPlaceLiveOwnedVenueOrderV1")
            .field("owned", &self.owned)
            .field("positive_transport_provenance", &true)
            .finish()
    }
}

/// Private, continuously held positive-place custody. It pins both the
/// durable positive dispatch barrier and the exact burned authorization
/// ledger/claim. It is never independently exposed to callers.
struct PmPhaseALivePlaceCustodyV1 {
    barrier: Option<PmPhaseAPlaceDispatchBarrierWitnessV1>,
    authorization: ConsumedAuthorizationConsumption,
    epoch: PmPhaseALiveCustodyEpochV1,
    recovery_only: bool,
    journal_backend: PmPhaseALiveCancelJournalBackendV1,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum PmPhaseALiveCancelJournalBackendV1 {
    MainV1,
    RecoveryContinuationV1,
}

enum PmPhaseALiveCustodyEpochV1 {
    Place(Arc<PmPhaseALiveDispatchEpoch>),
    Cancel(Arc<PmPhaseALiveCancelEpoch>),
}

/// Opaque positive-place result plus the custody needed by the production
/// cancel path. Unlike the compatibility result API, this cannot be split into
/// an acknowledgement and a free-standing authorization owner.
pub struct PmPhaseAPlaceLiveResultCustodyV1 {
    result: PmDurablePlaceResultAckV1,
    custody: PmPhaseALivePlaceCustodyV1,
}

impl PmPhaseAPlaceLiveResultCustodyV1 {
    #[must_use]
    pub const fn outcome(&self) -> PmPlaceResultKindV1 {
        self.result.outcome
    }
}

/// Opaque positive place-outcome bridge. Accepted order ownership, when
/// present, stays inside this value with the positive barrier and burned
/// authorization custody.
pub struct PmPhaseAPlaceLiveOutcomeCustodyV1 {
    bridge: PmDurablePlaceOutcomeBridgeAckV1,
    owned: Option<PmJournalOwnedVenueOrderV1>,
    custody: PmPhaseALivePlaceCustodyV1,
}

impl PmPhaseAPlaceLiveOutcomeCustodyV1 {
    #[must_use]
    pub const fn outcome(&self) -> PmPlaceResultKindV1 {
        self.bridge.outcome
    }

    pub fn exact_live_order_id(&self) -> Result<Option<FixedOrderId>, PmTrialLiveJournalError> {
        self.owned
            .as_ref()
            .map(PmJournalOwnedVenueOrderV1::exact_venue_order_id)
            .transpose()
    }
}

/// Opaque positive reconciliation custody. Only an `ExactLive` value can be
/// consumed into a recovery-cancel intent; all other states carry no cancel
/// dispatch entry point.
pub struct PmPhaseALiveReconciliationCustodyV1 {
    reconciliation: PmDurableReconciliationAckV1,
    dispatch_target: PmPhaseALiveReconciliationTargetV1,
    state: PmReconciliationOrderStateV1,
    owned: Option<PmJournalOwnedVenueOrderV1>,
    custody: Option<PmPhaseALivePlaceCustodyV1>,
}

enum PmPhaseALiveReconciliationTargetV1 {
    MainV1(CounterpartLinkV1),
    RecoveryContinuationV1(ContinuationDispatchTargetV1),
}

impl PmPhaseALiveReconciliationCustodyV1 {
    #[must_use]
    pub const fn state(&self) -> PmReconciliationOrderStateV1 {
        self.state
    }

    pub fn exact_live_order_id(&self) -> Result<Option<FixedOrderId>, PmTrialLiveJournalError> {
        self.owned
            .as_ref()
            .map(PmJournalOwnedVenueOrderV1::exact_venue_order_id)
            .transpose()
    }
}

/// Positive cancel intent with inseparable positive-place custody.
pub struct PmPhaseALiveCancelIntentCustodyV1 {
    intent: PmDurableCancelIntentAckV1,
    custody: PmPhaseALivePlaceCustodyV1,
}

/// Positive cancel preparation with inseparable positive-place custody.
pub struct PmPhaseALiveCancelPreparedCustodyV1 {
    prepared: PmDurableCancelPreparedAckV1,
    custody: PmPhaseALivePlaceCustodyV1,
}

impl PmPhaseALiveCancelPreparedCustodyV1 {
    #[must_use]
    pub const fn preparation(&self) -> &PmCancelPreparationViewV1 {
        &self.prepared.preparation
    }
}

struct PmPhaseALiveCancelEpoch;

/// Sole positive cancel-dispatch owner. The durable V1 acknowledgement,
/// positive-place custody, and live epoch cannot be split or serialized.
///
/// ```compile_fail
/// use reap_pm_controlled_trial_live::{
///     PmControlledTrialLiveJournals, PmPhaseALiveCancelDispatchOwnerV1,
///     PmCancelResultKindV1,
/// };
/// fn cannot_downgrade(
///     journals: &mut PmControlledTrialLiveJournals,
///     owner: PmPhaseALiveCancelDispatchOwnerV1,
/// ) {
///     let _ = journals.record_cancel_result(owner, PmCancelResultKindV1::Canceled);
/// }
/// ```
pub struct PmPhaseALiveCancelDispatchOwnerV1 {
    dispatch: PmDurableCancelDispatchAckV1,
    custody: PmPhaseALivePlaceCustodyV1,
}

/// Runner-side cancel owner after a journal-mediated exact durable-set check.
pub struct PmRevalidatedPhaseALiveCancelDispatchOwnerV1 {
    dispatch: PmDurableCancelDispatchAckV1,
    custody: PmPhaseALivePlaceCustodyV1,
}

/// Final cancel owner borrowing the journals across the transport boundary.
/// It can be consumed exactly once into either a no-send token or a
/// may-have-sent observation.
///
/// ```compile_fail
/// use reap_pm_controlled_trial_live::{
///     PmControlledTrialLiveJournals, PmIntentTerminalDispositionV1,
///     PmRevalidatedPhaseALiveCancelDispatchOwnerV1,
/// };
/// fn cannot_terminalize_while_borrowed(
///     journals: &mut PmControlledTrialLiveJournals,
///     owner: PmRevalidatedPhaseALiveCancelDispatchOwnerV1,
/// ) {
///     let final_owner = journals
///         .revalidate_phase_a_live_cancel_for_network_dispatch(owner)
///         .unwrap();
///     let _ = journals.record_terminal(
///         "2026-08-09T12:05:20Z".into(),
///         PmIntentTerminalDispositionV1::Stopped,
///     );
///     let _ = final_owner.sequence();
/// }
/// ```
pub struct PmPhaseALiveCancelNetworkDispatchOwnerV1<'journal> {
    journals: PmPhaseALiveCancelNetworkJournalV1<'journal>,
    dispatch: PmDurableCancelDispatchAckV1,
    custody: PmPhaseALivePlaceCustodyV1,
}

enum PmPhaseALiveCancelNetworkJournalV1<'journal> {
    MainV1(&'journal mut PmControlledTrialLiveJournals),
    RecoveryContinuationV1(&'journal mut PmControlledTrialLiveCancelRecoveryJournals),
}

/// Cancel transport observation containing no dispatch authority.
pub struct PmPhaseALiveCancelMayHaveBeenDispatchedV1 {
    dispatch: PmDurableCancelDispatchAckV1,
    custody: PmPhaseALivePlaceCustodyV1,
}

/// Proof that the positive cancel owner was consumed before any send could
/// occur. It can enter only the typed DefinitelyNotDispatched result exchange.
pub struct PmPhaseALiveCancelDefinitelyNotDispatchedV1 {
    dispatch: PmDurableCancelDispatchAckV1,
    custody: PmPhaseALivePlaceCustodyV1,
}

/// Positive cancel result custody. The cancel epoch remains outstanding until
/// this result is bridged and reconciled.
pub struct PmPhaseALiveCancelResultCustodyV1 {
    result: PmDurableCancelResultAckV1,
    custody: PmPhaseALivePlaceCustodyV1,
}

impl PmPhaseALiveCancelResultCustodyV1 {
    #[must_use]
    pub const fn outcome(&self) -> PmCancelResultKindV1 {
        self.result.outcome
    }
}

/// Positive cancel-outcome bridge retaining the outstanding cancel epoch and
/// all prior positive custody until reconciliation is durable.
pub struct PmPhaseALiveCancelOutcomeCustodyV1 {
    bridge: PmDurableCancelOutcomeBridgeAckV1,
    exact_venue_order_id: String,
    custody: PmPhaseALivePlaceCustodyV1,
}

macro_rules! cancel_dispatch_accessors {
    ($type:ty) => {
        impl $type {
            #[must_use]
            pub const fn sequence(&self) -> u8 {
                self.dispatch.core.sequence
            }

            #[must_use]
            pub const fn dispatch_class(&self) -> PmCancelDispatchClassV1 {
                self.dispatch.dispatch_class
            }

            #[must_use]
            pub const fn preparation(&self) -> &PmCancelPreparationViewV1 {
                &self.dispatch.preparation
            }

            #[must_use]
            pub const fn exact_venue_order_id(&self) -> FixedOrderId {
                self.dispatch.preparation.exact_venue_order_id()
            }

            #[must_use]
            pub const fn semantic_request_commitment(
                &self,
            ) -> OwnedCancelSemanticRequestCommitment {
                self.dispatch.preparation.semantic_request_commitment()
            }

            #[must_use]
            pub const fn l2_timestamp_seconds(&self) -> u64 {
                self.dispatch.preparation.l2_timestamp_seconds()
            }
        }
    };
}

cancel_dispatch_accessors!(PmPhaseALiveCancelDispatchOwnerV1);
cancel_dispatch_accessors!(PmRevalidatedPhaseALiveCancelDispatchOwnerV1);
cancel_dispatch_accessors!(PmPhaseALiveCancelMayHaveBeenDispatchedV1);
cancel_dispatch_accessors!(PmPhaseALiveCancelDefinitelyNotDispatchedV1);

impl PmRevalidatedPhaseALiveCancelDispatchOwnerV1 {
    /// Consume all cancel send authority on a proven pre-send failure.
    #[must_use]
    pub fn into_definitely_not_dispatched(self) -> PmPhaseALiveCancelDefinitelyNotDispatchedV1 {
        PmPhaseALiveCancelDefinitelyNotDispatchedV1 {
            dispatch: self.dispatch,
            custody: self.custody,
        }
    }
}

impl PmPhaseALiveCancelNetworkDispatchOwnerV1<'_> {
    #[must_use]
    pub const fn sequence(&self) -> u8 {
        self.dispatch.core.sequence
    }

    #[must_use]
    pub const fn dispatch_class(&self) -> PmCancelDispatchClassV1 {
        self.dispatch.dispatch_class
    }

    #[must_use]
    pub const fn preparation(&self) -> &PmCancelPreparationViewV1 {
        &self.dispatch.preparation
    }

    #[must_use]
    pub const fn exact_venue_order_id(&self) -> FixedOrderId {
        self.dispatch.preparation.exact_venue_order_id()
    }

    #[must_use]
    pub const fn semantic_request_commitment(&self) -> OwnedCancelSemanticRequestCommitment {
        self.dispatch.preparation.semantic_request_commitment()
    }

    #[must_use]
    pub const fn l2_timestamp_seconds(&self) -> u64 {
        self.dispatch.preparation.l2_timestamp_seconds()
    }

    pub fn into_may_have_been_dispatched(
        mut self,
    ) -> Result<PmPhaseALiveCancelMayHaveBeenDispatchedV1, PmTrialLiveJournalError> {
        self.validate_final_durable_set()?;
        Ok(PmPhaseALiveCancelMayHaveBeenDispatchedV1 {
            dispatch: self.dispatch,
            custody: self.custody,
        })
    }

    pub fn into_definitely_not_dispatched(
        mut self,
    ) -> Result<PmPhaseALiveCancelDefinitelyNotDispatchedV1, PmTrialLiveJournalError> {
        self.validate_final_durable_set()?;
        Ok(PmPhaseALiveCancelDefinitelyNotDispatchedV1 {
            dispatch: self.dispatch,
            custody: self.custody,
        })
    }

    fn validate_final_durable_set(&mut self) -> Result<(), PmTrialLiveJournalError> {
        match &mut self.journals {
            PmPhaseALiveCancelNetworkJournalV1::MainV1(journals) => {
                journals.require_phase_a_live_cancel_custody_epoch(&self.custody)?;
                self.dispatch.core.require_runtime(&journals.runtime)?;
                journals.require_phase_a_live_cancel_dispatch_tail(&self.dispatch)?;
                journals.validate_phase_a_live_cancel_durable_set(&mut self.custody)
            }
            PmPhaseALiveCancelNetworkJournalV1::RecoveryContinuationV1(journals) => {
                journals
                    .evidence
                    .inner
                    .require_phase_a_live_cancel_custody_epoch(&self.custody)?;
                self.dispatch
                    .core
                    .require_runtime(&journals.evidence.inner.runtime)?;
                journals.validate_recovery_continuation_durable_set(&mut self.custody)?;
                journals
                    .continuation
                    .as_ref()
                    .ok_or(PmTrialLiveJournalError::RecoveryOperationForbidden)?
                    .target_for_dispatch_ack(&continuation_ack(&self.dispatch.core))?;
                Ok(())
            }
        }
    }
}

impl std::fmt::Debug for PmPhaseALiveCancelDispatchOwnerV1 {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PmPhaseALiveCancelDispatchOwnerV1")
            .field("sequence", &self.sequence())
            .field("dispatch_class", &self.dispatch_class())
            .field("exact_venue_order_id", &self.exact_venue_order_id())
            .field("token_split_allowed", &false)
            .finish()
    }
}

impl std::fmt::Debug for PmRevalidatedPhaseALiveCancelDispatchOwnerV1 {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PmRevalidatedPhaseALiveCancelDispatchOwnerV1")
            .field("sequence", &self.sequence())
            .field("dispatch_class", &self.dispatch_class())
            .field("runner_join_revalidated", &true)
            .field("token_split_allowed", &false)
            .finish()
    }
}

impl std::fmt::Debug for PmPhaseALiveCancelNetworkDispatchOwnerV1<'_> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PmPhaseALiveCancelNetworkDispatchOwnerV1")
            .field("sequence", &self.sequence())
            .field("dispatch_class", &self.dispatch_class())
            .field("final_network_boundary_revalidated", &true)
            .field("token_split_allowed", &false)
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
    phase_a_live_dispatch_context: Option<PmPhaseAPlaceLiveDispatchContextV1>,
    phase_a_live_dispatch_epoch: Option<Arc<PmPhaseALiveDispatchEpoch>>,
    phase_a_live_cancel_epoch: Option<Arc<PmPhaseALiveCancelEpoch>>,
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
    phase_a_live_dispatch_context: Option<PmPhaseAPlaceLiveDispatchContextV1>,
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
            phase_a_live_dispatch_context: self.phase_a_live_dispatch_context,
            phase_a_live_dispatch_epoch: None,
            phase_a_live_cancel_epoch: None,
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
        let phase_a_live_dispatch_context = PmPhaseAPlaceLiveDispatchContextV1::for_exact_phase_a(
            config,
            authorization,
            runtime,
            &scope,
        )?;
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
            phase_a_live_dispatch_context,
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
        let (dispatch, owner, _mint_evidence) =
            self.record_place_dispatch_authorized_inner(proof)?;
        Ok((dispatch, owner))
    }

    /// Append the unchanged evidence-only V1 acknowledgement, then fsync a
    /// separate create-new positive Phase-A dispatch barrier. Only the latter
    /// may mint the move-only runner grant.
    pub fn record_phase_a_place_live_dispatch_authorized(
        &mut self,
        proof: PmPreparedConsumedAuthorizationProofV1,
    ) -> Result<PmPhaseAPlaceLiveDispatchOwnerV1, PmTrialLiveJournalError> {
        if self.phase_a_live_dispatch_context.is_none()
            || self.phase_a_live_dispatch_epoch.is_some()
            || self.phase_a_live_cancel_epoch.is_some()
        {
            return Err(PmTrialLiveJournalError::InvalidBinding);
        }
        let (dispatch, mut authorization, mint_evidence) =
            self.record_place_dispatch_authorized_inner(proof)?;
        let context = self
            .phase_a_live_dispatch_context
            .take()
            .ok_or(PmTrialLiveJournalError::InvalidBinding)?;
        let epoch = Arc::new(PmPhaseALiveDispatchEpoch);
        self.phase_a_live_dispatch_epoch = Some(Arc::clone(&epoch));
        let pending_grant = prepare_phase_a_place_dispatch_grant(
            context,
            &self.scope,
            &self.preflight,
            mint_evidence,
        )?;
        // The positive barrier is the sole additional fixed directory entry.
        // Refresh every already-held directory view before exposing authority.
        self.artifact_lease.refresh_after_bound_create()?;
        self.intent.file.refresh_parent_after_bound_create()?;
        self.dispatch.file.refresh_parent_after_bound_create()?;
        authorization
            .refresh_after_bound_artifact_create()
            .map_err(|_| PmTrialLiveJournalError::Protection)?;
        self.artifact_lease.validate()?;
        Ok(PmPhaseAPlaceLiveDispatchOwnerV1 {
            dispatch,
            grant: pending_grant.into_grant(),
            authorization,
            epoch,
        })
    }

    fn record_place_dispatch_authorized_inner(
        &mut self,
        proof: PmPreparedConsumedAuthorizationProofV1,
    ) -> Result<
        (
            PmDurablePlaceDispatchAckV1,
            ConsumedAuthorizationConsumption,
            PmPhaseAPlaceDispatchMintEvidenceV1,
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
        let mint_preparation = preparation.clone();
        let mint_consumption = proof.consumption.clone();
        let core = self.append_dispatch(DispatchRecordV1::PlaceDispatchAuthorized {
            prepared_sequence,
            prepared_record_fingerprint: prepared_record_fingerprint.clone(),
            consumption: proof.consumption,
            production_order_entry_authorized: false,
            real_order_submission_authorized: false,
            place_dispatch_allowance: 0,
        })?;
        let mint_evidence = PmPhaseAPlaceDispatchMintEvidenceV1 {
            preparation: mint_preparation,
            prepared_sequence,
            prepared_record_fingerprint,
            consumption: mint_consumption,
            v1_dispatch_authorized_sequence: core.sequence,
            v1_dispatch_authorized_fingerprint: core.record_fingerprint.clone(),
        };
        Ok((
            PmDurablePlaceDispatchAckV1 { core, preparation },
            proof.owner,
            mint_evidence,
        ))
    }

    pub fn record_place_result(
        &mut self,
        dispatch: PmDurablePlaceDispatchAckV1,
        outcome: PmPlaceResultKindV1,
        observed_order_id: Option<String>,
    ) -> Result<PmDurablePlaceResultAckV1, PmTrialLiveJournalError> {
        if self.phase_a_live_dispatch_epoch.is_some()
            || self.phase_a_live_cancel_epoch.is_some()
            || outcome == PmPlaceResultKindV1::DefinitelyNotDispatched
        {
            return Err(PmTrialLiveJournalError::InvalidTransition);
        }
        self.record_place_result_inner(dispatch, outcome, observed_order_id)
    }

    /// Perform the second consuming recheck at the actual future transport
    /// boundary. The journal must still own the matching outstanding epoch and
    /// all fixed leases; no terminal or generic result can have advanced.
    pub fn revalidate_phase_a_place_for_network_dispatch<'journal>(
        &'journal mut self,
        mut owner: PmRevalidatedPhaseAPlaceLiveDispatchOwnerV1,
    ) -> Result<PmPhaseAPlaceNetworkDispatchOwnerV1<'journal>, PmTrialLiveJournalError> {
        self.require_phase_a_live_epoch(&owner.epoch)?;
        owner.dispatch.core.require_runtime(&self.runtime)?;
        self.require_phase_a_live_dispatch_tail(&owner.dispatch)?;
        self.validate_phase_a_live_durable_set(&mut owner.grant, &mut owner.authorization)?;
        if !grant_matches_v1_dispatch(
            &owner.grant,
            owner.dispatch.core.sequence,
            &owner.dispatch.core.record_fingerprint,
        ) {
            return Err(PmTrialLiveJournalError::ForeignAcknowledgement);
        }
        let PmRevalidatedPhaseAPlaceLiveDispatchOwnerV1 {
            dispatch,
            grant,
            authorization,
            epoch,
        } = owner;
        Ok(PmPhaseAPlaceNetworkDispatchOwnerV1 {
            journals: self,
            dispatch,
            grant,
            authorization,
            epoch,
        })
    }

    /// Consume the sole combined DND token, durably record the terminal no-send
    /// result, and release the in-memory outstanding epoch only after fsync.
    pub fn record_phase_a_place_definitely_not_dispatched(
        &mut self,
        mut definitely_not_dispatched: PmPhaseAPlaceDefinitelyNotDispatchedV1,
    ) -> Result<
        (
            PmDurablePhaseAPlaceLiveResultAckV1,
            ConsumedAuthorizationConsumption,
        ),
        PmTrialLiveJournalError,
    > {
        self.require_phase_a_live_epoch(&definitely_not_dispatched.epoch)?;
        definitely_not_dispatched
            .dispatch
            .core
            .require_runtime(&self.runtime)?;
        self.require_phase_a_live_dispatch_tail(&definitely_not_dispatched.dispatch)?;
        self.validate_phase_a_live_durable_set(
            &mut definitely_not_dispatched.grant,
            &mut definitely_not_dispatched.authorization,
        )?;
        if !grant_matches_v1_dispatch(
            &definitely_not_dispatched.grant,
            definitely_not_dispatched.dispatch.core.sequence,
            &definitely_not_dispatched.dispatch.core.record_fingerprint,
        ) {
            return Err(PmTrialLiveJournalError::ForeignAcknowledgement);
        }
        let result = self.record_place_result_inner(
            definitely_not_dispatched.dispatch,
            PmPlaceResultKindV1::DefinitelyNotDispatched,
            None,
        )?;
        self.complete_phase_a_live_epoch(&definitely_not_dispatched.epoch)?;
        Ok((
            PmDurablePhaseAPlaceLiveResultAckV1 { result },
            definitely_not_dispatched.authorization,
        ))
    }

    /// Consume a typed may-have-been-dispatched observation before recording
    /// any caller-supplied venue outcome. The observation contains no grant.
    pub fn record_phase_a_place_live_result(
        &mut self,
        mut observation: PmPhaseAPlaceMayHaveBeenDispatchedV1,
        outcome: PmPlaceResultKindV1,
        observed_order_id: Option<String>,
    ) -> Result<
        (
            PmDurablePhaseAPlaceLiveResultAckV1,
            ConsumedAuthorizationConsumption,
        ),
        PmTrialLiveJournalError,
    > {
        if outcome == PmPlaceResultKindV1::DefinitelyNotDispatched {
            return Err(PmTrialLiveJournalError::InvalidTransition);
        }
        self.require_phase_a_live_epoch(&observation.epoch)?;
        observation.dispatch.core.require_runtime(&self.runtime)?;
        self.require_phase_a_live_dispatch_tail(&observation.dispatch)?;
        self.validate_phase_a_live_post_dispatch_set(
            &mut observation.barrier,
            &mut observation.authorization,
        )?;
        let result =
            self.record_place_result_inner(observation.dispatch, outcome, observed_order_id)?;
        self.complete_phase_a_live_epoch(&observation.epoch)?;
        Ok((
            PmDurablePhaseAPlaceLiveResultAckV1 { result },
            observation.authorization,
        ))
    }

    /// Production cancel lineage: record the positive place result while
    /// keeping the barrier and burned authorization inseparable from its
    /// acknowledgement.
    pub fn record_phase_a_place_live_result_with_custody(
        &mut self,
        mut observation: PmPhaseAPlaceMayHaveBeenDispatchedV1,
        outcome: PmPlaceResultKindV1,
        observed_order_id: Option<String>,
    ) -> Result<PmPhaseAPlaceLiveResultCustodyV1, PmTrialLiveJournalError> {
        if outcome == PmPlaceResultKindV1::DefinitelyNotDispatched {
            return Err(PmTrialLiveJournalError::InvalidTransition);
        }
        self.require_phase_a_live_epoch(&observation.epoch)?;
        observation.dispatch.core.require_runtime(&self.runtime)?;
        self.require_phase_a_live_dispatch_tail(&observation.dispatch)?;
        self.validate_phase_a_live_post_dispatch_set(
            &mut observation.barrier,
            &mut observation.authorization,
        )?;
        let result =
            self.record_place_result_inner(observation.dispatch, outcome, observed_order_id)?;
        Ok(PmPhaseAPlaceLiveResultCustodyV1 {
            result,
            custody: PmPhaseALivePlaceCustodyV1 {
                barrier: Some(observation.barrier),
                authorization: observation.authorization,
                epoch: PmPhaseALiveCustodyEpochV1::Place(observation.epoch),
                recovery_only: false,
                journal_backend: PmPhaseALiveCancelJournalBackendV1::MainV1,
            },
        })
    }

    fn record_place_result_inner(
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
        if self.phase_a_live_dispatch_epoch.is_some() || self.phase_a_live_cancel_epoch.is_some() {
            return Err(PmTrialLiveJournalError::InvalidTransition);
        }
        self.record_place_outcome_bridge_inner(result)
    }

    pub fn record_phase_a_place_live_outcome_bridge(
        &mut self,
        result: PmDurablePhaseAPlaceLiveResultAckV1,
    ) -> Result<
        (
            PmPhaseAPlaceLiveOutcomeBridgeAckV1,
            Option<PmPhaseAPlaceLiveOwnedVenueOrderV1>,
        ),
        PmTrialLiveJournalError,
    > {
        if self.phase_a_live_dispatch_epoch.is_some() || self.phase_a_live_cancel_epoch.is_some() {
            return Err(PmTrialLiveJournalError::InvalidTransition);
        }
        let (bridge, owned) = self.record_place_outcome_bridge_inner(result.result)?;
        Ok((
            PmPhaseAPlaceLiveOutcomeBridgeAckV1 { bridge },
            owned.map(|owned| PmPhaseAPlaceLiveOwnedVenueOrderV1 { owned }),
        ))
    }

    /// Bridge a positive place result without releasing its cancel custody.
    pub fn record_phase_a_place_live_outcome_bridge_with_custody(
        &mut self,
        mut result: PmPhaseAPlaceLiveResultCustodyV1,
    ) -> Result<PmPhaseAPlaceLiveOutcomeCustodyV1, PmTrialLiveJournalError> {
        if self.phase_a_live_cancel_epoch.is_some() {
            return Err(PmTrialLiveJournalError::InvalidTransition);
        }
        self.require_phase_a_live_custody_epoch(&result.custody)?;
        result.result.core.require_runtime(&self.runtime)?;
        self.require_place_result_tail(&result.result)?;
        self.validate_phase_a_live_cancel_durable_set(&mut result.custody)?;
        let (bridge, owned) = self.record_place_outcome_bridge_inner(result.result)?;
        Ok(PmPhaseAPlaceLiveOutcomeCustodyV1 {
            bridge,
            owned,
            custody: result.custody,
        })
    }

    fn record_place_outcome_bridge_inner(
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
        Ok((
            PmDurablePlaceOutcomeBridgeAckV1 {
                core,
                dispatch,
                outcome,
            },
            owned,
        ))
    }

    fn require_phase_a_live_epoch(
        &self,
        candidate: &Arc<PmPhaseALiveDispatchEpoch>,
    ) -> Result<(), PmTrialLiveJournalError> {
        match &self.phase_a_live_dispatch_epoch {
            Some(active)
                if Arc::ptr_eq(active, candidate)
                    && Arc::strong_count(active) == 2
                    && Arc::strong_count(candidate) == 2 =>
            {
                Ok(())
            }
            _ => Err(PmTrialLiveJournalError::ForeignAcknowledgement),
        }
    }

    fn require_phase_a_live_dispatch_tail(
        &self,
        dispatch: &PmDurablePlaceDispatchAckV1,
    ) -> Result<(), PmTrialLiveJournalError> {
        let line = self
            .dispatch
            .lines
            .last()
            .ok_or(PmTrialLiveJournalError::InvalidTransition)?;
        if line.sequence != dispatch.core.sequence
            || dispatch_fingerprint(line)? != dispatch.core.record_fingerprint
            || !matches!(line.body, DispatchRecordV1::PlaceDispatchAuthorized { .. })
        {
            return Err(PmTrialLiveJournalError::InvalidTransition);
        }
        Ok(())
    }

    fn validate_phase_a_live_journal_files(&mut self) -> Result<(), PmTrialLiveJournalError> {
        self.intent.file.validate_exact_bytes(&self.intent.bytes)?;
        self.dispatch
            .file
            .validate_exact_bytes(&self.dispatch.bytes)?;
        self.artifact_lease.validate()
    }

    fn validate_phase_a_live_durable_set(
        &mut self,
        grant: &mut PmRevalidatedPhaseAPlaceDispatchGrantV1,
        authorization: &mut ConsumedAuthorizationConsumption,
    ) -> Result<(), PmTrialLiveJournalError> {
        self.validate_phase_a_live_journal_files()?;
        authorization
            .revalidate_held_consumption_evidence()
            .map_err(|_| PmTrialLiveJournalError::Protection)?;
        grant.validate_held_barrier()?;
        self.artifact_lease.validate()
    }

    fn validate_phase_a_live_post_dispatch_set(
        &mut self,
        barrier: &mut PmPhaseAPlaceDispatchBarrierWitnessV1,
        authorization: &mut ConsumedAuthorizationConsumption,
    ) -> Result<(), PmTrialLiveJournalError> {
        self.validate_phase_a_live_journal_files()?;
        authorization
            .revalidate_held_consumption_evidence()
            .map_err(|_| PmTrialLiveJournalError::Protection)?;
        barrier.validate_held_barrier()?;
        self.artifact_lease.validate()
    }

    fn complete_phase_a_live_epoch(
        &mut self,
        candidate: &Arc<PmPhaseALiveDispatchEpoch>,
    ) -> Result<(), PmTrialLiveJournalError> {
        self.require_phase_a_live_epoch(candidate)?;
        self.phase_a_live_dispatch_epoch = None;
        Ok(())
    }

    fn require_phase_a_live_cancel_epoch(
        &self,
        candidate: &Arc<PmPhaseALiveCancelEpoch>,
    ) -> Result<(), PmTrialLiveJournalError> {
        match &self.phase_a_live_cancel_epoch {
            Some(active)
                if Arc::ptr_eq(active, candidate)
                    && Arc::strong_count(active) == 2
                    && Arc::strong_count(candidate) == 2 =>
            {
                Ok(())
            }
            _ => Err(PmTrialLiveJournalError::ForeignAcknowledgement),
        }
    }

    fn require_phase_a_live_custody_epoch(
        &self,
        custody: &PmPhaseALivePlaceCustodyV1,
    ) -> Result<(), PmTrialLiveJournalError> {
        match &custody.epoch {
            PmPhaseALiveCustodyEpochV1::Place(epoch) => self.require_phase_a_live_epoch(epoch),
            PmPhaseALiveCustodyEpochV1::Cancel(epoch) => {
                self.require_phase_a_live_cancel_epoch(epoch)
            }
        }
    }

    fn require_phase_a_live_cancel_custody_epoch(
        &self,
        custody: &PmPhaseALivePlaceCustodyV1,
    ) -> Result<(), PmTrialLiveJournalError> {
        let PmPhaseALiveCustodyEpochV1::Cancel(epoch) = &custody.epoch else {
            return Err(PmTrialLiveJournalError::InvalidTransition);
        };
        self.require_phase_a_live_cancel_epoch(epoch)
    }

    fn complete_phase_a_live_custody_epoch(
        &mut self,
        custody: &PmPhaseALivePlaceCustodyV1,
    ) -> Result<(), PmTrialLiveJournalError> {
        match &custody.epoch {
            PmPhaseALiveCustodyEpochV1::Place(epoch) => self.complete_phase_a_live_epoch(epoch),
            PmPhaseALiveCustodyEpochV1::Cancel(epoch) => {
                self.require_phase_a_live_cancel_epoch(epoch)?;
                self.phase_a_live_cancel_epoch = None;
                Ok(())
            }
        }
    }

    fn transition_phase_a_live_custody_to_cancel_epoch(
        &mut self,
        custody: &mut PmPhaseALivePlaceCustodyV1,
    ) -> Result<(), PmTrialLiveJournalError> {
        self.complete_phase_a_live_custody_epoch(custody)?;
        let epoch = Arc::new(PmPhaseALiveCancelEpoch);
        self.phase_a_live_cancel_epoch = Some(Arc::clone(&epoch));
        custody.epoch = PmPhaseALiveCustodyEpochV1::Cancel(epoch);
        Ok(())
    }

    fn validate_phase_a_live_cancel_durable_set(
        &mut self,
        custody: &mut PmPhaseALivePlaceCustodyV1,
    ) -> Result<(), PmTrialLiveJournalError> {
        if custody.journal_backend != PmPhaseALiveCancelJournalBackendV1::MainV1 {
            return Err(PmTrialLiveJournalError::InvalidTransition);
        }
        self.validate_phase_a_live_journal_files()?;
        custody
            .authorization
            .revalidate_held_consumption_evidence()
            .map_err(|_| PmTrialLiveJournalError::Protection)?;
        if let Some(barrier) = &mut custody.barrier {
            barrier.validate_held_barrier()?;
            let profile = barrier.profile();
            if profile.scope_fingerprint() != self.scope.scope_fingerprint
                || profile.public_request_identity() != self.place_identity
                || profile.expected_order_id() != self.place_identity.expected_order_id()
            {
                return Err(PmTrialLiveJournalError::InvalidBinding);
            }
        } else if !custody.recovery_only {
            return Err(PmTrialLiveJournalError::InvalidBinding);
        }
        self.artifact_lease.validate()
    }

    fn require_place_result_tail(
        &self,
        result: &PmDurablePlaceResultAckV1,
    ) -> Result<(), PmTrialLiveJournalError> {
        let line = self
            .dispatch
            .lines
            .last()
            .ok_or(PmTrialLiveJournalError::InvalidTransition)?;
        let DispatchRecordV1::PlaceResult {
            outcome,
            observed_order_id,
            ..
        } = &line.body
        else {
            return Err(PmTrialLiveJournalError::InvalidTransition);
        };
        if line.sequence != result.core.sequence
            || dispatch_fingerprint(line)? != result.core.record_fingerprint
            || *outcome != result.outcome
            || observed_order_id != &result.observed_order_id
        {
            return Err(PmTrialLiveJournalError::ForeignAcknowledgement);
        }
        Ok(())
    }

    fn require_place_bridge_tail(
        &self,
        bridge: &PmDurablePlaceOutcomeBridgeAckV1,
    ) -> Result<(), PmTrialLiveJournalError> {
        let line = self
            .intent
            .lines
            .last()
            .ok_or(PmTrialLiveJournalError::InvalidTransition)?;
        let IntentRecordV1::PlaceOutcomeBridge {
            dispatch, outcome, ..
        } = &line.body
        else {
            return Err(PmTrialLiveJournalError::InvalidTransition);
        };
        if line.sequence != bridge.core.sequence
            || intent_fingerprint(line)? != bridge.core.record_fingerprint
            || dispatch != &bridge.dispatch
            || *outcome != bridge.outcome
        {
            return Err(PmTrialLiveJournalError::ForeignAcknowledgement);
        }
        Ok(())
    }

    fn require_cancel_intent_tail(
        &self,
        intent: &PmDurableCancelIntentAckV1,
    ) -> Result<(), PmTrialLiveJournalError> {
        let line = self
            .intent
            .lines
            .last()
            .ok_or(PmTrialLiveJournalError::InvalidTransition)?;
        let IntentRecordV1::CancelIntent {
            exact_venue_order_id,
            dispatch_class,
            ..
        } = &line.body
        else {
            return Err(PmTrialLiveJournalError::InvalidTransition);
        };
        if line.sequence != intent.core.sequence
            || intent_fingerprint(line)? != intent.core.record_fingerprint
            || exact_venue_order_id != &intent.exact_venue_order_id
            || *dispatch_class != intent.dispatch_class
        {
            return Err(PmTrialLiveJournalError::ForeignAcknowledgement);
        }
        Ok(())
    }

    fn validate_cancel_prepared_supersession(
        &self,
        intent: &PmDurableCancelIntentAckV1,
    ) -> Result<(), PmTrialLiveJournalError> {
        let Some(DispatchLineV1 {
            body:
                DispatchRecordV1::CancelPrepared {
                    intent: prior_intent,
                    dispatch_class: prior_class,
                    preparation: prior_preparation,
                },
            ..
        }) = self.dispatch.lines.last()
        else {
            return Ok(());
        };
        let candidate = self
            .intent
            .lines
            .last()
            .ok_or(PmTrialLiveJournalError::InvalidTransition)?;
        let IntentRecordV1::CancelIntent {
            ownership_source,
            exact_venue_order_id,
            dispatch_class,
            ..
        } = &candidate.body
        else {
            return Err(PmTrialLiveJournalError::InvalidTransition);
        };
        if candidate.sequence != intent.core.sequence
            || intent_fingerprint(candidate)? != intent.core.record_fingerprint
            || exact_venue_order_id != &intent.exact_venue_order_id
            || *dispatch_class != intent.dispatch_class
            || dispatch_class == prior_class
            || prior_preparation.exact_venue_order_id() != exact_venue_order_id
            || ownership_source.sequence <= prior_intent.sequence
            || ownership_source.sequence.checked_add(1) != Some(candidate.sequence)
        {
            return Err(PmTrialLiveJournalError::InvalidTransition);
        }
        let source = self
            .intent
            .lines
            .get(usize::from(ownership_source.sequence))
            .ok_or(PmTrialLiveJournalError::InvalidTransition)?;
        let IntentRecordV1::Reconciliation {
            state: PmReconciliationOrderStateV1::ExactLive,
            exact_venue_order_id: Some(reconciled_id),
            dispatch: reconciled_target,
            ..
        } = &source.body
        else {
            return Err(PmTrialLiveJournalError::InvalidTransition);
        };
        let preserved_exposure = self
            .dispatch
            .lines
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
            .ok_or(PmTrialLiveJournalError::InvalidTransition)?;
        if intent_fingerprint(source)? != ownership_source.record_fingerprint
            || reconciled_id != exact_venue_order_id
            || reconciled_target.sequence != preserved_exposure.sequence
            || reconciled_target.record_fingerprint != dispatch_fingerprint(preserved_exposure)?
        {
            return Err(PmTrialLiveJournalError::InvalidTransition);
        }
        Ok(())
    }

    fn require_cancel_prepared_tail(
        &self,
        prepared: &PmDurableCancelPreparedAckV1,
    ) -> Result<(), PmTrialLiveJournalError> {
        let line = self
            .dispatch
            .lines
            .last()
            .ok_or(PmTrialLiveJournalError::InvalidTransition)?;
        let DispatchRecordV1::CancelPrepared {
            intent,
            dispatch_class,
            preparation,
        } = &line.body
        else {
            return Err(PmTrialLiveJournalError::InvalidTransition);
        };
        if line.sequence != prepared.core.sequence
            || dispatch_fingerprint(line)? != prepared.core.record_fingerprint
            || *dispatch_class != prepared.dispatch_class
            || preparation.view(*dispatch_class)? != prepared.preparation
            || preparation.exact_venue_order_id() != prepared.exact_venue_order_id
        {
            return Err(PmTrialLiveJournalError::ForeignAcknowledgement);
        }
        let intent_line = self
            .intent
            .lines
            .get(usize::from(intent.sequence))
            .ok_or(PmTrialLiveJournalError::InvalidTransition)?;
        if intent_fingerprint(intent_line)? != intent.record_fingerprint
            || !matches!(intent_line.body, IntentRecordV1::CancelIntent { .. })
        {
            return Err(PmTrialLiveJournalError::ForeignAcknowledgement);
        }
        Ok(())
    }

    fn require_phase_a_live_cancel_dispatch_tail(
        &self,
        dispatch: &PmDurableCancelDispatchAckV1,
    ) -> Result<(), PmTrialLiveJournalError> {
        let line = self
            .dispatch
            .lines
            .last()
            .ok_or(PmTrialLiveJournalError::InvalidTransition)?;
        let DispatchRecordV1::CancelDispatchAuthorized {
            prepared_sequence,
            prepared_record_fingerprint,
            dispatch_class,
            exact_venue_order_id,
            production_order_entry_authorized: false,
            real_order_submission_authorized: false,
            place_dispatch_allowance: 0,
        } = &line.body
        else {
            return Err(PmTrialLiveJournalError::InvalidTransition);
        };
        if line.sequence != dispatch.core.sequence
            || dispatch_fingerprint(line)? != dispatch.core.record_fingerprint
            || *dispatch_class != dispatch.dispatch_class
            || exact_venue_order_id != &dispatch.exact_venue_order_id
        {
            return Err(PmTrialLiveJournalError::ForeignAcknowledgement);
        }
        let prepared_line = self
            .dispatch
            .lines
            .get(usize::from(*prepared_sequence))
            .ok_or(PmTrialLiveJournalError::InvalidTransition)?;
        let DispatchRecordV1::CancelPrepared {
            intent,
            dispatch_class: prepared_class,
            preparation,
        } = &prepared_line.body
        else {
            return Err(PmTrialLiveJournalError::InvalidTransition);
        };
        if dispatch_fingerprint(prepared_line)? != *prepared_record_fingerprint
            || *prepared_class != dispatch.dispatch_class
            || preparation.view(*prepared_class)? != dispatch.preparation
        {
            return Err(PmTrialLiveJournalError::ForeignAcknowledgement);
        }
        let intent_line = self
            .intent
            .lines
            .get(usize::from(intent.sequence))
            .ok_or(PmTrialLiveJournalError::InvalidTransition)?;
        let IntentRecordV1::CancelIntent {
            exact_venue_order_id: intent_order_id,
            dispatch_class: intent_class,
            ..
        } = &intent_line.body
        else {
            return Err(PmTrialLiveJournalError::InvalidTransition);
        };
        if intent_fingerprint(intent_line)? != intent.record_fingerprint
            || intent_order_id != &dispatch.exact_venue_order_id
            || *intent_class != dispatch.dispatch_class
        {
            return Err(PmTrialLiveJournalError::ForeignAcknowledgement);
        }
        Ok(())
    }

    fn require_cancel_result_tail(
        &self,
        result: &PmDurableCancelResultAckV1,
    ) -> Result<(), PmTrialLiveJournalError> {
        let line = self
            .dispatch
            .lines
            .last()
            .ok_or(PmTrialLiveJournalError::InvalidTransition)?;
        let DispatchRecordV1::CancelResult {
            outcome,
            exact_venue_order_id,
            ..
        } = &line.body
        else {
            return Err(PmTrialLiveJournalError::InvalidTransition);
        };
        if line.sequence != result.core.sequence
            || dispatch_fingerprint(line)? != result.core.record_fingerprint
            || *outcome != result.outcome
            || exact_venue_order_id != &result.exact_venue_order_id
        {
            return Err(PmTrialLiveJournalError::ForeignAcknowledgement);
        }
        Ok(())
    }

    fn require_cancel_bridge_tail(
        &self,
        bridge: &PmDurableCancelOutcomeBridgeAckV1,
    ) -> Result<(), PmTrialLiveJournalError> {
        let line = self
            .intent
            .lines
            .last()
            .ok_or(PmTrialLiveJournalError::InvalidTransition)?;
        let IntentRecordV1::CancelOutcomeBridge { dispatch, .. } = &line.body else {
            return Err(PmTrialLiveJournalError::InvalidTransition);
        };
        if line.sequence != bridge.core.sequence
            || intent_fingerprint(line)? != bridge.core.record_fingerprint
            || dispatch != &bridge.dispatch
        {
            return Err(PmTrialLiveJournalError::ForeignAcknowledgement);
        }
        Ok(())
    }

    fn require_reconciliation_tail(
        &self,
        reconciliation: &PmDurableReconciliationAckV1,
    ) -> Result<(), PmTrialLiveJournalError> {
        let line = self
            .intent
            .lines
            .last()
            .ok_or(PmTrialLiveJournalError::InvalidTransition)?;
        if line.sequence != reconciliation.core.sequence
            || intent_fingerprint(line)? != reconciliation.core.record_fingerprint
            || !matches!(line.body, IntentRecordV1::Reconciliation { .. })
        {
            return Err(PmTrialLiveJournalError::ForeignAcknowledgement);
        }
        Ok(())
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
    /// Production primary-cancel entry. Accepted ownership cannot be detached
    /// from its positive barrier and burned authorization custody.
    pub fn record_phase_a_live_primary_cancel_intent_with_custody(
        &mut self,
        mut place: PmPhaseAPlaceLiveOutcomeCustodyV1,
        created_at_utc: String,
    ) -> Result<PmPhaseALiveCancelIntentCustodyV1, PmTrialLiveJournalError> {
        if self.phase_a_live_cancel_epoch.is_some()
            || place.custody.recovery_only
            || place.bridge.outcome != PmPlaceResultKindV1::Accepted
        {
            return Err(PmTrialLiveJournalError::InvalidTransition);
        }
        self.require_phase_a_live_custody_epoch(&place.custody)?;
        place.bridge.core.require_runtime(&self.runtime)?;
        self.validate_phase_a_live_cancel_durable_set(&mut place.custody)?;
        let owned = place
            .owned
            .take()
            .ok_or(PmTrialLiveJournalError::InvalidTransition)?;
        let intent = self.record_cancel_intent_inner(
            place.bridge.core,
            owned,
            PmCancelDispatchClassV1::Primary,
            created_at_utc,
        )?;
        Ok(PmPhaseALiveCancelIntentCustodyV1 {
            intent,
            custody: place.custody,
        })
    }

    /// Positive live-cancel custody seam. Legacy V1 owned-order evidence cannot
    /// enter this method. This compatibility wrapper remains evidence-only;
    /// the production cancel authority path requires the inseparable custody
    /// method above.
    pub fn record_phase_a_live_primary_cancel_intent(
        &mut self,
        place_bridge: PmPhaseAPlaceLiveOutcomeBridgeAckV1,
        owned: PmPhaseAPlaceLiveOwnedVenueOrderV1,
        created_at_utc: String,
    ) -> Result<PmDurablePhaseALiveCancelIntentAckV1, PmTrialLiveJournalError> {
        place_bridge.bridge.core.require_runtime(&self.runtime)?;
        let intent = self.record_cancel_intent_inner(
            place_bridge.bridge.core,
            owned.owned,
            PmCancelDispatchClassV1::Primary,
            created_at_utc,
        )?;
        Ok(PmDurablePhaseALiveCancelIntentAckV1 { intent })
    }

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

    /// Production recovery-cancel entry. Only a durable ExactLive positive
    /// reconciliation carrying the same barrier/authorization custody can
    /// advance the next exact recovery ordinal.
    pub fn record_phase_a_live_recovery_cancel_intent_with_custody(
        &mut self,
        mut reconciliation: PmPhaseALiveReconciliationCustodyV1,
        created_at_utc: String,
    ) -> Result<PmPhaseALiveCancelIntentCustodyV1, PmTrialLiveJournalError> {
        if reconciliation.state != PmReconciliationOrderStateV1::ExactLive {
            return Err(PmTrialLiveJournalError::InvalidTransition);
        }
        let mut custody = reconciliation
            .custody
            .take()
            .ok_or(PmTrialLiveJournalError::InvalidTransition)?;
        self.require_phase_a_live_custody_epoch(&custody)?;
        reconciliation
            .reconciliation
            .core
            .require_runtime(&self.runtime)?;
        self.require_reconciliation_tail(&reconciliation.reconciliation)?;
        self.validate_phase_a_live_cancel_durable_set(&mut custody)?;
        let owned = reconciliation
            .owned
            .take()
            .ok_or(PmTrialLiveJournalError::InvalidTransition)?;
        let ordinal = next_recovery_ordinal(
            &self.dispatch.lines,
            self.scope.trial.order.recovery_cancel_dispatch_budget,
        )?;
        let intent = self.record_cancel_intent_inner(
            reconciliation.reconciliation.core,
            owned,
            PmCancelDispatchClassV1::Recovery { ordinal },
            created_at_utc,
        )?;
        Ok(PmPhaseALiveCancelIntentCustodyV1 { intent, custody })
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
        let predecessor_line = self
            .intent
            .lines
            .last()
            .ok_or(PmTrialLiveJournalError::InvalidTransition)?;
        if predecessor_line.sequence != predecessor.sequence
            || intent_fingerprint(predecessor_line)? != predecessor.record_fingerprint
            || !matches!(
                &predecessor_line.body,
                IntentRecordV1::PlaceOutcomeBridge {
                    outcome: PmPlaceResultKindV1::Accepted,
                    observed_order_id: Some(order_id),
                    ..
                } | IntentRecordV1::Reconciliation {
                    state: PmReconciliationOrderStateV1::ExactLive,
                    exact_venue_order_id: Some(order_id),
                    ..
                } if order_id == &owned.exact_venue_order_id
            )
        {
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
        self.validate_cancel_prepared_supersession(&intent)?;
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

    /// Preserve positive place/custody provenance through cancel preparation.
    pub fn record_phase_a_live_cancel_prepared(
        &mut self,
        intent: PmDurablePhaseALiveCancelIntentAckV1,
        preparation: PmCancelPreparationV1,
    ) -> Result<PmDurablePhaseALiveCancelPreparedAckV1, PmTrialLiveJournalError> {
        let prepared = self.record_cancel_prepared(intent.intent, preparation)?;
        Ok(PmDurablePhaseALiveCancelPreparedAckV1 { prepared })
    }

    /// Prepare a production positive cancel without releasing prior custody.
    pub fn record_phase_a_live_cancel_prepared_with_custody(
        &mut self,
        mut intent: PmPhaseALiveCancelIntentCustodyV1,
        preparation: PmCancelPreparationV1,
    ) -> Result<PmPhaseALiveCancelPreparedCustodyV1, PmTrialLiveJournalError> {
        self.require_phase_a_live_custody_epoch(&intent.custody)?;
        intent.intent.core.require_runtime(&self.runtime)?;
        self.require_cancel_intent_tail(&intent.intent)?;
        self.validate_phase_a_live_cancel_durable_set(&mut intent.custody)?;
        let prepared = self.record_cancel_prepared(intent.intent, preparation)?;
        Ok(PmPhaseALiveCancelPreparedCustodyV1 {
            prepared,
            custody: intent.custody,
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

    /// Preserve positive place/custody provenance through the final durable
    /// cancel-dispatch evidence consumed by a future live cancel join.
    pub fn record_phase_a_live_cancel_dispatch_authorized(
        &mut self,
        prepared: PmDurablePhaseALiveCancelPreparedAckV1,
    ) -> Result<PmDurablePhaseALiveCancelDispatchAckV1, PmTrialLiveJournalError> {
        let dispatch = self.record_cancel_dispatch_authorized(prepared.prepared)?;
        Ok(PmDurablePhaseALiveCancelDispatchAckV1 { dispatch })
    }

    /// Durably append the unchanged evidence-only V1 cancel-dispatch record,
    /// then mint the sole in-memory positive cancel owner by joining it to the
    /// continuously held place custody and a fresh live epoch.
    pub fn record_phase_a_live_cancel_dispatch_authorized_with_custody(
        &mut self,
        mut prepared: PmPhaseALiveCancelPreparedCustodyV1,
    ) -> Result<PmPhaseALiveCancelDispatchOwnerV1, PmTrialLiveJournalError> {
        self.require_phase_a_live_custody_epoch(&prepared.custody)?;
        prepared.prepared.core.require_runtime(&self.runtime)?;
        self.require_cancel_prepared_tail(&prepared.prepared)?;
        self.validate_phase_a_live_cancel_durable_set(&mut prepared.custody)?;
        let dispatch = self.record_cancel_dispatch_authorized(prepared.prepared)?;
        self.transition_phase_a_live_custody_to_cancel_epoch(&mut prepared.custody)?;
        Ok(PmPhaseALiveCancelDispatchOwnerV1 {
            dispatch,
            custody: prepared.custody,
        })
    }

    /// First journal-mediated production cancel check. It validates the live
    /// epoch, exact current cancel tail, both journal files, the positive place
    /// barrier, and the held consumption ledger and claim.
    pub fn revalidate_phase_a_live_cancel_for_runner(
        &mut self,
        mut owner: PmPhaseALiveCancelDispatchOwnerV1,
    ) -> Result<PmRevalidatedPhaseALiveCancelDispatchOwnerV1, PmTrialLiveJournalError> {
        self.require_phase_a_live_cancel_custody_epoch(&owner.custody)?;
        owner.dispatch.core.require_runtime(&self.runtime)?;
        self.require_phase_a_live_cancel_dispatch_tail(&owner.dispatch)?;
        self.validate_phase_a_live_cancel_durable_set(&mut owner.custody)?;
        Ok(PmRevalidatedPhaseALiveCancelDispatchOwnerV1 {
            dispatch: owner.dispatch,
            custody: owner.custody,
        })
    }

    /// Final journal-borrowed production cancel check. The returned owner
    /// keeps all journal leases and the mutable journal borrow alive through
    /// the transport-bound conversion.
    pub fn revalidate_phase_a_live_cancel_for_network_dispatch<'journal>(
        &'journal mut self,
        mut owner: PmRevalidatedPhaseALiveCancelDispatchOwnerV1,
    ) -> Result<PmPhaseALiveCancelNetworkDispatchOwnerV1<'journal>, PmTrialLiveJournalError> {
        self.require_phase_a_live_cancel_custody_epoch(&owner.custody)?;
        owner.dispatch.core.require_runtime(&self.runtime)?;
        self.require_phase_a_live_cancel_dispatch_tail(&owner.dispatch)?;
        self.validate_phase_a_live_cancel_durable_set(&mut owner.custody)?;
        Ok(PmPhaseALiveCancelNetworkDispatchOwnerV1 {
            journals: PmPhaseALiveCancelNetworkJournalV1::MainV1(self),
            dispatch: owner.dispatch,
            custody: owner.custody,
        })
    }

    /// Record a cancel result only after the final owner was consumed into a
    /// may-have-sent observation. The outstanding epoch is deliberately kept
    /// through outcome bridging and reconciliation.
    pub fn record_phase_a_live_cancel_result(
        &mut self,
        mut observation: PmPhaseALiveCancelMayHaveBeenDispatchedV1,
        outcome: PmCancelResultKindV1,
    ) -> Result<PmPhaseALiveCancelResultCustodyV1, PmTrialLiveJournalError> {
        if outcome == PmCancelResultKindV1::DefinitelyNotDispatched {
            return Err(PmTrialLiveJournalError::InvalidTransition);
        }
        self.require_phase_a_live_cancel_custody_epoch(&observation.custody)?;
        observation.dispatch.core.require_runtime(&self.runtime)?;
        self.require_phase_a_live_cancel_dispatch_tail(&observation.dispatch)?;
        self.validate_phase_a_live_cancel_durable_set(&mut observation.custody)?;
        let result = self.record_cancel_result_inner(observation.dispatch, outcome)?;
        Ok(PmPhaseALiveCancelResultCustodyV1 {
            result,
            custody: observation.custody,
        })
    }

    /// Record the exclusive pre-send cancel result. Generic result APIs cannot
    /// author this outcome.
    pub fn record_phase_a_live_cancel_definitely_not_dispatched(
        &mut self,
        mut definitely_not_dispatched: PmPhaseALiveCancelDefinitelyNotDispatchedV1,
    ) -> Result<PmPhaseALiveCancelResultCustodyV1, PmTrialLiveJournalError> {
        self.require_phase_a_live_cancel_custody_epoch(&definitely_not_dispatched.custody)?;
        definitely_not_dispatched
            .dispatch
            .core
            .require_runtime(&self.runtime)?;
        self.require_phase_a_live_cancel_dispatch_tail(&definitely_not_dispatched.dispatch)?;
        self.validate_phase_a_live_cancel_durable_set(&mut definitely_not_dispatched.custody)?;
        let result = self.record_cancel_result_inner(
            definitely_not_dispatched.dispatch,
            PmCancelResultKindV1::DefinitelyNotDispatched,
        )?;
        Ok(PmPhaseALiveCancelResultCustodyV1 {
            result,
            custody: definitely_not_dispatched.custody,
        })
    }

    pub fn record_cancel_result(
        &mut self,
        dispatch: PmDurableCancelDispatchAckV1,
        outcome: PmCancelResultKindV1,
    ) -> Result<PmDurableCancelResultAckV1, PmTrialLiveJournalError> {
        if self.phase_a_live_dispatch_epoch.is_some()
            || self.phase_a_live_cancel_epoch.is_some()
            || outcome == PmCancelResultKindV1::DefinitelyNotDispatched
        {
            return Err(PmTrialLiveJournalError::InvalidTransition);
        }
        self.record_cancel_result_inner(dispatch, outcome)
    }

    fn record_cancel_result_inner(
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
        if self.phase_a_live_dispatch_epoch.is_some() || self.phase_a_live_cancel_epoch.is_some() {
            return Err(PmTrialLiveJournalError::InvalidTransition);
        }
        self.record_cancel_outcome_bridge_inner(result)
    }

    /// Bridge a positive cancel result while keeping the cancel epoch and all
    /// prior place custody inseparable.
    pub fn record_phase_a_live_cancel_outcome_bridge_with_custody(
        &mut self,
        mut result: PmPhaseALiveCancelResultCustodyV1,
    ) -> Result<PmPhaseALiveCancelOutcomeCustodyV1, PmTrialLiveJournalError> {
        self.require_phase_a_live_cancel_custody_epoch(&result.custody)?;
        result.result.core.require_runtime(&self.runtime)?;
        self.require_cancel_result_tail(&result.result)?;
        self.validate_phase_a_live_cancel_durable_set(&mut result.custody)?;
        let exact_venue_order_id = result.result.exact_venue_order_id.clone();
        let bridge = self.record_cancel_outcome_bridge_inner(result.result)?;
        Ok(PmPhaseALiveCancelOutcomeCustodyV1 {
            bridge,
            exact_venue_order_id,
            custody: result.custody,
        })
    }

    fn record_cancel_outcome_bridge_inner(
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

    /// Reconcile a positive place outcome without dropping the outstanding
    /// custody epoch. ExactLive and Ambiguous remain unresolved; a durable
    /// zero-exposure state releases the epoch and carries no further cancel
    /// custody.
    pub fn record_phase_a_place_live_reconciliation_with_custody(
        &mut self,
        mut place: PmPhaseAPlaceLiveOutcomeCustodyV1,
        observed_at_utc: String,
        state: PmReconciliationOrderStateV1,
        exact_venue_order_id: Option<String>,
    ) -> Result<PmPhaseALiveReconciliationCustodyV1, PmTrialLiveJournalError> {
        if self.phase_a_live_cancel_epoch.is_some()
            || place.bridge.outcome == PmPlaceResultKindV1::DefinitelyNotDispatched
        {
            return Err(PmTrialLiveJournalError::InvalidTransition);
        }
        self.require_phase_a_live_custody_epoch(&place.custody)?;
        place.bridge.core.require_runtime(&self.runtime)?;
        self.require_place_bridge_tail(&place.bridge)?;
        self.validate_phase_a_live_cancel_durable_set(&mut place.custody)?;
        let dispatch_target = place.bridge.dispatch.clone();
        let (reconciliation, owned) = self.record_reconciliation_inner(
            dispatch_target.clone(),
            observed_at_utc,
            state,
            exact_venue_order_id,
        )?;
        self.finish_phase_a_live_reconciliation(
            reconciliation,
            dispatch_target,
            state,
            owned,
            place.custody,
        )
    }

    /// Re-observe an unresolved positive lineage. This consumes the previous
    /// reconciliation owner, so an ExactLive ownership token can never be
    /// retained while a newer observation is recorded.
    pub fn record_phase_a_live_reconciliation_again_with_custody(
        &mut self,
        mut previous: PmPhaseALiveReconciliationCustodyV1,
        observed_at_utc: String,
        state: PmReconciliationOrderStateV1,
        exact_venue_order_id: Option<String>,
    ) -> Result<PmPhaseALiveReconciliationCustodyV1, PmTrialLiveJournalError> {
        let mut custody = previous
            .custody
            .take()
            .ok_or(PmTrialLiveJournalError::InvalidTransition)?;
        self.require_phase_a_live_custody_epoch(&custody)?;
        previous
            .reconciliation
            .core
            .require_runtime(&self.runtime)?;
        self.validate_phase_a_live_cancel_durable_set(&mut custody)?;
        let PmPhaseALiveReconciliationTargetV1::MainV1(dispatch_target) = previous.dispatch_target
        else {
            return Err(PmTrialLiveJournalError::InvalidTransition);
        };
        let (reconciliation, owned) = self.record_reconciliation_inner(
            dispatch_target.clone(),
            observed_at_utc,
            state,
            exact_venue_order_id,
        )?;
        self.finish_phase_a_live_reconciliation(
            reconciliation,
            dispatch_target,
            state,
            owned,
            custody,
        )
    }

    /// Reconcile a positive cancel outcome. The live cancel epoch remains the
    /// journal exclusion marker until this append is durable.
    pub fn record_phase_a_live_cancel_reconciliation_with_custody(
        &mut self,
        mut cancel: PmPhaseALiveCancelOutcomeCustodyV1,
        observed_at_utc: String,
        state: PmReconciliationOrderStateV1,
        exact_venue_order_id: Option<String>,
    ) -> Result<PmPhaseALiveReconciliationCustodyV1, PmTrialLiveJournalError> {
        self.require_phase_a_live_cancel_custody_epoch(&cancel.custody)?;
        cancel.bridge.core.require_runtime(&self.runtime)?;
        self.require_cancel_bridge_tail(&cancel.bridge)?;
        self.validate_phase_a_live_cancel_durable_set(&mut cancel.custody)?;
        if matches!(
            state,
            PmReconciliationOrderStateV1::ExactLive
                | PmReconciliationOrderStateV1::ExactCanceled
                | PmReconciliationOrderStateV1::ExactFilled
        ) && exact_venue_order_id.as_deref() != Some(cancel.exact_venue_order_id.as_str())
        {
            return Err(PmTrialLiveJournalError::InvalidBinding);
        }
        let dispatch_target = cancel.bridge.dispatch.clone();
        let (reconciliation, owned) = self.record_reconciliation_inner(
            dispatch_target.clone(),
            observed_at_utc,
            state,
            exact_venue_order_id,
        )?;
        self.finish_phase_a_live_reconciliation(
            reconciliation,
            dispatch_target,
            state,
            owned,
            cancel.custody,
        )
    }

    fn finish_phase_a_live_reconciliation(
        &mut self,
        reconciliation: PmDurableReconciliationAckV1,
        dispatch_target: CounterpartLinkV1,
        state: PmReconciliationOrderStateV1,
        owned: Option<PmJournalOwnedVenueOrderV1>,
        custody: PmPhaseALivePlaceCustodyV1,
    ) -> Result<PmPhaseALiveReconciliationCustodyV1, PmTrialLiveJournalError> {
        let unresolved = matches!(
            state,
            PmReconciliationOrderStateV1::ExactLive | PmReconciliationOrderStateV1::Ambiguous
        );
        let custody = if unresolved {
            Some(custody)
        } else {
            self.complete_phase_a_live_custody_epoch(&custody)?;
            None
        };
        Ok(PmPhaseALiveReconciliationCustodyV1 {
            reconciliation,
            dispatch_target: PmPhaseALiveReconciliationTargetV1::MainV1(dispatch_target),
            state,
            owned,
            custody,
        })
    }

    pub fn record_phase_a_place_live_reconciliation(
        &mut self,
        place_bridge: PmPhaseAPlaceLiveOutcomeBridgeAckV1,
        observed_at_utc: String,
        state: PmReconciliationOrderStateV1,
        exact_venue_order_id: Option<String>,
    ) -> Result<
        (
            PmDurableReconciliationAckV1,
            Option<PmPhaseAPlaceLiveOwnedVenueOrderV1>,
        ),
        PmTrialLiveJournalError,
    > {
        if self.phase_a_live_dispatch_epoch.is_some() || self.phase_a_live_cancel_epoch.is_some() {
            return Err(PmTrialLiveJournalError::InvalidTransition);
        }
        place_bridge.bridge.core.require_runtime(&self.runtime)?;
        if place_bridge.bridge.outcome == PmPlaceResultKindV1::DefinitelyNotDispatched {
            return Err(PmTrialLiveJournalError::InvalidTransition);
        }
        let (ack, owned) = self.record_reconciliation_inner(
            place_bridge.bridge.dispatch,
            observed_at_utc,
            state,
            exact_venue_order_id,
        )?;
        Ok((
            ack,
            owned.map(|owned| PmPhaseAPlaceLiveOwnedVenueOrderV1 { owned }),
        ))
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
        if self.phase_a_live_dispatch_epoch.is_some() || self.phase_a_live_cancel_epoch.is_some() {
            return Err(PmTrialLiveJournalError::InvalidTransition);
        }
        place_bridge.core.require_runtime(&self.runtime)?;
        if place_bridge.outcome == PmPlaceResultKindV1::DefinitelyNotDispatched {
            return Err(PmTrialLiveJournalError::InvalidTransition);
        }
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
        if self.phase_a_live_dispatch_epoch.is_some() || self.phase_a_live_cancel_epoch.is_some() {
            return Err(PmTrialLiveJournalError::InvalidTransition);
        }
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
        let place_dispatch_recorded = self
            .dispatch
            .lines
            .iter()
            .any(|line| matches!(&line.body, DispatchRecordV1::PlaceDispatchAuthorized { .. }));
        if self.phase_a_live_dispatch_epoch.is_some()
            || self.phase_a_live_cancel_epoch.is_some()
            || (place_dispatch_recorded
                && !phase_a_live_terminal_is_safe(&self.intent.lines, &self.dispatch.lines))
        {
            return Err(PmTrialLiveJournalError::InvalidTransition);
        }
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

fn phase_a_live_terminal_is_safe(intent: &[IntentLineV1], dispatch: &[DispatchLineV1]) -> bool {
    let Some(latest_exposure) = dispatch.iter().rev().find(|line| {
        matches!(
            &line.body,
            DispatchRecordV1::PlaceDispatchAuthorized { .. }
                | DispatchRecordV1::PlaceResult { .. }
                | DispatchRecordV1::CancelDispatchAuthorized { .. }
                | DispatchRecordV1::CancelResult { .. }
        )
    }) else {
        return false;
    };
    if matches!(
        &latest_exposure.body,
        DispatchRecordV1::PlaceResult {
            outcome: PmPlaceResultKindV1::DefinitelyNotDispatched,
            ..
        }
    ) {
        return true;
    }
    let Some((state, target_sequence)) = intent.iter().rev().find_map(|line| match &line.body {
        IntentRecordV1::Reconciliation {
            state, dispatch, ..
        } => Some((*state, dispatch.sequence)),
        _ => None,
    }) else {
        return false;
    };
    matches!(
        state,
        PmReconciliationOrderStateV1::Absent
            | PmReconciliationOrderStateV1::ExactCanceled
            | PmReconciliationOrderStateV1::ExactFilled
    ) && target_sequence >= latest_exposure.sequence
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
        || consumption_files.contains(&PM_PHASE_A_LIVE_DISPATCH_BARRIER_FILE_V1)
        || consumption_files.contains(&PM_PHASE_A_LIVE_CANCEL_RECOVERY_CONTINUATION_INTENT_FILE_V1)
        || consumption_files
            .contains(&PM_PHASE_A_LIVE_CANCEL_RECOVERY_CONTINUATION_DISPATCH_FILE_V1)
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
        | PmPlaceResultKindV1::DefinitelyNotDispatched
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

fn validate_phase_a_live_recovery_runtime(
    config: &CanonicalTrialConfig,
    authorization: &CanonicalAuthorization,
    current_runtime: &AuthorizationRuntimeBinding,
    projection: &PmTrialLiveRecoveryProjectionV1,
) -> Result<(), PmTrialLiveJournalError> {
    let authorization_value = authorization.value();
    let current_utc = validate_utc(&current_runtime.observed_at_utc)?;
    let original_utc = validate_utc(&projection.scope.runtime_observed_at_utc)?;
    let cleanup_not_after_utc = validate_utc(&authorization_value.cleanup_not_after_utc)?;
    if config.value().phase != reap_pm_controlled_trial::TrialPhase::APlaceCancel
        || authorization_value.phase != reap_pm_controlled_trial::TrialPhase::APlaceCancel
        || authorization_value.trial != *config.value()
        || current_runtime.release_binary_sha256 != authorization_value.build.release_binary_sha256
        || current_runtime.release_binary_length != authorization_value.build.release_binary_length
        || current_runtime.host != authorization_value.host
        || current_runtime.release_binary_sha256 != projection.scope.release_binary_sha256
        || current_runtime.release_binary_length != projection.scope.release_binary_length
        || current_runtime.host != projection.scope.host
        || projection.scope.credential_slot_id != config.value().credential_slot.slot_id
        || projection
            .scope
            .credential_slot_nonsecret_fingerprint_sha256
            != config.value().credential_slot.nonsecret_fingerprint_sha256
        || projection.scope.authorization_cleanup_not_after_utc
            != authorization_value.cleanup_not_after_utc
        || current_utc < original_utc
        || current_utc > cleanup_not_after_utc
    {
        return Err(PmTrialLiveJournalError::InvalidBinding);
    }
    Ok(())
}

// Recovery methods are implemented after the verifier has produced an exact
// move-only projection; this type exposes no place path.
pub struct PmControlledTrialLiveRecoveryJournals {
    pub(crate) inner: PmControlledTrialLiveJournals,
    pub(crate) projection: PmTrialLiveRecoveryProjectionV1,
    initial_reconciliation_available: bool,
}

/// Capability-narrow production recovery-cancel journal owner. Only this type
/// retains reopened burned authorization custody and exposes the positive
/// recovery cancel chain.
pub struct PmControlledTrialLiveCancelRecoveryJournals {
    evidence: PmControlledTrialLiveRecoveryJournals,
    custody: Option<PmPhaseALivePlaceCustodyV1>,
    continuation: Option<RecoveryContinuationJournalsV1>,
    required_action: Option<PmPhaseALiveCancelRecoveryRequiredActionV1>,
}

impl PmControlledTrialLiveRecoveryJournals {
    /// Evidence-only compatibility constructor. It exposes reconciliation and
    /// legacy false-V1 journal APIs but mints no production cancel custody.
    pub fn open(
        config: &CanonicalTrialConfig,
        authorization: &CanonicalAuthorization,
        projection: PmTrialLiveRecoveryProjectionV1,
    ) -> Result<Self, PmTrialLiveJournalError> {
        if projection.recovery_continuation_basis.is_some() {
            return Err(PmTrialLiveJournalError::RecoveryOperationForbidden);
        }
        Self::open_inner(config, authorization, projection, false)
    }

    fn open_inner(
        config: &CanonicalTrialConfig,
        authorization: &CanonicalAuthorization,
        projection: PmTrialLiveRecoveryProjectionV1,
        allow_completed_recovery_continuation_terminal: bool,
    ) -> Result<Self, PmTrialLiveJournalError> {
        let normal_recovery = matches!(
            projection.classification,
            PmTrialLiveRecoveryClassificationV1::PlaceMayHaveBeenSentNoResend
                | PmTrialLiveRecoveryClassificationV1::RecoveryCancelOnly { .. }
                | PmTrialLiveRecoveryClassificationV1::ReconcileBeforeRecoveryCancel { .. }
        );
        let completed_continuation_terminal = allow_completed_recovery_continuation_terminal
            && projection.is_completed_recovery_continuation_terminal();
        if !normal_recovery && !completed_continuation_terminal {
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
            phase_a_live_dispatch_context: None,
            phase_a_live_dispatch_epoch: None,
            phase_a_live_cancel_epoch: None,
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
        if !self.initial_reconciliation_available
            || self.inner.phase_a_live_dispatch_epoch.is_some()
            || self.inner.phase_a_live_cancel_epoch.is_some()
        {
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

    /// Positive-barrier recovery reconciliation. Only this seam can mint the
    /// live-owned order required by future production recovery cancel.
    pub fn record_phase_a_live_reconciliation(
        &mut self,
        observed_at_utc: String,
        state: PmReconciliationOrderStateV1,
        exact_venue_order_id: Option<String>,
    ) -> Result<
        (
            PmDurableReconciliationAckV1,
            Option<PmPhaseAPlaceLiveOwnedVenueOrderV1>,
        ),
        PmTrialLiveJournalError,
    > {
        if !self.initial_reconciliation_available
            || self.inner.phase_a_live_dispatch_epoch.is_some()
            || self.inner.phase_a_live_cancel_epoch.is_some()
            || self
                .projection
                .phase_a_live_dispatch_barrier_fingerprint
                .is_none()
        {
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
        let (ack, owned) = self.inner.record_reconciliation_inner(
            self.projection.reconciliation_target.clone(),
            observed_at_utc,
            state,
            exact_venue_order_id,
        )?;
        self.initial_reconciliation_available = false;
        Ok((
            ack,
            owned.map(|owned| PmPhaseAPlaceLiveOwnedVenueOrderV1 { owned }),
        ))
    }

    pub fn record_recovery_cancel_intent(
        &mut self,
        reconciliation: PmDurableReconciliationAckV1,
        owned: PmJournalOwnedVenueOrderV1,
        created_at_utc: String,
    ) -> Result<PmDurableCancelIntentAckV1, PmTrialLiveJournalError> {
        let ordinal = next_recovery_ordinal(
            &self.inner.dispatch.lines,
            self.inner.scope.trial.order.recovery_cancel_dispatch_budget,
        )?;
        self.inner
            .record_recovery_cancel_intent(reconciliation, owned, ordinal, created_at_utc)
    }

    pub fn record_phase_a_live_recovery_cancel_intent(
        &mut self,
        reconciliation: PmDurableReconciliationAckV1,
        owned: PmPhaseAPlaceLiveOwnedVenueOrderV1,
        created_at_utc: String,
    ) -> Result<PmDurablePhaseALiveCancelIntentAckV1, PmTrialLiveJournalError> {
        let ordinal = next_recovery_ordinal(
            &self.inner.dispatch.lines,
            self.inner.scope.trial.order.recovery_cancel_dispatch_budget,
        )?;
        let intent = self.inner.record_recovery_cancel_intent(
            reconciliation,
            owned.owned,
            ordinal,
            created_at_utc,
        )?;
        Ok(PmDurablePhaseALiveCancelIntentAckV1 { intent })
    }

    pub fn record_cancel_prepared(
        &mut self,
        intent: PmDurableCancelIntentAckV1,
        preparation: PmCancelPreparationV1,
    ) -> Result<PmDurableCancelPreparedAckV1, PmTrialLiveJournalError> {
        self.inner.record_cancel_prepared(intent, preparation)
    }

    pub fn record_phase_a_live_cancel_prepared(
        &mut self,
        intent: PmDurablePhaseALiveCancelIntentAckV1,
        preparation: PmCancelPreparationV1,
    ) -> Result<PmDurablePhaseALiveCancelPreparedAckV1, PmTrialLiveJournalError> {
        self.inner
            .record_phase_a_live_cancel_prepared(intent, preparation)
    }

    pub fn record_cancel_dispatch_authorized(
        &mut self,
        prepared: PmDurableCancelPreparedAckV1,
    ) -> Result<PmDurableCancelDispatchAckV1, PmTrialLiveJournalError> {
        self.inner.record_cancel_dispatch_authorized(prepared)
    }

    pub fn record_phase_a_live_cancel_dispatch_authorized(
        &mut self,
        prepared: PmDurablePhaseALiveCancelPreparedAckV1,
    ) -> Result<PmDurablePhaseALiveCancelDispatchAckV1, PmTrialLiveJournalError> {
        self.inner
            .record_phase_a_live_cancel_dispatch_authorized(prepared)
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

impl PmControlledTrialLiveCancelRecoveryJournals {
    /// Production recovery-cancel constructor. The caller must re-observe the
    /// exact authorized release and full host/boot/user/egress identity with a
    /// canonical UTC observation that has not passed the cleanup deadline.
    /// This wall-clock binding is not a monotonic freshness proof; the future
    /// runner must capture its process clock again before final cancel. This
    /// constructor does not restore placement authority.
    pub fn open_phase_a_live_cancel(
        config: &CanonicalTrialConfig,
        authorization: &CanonicalAuthorization,
        current_runtime: &AuthorizationRuntimeBinding,
        projection: PmTrialLiveRecoveryProjectionV1,
    ) -> Result<Self, PmTrialLiveJournalError> {
        validate_phase_a_live_recovery_runtime(
            config,
            authorization,
            current_runtime,
            &projection,
        )?;
        let continuation_basis = projection.recovery_continuation_basis.clone();
        let required_action = projection
            .phase_a_live_cancel_recovery_required_action
            .as_ref()
            .copied();
        let mut evidence = PmControlledTrialLiveRecoveryJournals::open_inner(
            config,
            authorization,
            projection,
            true,
        )?;
        let barrier = evidence
            .projection
            .phase_a_live_dispatch_barrier_fingerprint
            .as_deref()
            .map(|barrier_fingerprint| {
                reopen_phase_a_place_dispatch_barrier_witness(
                    config,
                    authorization,
                    &evidence.projection.scope,
                    &evidence.inner.preflight,
                    &evidence.projection.dispatch_lines,
                    barrier_fingerprint,
                )
            })
            .transpose()?;
        // Exact burned evidence plus a later exact-ID reconciliation is the
        // recovery cleanup root. The optional positive barrier strengthens
        // provenance when intact but its loss cannot suppress cancellation.
        let mut consumed_authorization =
            reopen_consumed_authorization_consumption(config, authorization)
                .map_err(|_| PmTrialLiveJournalError::Protection)?;
        let (mut continuation, journal_backend) = if let Some(basis) = continuation_basis {
            let (mut continuation, created) = RecoveryContinuationJournalsV1::create_or_open(
                config,
                authorization,
                &evidence.projection.scope,
                &basis,
                evidence.projection.consumption.consumed_fingerprints()?,
                &evidence.inner.preflight,
                current_runtime,
            )?;
            continuation.validate_against_consumption_registry(
                evidence
                    .projection
                    .consumption
                    .recovery_continuation_registry(),
            )?;
            if created {
                // Both new fixed continuation entries are now fully bound and
                // durable. Refresh every retained view of their parent before
                // exposing recovery-cancel custody.
                evidence.inner.artifact_lease.refresh_after_bound_create()?;
                evidence
                    .inner
                    .intent
                    .file
                    .refresh_parent_after_bound_create()?;
                evidence
                    .inner
                    .dispatch
                    .file
                    .refresh_parent_after_bound_create()?;
                consumed_authorization
                    .refresh_after_bound_artifact_create()
                    .map_err(|_| PmTrialLiveJournalError::Protection)?;
                continuation.refresh_after_bound_create()?;
            }
            // The monotonic registry may be one deterministic record ahead of
            // the pair. Revalidate the entire immutable root immediately
            // before source-completing that exact Prepared or Terminal plan.
            evidence.inner.validate_phase_a_live_journal_files()?;
            evidence.inner.artifact_lease.validate()?;
            consumed_authorization
                .revalidate_held_consumption_evidence()
                .map_err(|_| PmTrialLiveJournalError::Protection)?;
            continuation.validate_exact()?;
            continuation.complete_consumption_registry(&mut consumed_authorization)?;
            consumed_authorization
                .revalidate_held_consumption_evidence()
                .map_err(|_| PmTrialLiveJournalError::Protection)?;
            continuation.validate_fully_anchored_against_consumption_registry(
                consumed_authorization.recovery_continuation_registry(),
            )?;
            (
                Some(continuation),
                PmPhaseALiveCancelJournalBackendV1::RecoveryContinuationV1,
            )
        } else {
            (None, PmPhaseALiveCancelJournalBackendV1::MainV1)
        };
        if let Some(continuation) = &mut continuation {
            continuation.validate_exact()?;
        }
        let continuation_terminal_complete = continuation
            .as_ref()
            .map(|continuation| {
                continuation.has_complete_anchored_terminal_plan(
                    consumed_authorization.recovery_continuation_registry(),
                )
            })
            .transpose()?
            .unwrap_or(false);
        if continuation_terminal_complete {
            evidence.initial_reconciliation_available = false;
        }
        evidence.inner.validate_phase_a_live_journal_files()?;
        consumed_authorization
            .revalidate_held_consumption_evidence()
            .map_err(|_| PmTrialLiveJournalError::Protection)?;
        if continuation_terminal_complete {
            evidence.inner.artifact_lease.validate()?;
            return Ok(Self {
                evidence,
                custody: None,
                continuation,
                required_action: Some(
                    PmPhaseALiveCancelRecoveryRequiredActionV1::TerminalEvidenceOnly,
                ),
            });
        }
        let epoch = Arc::new(PmPhaseALiveDispatchEpoch);
        evidence.inner.phase_a_live_dispatch_epoch = Some(Arc::clone(&epoch));
        Ok(Self {
            evidence,
            custody: Some(PmPhaseALivePlaceCustodyV1 {
                barrier,
                authorization: consumed_authorization,
                epoch: PmPhaseALiveCustodyEpochV1::Place(epoch),
                recovery_only: true,
                journal_backend,
            }),
            continuation,
            required_action,
        })
    }

    #[must_use]
    pub const fn preflight_binding(&self) -> &PmTrialLivePreflightBindingV1 {
        &self.evidence.inner.preflight
    }

    #[must_use]
    pub const fn required_action(&self) -> Option<&PmPhaseALiveCancelRecoveryRequiredActionV1> {
        self.required_action.as_ref()
    }

    fn validate_recovery_continuation_durable_set(
        &mut self,
        custody: &mut PmPhaseALivePlaceCustodyV1,
    ) -> Result<(), PmTrialLiveJournalError> {
        if custody.journal_backend != PmPhaseALiveCancelJournalBackendV1::RecoveryContinuationV1
            || !custody.recovery_only
            || custody.barrier.is_some()
        {
            return Err(PmTrialLiveJournalError::InvalidTransition);
        }
        self.evidence
            .inner
            .require_phase_a_live_custody_epoch(custody)?;
        self.evidence.inner.validate_phase_a_live_journal_files()?;
        custody
            .authorization
            .revalidate_held_consumption_evidence()
            .map_err(|_| PmTrialLiveJournalError::Protection)?;
        let continuation = self
            .continuation
            .as_mut()
            .ok_or(PmTrialLiveJournalError::RecoveryOperationForbidden)?;
        continuation.validate_exact()?;
        continuation.validate_fully_anchored_against_consumption_registry(
            custody.authorization.recovery_continuation_registry(),
        )?;
        self.evidence.inner.artifact_lease.validate()
    }

    fn finish_recovery_continuation_reconciliation(
        &mut self,
        reconciliation: PmDurableReconciliationAckV1,
        dispatch_target: ContinuationDispatchTargetV1,
        state: PmReconciliationOrderStateV1,
        owned: Option<PmJournalOwnedVenueOrderV1>,
        custody: PmPhaseALivePlaceCustodyV1,
    ) -> Result<PmPhaseALiveReconciliationCustodyV1, PmTrialLiveJournalError> {
        let unresolved = matches!(
            state,
            PmReconciliationOrderStateV1::ExactLive | PmReconciliationOrderStateV1::Ambiguous
        );
        let custody = if unresolved {
            Some(custody)
        } else {
            // A safe observation removes live-order custody from the returned
            // reconciliation token, but the recovery wrapper must retain the
            // full held durable root and outstanding epoch until Terminal is
            // itself durable.
            self.custody = Some(custody);
            None
        };
        Ok(PmPhaseALiveReconciliationCustodyV1 {
            reconciliation,
            dispatch_target: PmPhaseALiveReconciliationTargetV1::RecoveryContinuationV1(
                dispatch_target,
            ),
            state,
            owned,
            custody,
        })
    }

    /// First exact recovery observation. Only this type can join that
    /// observation to reopened positive recovery-cancel custody.
    pub fn record_phase_a_live_reconciliation_with_custody(
        &mut self,
        observed_at_utc: String,
        state: PmReconciliationOrderStateV1,
        exact_venue_order_id: Option<String>,
    ) -> Result<PmPhaseALiveReconciliationCustodyV1, PmTrialLiveJournalError> {
        if !self.evidence.initial_reconciliation_available {
            return Err(PmTrialLiveJournalError::RecoveryOperationForbidden);
        }
        let mut custody = self
            .custody
            .take()
            .ok_or(PmTrialLiveJournalError::RecoveryOperationForbidden)?;
        if custody.journal_backend == PmPhaseALiveCancelJournalBackendV1::RecoveryContinuationV1 {
            if self.required_action
                != Some(PmPhaseALiveCancelRecoveryRequiredActionV1::ReconcileCurrentExposure)
            {
                return Err(PmTrialLiveJournalError::RecoveryOperationForbidden);
            }
            self.validate_recovery_continuation_durable_set(&mut custody)?;
            let continuation = self
                .continuation
                .as_mut()
                .ok_or(PmTrialLiveJournalError::RecoveryOperationForbidden)?;
            let dispatch_target = continuation.current_reconciliation_target()?;
            let ack = continuation.record_reconciliation(
                dispatch_target.clone(),
                observed_at_utc,
                state,
                exact_venue_order_id.clone(),
            )?;
            let reconciliation = PmDurableReconciliationAckV1 {
                core: continuation_core(&self.evidence.inner.runtime, ack),
            };
            let owned = if state == PmReconciliationOrderStateV1::ExactLive {
                Some(PmJournalOwnedVenueOrderV1 {
                    runtime: Arc::clone(&self.evidence.inner.runtime),
                    exact_venue_order_id: exact_venue_order_id
                        .ok_or(PmTrialLiveJournalError::InvalidTransition)?,
                })
            } else {
                None
            };
            self.evidence.initial_reconciliation_available = false;
            self.required_action = None;
            return self.finish_recovery_continuation_reconciliation(
                reconciliation,
                dispatch_target,
                state,
                owned,
                custody,
            );
        }
        if let PmTrialLiveRecoveryClassificationV1::RecoveryCancelOnly {
            exact_venue_order_id: expected,
        } = &self.evidence.projection.classification
            && (state != PmReconciliationOrderStateV1::ExactLive
                || exact_venue_order_id.as_deref() != Some(expected))
        {
            return Err(PmTrialLiveJournalError::InvalidBinding);
        }
        self.evidence
            .inner
            .require_phase_a_live_custody_epoch(&custody)?;
        self.evidence
            .inner
            .validate_phase_a_live_cancel_durable_set(&mut custody)?;
        let dispatch_target = self.evidence.projection.reconciliation_target.clone();
        let (reconciliation, owned) = self.evidence.inner.record_reconciliation_inner(
            dispatch_target.clone(),
            observed_at_utc,
            state,
            exact_venue_order_id,
        )?;
        self.evidence.initial_reconciliation_available = false;
        self.evidence.inner.finish_phase_a_live_reconciliation(
            reconciliation,
            dispatch_target,
            state,
            owned,
            custody,
        )
    }

    pub fn record_phase_a_live_recovery_cancel_intent_with_custody(
        &mut self,
        mut reconciliation: PmPhaseALiveReconciliationCustodyV1,
        created_at_utc: String,
    ) -> Result<PmPhaseALiveCancelIntentCustodyV1, PmTrialLiveJournalError> {
        let continuation_target = match &reconciliation.dispatch_target {
            PmPhaseALiveReconciliationTargetV1::RecoveryContinuationV1(target) => {
                Some(target.clone())
            }
            PmPhaseALiveReconciliationTargetV1::MainV1(_) => None,
        };
        if let Some(continuation_target) = continuation_target {
            if reconciliation.state != PmReconciliationOrderStateV1::ExactLive {
                return Err(PmTrialLiveJournalError::InvalidTransition);
            }
            let mut custody = reconciliation
                .custody
                .take()
                .ok_or(PmTrialLiveJournalError::InvalidTransition)?;
            self.validate_recovery_continuation_durable_set(&mut custody)?;
            reconciliation
                .reconciliation
                .core
                .require_runtime(&self.evidence.inner.runtime)?;
            let owned = reconciliation
                .owned
                .take()
                .ok_or(PmTrialLiveJournalError::InvalidTransition)?;
            let exact_venue_order_id = owned.exact_venue_order_id.clone();
            let continuation = self
                .continuation
                .as_mut()
                .ok_or(PmTrialLiveJournalError::RecoveryOperationForbidden)?;
            if continuation.current_reconciliation_target()? != continuation_target {
                return Err(PmTrialLiveJournalError::InvalidRecord);
            }
            let dispatch_class = PmCancelDispatchClassV1::Recovery {
                ordinal: continuation.next_recovery_ordinal()?,
            };
            let ack = continuation.record_cancel_intent(
                &continuation_ack(&reconciliation.reconciliation.core),
                dispatch_class,
                created_at_utc,
            )?;
            return Ok(PmPhaseALiveCancelIntentCustodyV1 {
                intent: PmDurableCancelIntentAckV1 {
                    core: continuation_core(&self.evidence.inner.runtime, ack),
                    dispatch_class,
                    exact_venue_order_id,
                },
                custody,
            });
        }
        self.evidence
            .inner
            .record_phase_a_live_recovery_cancel_intent_with_custody(reconciliation, created_at_utc)
    }

    pub fn record_phase_a_live_cancel_prepared_with_custody(
        &mut self,
        mut intent: PmPhaseALiveCancelIntentCustodyV1,
        preparation: PmCancelPreparationV1,
    ) -> Result<PmPhaseALiveCancelPreparedCustodyV1, PmTrialLiveJournalError> {
        if intent.custody.journal_backend
            == PmPhaseALiveCancelJournalBackendV1::RecoveryContinuationV1
        {
            self.validate_recovery_continuation_durable_set(&mut intent.custody)?;
            intent
                .intent
                .core
                .require_runtime(&self.evidence.inner.runtime)?;
            let continuation = self
                .continuation
                .as_mut()
                .ok_or(PmTrialLiveJournalError::RecoveryOperationForbidden)?;
            let (ack, preparation_view) = continuation.record_cancel_prepared_ledger_first(
                &mut intent.custody.authorization,
                &continuation_ack(&intent.intent.core),
                intent.intent.dispatch_class,
                preparation,
            )?;
            intent
                .custody
                .authorization
                .revalidate_held_consumption_evidence()
                .map_err(|_| PmTrialLiveJournalError::Protection)?;
            continuation.validate_fully_anchored_against_consumption_registry(
                intent
                    .custody
                    .authorization
                    .recovery_continuation_registry(),
            )?;
            return Ok(PmPhaseALiveCancelPreparedCustodyV1 {
                prepared: PmDurableCancelPreparedAckV1 {
                    core: continuation_core(&self.evidence.inner.runtime, ack),
                    dispatch_class: intent.intent.dispatch_class,
                    exact_venue_order_id: intent.intent.exact_venue_order_id,
                    preparation: preparation_view,
                },
                custody: intent.custody,
            });
        }
        self.evidence
            .inner
            .record_phase_a_live_cancel_prepared_with_custody(intent, preparation)
    }

    pub fn record_phase_a_live_cancel_dispatch_authorized_with_custody(
        &mut self,
        mut prepared: PmPhaseALiveCancelPreparedCustodyV1,
    ) -> Result<PmPhaseALiveCancelDispatchOwnerV1, PmTrialLiveJournalError> {
        if prepared.custody.journal_backend
            == PmPhaseALiveCancelJournalBackendV1::RecoveryContinuationV1
        {
            self.validate_recovery_continuation_durable_set(&mut prepared.custody)?;
            prepared
                .prepared
                .core
                .require_runtime(&self.evidence.inner.runtime)?;
            let continuation = self
                .continuation
                .as_mut()
                .ok_or(PmTrialLiveJournalError::RecoveryOperationForbidden)?;
            let ack = continuation.record_cancel_dispatch_authorized(
                &continuation_ack(&prepared.prepared.core),
                prepared.prepared.dispatch_class,
                &prepared.prepared.exact_venue_order_id,
            )?;
            let dispatch = PmDurableCancelDispatchAckV1 {
                core: continuation_core(&self.evidence.inner.runtime, ack),
                dispatch_class: prepared.prepared.dispatch_class,
                exact_venue_order_id: prepared.prepared.exact_venue_order_id,
                preparation: prepared.prepared.preparation,
            };
            self.evidence
                .inner
                .transition_phase_a_live_custody_to_cancel_epoch(&mut prepared.custody)?;
            return Ok(PmPhaseALiveCancelDispatchOwnerV1 {
                dispatch,
                custody: prepared.custody,
            });
        }
        self.evidence
            .inner
            .record_phase_a_live_cancel_dispatch_authorized_with_custody(prepared)
    }

    pub fn revalidate_phase_a_live_cancel_for_runner(
        &mut self,
        mut owner: PmPhaseALiveCancelDispatchOwnerV1,
    ) -> Result<PmRevalidatedPhaseALiveCancelDispatchOwnerV1, PmTrialLiveJournalError> {
        if owner.custody.journal_backend
            == PmPhaseALiveCancelJournalBackendV1::RecoveryContinuationV1
        {
            self.evidence
                .inner
                .require_phase_a_live_cancel_custody_epoch(&owner.custody)?;
            self.validate_recovery_continuation_durable_set(&mut owner.custody)?;
            owner
                .dispatch
                .core
                .require_runtime(&self.evidence.inner.runtime)?;
            self.continuation
                .as_ref()
                .ok_or(PmTrialLiveJournalError::RecoveryOperationForbidden)?
                .target_for_dispatch_ack(&continuation_ack(&owner.dispatch.core))?;
            return Ok(PmRevalidatedPhaseALiveCancelDispatchOwnerV1 {
                dispatch: owner.dispatch,
                custody: owner.custody,
            });
        }
        self.evidence
            .inner
            .revalidate_phase_a_live_cancel_for_runner(owner)
    }

    pub fn revalidate_phase_a_live_cancel_for_network_dispatch<'journal>(
        &'journal mut self,
        mut owner: PmRevalidatedPhaseALiveCancelDispatchOwnerV1,
    ) -> Result<PmPhaseALiveCancelNetworkDispatchOwnerV1<'journal>, PmTrialLiveJournalError> {
        if owner.custody.journal_backend
            == PmPhaseALiveCancelJournalBackendV1::RecoveryContinuationV1
        {
            self.evidence
                .inner
                .require_phase_a_live_cancel_custody_epoch(&owner.custody)?;
            self.validate_recovery_continuation_durable_set(&mut owner.custody)?;
            owner
                .dispatch
                .core
                .require_runtime(&self.evidence.inner.runtime)?;
            self.continuation
                .as_ref()
                .ok_or(PmTrialLiveJournalError::RecoveryOperationForbidden)?
                .target_for_dispatch_ack(&continuation_ack(&owner.dispatch.core))?;
            return Ok(PmPhaseALiveCancelNetworkDispatchOwnerV1 {
                journals: PmPhaseALiveCancelNetworkJournalV1::RecoveryContinuationV1(self),
                dispatch: owner.dispatch,
                custody: owner.custody,
            });
        }
        self.evidence
            .inner
            .revalidate_phase_a_live_cancel_for_network_dispatch(owner)
    }

    pub fn record_phase_a_live_cancel_result(
        &mut self,
        mut observation: PmPhaseALiveCancelMayHaveBeenDispatchedV1,
        outcome: PmCancelResultKindV1,
    ) -> Result<PmPhaseALiveCancelResultCustodyV1, PmTrialLiveJournalError> {
        if observation.custody.journal_backend
            == PmPhaseALiveCancelJournalBackendV1::RecoveryContinuationV1
        {
            if outcome == PmCancelResultKindV1::DefinitelyNotDispatched {
                return Err(PmTrialLiveJournalError::InvalidTransition);
            }
            self.evidence
                .inner
                .require_phase_a_live_cancel_custody_epoch(&observation.custody)?;
            self.validate_recovery_continuation_durable_set(&mut observation.custody)?;
            observation
                .dispatch
                .core
                .require_runtime(&self.evidence.inner.runtime)?;
            let ack = self
                .continuation
                .as_mut()
                .ok_or(PmTrialLiveJournalError::RecoveryOperationForbidden)?
                .record_cancel_result(
                    &continuation_ack(&observation.dispatch.core),
                    outcome,
                    &observation.dispatch.exact_venue_order_id,
                )?;
            return Ok(PmPhaseALiveCancelResultCustodyV1 {
                result: PmDurableCancelResultAckV1 {
                    core: continuation_core(&self.evidence.inner.runtime, ack),
                    outcome,
                    exact_venue_order_id: observation.dispatch.exact_venue_order_id,
                },
                custody: observation.custody,
            });
        }
        self.evidence
            .inner
            .record_phase_a_live_cancel_result(observation, outcome)
    }

    pub fn record_phase_a_live_cancel_definitely_not_dispatched(
        &mut self,
        mut definitely_not_dispatched: PmPhaseALiveCancelDefinitelyNotDispatchedV1,
    ) -> Result<PmPhaseALiveCancelResultCustodyV1, PmTrialLiveJournalError> {
        if definitely_not_dispatched.custody.journal_backend
            == PmPhaseALiveCancelJournalBackendV1::RecoveryContinuationV1
        {
            self.evidence
                .inner
                .require_phase_a_live_cancel_custody_epoch(&definitely_not_dispatched.custody)?;
            self.validate_recovery_continuation_durable_set(
                &mut definitely_not_dispatched.custody,
            )?;
            definitely_not_dispatched
                .dispatch
                .core
                .require_runtime(&self.evidence.inner.runtime)?;
            let outcome = PmCancelResultKindV1::DefinitelyNotDispatched;
            let ack = self
                .continuation
                .as_mut()
                .ok_or(PmTrialLiveJournalError::RecoveryOperationForbidden)?
                .record_cancel_result(
                    &continuation_ack(&definitely_not_dispatched.dispatch.core),
                    outcome,
                    &definitely_not_dispatched.dispatch.exact_venue_order_id,
                )?;
            return Ok(PmPhaseALiveCancelResultCustodyV1 {
                result: PmDurableCancelResultAckV1 {
                    core: continuation_core(&self.evidence.inner.runtime, ack),
                    outcome,
                    exact_venue_order_id: definitely_not_dispatched.dispatch.exact_venue_order_id,
                },
                custody: definitely_not_dispatched.custody,
            });
        }
        self.evidence
            .inner
            .record_phase_a_live_cancel_definitely_not_dispatched(definitely_not_dispatched)
    }

    pub fn record_phase_a_live_cancel_outcome_bridge_with_custody(
        &mut self,
        mut result: PmPhaseALiveCancelResultCustodyV1,
    ) -> Result<PmPhaseALiveCancelOutcomeCustodyV1, PmTrialLiveJournalError> {
        if result.custody.journal_backend
            == PmPhaseALiveCancelJournalBackendV1::RecoveryContinuationV1
        {
            self.evidence
                .inner
                .require_phase_a_live_cancel_custody_epoch(&result.custody)?;
            self.validate_recovery_continuation_durable_set(&mut result.custody)?;
            result
                .result
                .core
                .require_runtime(&self.evidence.inner.runtime)?;
            let exact_venue_order_id = result.result.exact_venue_order_id.clone();
            let dispatch = result.result.core.link();
            let ack = self
                .continuation
                .as_mut()
                .ok_or(PmTrialLiveJournalError::RecoveryOperationForbidden)?
                .record_cancel_outcome_bridge(
                    &continuation_ack(&result.result.core),
                    result.result.outcome,
                    &exact_venue_order_id,
                )?;
            return Ok(PmPhaseALiveCancelOutcomeCustodyV1 {
                bridge: PmDurableCancelOutcomeBridgeAckV1 {
                    core: continuation_core(&self.evidence.inner.runtime, ack),
                    dispatch,
                },
                exact_venue_order_id,
                custody: result.custody,
            });
        }
        self.evidence
            .inner
            .record_phase_a_live_cancel_outcome_bridge_with_custody(result)
    }

    /// Recovers the exact outcome bridge after a crash with a durable
    /// continuation cancel result. The result facts are read from the bound
    /// journal and are never supplied by the caller.
    pub fn resume_phase_a_live_cancel_outcome_with_custody(
        &mut self,
    ) -> Result<PmPhaseALiveCancelOutcomeCustodyV1, PmTrialLiveJournalError> {
        if self.required_action
            != Some(PmPhaseALiveCancelRecoveryRequiredActionV1::ResumeCancelOutcome)
        {
            return Err(PmTrialLiveJournalError::RecoveryOperationForbidden);
        }
        let mut custody = self
            .custody
            .take()
            .ok_or(PmTrialLiveJournalError::RecoveryOperationForbidden)?;
        if custody.journal_backend != PmPhaseALiveCancelJournalBackendV1::RecoveryContinuationV1 {
            return Err(PmTrialLiveJournalError::RecoveryOperationForbidden);
        }
        self.validate_recovery_continuation_durable_set(&mut custody)?;
        let resumed = self
            .continuation
            .as_mut()
            .ok_or(PmTrialLiveJournalError::RecoveryOperationForbidden)?
            .resume_cancel_outcome_bridge()?;
        let result = continuation_core(&self.evidence.inner.runtime, resumed.result);
        let bridge = PmDurableCancelOutcomeBridgeAckV1 {
            core: continuation_core(&self.evidence.inner.runtime, resumed.bridge),
            dispatch: result.link(),
        };
        self.evidence
            .inner
            .transition_phase_a_live_custody_to_cancel_epoch(&mut custody)?;
        self.evidence.initial_reconciliation_available = false;
        self.required_action = None;
        Ok(PmPhaseALiveCancelOutcomeCustodyV1 {
            bridge,
            exact_venue_order_id: resumed.exact_venue_order_id,
            custody,
        })
    }

    pub fn record_phase_a_live_cancel_reconciliation_with_custody(
        &mut self,
        mut cancel: PmPhaseALiveCancelOutcomeCustodyV1,
        observed_at_utc: String,
        state: PmReconciliationOrderStateV1,
        exact_venue_order_id: Option<String>,
    ) -> Result<PmPhaseALiveReconciliationCustodyV1, PmTrialLiveJournalError> {
        if cancel.custody.journal_backend
            == PmPhaseALiveCancelJournalBackendV1::RecoveryContinuationV1
        {
            self.evidence
                .inner
                .require_phase_a_live_cancel_custody_epoch(&cancel.custody)?;
            self.validate_recovery_continuation_durable_set(&mut cancel.custody)?;
            cancel
                .bridge
                .core
                .require_runtime(&self.evidence.inner.runtime)?;
            if matches!(
                state,
                PmReconciliationOrderStateV1::ExactLive
                    | PmReconciliationOrderStateV1::ExactCanceled
                    | PmReconciliationOrderStateV1::ExactFilled
            ) && exact_venue_order_id.as_deref() != Some(cancel.exact_venue_order_id.as_str())
            {
                return Err(PmTrialLiveJournalError::InvalidBinding);
            }
            let continuation = self
                .continuation
                .as_mut()
                .ok_or(PmTrialLiveJournalError::RecoveryOperationForbidden)?;
            let result_ack = ContinuationAckV1 {
                sequence: cancel.bridge.dispatch.sequence,
                record_fingerprint: cancel.bridge.dispatch.record_fingerprint.clone(),
            };
            let dispatch_target = continuation.target_for_dispatch_ack(&result_ack)?;
            let ack = continuation.record_reconciliation(
                dispatch_target.clone(),
                observed_at_utc,
                state,
                exact_venue_order_id.clone(),
            )?;
            let reconciliation = PmDurableReconciliationAckV1 {
                core: continuation_core(&self.evidence.inner.runtime, ack),
            };
            let owned = if state == PmReconciliationOrderStateV1::ExactLive {
                Some(PmJournalOwnedVenueOrderV1 {
                    runtime: Arc::clone(&self.evidence.inner.runtime),
                    exact_venue_order_id: exact_venue_order_id
                        .ok_or(PmTrialLiveJournalError::InvalidTransition)?,
                })
            } else {
                None
            };
            return self.finish_recovery_continuation_reconciliation(
                reconciliation,
                dispatch_target,
                state,
                owned,
                cancel.custody,
            );
        }
        self.evidence
            .inner
            .record_phase_a_live_cancel_reconciliation_with_custody(
                cancel,
                observed_at_utc,
                state,
                exact_venue_order_id,
            )
    }

    pub fn record_phase_a_live_reconciliation_again_with_custody(
        &mut self,
        mut previous: PmPhaseALiveReconciliationCustodyV1,
        observed_at_utc: String,
        state: PmReconciliationOrderStateV1,
        exact_venue_order_id: Option<String>,
    ) -> Result<PmPhaseALiveReconciliationCustodyV1, PmTrialLiveJournalError> {
        if matches!(
            &previous.dispatch_target,
            PmPhaseALiveReconciliationTargetV1::RecoveryContinuationV1(_)
        ) {
            let mut custody = previous
                .custody
                .take()
                .ok_or(PmTrialLiveJournalError::InvalidTransition)?;
            self.validate_recovery_continuation_durable_set(&mut custody)?;
            previous
                .reconciliation
                .core
                .require_runtime(&self.evidence.inner.runtime)?;
            let continuation = self
                .continuation
                .as_mut()
                .ok_or(PmTrialLiveJournalError::RecoveryOperationForbidden)?;
            continuation.validate_intent_ack(&continuation_ack(&previous.reconciliation.core))?;
            let dispatch_target = continuation.current_reconciliation_target()?;
            let ack = continuation.record_reconciliation(
                dispatch_target.clone(),
                observed_at_utc,
                state,
                exact_venue_order_id.clone(),
            )?;
            let reconciliation = PmDurableReconciliationAckV1 {
                core: continuation_core(&self.evidence.inner.runtime, ack),
            };
            let owned = if state == PmReconciliationOrderStateV1::ExactLive {
                Some(PmJournalOwnedVenueOrderV1 {
                    runtime: Arc::clone(&self.evidence.inner.runtime),
                    exact_venue_order_id: exact_venue_order_id
                        .ok_or(PmTrialLiveJournalError::InvalidTransition)?,
                })
            } else {
                None
            };
            return self.finish_recovery_continuation_reconciliation(
                reconciliation,
                dispatch_target,
                state,
                owned,
                custody,
            );
        }
        self.evidence
            .inner
            .record_phase_a_live_reconciliation_again_with_custody(
                previous,
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
        if self.continuation.is_some() {
            if matches!(
                self.required_action,
                Some(
                    PmPhaseALiveCancelRecoveryRequiredActionV1::ReconcileCurrentExposure
                        | PmPhaseALiveCancelRecoveryRequiredActionV1::ResumeCancelOutcome
                        | PmPhaseALiveCancelRecoveryRequiredActionV1::CompletePendingTerminal
                        | PmPhaseALiveCancelRecoveryRequiredActionV1::TerminalEvidenceOnly
                )
            ) {
                return Err(PmTrialLiveJournalError::RecoveryOperationForbidden);
            }
            if !self
                .continuation
                .as_ref()
                .ok_or(PmTrialLiveJournalError::RecoveryOperationForbidden)?
                .terminal_safe()?
            {
                return Err(PmTrialLiveJournalError::InvalidTransition);
            }
            let mut custody = self
                .custody
                .take()
                .ok_or(PmTrialLiveJournalError::RecoveryOperationForbidden)?;
            self.validate_recovery_continuation_durable_set(&mut custody)?;
            let ack = self
                .continuation
                .as_mut()
                .ok_or(PmTrialLiveJournalError::RecoveryOperationForbidden)?
                .record_terminal(&mut custody.authorization, terminal_at_utc, disposition)?;
            self.evidence
                .inner
                .complete_phase_a_live_custody_epoch(&custody)?;
            return Ok(PmDurableIntentTerminalAckV1 {
                core: continuation_core(&self.evidence.inner.runtime, ack),
            });
        }
        self.evidence
            .inner
            .record_terminal(terminal_at_utc, disposition)
    }

    /// Compatibility-denied seam. Recovery now source-completes the exact
    /// ledger-anchored Terminal plan inside `open_phase_a_live_cancel` before
    /// exposing this wrapper, so no caller-triggered completion remains.
    pub fn complete_pending_terminal(
        &mut self,
    ) -> Result<PmDurableIntentTerminalAckV1, PmTrialLiveJournalError> {
        Err(PmTrialLiveJournalError::RecoveryOperationForbidden)
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

fn next_recovery_ordinal(
    dispatch: &[DispatchLineV1],
    recovery_cancel_dispatch_budget: u8,
) -> Result<u8, PmTrialLiveJournalError> {
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
    let next = highest
        .checked_add(1)
        .ok_or(PmTrialLiveJournalError::BoundExceeded)?;
    if next == 0 || next > recovery_cancel_dispatch_budget {
        return Err(PmTrialLiveJournalError::BoundExceeded);
    }
    Ok(next)
}
