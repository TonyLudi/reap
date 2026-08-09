mod credential_custody;

use std::{
    fmt,
    path::PathBuf,
    sync::{
        Arc, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use async_trait::async_trait;
use reap_pm_controlled_trial_live::{
    PmCancelDispatchClassV1, PmDurablePlacePreparedAckV1,
    PmRevalidatedPhaseALiveCancelDispatchOwnerV1,
};
use reap_pm_core::PmConditionId;
use reap_polymarket_auth::{
    AuthenticatedL2Headers, AuthenticatedOwnedCancelRequest, AuthenticatedPlaceRequest,
    AuthenticatedUserSubscription, CredentialOwnedUserFrame, EoaAddress, FixedEoaSigner,
    L2Credentials, L2Timestamp, PlacePublicRequestIdentity, PmAuthError, PmClobDomain,
    SerializedPlaceRequest, derive_place_public_request_identity,
};
use reap_polymarket_live_adapter::{
    PmCancelMutationTimeFinalizer, PmCancelMutationTimeProof, PmHttpReadAuthorityProvider,
    PmLiveAdapterError, PmPlaceMutationAuthenticationError, PmPlaceMutationTimeFinalizer,
    PmPlaceMutationTimeProof, PmUserWsReadAuthorityProvider,
};
use reap_polymarket_wire::{
    PmClobV2SignatureType, PmLiveOpenOrderPage, PmLiveOrder, PmLiveTradePage, PmLiveUserFrame,
};
use thiserror::Error;
use tokio::{
    sync::{mpsc, oneshot},
    task::JoinHandle,
};

use self::credential_custody::{
    FreshPlaceCredentialFiles, FreshPlaceCredentialHandoff, FreshPlaceCredentialTeardown,
    RecoveryOnlyCredentialFiles, RecoveryOnlyCredentialHandoff, RecoveryOnlyCredentialTeardown,
};
use super::{
    AuthenticatedExactOwnedOrderRead, AuthorizedL2Timestamp,
    SealedExactOwnedOrderReadAuthentication,
};

const COMMON_AUTHORITY_CAPACITY: usize = 8;
const PLACE_AUTHORITY_CAPACITY: usize = 1;
const CANCEL_AUTHORITY_CAPACITY: usize = 1;
const MAX_EXACT_CANCEL_AUTHENTICATIONS_PER_AUTHORITY: u8 = 3;
const MAX_JOIN_TIMEOUT: Duration = Duration::from_secs(60);

#[cfg(test)]
struct PlaceRequestTestPause {
    admitted: std::sync::mpsc::SyncSender<()>,
    release: std::sync::mpsc::Receiver<()>,
}

#[cfg(test)]
impl PlaceRequestTestPause {
    fn wait(self) {
        let _ = self.admitted.send(());
        let _ = self.release.recv();
    }
}

#[cfg(test)]
struct CancelRequestTestPause {
    admitted: std::sync::mpsc::SyncSender<()>,
    release: std::sync::mpsc::Receiver<()>,
}

#[cfg(test)]
impl CancelRequestTestPause {
    fn wait(self) {
        let _ = self.admitted.send(());
        let _ = self.release.recv();
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub(super) enum CredentialAuthorityError {
    #[error("credential authority requires an active Tokio runtime")]
    ActiveRuntimeRequired,
    #[error("credential authority request channel is saturated")]
    AuthoritySaturated,
    #[error("credential authority request channel is closed")]
    AuthorityClosed,
    #[error("the sole fresh-place signing attempt was already consumed")]
    PlaceAlreadyConsumed,
    #[error("the later place dispatch authorization does not match the retained prepared identity")]
    PlaceDispatchBindingMismatch,
    #[error("no successfully retained place identity is available for staged key removal")]
    PlacePreparedIdentityUnavailable,
    #[error("the staged key removal identity does not match the retained prepared place")]
    PlacePreparedIdentityMismatch,
    #[error("fresh-place authentication requires the closed PM-T2 proxy signature profile")]
    PlaceProfileMismatch,
    #[error("fresh-place public identity does not match the sealed unsigned order and domain")]
    PlacePublicIdentityMismatch,
    #[error(transparent)]
    PlaceAuthentication(#[from] PmPlaceMutationAuthenticationError),
    #[error("the L2 signer does not match the sealed proxy order signer")]
    PlaceSignerMismatch,
    #[error("the authenticated response owner does not match the sole L2 bundle")]
    CredentialOwnerMismatch,
    #[error("the task-local EOA signer has not yet been destroyed")]
    SignerStillOwned,
    #[error("protected credential custody could not be loaded")]
    CredentialCustodyLoadFailed,
    #[error("the staged credential teardown could not be completed safely")]
    StagedCredentialTeardownFailed,
    #[error("credential authority shutdown bounds must each be within 1ns..=60s")]
    InvalidShutdownBounds,
    #[error(transparent)]
    Auth(#[from] PmAuthError),
}

/// Fixed finite waits for graceful join and the join after forced abort.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct CredentialAuthorityShutdownBounds {
    graceful_join: Duration,
    abort_join: Duration,
}

impl CredentialAuthorityShutdownBounds {
    pub(super) fn new(
        graceful_join: Duration,
        abort_join: Duration,
    ) -> Result<Self, CredentialAuthorityError> {
        if graceful_join.is_zero()
            || abort_join.is_zero()
            || graceful_join > MAX_JOIN_TIMEOUT
            || abort_join > MAX_JOIN_TIMEOUT
        {
            return Err(CredentialAuthorityError::InvalidShutdownBounds);
        }
        Ok(Self {
            graceful_join,
            abort_join,
        })
    }

    const fn fixed(graceful_join: Duration, abort_join: Duration) -> Self {
        Self {
            graceful_join,
            abort_join,
        }
    }
}

pub(super) const DEFAULT_AUTHORITY_SHUTDOWN_BOUNDS: CredentialAuthorityShutdownBounds =
    CredentialAuthorityShutdownBounds::fixed(Duration::from_secs(30), Duration::from_secs(5));

/// Truthful terminal result after the task was joined and the L2 staged files
/// were durably removed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct CredentialAuthorityShutdownOutcome {
    shutdown_requested: bool,
    abort_requested: bool,
    task_joined: bool,
    task_completed_cleanly: bool,
    credentials_dropped: bool,
    staged_l2_files_removed: bool,
}

impl CredentialAuthorityShutdownOutcome {
    pub(super) const fn shutdown_requested(self) -> bool {
        self.shutdown_requested
    }

    pub(super) const fn abort_requested(self) -> bool {
        self.abort_requested
    }

    pub(super) const fn task_joined(self) -> bool {
        self.task_joined
    }

    pub(super) const fn task_completed_cleanly(self) -> bool {
        self.task_completed_cleanly
    }

    pub(super) const fn credentials_dropped(self) -> bool {
        self.credentials_dropped
    }

    pub(super) const fn staged_l2_files_removed(self) -> bool {
        self.staged_l2_files_removed
    }
}

struct AuthorityIdentity {
    signer_dropped: AtomicBool,
    prepared_public_identity: OnceLock<PlacePublicRequestIdentity>,
}

impl AuthorityIdentity {
    fn new() -> Self {
        Self {
            signer_dropped: AtomicBool::new(false),
            prepared_public_identity: OnceLock::new(),
        }
    }

    fn signer_dropped(&self) -> bool {
        self.signer_dropped.load(Ordering::Acquire)
    }

    fn publish_prepared_public_identity(&self, identity: PlacePublicRequestIdentity) {
        if self.prepared_public_identity.set(identity).is_err() {
            std::process::abort();
        }
    }

    fn prepared_public_identity(&self) -> Option<PlacePublicRequestIdentity> {
        self.prepared_public_identity.get().copied()
    }
}

struct TaskSignerCustody {
    signer: Option<FixedEoaSigner>,
    identity: Arc<AuthorityIdentity>,
}

impl TaskSignerCustody {
    fn new(signer: FixedEoaSigner, identity: Arc<AuthorityIdentity>) -> Self {
        Self {
            signer: Some(signer),
            identity,
        }
    }

    fn take(&mut self) -> Option<FixedEoaSigner> {
        self.signer.take()
    }

    fn publish_signer_dropped(&self) {
        self.identity.signer_dropped.store(true, Ordering::Release);
    }

    fn publish_prepared_public_identity(&self, identity: PlacePublicRequestIdentity) {
        self.identity.publish_prepared_public_identity(identity);
    }
}

impl Drop for TaskSignerCustody {
    fn drop(&mut self) {
        // Publish only after the task-local signer value is destroyed.
        drop(self.signer.take());
        self.publish_signer_dropped();
    }
}

/// Fresh-mode owner of the four-file custody handoff. Spawning transfers the
/// signer and sole L2 bundle into one supervised task.
pub(super) struct FreshCredentialAuthorityOwner {
    custody: FreshPlaceCredentialHandoff,
}

impl FreshCredentialAuthorityOwner {
    /// Load the exact protected four-file custody directly into this sealed
    /// authority owner. No raw signer, L2 bundle, handoff, or teardown value is
    /// exposed to the caller.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn load_from_protected_files(
        directory: PathBuf,
        private_key_entry: String,
        api_key_entry: String,
        l2_secret_entry: String,
        passphrase_entry: String,
        configured_signer: EoaAddress,
    ) -> Result<Self, CredentialAuthorityError> {
        let custody = FreshPlaceCredentialFiles::new(
            directory,
            private_key_entry,
            api_key_entry,
            l2_secret_entry,
            passphrase_entry,
        )
        .load(configured_signer)
        .map_err(|_| CredentialAuthorityError::CredentialCustodyLoadFailed)?;
        Ok(Self::from_custody(custody))
    }

    const fn from_custody(custody: FreshPlaceCredentialHandoff) -> Self {
        Self { custody }
    }

    pub(super) fn spawn_with_mutation_time_finalizers(
        self,
        place_time_finalizer: PmPlaceMutationTimeFinalizer,
        cancel_time_finalizer: PmCancelMutationTimeFinalizer,
    ) -> Result<FreshCredentialAuthorityRoles, CredentialAuthorityError> {
        let runtime = tokio::runtime::Handle::try_current()
            .map_err(|_| CredentialAuthorityError::ActiveRuntimeRequired)?;
        let (signer, credentials, teardown) = self.custody.into_authorities_and_teardown();
        let identity = Arc::new(AuthorityIdentity::new());
        let (common_sender, common_receiver) = mpsc::channel(COMMON_AUTHORITY_CAPACITY);
        let (place_sender, place_receiver) = mpsc::channel(PLACE_AUTHORITY_CAPACITY);
        let (cancel_sender, cancel_receiver) = mpsc::channel(CANCEL_AUTHORITY_CAPACITY);
        let (shutdown, shutdown_receiver) = oneshot::channel();
        let task = runtime.spawn(run_fresh_authority(
            TaskSignerCustody::new(signer, Arc::clone(&identity)),
            credentials,
            FreshAuthorityTaskInputs {
                place_time_finalizer,
                cancel_time_finalizer,
                common: common_receiver,
                place: place_receiver,
                cancel: cancel_receiver,
                shutdown: shutdown_receiver,
            },
        ));
        let task = ArmedTaskSupervisor::new(shutdown, task);

        Ok(FreshCredentialAuthorityRoles {
            place: FreshPlaceAuthenticationOnce {
                sender: place_sender,
            },
            cancel: ExactOwnedCancelAuthenticationRole::new(cancel_sender),
            http: FixedHttpAuthenticationRole {
                sender: common_sender.clone(),
            },
            user_ws: FixedUserWsAuthenticationRole {
                sender: common_sender,
            },
            supervisor: FreshCredentialAuthoritySupervisor {
                task,
                teardown,
                identity,
                private_key_removed: false,
            },
        })
    }

    #[cfg(test)]
    fn spawn(
        self,
        place_time_finalizer: PmPlaceMutationTimeFinalizer,
    ) -> Result<FreshCredentialAuthorityRoles, CredentialAuthorityError> {
        self.spawn_with_mutation_time_finalizers(
            place_time_finalizer,
            tests::unused_cancel_time_finalizer(),
        )
    }
}

impl fmt::Debug for FreshCredentialAuthorityOwner {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("FreshCredentialAuthorityOwner([REDACTED])")
    }
}

/// Recovery-mode owner of the three-file L2 custody handoff. Its spawn path
/// creates no signer value, place channel, or place role.
pub(super) struct RecoveryCredentialAuthorityOwner {
    custody: RecoveryOnlyCredentialHandoff,
}

impl RecoveryCredentialAuthorityOwner {
    /// Load the exact protected three-file recovery custody directly into this
    /// sealed authority owner. This path cannot accept or produce a signer or
    /// fresh-place capability.
    pub(super) fn load_from_protected_files(
        directory: PathBuf,
        api_key_entry: String,
        l2_secret_entry: String,
        passphrase_entry: String,
        configured_signer: EoaAddress,
    ) -> Result<Self, CredentialAuthorityError> {
        let custody = RecoveryOnlyCredentialFiles::new(
            directory,
            api_key_entry,
            l2_secret_entry,
            passphrase_entry,
        )
        .load(configured_signer)
        .map_err(|_| CredentialAuthorityError::CredentialCustodyLoadFailed)?;
        Ok(Self::from_custody(custody))
    }

    const fn from_custody(custody: RecoveryOnlyCredentialHandoff) -> Self {
        Self { custody }
    }

    pub(super) fn spawn_with_cancel_time_finalizer(
        self,
        cancel_time_finalizer: PmCancelMutationTimeFinalizer,
    ) -> Result<RecoveryCredentialAuthorityRoles, CredentialAuthorityError> {
        let runtime = tokio::runtime::Handle::try_current()
            .map_err(|_| CredentialAuthorityError::ActiveRuntimeRequired)?;
        let (credentials, teardown) = self.custody.into_authority_and_teardown();
        let (common_sender, common_receiver) = mpsc::channel(COMMON_AUTHORITY_CAPACITY);
        let (cancel_sender, cancel_receiver) = mpsc::channel(CANCEL_AUTHORITY_CAPACITY);
        let (shutdown, shutdown_receiver) = oneshot::channel();
        let task = runtime.spawn(run_recovery_authority(
            credentials,
            cancel_time_finalizer,
            common_receiver,
            cancel_receiver,
            shutdown_receiver,
        ));
        let task = ArmedTaskSupervisor::new(shutdown, task);

        Ok(RecoveryCredentialAuthorityRoles {
            cancel: ExactOwnedCancelAuthenticationRole::new(cancel_sender),
            http: FixedHttpAuthenticationRole {
                sender: common_sender.clone(),
            },
            user_ws: FixedUserWsAuthenticationRole {
                sender: common_sender,
            },
            supervisor: RecoveryCredentialAuthoritySupervisor { task, teardown },
        })
    }

    #[cfg(test)]
    pub(super) fn spawn(
        self,
    ) -> Result<RecoveryCredentialAuthorityRoles, CredentialAuthorityError> {
        self.spawn_with_cancel_time_finalizer(tests::unused_cancel_time_finalizer())
    }
}

impl fmt::Debug for RecoveryCredentialAuthorityOwner {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("RecoveryCredentialAuthorityOwner([REDACTED])")
    }
}

#[must_use = "all roles and the armed supervisor must remain owned"]
pub(super) struct FreshCredentialAuthorityRoles {
    place: FreshPlaceAuthenticationOnce,
    cancel: ExactOwnedCancelAuthenticationRole,
    http: FixedHttpAuthenticationRole,
    user_ws: FixedUserWsAuthenticationRole,
    supervisor: FreshCredentialAuthoritySupervisor,
}

impl FreshCredentialAuthorityRoles {
    /// Consume the unsplit authority bundle at the runner-private runtime
    /// assembly boundary. Production composition must immediately place the
    /// HTTP and user-WS handles into one fixed connectivity owner; this is not
    /// a general credential-role accessor.
    pub(super) fn into_private_read_runtime_parts(
        self,
    ) -> (
        FreshPlaceAuthenticationOnce,
        ExactOwnedCancelAuthenticationRole,
        FixedHttpAuthenticationRole,
        FixedUserWsAuthenticationRole,
        FreshCredentialAuthoritySupervisor,
    ) {
        (
            self.place,
            self.cancel,
            self.http,
            self.user_ws,
            self.supervisor,
        )
    }

    #[cfg(test)]
    pub(super) fn into_roles(
        self,
    ) -> (
        FreshPlaceAuthenticationOnce,
        ExactOwnedCancelAuthenticationRole,
        FixedHttpAuthenticationRole,
        FixedUserWsAuthenticationRole,
        FreshCredentialAuthoritySupervisor,
    ) {
        (
            self.place,
            self.cancel,
            self.http,
            self.user_ws,
            self.supervisor,
        )
    }
}

impl fmt::Debug for FreshCredentialAuthorityRoles {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("FreshCredentialAuthorityRoles([REDACTED])")
    }
}

// BEGIN RECOVERY_ROLE_SURFACE
#[must_use = "all roles and the armed supervisor must remain owned"]
pub(super) struct RecoveryCredentialAuthorityRoles {
    cancel: ExactOwnedCancelAuthenticationRole,
    http: FixedHttpAuthenticationRole,
    user_ws: FixedUserWsAuthenticationRole,
    supervisor: RecoveryCredentialAuthoritySupervisor,
}

impl RecoveryCredentialAuthorityRoles {
    /// Recovery consumes the complete L2-only bundle at the same fixed
    /// read-runtime assembly boundary. No signer or place type exists here.
    pub(super) fn into_private_read_runtime_parts(
        self,
    ) -> (
        ExactOwnedCancelAuthenticationRole,
        FixedHttpAuthenticationRole,
        FixedUserWsAuthenticationRole,
        RecoveryCredentialAuthoritySupervisor,
    ) {
        (self.cancel, self.http, self.user_ws, self.supervisor)
    }

    #[cfg(test)]
    pub(super) fn into_roles(
        self,
    ) -> (
        ExactOwnedCancelAuthenticationRole,
        FixedHttpAuthenticationRole,
        FixedUserWsAuthenticationRole,
        RecoveryCredentialAuthoritySupervisor,
    ) {
        (self.cancel, self.http, self.user_ws, self.supervisor)
    }
}

impl fmt::Debug for RecoveryCredentialAuthorityRoles {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("RecoveryCredentialAuthorityRoles([REDACTED])")
    }
}
// END RECOVERY_ROLE_SURFACE

/// The only fresh-mode place handle. The method consumes the handle, while
/// the task independently consumes its `Option<FixedEoaSigner>` as defense
/// in depth against an accidentally duplicated sender.
pub(super) struct FreshPlaceAuthenticationOnce {
    sender: mpsc::Sender<PlaceAuthorityRequest>,
}

impl FreshPlaceAuthenticationOnce {
    /// Consume the sole public preparation handle. The authority validates the
    /// closed PM-T2 proxy profile, consumes and destroys its only EOA signer,
    /// signs and serializes exactly once, and retains the serialized request
    /// inside the supervised L2 task. No L2 HMAC is computed at this stage.
    pub(super) async fn prepare_place_once(
        self,
        request: SealedPmT2ProxyPlacePreparation,
    ) -> Result<SignerDroppedPlacePreparation, CredentialAuthorityError> {
        let sender = self.sender;
        let public_identity = request_place(&sender, |response| PlaceAuthorityRequest::Prepare {
            request,
            response,
        })
        .await?;
        Ok(SignerDroppedPlacePreparation {
            public_identity,
            sender,
        })
    }

    #[cfg(test)]
    fn duplicate_for_task_gate(&self) -> Self {
        Self {
            sender: self.sender.clone(),
        }
    }
}

impl fmt::Debug for FreshPlaceAuthenticationOnce {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("FreshPlaceAuthenticationOnce(<opaque>)")
    }
}

/// Public, move-only inputs for the sole fixed PM-T2 proxy preparation. The
/// expected identity is independently derived by the non-secret session owner
/// and re-derived by the signer-owning task before signing.
pub(super) struct SealedPmT2ProxyPlacePreparation {
    domain: PmClobDomain,
    unsigned_order: reap_polymarket_wire::PmUnsignedClobV2Order,
    expected_public_identity: PlacePublicRequestIdentity,
    #[cfg(test)]
    test_pause: Option<PlaceRequestTestPause>,
}

impl SealedPmT2ProxyPlacePreparation {
    pub(super) const fn new(
        domain: PmClobDomain,
        unsigned_order: reap_polymarket_wire::PmUnsignedClobV2Order,
        expected_public_identity: PlacePublicRequestIdentity,
    ) -> Self {
        Self {
            domain,
            unsigned_order,
            expected_public_identity,
            #[cfg(test)]
            test_pause: None,
        }
    }

    #[cfg(test)]
    fn with_test_pause(mut self, pause: PlaceRequestTestPause) -> Self {
        self.test_pause = Some(pause);
        self
    }

    fn wait_for_test_pause(&mut self) {
        #[cfg(test)]
        if let Some(pause) = self.test_pause.take() {
            pause.wait();
        }
    }
}

impl fmt::Debug for SealedPmT2ProxyPlacePreparation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SealedPmT2ProxyPlacePreparation(<public; opaque>)")
    }
}

/// The only continuation produced by successful preparation. It exposes the
/// public journal identity and nothing from the signed body or L2 bundle.
/// Production intentionally has no final-HMAC method until a sealed,
/// runner-private online permit exists.
#[must_use = "the signer-dropped prepared place must be durably joined or shut down"]
pub(super) struct SignerDroppedPlacePreparation {
    public_identity: PlacePublicRequestIdentity,
    sender: mpsc::Sender<PlaceAuthorityRequest>,
}

impl SignerDroppedPlacePreparation {
    pub(super) const fn public_identity(&self) -> PlacePublicRequestIdentity {
        self.public_identity
    }
}

// BEGIN TEST_ONLY_PLACE_HMAC_ADMISSION
#[cfg(test)]
impl SignerDroppedPlacePreparation {
    async fn finalize_with_admission(
        self,
        authorization: PlaceHmacAdmission,
    ) -> Result<OpaqueAuthenticatedPlaceRequest, CredentialAuthorityError> {
        request_place(&self.sender, |response| PlaceAuthorityRequest::Finalize {
            authorization,
            response,
        })
        .await
    }

    async fn finalize_place_once_for_test(
        self,
        public_identity: PlacePublicRequestIdentity,
        expected_l2_timestamp_seconds: u64,
        proof: PmPlaceMutationTimeProof,
    ) -> Result<OpaqueAuthenticatedPlaceRequest, CredentialAuthorityError> {
        self.finalize_with_admission(PlaceHmacAdmission {
            public_identity,
            expected_l2_timestamp_seconds,
            proof,
            test_pause: None,
        })
        .await
    }

    async fn finalize_place_once_paused_for_test(
        self,
        public_identity: PlacePublicRequestIdentity,
        expected_l2_timestamp_seconds: u64,
        proof: PmPlaceMutationTimeProof,
        test_pause: PlaceRequestTestPause,
    ) -> Result<OpaqueAuthenticatedPlaceRequest, CredentialAuthorityError> {
        self.finalize_with_admission(PlaceHmacAdmission {
            public_identity,
            expected_l2_timestamp_seconds,
            proof,
            test_pause: Some(test_pause),
        })
        .await
    }
}
// END TEST_ONLY_PLACE_HMAC_ADMISSION

impl fmt::Debug for SignerDroppedPlacePreparation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SignerDroppedPlacePreparation")
            .field("public_identity", &self.public_identity)
            .field("signer_dropped", &true)
            .finish_non_exhaustive()
    }
}

/// Task-bound join of an eventual sealed online permit and the source-issued
/// place-time proof. Production currently has no constructor or admission
/// method; only the explicitly test-gated scalar harness can construct it.
struct PlaceHmacAdmission {
    public_identity: PlacePublicRequestIdentity,
    expected_l2_timestamp_seconds: u64,
    proof: PmPlaceMutationTimeProof,
    #[cfg(test)]
    test_pause: Option<PlaceRequestTestPause>,
}

impl PlaceHmacAdmission {
    fn wait_for_test_pause(&mut self) {
        #[cfg(test)]
        if let Some(pause) = self.test_pause.take() {
            pause.wait();
        }
    }
}

impl fmt::Debug for PlaceHmacAdmission {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PlaceHmacAdmission(<opaque>)")
    }
}

/// Final L2-authenticated place request. The signed body, exact runtime body
/// commitment, credentials, sender, and admitted timestamp remain sealed.
/// This slice intentionally provides no production dispatch/decomposition
/// method, so it does not enable transport.
#[must_use = "the authenticated place remains a linear mutation authority"]
pub(super) struct OpaqueAuthenticatedPlaceRequest {
    request: AuthenticatedPlaceRequest,
}

impl fmt::Debug for OpaqueAuthenticatedPlaceRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("OpaqueAuthenticatedPlaceRequest([REDACTED])")
    }
}

/// Locally bounded exact-owned cancel authenticator. Durable primary/recovery
/// budgets remain an upper journal responsibility; this independent ceiling
/// prevents an unbounded in-process caller loop.
pub(super) struct ExactOwnedCancelAuthenticationRole {
    sender: mpsc::Sender<CancelAuthorityRequest>,
    remaining_attempts: u8,
}

impl ExactOwnedCancelAuthenticationRole {
    fn new(sender: mpsc::Sender<CancelAuthorityRequest>) -> Self {
        Self {
            sender,
            remaining_attempts: MAX_EXACT_CANCEL_AUTHENTICATIONS_PER_AUTHORITY,
        }
    }

    /// Consume one positive A3 cancel owner and one cancel-purpose source proof.
    /// The owner can leave the credential task only sealed with the exact
    /// authenticated request, or unchanged inside a typed proven pre-send
    /// failure. Once admitted, cancellation or a lost reply is process-fatal.
    pub(super) async fn authenticate_exact_owned_cancel(
        &mut self,
        owner: PmRevalidatedPhaseALiveCancelDispatchOwnerV1,
        proof: PmCancelMutationTimeProof,
    ) -> Result<OpaqueAuthenticatedExactOwnedCancel, CancelAuthenticationPreSendFailure> {
        if self.remaining_attempts == 0 {
            return Err(CancelAuthenticationPreSendFailure::new(
                owner,
                CancelAuthenticationPreSendFailureKind::BudgetExhausted,
            ));
        }
        self.remaining_attempts -= 1;
        request_cancel(
            &self.sender,
            CancelHmacAdmission {
                owner,
                proof,
                #[cfg(test)]
                test_pause: None,
            },
        )
        .await
    }

    #[cfg(test)]
    async fn admit_pause_for_cancellation_test(
        &mut self,
        pause: CancelRequestTestPause,
    ) -> Result<(), CredentialAuthorityError> {
        request_cancel_pause(&self.sender, pause).await
    }
}

impl fmt::Debug for ExactOwnedCancelAuthenticationRole {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ExactOwnedCancelAuthenticationRole(<opaque>)")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum CancelAuthenticationPreSendFailureKind {
    BudgetExhausted,
    AuthoritySaturated,
    AuthorityClosed,
    DispatchClassMismatch,
    RequestBindingMismatch,
    AuthenticationFailed,
}

/// Closed failure retaining the unchanged positive A3 owner. It contains no
/// request bytes, timestamp capability, signature, credentials, or transport.
#[must_use = "the positive cancel owner must be recovered or terminalized"]
pub(super) struct CancelAuthenticationPreSendFailure {
    owner: Box<PmRevalidatedPhaseALiveCancelDispatchOwnerV1>,
    kind: CancelAuthenticationPreSendFailureKind,
}

impl CancelAuthenticationPreSendFailure {
    fn new(
        owner: PmRevalidatedPhaseALiveCancelDispatchOwnerV1,
        kind: CancelAuthenticationPreSendFailureKind,
    ) -> Self {
        Self {
            owner: Box::new(owner),
            kind,
        }
    }

    #[must_use]
    pub(super) const fn kind(&self) -> CancelAuthenticationPreSendFailureKind {
        self.kind
    }

    #[must_use]
    pub(super) fn into_owner(self) -> PmRevalidatedPhaseALiveCancelDispatchOwnerV1 {
        *self.owner
    }
}

impl fmt::Debug for CancelAuthenticationPreSendFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CancelAuthenticationPreSendFailure")
            .field("kind", &self.kind)
            .field("positive_owner_retained", &true)
            .finish()
    }
}

struct CancelHmacAdmission {
    owner: PmRevalidatedPhaseALiveCancelDispatchOwnerV1,
    proof: PmCancelMutationTimeProof,
    #[cfg(test)]
    test_pause: Option<CancelRequestTestPause>,
}

impl CancelHmacAdmission {
    fn wait_for_test_pause(&mut self) {
        #[cfg(test)]
        if let Some(pause) = self.test_pause.take() {
            pause.wait();
        }
    }
}

impl fmt::Debug for CancelHmacAdmission {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("CancelHmacAdmission(<opaque-positive-owner-and-time-proof>)")
    }
}

/// Final exact-owned cancel HMAC inseparably sealed with the positive A3
/// owner. No production getter, decomposition, dispatch, or transport method
/// exists in this slice.
#[must_use = "the authenticated positive cancel remains a linear authority"]
pub(super) struct OpaqueAuthenticatedExactOwnedCancel {
    request: AuthenticatedOwnedCancelRequest,
    owner: PmRevalidatedPhaseALiveCancelDispatchOwnerV1,
}

impl fmt::Debug for OpaqueAuthenticatedExactOwnedCancel {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("OpaqueAuthenticatedExactOwnedCancel([REDACTED])")
    }
}

/// Fixed authenticated HTTP reads and complete parsed-response bindings.
/// This handle is independent from user-WS work while both share the sole L2
/// credential task.
pub(super) struct FixedHttpAuthenticationRole {
    sender: mpsc::Sender<CommonAuthorityRequest>,
}

impl FixedHttpAuthenticationRole {
    pub(super) async fn authenticate_open_orders(
        &mut self,
        timestamp: AuthorizedL2Timestamp,
    ) -> Result<AuthenticatedL2Headers, CredentialAuthorityError> {
        request_common(&self.sender, |response| {
            CommonAuthorityRequest::OpenOrders {
                timestamp: timestamp.into_inner(),
                response,
            }
        })
        .await
    }

    pub(super) async fn authenticate_trades(
        &mut self,
        timestamp: AuthorizedL2Timestamp,
    ) -> Result<AuthenticatedL2Headers, CredentialAuthorityError> {
        request_common(&self.sender, |response| CommonAuthorityRequest::Trades {
            timestamp: timestamp.into_inner(),
            response,
        })
        .await
    }

    pub(super) async fn authenticate_balance_allowance(
        &mut self,
        timestamp: AuthorizedL2Timestamp,
    ) -> Result<AuthenticatedL2Headers, CredentialAuthorityError> {
        request_common(&self.sender, |response| {
            CommonAuthorityRequest::BalanceAllowance {
                timestamp: timestamp.into_inner(),
                response,
            }
        })
        .await
    }

    pub(super) async fn authenticate_closed_only(
        &mut self,
        timestamp: AuthorizedL2Timestamp,
    ) -> Result<AuthenticatedL2Headers, CredentialAuthorityError> {
        request_common(&self.sender, |response| {
            CommonAuthorityRequest::ClosedOnly {
                timestamp: timestamp.into_inner(),
                response,
            }
        })
        .await
    }

    pub(super) async fn authenticate_exact_owned_order(
        &mut self,
        request: SealedExactOwnedOrderReadAuthentication,
    ) -> Result<AuthenticatedExactOwnedOrderRead, CredentialAuthorityError> {
        request_common(&self.sender, |response| {
            CommonAuthorityRequest::ExactOrder { request, response }
        })
        .await
    }

    pub(super) async fn bind_open_orders(
        &mut self,
        page: PmLiveOpenOrderPage,
    ) -> Result<PmLiveOpenOrderPage, CredentialAuthorityError> {
        request_common(&self.sender, |response| {
            CommonAuthorityRequest::BindOpenOrders { page, response }
        })
        .await
    }

    pub(super) async fn bind_trades(
        &mut self,
        page: PmLiveTradePage,
    ) -> Result<PmLiveTradePage, CredentialAuthorityError> {
        request_common(&self.sender, |response| {
            CommonAuthorityRequest::BindTrades { page, response }
        })
        .await
    }

    pub(super) async fn bind_exact_order(
        &mut self,
        order: PmLiveOrder,
    ) -> Result<PmLiveOrder, CredentialAuthorityError> {
        request_common(&self.sender, |response| {
            CommonAuthorityRequest::BindExactOrder {
                order: Box::new(order),
                response,
            }
        })
        .await
    }
}

impl fmt::Debug for FixedHttpAuthenticationRole {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("FixedHttpAuthenticationRole(<opaque>)")
    }
}

#[async_trait]
impl PmHttpReadAuthorityProvider for FixedHttpAuthenticationRole {
    async fn authenticate_open_orders(
        &mut self,
        timestamp: L2Timestamp,
    ) -> Result<AuthenticatedL2Headers, PmLiveAdapterError> {
        FixedHttpAuthenticationRole::authenticate_open_orders(
            self,
            AuthorizedL2Timestamp::new(timestamp),
        )
        .await
        .map_err(map_external_read_error)
    }

    async fn authenticate_trades(
        &mut self,
        timestamp: L2Timestamp,
    ) -> Result<AuthenticatedL2Headers, PmLiveAdapterError> {
        FixedHttpAuthenticationRole::authenticate_trades(
            self,
            AuthorizedL2Timestamp::new(timestamp),
        )
        .await
        .map_err(map_external_read_error)
    }

    async fn authenticate_balance_allowance(
        &mut self,
        timestamp: L2Timestamp,
    ) -> Result<AuthenticatedL2Headers, PmLiveAdapterError> {
        FixedHttpAuthenticationRole::authenticate_balance_allowance(
            self,
            AuthorizedL2Timestamp::new(timestamp),
        )
        .await
        .map_err(map_external_read_error)
    }

    async fn authenticate_closed_only(
        &mut self,
        timestamp: L2Timestamp,
    ) -> Result<AuthenticatedL2Headers, PmLiveAdapterError> {
        FixedHttpAuthenticationRole::authenticate_closed_only(
            self,
            AuthorizedL2Timestamp::new(timestamp),
        )
        .await
        .map_err(map_external_read_error)
    }

    async fn authenticate_exact_order(
        &mut self,
        timestamp: L2Timestamp,
        order_id: reap_polymarket_auth::FixedOrderId,
    ) -> Result<AuthenticatedL2Headers, PmLiveAdapterError> {
        let authenticated = FixedHttpAuthenticationRole::authenticate_exact_owned_order(
            self,
            SealedExactOwnedOrderReadAuthentication::new(
                order_id,
                AuthorizedL2Timestamp::new(timestamp),
            ),
        )
        .await
        .map_err(map_external_read_error)?;
        let (headers, sealed_order_id, sealed_timestamp) = authenticated.into_parts();
        if sealed_order_id != order_id || sealed_timestamp != timestamp {
            return Err(PmLiveAdapterError::CredentialAuthorityClosed);
        }
        Ok(headers)
    }

    async fn bind_open_orders(
        &mut self,
        page: PmLiveOpenOrderPage,
    ) -> Result<PmLiveOpenOrderPage, PmLiveAdapterError> {
        FixedHttpAuthenticationRole::bind_open_orders(self, page)
            .await
            .map_err(map_external_read_error)
    }

    async fn bind_trades(
        &mut self,
        page: PmLiveTradePage,
    ) -> Result<PmLiveTradePage, PmLiveAdapterError> {
        FixedHttpAuthenticationRole::bind_trades(self, page)
            .await
            .map_err(map_external_read_error)
    }

    async fn bind_exact_order(
        &mut self,
        order: PmLiveOrder,
    ) -> Result<PmLiveOrder, PmLiveAdapterError> {
        FixedHttpAuthenticationRole::bind_exact_order(self, order)
            .await
            .map_err(map_external_read_error)
    }
}

/// User-WS subscription and complete-frame owner binding handle, separated
/// from HTTP so acknowledged stream setup can overlap final account cuts.
pub(super) struct FixedUserWsAuthenticationRole {
    sender: mpsc::Sender<CommonAuthorityRequest>,
}

impl FixedUserWsAuthenticationRole {
    pub(super) async fn user_subscription(
        &mut self,
        condition: PmConditionId,
    ) -> Result<AuthenticatedUserSubscription, CredentialAuthorityError> {
        request_common(&self.sender, |response| {
            CommonAuthorityRequest::UserSubscription {
                condition,
                response,
            }
        })
        .await
    }

    pub(super) async fn bind_user_frame(
        &mut self,
        frame: PmLiveUserFrame,
    ) -> Result<CredentialOwnedUserFrame, CredentialAuthorityError> {
        request_common(&self.sender, |response| {
            CommonAuthorityRequest::BindUserFrame { frame, response }
        })
        .await
    }
}

impl fmt::Debug for FixedUserWsAuthenticationRole {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("FixedUserWsAuthenticationRole(<opaque>)")
    }
}

#[async_trait]
impl PmUserWsReadAuthorityProvider for FixedUserWsAuthenticationRole {
    async fn authenticate_user_subscription(
        &mut self,
        condition: PmConditionId,
    ) -> Result<AuthenticatedUserSubscription, PmLiveAdapterError> {
        FixedUserWsAuthenticationRole::user_subscription(self, condition)
            .await
            .map_err(map_external_read_error)
    }

    async fn bind_user_frame(
        &mut self,
        frame: PmLiveUserFrame,
    ) -> Result<CredentialOwnedUserFrame, PmLiveAdapterError> {
        FixedUserWsAuthenticationRole::bind_user_frame(self, frame)
            .await
            .map_err(map_external_read_error)
    }
}

fn map_external_read_error(error: CredentialAuthorityError) -> PmLiveAdapterError {
    match error {
        CredentialAuthorityError::CredentialOwnerMismatch => {
            PmLiveAdapterError::CredentialOwnerMismatch
        }
        CredentialAuthorityError::Auth(error) => PmLiveAdapterError::Auth(error),
        _ => PmLiveAdapterError::CredentialAuthorityClosed,
    }
}

#[must_use = "dropping an armed credential supervisor aborts the process"]
pub(super) struct FreshCredentialAuthoritySupervisor {
    task: ArmedTaskSupervisor,
    teardown: FreshPlaceCredentialTeardown,
    identity: Arc<AuthorityIdentity>,
    private_key_removed: bool,
}

impl FreshCredentialAuthoritySupervisor {
    /// The future journal/session owner may call this only after durable
    /// A3 `PlacePrepared`. This lower authority neither validates nor mints
    /// that durable ack; it requires the caller to borrow the exact move-only
    /// durable acknowledgement, verifies its public request identity against
    /// the supervised task's retained identity, and requires the task-local
    /// signer to be destroyed. The acknowledgement remains owned by the A3
    /// flow for the later authorization-consumption transition.
    pub(super) fn remove_private_key_after_prepared(
        &mut self,
        prepared: &PmDurablePlacePreparedAckV1,
    ) -> Result<(), CredentialAuthorityError> {
        let durable = prepared.preparation();
        self.remove_private_key_after_prepared_identity(
            durable.expected_order_id(),
            durable.semantic_request_commitment(),
        )
    }

    fn remove_private_key_after_prepared_identity(
        &mut self,
        expected_order_id: reap_polymarket_auth::ExpectedOrderId,
        semantic_request_commitment: reap_polymarket_auth::PlaceSemanticRequestCommitment,
    ) -> Result<(), CredentialAuthorityError> {
        if !self.identity.signer_dropped() {
            return Err(CredentialAuthorityError::SignerStillOwned);
        }
        let Some(prepared_identity) = self.identity.prepared_public_identity() else {
            return Err(CredentialAuthorityError::PlacePreparedIdentityUnavailable);
        };
        if prepared_identity.expected_order_id() != expected_order_id
            || prepared_identity.semantic_request_commitment() != semantic_request_commitment
        {
            return Err(CredentialAuthorityError::PlacePreparedIdentityMismatch);
        }
        self.teardown
            .remove_private_key()
            .map_err(|_| CredentialAuthorityError::StagedCredentialTeardownFailed)?;
        self.private_key_removed = true;
        Ok(())
    }

    #[cfg(test)]
    fn remove_private_key_after_prepared_for_test(
        &mut self,
        identity: PlacePublicRequestIdentity,
    ) -> Result<(), CredentialAuthorityError> {
        self.remove_private_key_after_prepared_identity(
            identity.expected_order_id(),
            identity.semantic_request_commitment(),
        )
    }

    pub(super) async fn shutdown_bounded(
        self,
        bounds: CredentialAuthorityShutdownBounds,
    ) -> Result<CredentialAuthorityShutdownOutcome, CredentialAuthorityError> {
        let Self {
            task,
            mut teardown,
            identity,
            private_key_removed,
        } = self;
        let joined = task.join_bounded(bounds).await;
        if !identity.signer_dropped() {
            // A joined fresh task cannot still own its signer. Treat any
            // violation of that custody invariant as fail-stop.
            std::process::abort();
        }
        if !private_key_removed {
            teardown
                .remove_private_key()
                .map_err(|_| CredentialAuthorityError::StagedCredentialTeardownFailed)?;
        }
        teardown
            .remove_l2_files()
            .map_err(|_| CredentialAuthorityError::StagedCredentialTeardownFailed)?;
        Ok(joined.complete_with_teardown())
    }

    #[cfg(test)]
    fn signer_dropped_for_test(&self) -> bool {
        self.identity.signer_dropped()
    }
}

impl fmt::Debug for FreshCredentialAuthoritySupervisor {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("FreshCredentialAuthoritySupervisor([REDACTED; ARMED])")
    }
}

#[must_use = "dropping an armed credential supervisor aborts the process"]
pub(super) struct RecoveryCredentialAuthoritySupervisor {
    task: ArmedTaskSupervisor,
    teardown: RecoveryOnlyCredentialTeardown,
}

impl RecoveryCredentialAuthoritySupervisor {
    pub(super) async fn shutdown_bounded(
        self,
        bounds: CredentialAuthorityShutdownBounds,
    ) -> Result<CredentialAuthorityShutdownOutcome, CredentialAuthorityError> {
        let Self { task, mut teardown } = self;
        let joined = task.join_bounded(bounds).await;
        teardown
            .remove_l2_files()
            .map_err(|_| CredentialAuthorityError::StagedCredentialTeardownFailed)?;
        Ok(joined.complete_with_teardown())
    }
}

impl fmt::Debug for RecoveryCredentialAuthoritySupervisor {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("RecoveryCredentialAuthoritySupervisor([REDACTED; ARMED])")
    }
}

struct ArmedTaskSupervisor {
    shutdown: Option<oneshot::Sender<()>>,
    task: Option<JoinHandle<()>>,
    armed: bool,
}

impl ArmedTaskSupervisor {
    fn new(shutdown: oneshot::Sender<()>, task: JoinHandle<()>) -> Self {
        Self {
            shutdown: Some(shutdown),
            task: Some(task),
            armed: true,
        }
    }

    async fn join_bounded(mut self, bounds: CredentialAuthorityShutdownBounds) -> TaskJoinOutcome {
        let shutdown_requested = self.shutdown.take().is_some_and(|shutdown| {
            let _ = shutdown.send(());
            true
        });
        let Some(mut task) = self.task.take() else {
            std::process::abort();
        };

        let (abort_requested, task_completed_cleanly) =
            match tokio::time::timeout(bounds.graceful_join, &mut task).await {
                Ok(join) => (false, join.is_ok()),
                Err(_) => {
                    task.abort();
                    match tokio::time::timeout(bounds.abort_join, &mut task).await {
                        Ok(join) => (true, join.is_ok()),
                        // A still-live task owns secrets. Returning or
                        // unwinding here would detach it.
                        Err(_) => std::process::abort(),
                    }
                }
            };
        self.armed = false;
        TaskJoinOutcome {
            shutdown_requested,
            abort_requested,
            task_completed_cleanly,
        }
    }
}

impl Drop for ArmedTaskSupervisor {
    fn drop(&mut self) {
        if self.armed {
            // This covers ordinary early drop and cancellation of the
            // bounded shutdown future. Neither may detach secret custody.
            std::process::abort();
        }
    }
}

impl fmt::Debug for ArmedTaskSupervisor {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ArmedTaskSupervisor([REDACTED; ARMED])")
    }
}

struct TaskJoinOutcome {
    shutdown_requested: bool,
    abort_requested: bool,
    task_completed_cleanly: bool,
}

impl TaskJoinOutcome {
    const fn complete_with_teardown(self) -> CredentialAuthorityShutdownOutcome {
        CredentialAuthorityShutdownOutcome {
            shutdown_requested: self.shutdown_requested,
            abort_requested: self.abort_requested,
            task_joined: true,
            task_completed_cleanly: self.task_completed_cleanly,
            credentials_dropped: true,
            staged_l2_files_removed: true,
        }
    }
}

enum PlaceAuthorityRequest {
    Prepare {
        request: SealedPmT2ProxyPlacePreparation,
        response: oneshot::Sender<Result<PlacePublicRequestIdentity, CredentialAuthorityError>>,
    },
    Finalize {
        authorization: PlaceHmacAdmission,
        response:
            oneshot::Sender<Result<OpaqueAuthenticatedPlaceRequest, CredentialAuthorityError>>,
    },
}

struct RetainedPreparedPlace {
    serialized: SerializedPlaceRequest,
    public_identity: PlacePublicRequestIdentity,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CancelAuthorityMode {
    FreshPrimary,
    RecoveryOnly,
}

impl CancelAuthorityMode {
    const fn accepts(self, class: PmCancelDispatchClassV1) -> bool {
        matches!(
            (self, class),
            (Self::FreshPrimary, PmCancelDispatchClassV1::Primary)
                | (Self::RecoveryOnly, PmCancelDispatchClassV1::Recovery { .. })
        )
    }
}

type CancelAuthenticationResponse =
    Result<OpaqueAuthenticatedExactOwnedCancel, CancelAuthenticationPreSendFailure>;

enum CancelAuthorityRequest {
    Authenticate {
        admission: Box<CancelHmacAdmission>,
        response: oneshot::Sender<CancelAuthenticationResponse>,
    },
    #[cfg(test)]
    PauseForCancellationTest {
        pause: CancelRequestTestPause,
        response: oneshot::Sender<()>,
    },
}

enum CommonAuthorityRequest {
    OpenOrders {
        timestamp: L2Timestamp,
        response: oneshot::Sender<Result<AuthenticatedL2Headers, CredentialAuthorityError>>,
    },
    Trades {
        timestamp: L2Timestamp,
        response: oneshot::Sender<Result<AuthenticatedL2Headers, CredentialAuthorityError>>,
    },
    BalanceAllowance {
        timestamp: L2Timestamp,
        response: oneshot::Sender<Result<AuthenticatedL2Headers, CredentialAuthorityError>>,
    },
    ClosedOnly {
        timestamp: L2Timestamp,
        response: oneshot::Sender<Result<AuthenticatedL2Headers, CredentialAuthorityError>>,
    },
    ExactOrder {
        request: SealedExactOwnedOrderReadAuthentication,
        response:
            oneshot::Sender<Result<AuthenticatedExactOwnedOrderRead, CredentialAuthorityError>>,
    },
    BindOpenOrders {
        page: PmLiveOpenOrderPage,
        response: oneshot::Sender<Result<PmLiveOpenOrderPage, CredentialAuthorityError>>,
    },
    BindTrades {
        page: PmLiveTradePage,
        response: oneshot::Sender<Result<PmLiveTradePage, CredentialAuthorityError>>,
    },
    BindExactOrder {
        order: Box<PmLiveOrder>,
        response: oneshot::Sender<Result<PmLiveOrder, CredentialAuthorityError>>,
    },
    UserSubscription {
        condition: PmConditionId,
        response: oneshot::Sender<Result<AuthenticatedUserSubscription, CredentialAuthorityError>>,
    },
    BindUserFrame {
        frame: PmLiveUserFrame,
        response: oneshot::Sender<Result<CredentialOwnedUserFrame, CredentialAuthorityError>>,
    },
}

struct FreshAuthorityTaskInputs {
    place_time_finalizer: PmPlaceMutationTimeFinalizer,
    cancel_time_finalizer: PmCancelMutationTimeFinalizer,
    common: mpsc::Receiver<CommonAuthorityRequest>,
    place: mpsc::Receiver<PlaceAuthorityRequest>,
    cancel: mpsc::Receiver<CancelAuthorityRequest>,
    shutdown: oneshot::Receiver<()>,
}

async fn run_fresh_authority(
    mut signer: TaskSignerCustody,
    credentials: L2Credentials,
    inputs: FreshAuthorityTaskInputs,
) {
    let FreshAuthorityTaskInputs {
        mut place_time_finalizer,
        mut cancel_time_finalizer,
        mut common,
        mut place,
        mut cancel,
        mut shutdown,
    } = inputs;
    let mut common_open = true;
    let mut place_open = true;
    let mut cancel_open = true;
    let mut retained_place = None;
    loop {
        tokio::select! {
            biased;
            _ = &mut shutdown => {
                common.close();
                place.close();
                cancel.close();
                return;
            }
            request = common.recv(), if common_open => match request {
                Some(request) => handle_common_request(&credentials, request),
                None => common_open = false,
            },
            request = place.recv(), if place_open => match request {
                Some(request) => handle_place_request(
                    request,
                    &mut signer,
                    &credentials,
                    &mut place_time_finalizer,
                    &mut retained_place,
                ),
                None => place_open = false,
            },
            request = cancel.recv(), if cancel_open => match request {
                Some(request) => handle_cancel_request(
                    request,
                    CancelAuthorityMode::FreshPrimary,
                    &credentials,
                    &mut cancel_time_finalizer,
                ),
                None => cancel_open = false,
            },
            else => return,
        }
    }
}

// BEGIN RECOVERY_TASK
async fn run_recovery_authority(
    credentials: L2Credentials,
    mut cancel_time_finalizer: PmCancelMutationTimeFinalizer,
    mut common: mpsc::Receiver<CommonAuthorityRequest>,
    mut cancel: mpsc::Receiver<CancelAuthorityRequest>,
    mut shutdown: oneshot::Receiver<()>,
) {
    let mut common_open = true;
    let mut cancel_open = true;
    loop {
        tokio::select! {
            biased;
            _ = &mut shutdown => {
                common.close();
                cancel.close();
                return;
            }
            request = common.recv(), if common_open => match request {
                Some(request) => handle_common_request(&credentials, request),
                None => common_open = false,
            },
            request = cancel.recv(), if cancel_open => match request {
                Some(request) => handle_cancel_request(
                    request,
                    CancelAuthorityMode::RecoveryOnly,
                    &credentials,
                    &mut cancel_time_finalizer,
                ),
                None => cancel_open = false,
            },
            else => return,
        }
    }
}
// END RECOVERY_TASK

fn handle_place_request(
    request: PlaceAuthorityRequest,
    signer: &mut TaskSignerCustody,
    credentials: &L2Credentials,
    place_time_finalizer: &mut PmPlaceMutationTimeFinalizer,
    retained_place: &mut Option<RetainedPreparedPlace>,
) {
    match request {
        PlaceAuthorityRequest::Prepare { request, response } => {
            respond_place_preparation(request, response, signer, credentials, retained_place);
        }
        PlaceAuthorityRequest::Finalize {
            authorization,
            response,
        } => {
            respond_place_finalization(
                authorization,
                response,
                credentials,
                place_time_finalizer,
                retained_place,
            );
        }
    }
}

fn respond_place_preparation(
    mut request: SealedPmT2ProxyPlacePreparation,
    response: oneshot::Sender<Result<PlacePublicRequestIdentity, CredentialAuthorityError>>,
    signer: &mut TaskSignerCustody,
    credentials: &L2Credentials,
    retained_place: &mut Option<RetainedPreparedPlace>,
) {
    request.wait_for_test_pause();
    let Some(signer_value) = signer.take() else {
        let _ = response.send(Err(CredentialAuthorityError::PlaceAlreadyConsumed));
        return;
    };
    let prepared = prepare_place(&signer_value, credentials, request);
    drop(signer_value);
    signer.publish_signer_dropped();
    // Retention and the reply both happen strictly after the signer value is
    // destroyed and that fact is published with Release ordering.
    let value = prepared.map(|prepared| {
        let public_identity = prepared.public_identity;
        *retained_place = Some(prepared);
        signer.publish_prepared_public_identity(public_identity);
        public_identity
    });
    let _ = response.send(value);
}

fn prepare_place(
    signer: &FixedEoaSigner,
    credentials: &L2Credentials,
    request: SealedPmT2ProxyPlacePreparation,
) -> Result<RetainedPreparedPlace, CredentialAuthorityError> {
    if request.unsigned_order.signature_profile() != PmClobV2SignatureType::Proxy
        || request.unsigned_order.maker() == request.unsigned_order.signer()
    {
        return Err(CredentialAuthorityError::PlaceProfileMismatch);
    }
    if request.unsigned_order.signer() != signer.address().as_core()
        || request.unsigned_order.signer() != credentials.address().as_core()
    {
        return Err(CredentialAuthorityError::PlaceSignerMismatch);
    }
    let derived = derive_place_public_request_identity(request.domain, request.unsigned_order);
    if derived != request.expected_public_identity {
        return Err(CredentialAuthorityError::PlacePublicIdentityMismatch);
    }
    let signed = signer.sign_clob_v2_order(request.domain, request.unsigned_order)?;
    let serialized = credentials.serialize_gtc_post_only(signed)?;
    if serialized.expected_order_id() != derived.expected_order_id()
        || serialized.semantic_request_commitment() != derived.semantic_request_commitment()
    {
        return Err(CredentialAuthorityError::PlacePublicIdentityMismatch);
    }
    Ok(RetainedPreparedPlace {
        serialized,
        public_identity: derived,
    })
}

fn respond_place_finalization(
    mut authorization: PlaceHmacAdmission,
    response: oneshot::Sender<Result<OpaqueAuthenticatedPlaceRequest, CredentialAuthorityError>>,
    credentials: &L2Credentials,
    place_time_finalizer: &mut PmPlaceMutationTimeFinalizer,
    retained_place: &mut Option<RetainedPreparedPlace>,
) {
    authorization.wait_for_test_pause();
    let Some(prepared) = retained_place.as_ref() else {
        let _ = response.send(Err(CredentialAuthorityError::PlaceAlreadyConsumed));
        return;
    };
    if authorization.public_identity != prepared.public_identity {
        let _ = response.send(Err(CredentialAuthorityError::PlaceDispatchBindingMismatch));
        return;
    }
    let Some(prepared) = retained_place.take() else {
        std::process::abort();
    };
    let authenticated = place_time_finalizer
        .authenticate_exact_place(
            authorization.proof,
            authorization.expected_l2_timestamp_seconds,
            credentials,
            prepared.serialized,
        )
        .map_err(Into::into)
        .and_then(|authenticated| {
            if authenticated.expected_order_id() != prepared.public_identity.expected_order_id()
                || authenticated.semantic_request_commitment()
                    != prepared.public_identity.semantic_request_commitment()
            {
                return Err(CredentialAuthorityError::PlacePublicIdentityMismatch);
            }
            Ok(OpaqueAuthenticatedPlaceRequest {
                request: authenticated,
            })
        });
    let _ = response.send(authenticated);
}

fn handle_cancel_request(
    request: CancelAuthorityRequest,
    mode: CancelAuthorityMode,
    credentials: &L2Credentials,
    cancel_time_finalizer: &mut PmCancelMutationTimeFinalizer,
) {
    match request {
        CancelAuthorityRequest::Authenticate {
            admission,
            response,
        } => {
            let value =
                authenticate_cancel_admission(*admission, mode, credentials, cancel_time_finalizer);
            if response.send(value).is_err() {
                // Losing the sole positive owner or an authenticated request
                // after admission makes send status/custody ambiguous.
                std::process::abort();
            }
        }
        #[cfg(test)]
        CancelAuthorityRequest::PauseForCancellationTest { pause, response } => {
            pause.wait();
            if response.send(()).is_err() {
                std::process::abort();
            }
        }
    }
}

fn authenticate_cancel_admission(
    mut admission: CancelHmacAdmission,
    mode: CancelAuthorityMode,
    credentials: &L2Credentials,
    cancel_time_finalizer: &mut PmCancelMutationTimeFinalizer,
) -> Result<OpaqueAuthenticatedExactOwnedCancel, CancelAuthenticationPreSendFailure> {
    admission.wait_for_test_pause();
    let CancelHmacAdmission { owner, proof, .. } = admission;
    if !mode.accepts(owner.dispatch_class()) {
        return Err(CancelAuthenticationPreSendFailure::new(
            owner,
            CancelAuthenticationPreSendFailureKind::DispatchClassMismatch,
        ));
    }

    let order_id = owner.exact_venue_order_id();
    let semantic_request_commitment = owner.semantic_request_commitment();
    let expected_l2_timestamp_seconds = owner.l2_timestamp_seconds();
    let preparation = owner.preparation();
    if preparation.exact_venue_order_id() != order_id
        || preparation.semantic_request_commitment() != semantic_request_commitment
        || preparation.l2_timestamp_seconds() != expected_l2_timestamp_seconds
    {
        return Err(CancelAuthenticationPreSendFailure::new(
            owner,
            CancelAuthenticationPreSendFailureKind::RequestBindingMismatch,
        ));
    }

    let serialized = match credentials.serialize_owned_cancel(order_id) {
        Ok(serialized) => serialized,
        Err(_) => {
            return Err(CancelAuthenticationPreSendFailure::new(
                owner,
                CancelAuthenticationPreSendFailureKind::AuthenticationFailed,
            ));
        }
    };
    if serialized.order_id() != order_id
        || serialized.semantic_request_commitment() != semantic_request_commitment
    {
        return Err(CancelAuthenticationPreSendFailure::new(
            owner,
            CancelAuthenticationPreSendFailureKind::RequestBindingMismatch,
        ));
    }

    let authenticated = match cancel_time_finalizer.authenticate_exact_owned_cancel(
        proof,
        expected_l2_timestamp_seconds,
        credentials,
        serialized,
    ) {
        Ok(authenticated) => authenticated,
        Err(_error) => {
            return Err(CancelAuthenticationPreSendFailure::new(
                owner,
                CancelAuthenticationPreSendFailureKind::AuthenticationFailed,
            ));
        }
    };
    if authenticated.order_id() != order_id
        || authenticated.semantic_request_commitment() != semantic_request_commitment
        || owner.dispatch_class() != preparation.dispatch_class()
    {
        return Err(CancelAuthenticationPreSendFailure::new(
            owner,
            CancelAuthenticationPreSendFailureKind::RequestBindingMismatch,
        ));
    }

    Ok(OpaqueAuthenticatedExactOwnedCancel {
        request: authenticated,
        owner,
    })
}

fn handle_common_request(credentials: &L2Credentials, request: CommonAuthorityRequest) {
    match request {
        CommonAuthorityRequest::OpenOrders {
            timestamp,
            response,
        } => {
            let _ = response.send(
                credentials
                    .authenticate_open_orders(timestamp)
                    .map_err(Into::into),
            );
        }
        CommonAuthorityRequest::Trades {
            timestamp,
            response,
        } => {
            let _ = response.send(
                credentials
                    .authenticate_trades(timestamp)
                    .map_err(Into::into),
            );
        }
        CommonAuthorityRequest::BalanceAllowance {
            timestamp,
            response,
        } => {
            let _ = response.send(
                credentials
                    .authenticate_balance_allowance(timestamp)
                    .map_err(Into::into),
            );
        }
        CommonAuthorityRequest::ClosedOnly {
            timestamp,
            response,
        } => {
            let _ = response.send(
                credentials
                    .authenticate_closed_only(timestamp)
                    .map_err(Into::into),
            );
        }
        CommonAuthorityRequest::ExactOrder { request, response } => {
            let timestamp = request.timestamp.into_inner();
            let order_id = request.order_id;
            let value = credentials
                .authenticate_order_detail(timestamp, order_id)
                .map(|headers| AuthenticatedExactOwnedOrderRead::new(headers, order_id, timestamp))
                .map_err(Into::into);
            let _ = response.send(value);
        }
        CommonAuthorityRequest::BindOpenOrders { page, response } => {
            let matches = page
                .orders()
                .iter()
                .all(|order| credentials.matches_credential_owner(order.owner()));
            let value = owner_match(matches).map(|()| page);
            let _ = response.send(value);
        }
        CommonAuthorityRequest::BindTrades { page, response } => {
            let matches = page
                .trades()
                .iter()
                .all(|trade| credentials.matches_credential_owner(trade.owner()));
            let value = owner_match(matches).map(|()| page);
            let _ = response.send(value);
        }
        CommonAuthorityRequest::BindExactOrder { order, response } => {
            let value =
                owner_match(credentials.matches_credential_owner(order.owner())).map(|()| *order);
            let _ = response.send(value);
        }
        CommonAuthorityRequest::UserSubscription {
            condition,
            response,
        } => {
            let _ = response.send(credentials.user_subscription(condition).map_err(Into::into));
        }
        CommonAuthorityRequest::BindUserFrame { frame, response } => {
            let _ = response.send(
                credentials
                    .bind_user_stream_frame(frame)
                    .map_err(Into::into),
            );
        }
    }
}

fn owner_match(matches: bool) -> Result<(), CredentialAuthorityError> {
    if matches {
        Ok(())
    } else {
        Err(CredentialAuthorityError::CredentialOwnerMismatch)
    }
}

async fn request_place<T>(
    sender: &mpsc::Sender<PlaceAuthorityRequest>,
    make: impl FnOnce(oneshot::Sender<Result<T, CredentialAuthorityError>>) -> PlaceAuthorityRequest,
) -> Result<T, CredentialAuthorityError> {
    let (response, receive) = oneshot::channel();
    sender
        .try_send(make(response))
        .map_err(classify_place_send)?;
    let mut admitted = AdmittedPlaceRequestGuard { armed: true };
    let result = match receive.await {
        Ok(result) => result,
        // An admitted prepare/finalize request may have changed signer/body
        // custody or consumed the only HMAC opportunity. Losing its terminal
        // reply is ambiguous and must not unwind into a reusable session.
        Err(_) => std::process::abort(),
    };
    admitted.disarm();
    result
}

struct AdmittedPlaceRequestGuard {
    armed: bool,
}

impl AdmittedPlaceRequestGuard {
    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for AdmittedPlaceRequestGuard {
    fn drop(&mut self) {
        if self.armed {
            // Dropping the caller future after channel admission would lose
            // whether signing/retention or the final HMAC already occurred.
            std::process::abort();
        }
    }
}

async fn request_cancel(
    sender: &mpsc::Sender<CancelAuthorityRequest>,
    admission: CancelHmacAdmission,
) -> Result<OpaqueAuthenticatedExactOwnedCancel, CancelAuthenticationPreSendFailure> {
    let (response, receive) = oneshot::channel();
    let request = CancelAuthorityRequest::Authenticate {
        admission: Box::new(admission),
        response,
    };
    if let Err(error) = sender.try_send(request) {
        let (request, kind) = match error {
            mpsc::error::TrySendError::Full(request) => (
                request,
                CancelAuthenticationPreSendFailureKind::AuthoritySaturated,
            ),
            mpsc::error::TrySendError::Closed(request) => (
                request,
                CancelAuthenticationPreSendFailureKind::AuthorityClosed,
            ),
        };
        let admission = match request {
            CancelAuthorityRequest::Authenticate { admission, .. } => admission,
            #[cfg(test)]
            CancelAuthorityRequest::PauseForCancellationTest { .. } => std::process::abort(),
        };
        let CancelHmacAdmission { owner, .. } = *admission;
        return Err(CancelAuthenticationPreSendFailure::new(owner, kind));
    }
    let mut admitted = AdmittedCancelRequestGuard { armed: true };
    let result = match receive.await {
        Ok(result) => result,
        // The task may have consumed the positive owner/proof and minted an
        // HMAC. A lost terminal reply cannot unwind into a reusable session.
        Err(_) => std::process::abort(),
    };
    admitted.disarm();
    result
}

#[cfg(test)]
async fn request_cancel_pause(
    sender: &mpsc::Sender<CancelAuthorityRequest>,
    pause: CancelRequestTestPause,
) -> Result<(), CredentialAuthorityError> {
    let (response, receive) = oneshot::channel();
    sender
        .try_send(CancelAuthorityRequest::PauseForCancellationTest { pause, response })
        .map_err(classify_cancel_pause_send)?;
    let mut admitted = AdmittedCancelRequestGuard { armed: true };
    match receive.await {
        Ok(()) => admitted.disarm(),
        Err(_) => std::process::abort(),
    }
    Ok(())
}

struct AdmittedCancelRequestGuard {
    armed: bool,
}

impl AdmittedCancelRequestGuard {
    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for AdmittedCancelRequestGuard {
    fn drop(&mut self) {
        if self.armed {
            // An admitted cancel owns the sole positive A3 token. Future
            // cancellation must never detach the credential task or lose the
            // typed pre-send/send-status outcome.
            std::process::abort();
        }
    }
}

async fn request_common<T>(
    sender: &mpsc::Sender<CommonAuthorityRequest>,
    make: impl FnOnce(oneshot::Sender<Result<T, CredentialAuthorityError>>) -> CommonAuthorityRequest,
) -> Result<T, CredentialAuthorityError> {
    let (response, receive) = oneshot::channel();
    sender
        .try_send(make(response))
        .map_err(classify_common_send)?;
    receive
        .await
        .map_err(|_| CredentialAuthorityError::AuthorityClosed)?
}

fn classify_place_send(
    error: mpsc::error::TrySendError<PlaceAuthorityRequest>,
) -> CredentialAuthorityError {
    match error {
        mpsc::error::TrySendError::Full(request) => {
            drop(request);
            CredentialAuthorityError::AuthoritySaturated
        }
        mpsc::error::TrySendError::Closed(request) => {
            drop(request);
            CredentialAuthorityError::AuthorityClosed
        }
    }
}

fn classify_common_send(
    error: mpsc::error::TrySendError<CommonAuthorityRequest>,
) -> CredentialAuthorityError {
    match error {
        mpsc::error::TrySendError::Full(request) => {
            drop(request);
            CredentialAuthorityError::AuthoritySaturated
        }
        mpsc::error::TrySendError::Closed(request) => {
            drop(request);
            CredentialAuthorityError::AuthorityClosed
        }
    }
}

#[cfg(test)]
fn classify_cancel_pause_send(
    error: mpsc::error::TrySendError<CancelAuthorityRequest>,
) -> CredentialAuthorityError {
    match error {
        mpsc::error::TrySendError::Full(request) => {
            drop(request);
            CredentialAuthorityError::AuthoritySaturated
        }
        mpsc::error::TrySendError::Closed(request) => {
            drop(request);
            CredentialAuthorityError::AuthorityClosed
        }
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use std::{
        convert::Infallible,
        fs,
        os::unix::{fs::PermissionsExt as _, process::ExitStatusExt as _},
        path::Path,
        process::Command,
        time::Duration,
    };

    use reap_pm_core::{
        ConnectionEpoch, EvmAddress, PmMarketId, PmOrderSalt, PmOrderSide, PmPrice, PmQuantity,
        PmTick, PmTokenId, U256,
    };
    use reap_polymarket_auth::{
        AuthenticatedUserSubscriptionSink, EoaAddress, FixedOrderId, FixedPlaceRequestSink,
        L2HeaderSink,
    };
    use reap_polymarket_live_adapter::{
        PmCancelServerTimeHttpRole, PmPlaceServerTimeHttpRole, PmProductClockOwner,
        PmPublicConnectivityOwner, PmPublicHttpConfig, PmPublicWsConfig,
    };
    use reap_polymarket_wire::{
        PmBookParserConfig, PmUnsignedClobV2Order, PmWireScope, parse_live_open_order_page,
        parse_live_order_detail, parse_live_trade_page, parse_live_user_frame,
    };
    use tempfile::TempDir;
    use tokio::{
        io::{AsyncReadExt as _, AsyncWriteExt as _},
        net::TcpListener,
        task::JoinHandle,
    };

    use super::*;
    use crate::controlled_trial::{AuthorizedL2Timestamp, SealedExactOwnedOrderReadAuthentication};

    const KEY: &str = "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";
    const SIGNER: &str = "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266";
    const PROXY: &str = "0x70997970C51812dc3A010C7d01b50e0d17dc79C8";
    const API_KEY: &str = "00000000-0000-4000-8000-000000000001";
    const FOREIGN_API_KEY: &str = "00000000-0000-4000-8000-000000000002";
    const L2_SECRET: &str = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";
    const PASSPHRASE: &str = "synthetic-passphrase";
    const CONDITION: &str = "0x1111111111111111111111111111111111111111111111111111111111111111";
    const AUTH_SECONDS: u64 = 1_780_449_126;
    const CANCELLATION_CHILD_CASE: &str = "REAP_PM_AUTHORITY_CANCELLATION_CHILD_CASE";

    fn write(directory: &Path, name: &str, value: &str) {
        let path = directory.join(name);
        fs::write(&path, value).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
    }

    fn stage_four() -> TempDir {
        let directory = tempfile::tempdir().unwrap();
        fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700)).unwrap();
        write(directory.path(), "private-key", KEY);
        write(directory.path(), "api-key", API_KEY);
        write(directory.path(), "l2-secret", L2_SECRET);
        write(directory.path(), "passphrase", PASSPHRASE);
        directory
    }

    fn configured_eoa() -> EoaAddress {
        EoaAddress::parse(SIGNER).unwrap()
    }

    fn fresh_owner(directory: &Path) -> FreshCredentialAuthorityOwner {
        FreshCredentialAuthorityOwner::load_from_protected_files(
            directory.to_owned(),
            "private-key".into(),
            "api-key".into(),
            "l2-secret".into(),
            "passphrase".into(),
            configured_eoa(),
        )
        .unwrap()
    }

    fn recovery_owner(directory: &Path) -> RecoveryCredentialAuthorityOwner {
        RecoveryCredentialAuthorityOwner::load_from_protected_files(
            directory.to_owned(),
            "api-key".into(),
            "l2-secret".into(),
            "passphrase".into(),
            configured_eoa(),
        )
        .unwrap()
    }

    fn recovery_handoff(
        directory: &Path,
    ) -> super::credential_custody::RecoveryOnlyCredentialHandoff {
        super::credential_custody::RecoveryOnlyCredentialFiles::new(
            directory.to_owned(),
            "api-key".into(),
            "l2-secret".into(),
            "passphrase".into(),
        )
        .load(configured_eoa())
        .unwrap()
    }

    fn timestamp() -> AuthorizedL2Timestamp {
        AuthorizedL2Timestamp::new(L2Timestamp::from_unix_seconds(AUTH_SECONDS).unwrap())
    }

    fn test_wire_scope() -> PmWireScope {
        PmWireScope::new(
            PmConditionId::parse(CONDITION).unwrap(),
            PmMarketId::parse(CONDITION).unwrap(),
            PmTokenId::new(U256::from_u64(1_234)).unwrap(),
        )
    }

    fn mutation_time_roles(
        origin: &str,
    ) -> (
        (PmPlaceServerTimeHttpRole, PmPlaceMutationTimeFinalizer),
        (PmCancelServerTimeHttpRole, PmCancelMutationTimeFinalizer),
    ) {
        let scope = test_wire_scope();
        let http = PmPublicHttpConfig::loopback_evidence(
            origin,
            Duration::from_secs(1),
            Duration::from_secs(2),
        )
        .unwrap();
        let parser = PmBookParserConfig::new_condition_bound(
            scope,
            PmTick::parse_decimal("0.01").unwrap(),
            PmQuantity::parse_decimal("5").unwrap(),
            false,
        );
        let public_ws = PmPublicWsConfig::loopback_evidence(
            "ws://127.0.0.1:9/ws/market",
            scope,
            Duration::from_secs(1),
            Duration::from_secs(20),
            Duration::from_secs(10),
            Duration::from_secs(2),
            64 * 1_024,
            1,
            Duration::from_millis(1),
            8,
            ConnectionEpoch::new(1),
        )
        .unwrap();
        let (
            metadata,
            book,
            read_time,
            private_read,
            place_time,
            cancel_time,
            public_ws,
            user_clock,
            actor_clock,
            okx_clock,
        ) = PmPublicConnectivityOwner::new(http, parser, public_ws, PmProductClockOwner::system())
            .unwrap()
            .into_roles()
            .into_roles();
        drop((
            metadata,
            book,
            read_time,
            private_read,
            public_ws,
            user_clock,
            actor_clock,
            okx_clock,
        ));
        (place_time.into_roles(), cancel_time.into_roles())
    }

    fn place_time_roles(origin: &str) -> (PmPlaceServerTimeHttpRole, PmPlaceMutationTimeFinalizer) {
        let (place, cancel) = mutation_time_roles(origin);
        drop(cancel);
        place
    }

    fn unused_place_time_finalizer() -> PmPlaceMutationTimeFinalizer {
        place_time_roles("http://127.0.0.1:9").1
    }

    pub(super) fn unused_cancel_time_finalizer() -> PmCancelMutationTimeFinalizer {
        let (place, cancel) = mutation_time_roles("http://127.0.0.1:9");
        drop(place);
        cancel.1
    }

    async fn start_time_server(request_count: usize) -> (String, JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let task = tokio::spawn(async move {
            for _ in 0..request_count {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = Vec::with_capacity(1_024);
                loop {
                    let mut buffer = [0_u8; 512];
                    let read = socket.read(&mut buffer).await.unwrap();
                    assert!(read > 0);
                    request.extend_from_slice(&buffer[..read]);
                    if request.windows(4).any(|window| window == b"\r\n\r\n") {
                        break;
                    }
                    assert!(request.len() <= 8 * 1_024);
                }
                assert!(request.starts_with(b"GET /time HTTP/1.1\r\n"));
                let body = AUTH_SECONDS.to_string();
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len(),
                );
                socket.write_all(response.as_bytes()).await.unwrap();
                socket.shutdown().await.unwrap();
            }
        });
        (origin, task)
    }

    fn proxy_order() -> PmUnsignedClobV2Order {
        PmUnsignedClobV2Order::new_pm_t2_proxy(
            PmOrderSalt::from_u64(1_713_398_400_000).unwrap(),
            EvmAddress::parse(PROXY).unwrap(),
            EvmAddress::parse(SIGNER).unwrap(),
            PmTokenId::new(U256::from_u64(1_234)).unwrap(),
            PmOrderSide::Buy,
            PmPrice::parse_decimal("0.40").unwrap(),
            PmQuantity::parse_decimal("10").unwrap(),
            PmTick::parse_decimal("0.01").unwrap(),
            PmQuantity::parse_decimal("5").unwrap(),
            1_713_398_400_000,
        )
        .unwrap()
    }

    fn eoa_order() -> PmUnsignedClobV2Order {
        let signer = EvmAddress::parse(SIGNER).unwrap();
        PmUnsignedClobV2Order::new_goal_f(
            PmOrderSalt::from_u64(479_249_096_354).unwrap(),
            signer,
            signer,
            PmTokenId::new(U256::from_u64(1_234)).unwrap(),
            PmOrderSide::Buy,
            PmPrice::parse_decimal("0.40").unwrap(),
            PmQuantity::parse_decimal("10").unwrap(),
            PmTick::parse_decimal("0.01").unwrap(),
            PmQuantity::parse_decimal("5").unwrap(),
            1_713_398_400_000,
        )
        .unwrap()
    }

    fn sealed_place(order: PmUnsignedClobV2Order) -> SealedPmT2ProxyPlacePreparation {
        let domain = reap_polymarket_auth::PmClobDomain::Standard;
        let identity = derive_place_public_request_identity(domain, order);
        SealedPmT2ProxyPlacePreparation::new(domain, order, identity)
    }

    fn all_staged_absent(directory: &Path) -> bool {
        ["private-key", "api-key", "l2-secret", "passphrase"]
            .iter()
            .all(|name| !directory.join(name).exists())
    }

    fn order_json(order_id: FixedOrderId, owner: &str) -> String {
        format!(
            r#"{{"id":"{order_id}","market":"{CONDITION}","asset_id":"1234","side":"BUY","original_size":"10.000000","size_matched":"0","price":"0.400000","status":"LIVE","maker_address":"{PROXY}","owner":"{owner}","expiration":"0","created_at":1780449126}}"#,
        )
    }

    fn trade_json(order_id: FixedOrderId, owner: &str) -> String {
        format!(
            r#"{{"id":"trade-1","market":"{CONDITION}","asset_id":"1234","side":"SELL","size":"2.500000","price":"0.400000","status":"CONFIRMED","match_time":"1780449126002","last_update":"1780449126003","order_id":"{order_id}","maker_orders":[],"maker_address":"{PROXY}","owner":"{owner}"}}"#,
        )
    }

    fn user_frame_json(order_id: FixedOrderId, owner: &str) -> String {
        format!(
            r#"{{"event_type":"order","id":"{order_id}","owner":"{owner}","market":"{CONDITION}","asset_id":"1234","side":"BUY","original_size":"10.000000","size_matched":"0","price":"0.400000","type":"PLACEMENT","order_owner":"{owner}","timestamp":"1780449126000","associate_trades":null,"outcome":"YES","created_at":"1780449126000","expiration":"0","order_type":"GTC","status":"LIVE","maker_address":"{PROXY}"}}"#,
        )
    }

    #[derive(Default)]
    struct HeaderCapture {
        address: String,
        signature: String,
        timestamp: String,
        api_key: String,
        passphrase: String,
    }

    impl L2HeaderSink for HeaderCapture {
        type Error = Infallible;

        fn set_polymarket_l2_headers(
            &mut self,
            poly_address: &str,
            poly_signature: &str,
            poly_timestamp: &str,
            poly_api_key: &str,
            poly_passphrase: &str,
        ) -> Result<(), Self::Error> {
            self.address = poly_address.into();
            self.signature = poly_signature.into();
            self.timestamp = poly_timestamp.into();
            self.api_key = poly_api_key.into();
            self.passphrase = poly_passphrase.into();
            Ok(())
        }
    }

    #[derive(Default)]
    struct MutationCapture {
        address: String,
        timestamp: String,
        api_key: String,
        body: Vec<u8>,
    }

    impl FixedPlaceRequestSink for MutationCapture {
        type Output = ();
        type Error = Infallible;

        #[allow(
            clippy::too_many_arguments,
            reason = "the synthetic sink captures the complete fixed-purpose trait boundary"
        )]
        fn send_gtc_post_only(
            &mut self,
            poly_address: &str,
            _poly_signature: &str,
            poly_timestamp: &str,
            poly_api_key: &str,
            _poly_passphrase: &str,
            _expected_making_amount: U256,
            _expected_taking_amount: U256,
            exact_body: &[u8],
        ) -> Result<Self::Output, Self::Error> {
            self.address = poly_address.into();
            self.timestamp = poly_timestamp.into();
            self.api_key = poly_api_key.into();
            self.body.extend_from_slice(exact_body);
            Ok(())
        }
    }

    #[derive(Default)]
    struct FrameCapture(Vec<u8>);

    impl AuthenticatedUserSubscriptionSink for FrameCapture {
        type Output = ();
        type Error = Infallible;

        fn send_user_subscription(
            &mut self,
            exact_frame: &[u8],
        ) -> Result<Self::Output, Self::Error> {
            self.0.extend_from_slice(exact_frame);
            Ok(())
        }
    }

    fn capture_headers(
        result: Result<AuthenticatedL2Headers, CredentialAuthorityError>,
    ) -> Result<HeaderCapture, CredentialAuthorityError> {
        let mut capture = HeaderCapture::default();
        result?.apply_to(&mut capture).unwrap();
        Ok(capture)
    }

    fn normal_bounds() -> CredentialAuthorityShutdownBounds {
        CredentialAuthorityShutdownBounds::new(Duration::from_secs(2), Duration::from_secs(2))
            .unwrap()
    }

    async fn cancellation_child(case: &str) {
        match case {
            "prepare" => {
                let directory = stage_four();
                let roles = fresh_owner(directory.path())
                    .spawn(unused_place_time_finalizer())
                    .unwrap();
                let (place, cancel, http, user_ws, supervisor) = roles.into_roles();
                let (admitted, admitted_receive) = std::sync::mpsc::sync_channel(0);
                let (release, release_receive) = std::sync::mpsc::channel();
                let request = sealed_place(proxy_order()).with_test_pause(PlaceRequestTestPause {
                    admitted,
                    release: release_receive,
                });
                let task = tokio::spawn(place.prepare_place_once(request));
                tokio::task::spawn_blocking(move || admitted_receive.recv().unwrap())
                    .await
                    .unwrap();
                task.abort();
                let _ = task.await;
                drop(release);
                drop((cancel, http, user_ws));
                let _ = supervisor.shutdown_bounded(normal_bounds()).await;
                drop(directory);
            }
            "finalize" => {
                let (origin, time_server) = start_time_server(1).await;
                let (place_time_http, place_time_finalizer) = place_time_roles(&origin);
                let directory = stage_four();
                let roles = fresh_owner(directory.path())
                    .spawn(place_time_finalizer)
                    .unwrap();
                let (place, cancel, http, user_ws, supervisor) = roles.into_roles();
                let prepared = place
                    .prepare_place_once(sealed_place(proxy_order()))
                    .await
                    .unwrap();
                let identity = prepared.public_identity();
                let proof = place_time_http.fresh_place_time().await.unwrap();
                time_server.await.unwrap();
                let (admitted, admitted_receive) = std::sync::mpsc::sync_channel(0);
                let (release, release_receive) = std::sync::mpsc::channel();
                let task = tokio::spawn(prepared.finalize_place_once_paused_for_test(
                    identity,
                    AUTH_SECONDS,
                    proof,
                    PlaceRequestTestPause {
                        admitted,
                        release: release_receive,
                    },
                ));
                tokio::task::spawn_blocking(move || admitted_receive.recv().unwrap())
                    .await
                    .unwrap();
                task.abort();
                let _ = task.await;
                drop(release);
                drop((cancel, http, user_ws));
                let _ = supervisor.shutdown_bounded(normal_bounds()).await;
                drop(directory);
            }
            "cancel" => {
                let directory = stage_four();
                let roles = fresh_owner(directory.path())
                    .spawn(unused_place_time_finalizer())
                    .unwrap();
                let (place, mut cancel, http, user_ws, supervisor) = roles.into_roles();
                let (admitted, admitted_receive) = std::sync::mpsc::sync_channel(0);
                let (release, release_receive) = std::sync::mpsc::channel();
                let task = tokio::spawn(async move {
                    cancel
                        .admit_pause_for_cancellation_test(CancelRequestTestPause {
                            admitted,
                            release: release_receive,
                        })
                        .await
                });
                tokio::task::spawn_blocking(move || admitted_receive.recv().unwrap())
                    .await
                    .unwrap();
                task.abort();
                let _ = task.await;
                drop(release);
                drop((place, http, user_ws));
                let _ = supervisor.shutdown_bounded(normal_bounds()).await;
                drop(directory);
            }
            _ => panic!("unknown cancellation child case"),
        }
        panic!("dropping an admitted mutation future returned instead of aborting the process");
    }

    #[test]
    fn admitted_place_and_cancel_future_cancellation_abort_the_process() {
        if let Ok(case) = std::env::var(CANCELLATION_CHILD_CASE) {
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(3)
                .enable_all()
                .build()
                .unwrap();
            runtime.block_on(cancellation_child(&case));
            return;
        }

        let executable = std::env::current_exe().unwrap();
        for case in ["prepare", "finalize", "cancel"] {
            let status = Command::new(&executable)
                .arg("--exact")
                .arg(
                    "controlled_trial::authority::tests::admitted_place_and_cancel_future_cancellation_abort_the_process",
                )
                .arg("--nocapture")
                .env(CANCELLATION_CHILD_CASE, case)
                .status()
                .unwrap();
            assert_eq!(
                status.signal(),
                Some(libc::SIGABRT),
                "{case} cancellation must abort rather than unwind or detach",
            );
        }
    }

    #[test]
    fn cancel_task_mode_accepts_only_its_exact_durable_dispatch_class() {
        assert!(CancelAuthorityMode::FreshPrimary.accepts(PmCancelDispatchClassV1::Primary));
        assert!(!CancelAuthorityMode::RecoveryOnly.accepts(PmCancelDispatchClassV1::Primary));
        assert!(
            CancelAuthorityMode::RecoveryOnly
                .accepts(PmCancelDispatchClassV1::Recovery { ordinal: 1 })
        );
        assert!(
            !CancelAuthorityMode::FreshPrimary
                .accepts(PmCancelDispatchClassV1::Recovery { ordinal: 1 })
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn dropping_an_unpolled_prepare_future_preserves_the_sole_task_authority() {
        let directory = stage_four();
        let roles = fresh_owner(directory.path())
            .spawn(unused_place_time_finalizer())
            .unwrap();
        let (place, cancel, http, user_ws, supervisor) = roles.into_roles();
        let later = place.duplicate_for_task_gate();
        let never_polled = place.prepare_place_once(sealed_place(proxy_order()));
        drop(never_polled);
        let prepared = later
            .prepare_place_once(sealed_place(proxy_order()))
            .await
            .unwrap();
        assert_eq!(
            prepared.public_identity(),
            derive_place_public_request_identity(PmClobDomain::Standard, proxy_order())
        );
        drop((prepared, cancel, http, user_ws));
        let outcome = supervisor.shutdown_bounded(normal_bounds()).await.unwrap();
        assert!(outcome.task_completed_cleanly());
        assert!(all_staged_absent(directory.path()));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn fresh_authority_places_once_then_reads_and_binds_with_the_same_l2() {
        let public_identity = derive_place_public_request_identity(
            reap_polymarket_auth::PmClobDomain::Standard,
            proxy_order(),
        );
        let order_id = FixedOrderId::from(public_identity.expected_order_id());
        let open_page = parse_live_open_order_page(
            format!(
                r#"{{"data":[{}],"next_cursor":"LTE=","limit":128,"count":1}}"#,
                order_json(order_id, API_KEY),
            )
            .as_bytes(),
        )
        .unwrap();
        let trade_page = parse_live_trade_page(
            format!(
                r#"{{"data":[{}],"next_cursor":"LTE=","limit":128,"count":1}}"#,
                trade_json(order_id, API_KEY),
            )
            .as_bytes(),
        )
        .unwrap();
        let order_detail =
            parse_live_order_detail(order_json(order_id, API_KEY).as_bytes()).unwrap();
        let foreign_open_page = parse_live_open_order_page(
            format!(
                r#"{{"data":[{}],"next_cursor":"LTE=","limit":128,"count":1}}"#,
                order_json(order_id, FOREIGN_API_KEY),
            )
            .as_bytes(),
        )
        .unwrap();
        let condition = PmConditionId::parse(CONDITION).unwrap();
        let user_frame =
            parse_live_user_frame(user_frame_json(order_id, API_KEY).as_bytes()).unwrap();
        let foreign_user_frame =
            parse_live_user_frame(user_frame_json(order_id, FOREIGN_API_KEY).as_bytes()).unwrap();

        let (time_origin, time_server) = start_time_server(2).await;
        let (place_time_http, place_time_finalizer) = place_time_roles(&time_origin);
        let directory = stage_four();
        let roles = fresh_owner(directory.path())
            .spawn(place_time_finalizer)
            .unwrap();
        let (place, cancel, mut http, mut user_ws, mut supervisor) = roles.into_roles();
        let duplicate_place = place.duplicate_for_task_gate();

        let prepared_result = place.prepare_place_once(sealed_place(proxy_order())).await;
        let signer_dropped_before_reply = supervisor.signer_dropped_for_test();
        let second_place_result = duplicate_place
            .prepare_place_once(sealed_place(proxy_order()))
            .await;
        let staged_files_remain_before_prepared_or_shutdown =
            directory.path().join("private-key").exists()
                && directory.path().join("api-key").exists()
                && directory.path().join("l2-secret").exists()
                && directory.path().join("passphrase").exists();

        let prepared = prepared_result.unwrap();
        let prepared_public_identity = prepared.public_identity();
        let wrong_prepared_identity = derive_place_public_request_identity(
            reap_polymarket_auth::PmClobDomain::NegativeRisk,
            proxy_order(),
        );
        let wrong_key_removal =
            supervisor.remove_private_key_after_prepared_for_test(wrong_prepared_identity);
        let private_key_remains_after_wrong_identity =
            directory.path().join("private-key").exists();
        let key_removal =
            supervisor.remove_private_key_after_prepared_for_test(prepared_public_identity);
        let private_key_absent_after_explicit_removal =
            !directory.path().join("private-key").exists();
        let l2_files_remain_after_explicit_key_removal = directory.path().join("api-key").exists()
            && directory.path().join("l2-secret").exists()
            && directory.path().join("passphrase").exists();
        let mismatched_test_continuation = SignerDroppedPlacePreparation {
            public_identity: prepared_public_identity,
            sender: prepared.sender.clone(),
        };
        let wrong_time = place_time_http.fresh_place_time().await.unwrap();
        let dispatch_mismatch = mismatched_test_continuation
            .finalize_place_once_for_test(wrong_prepared_identity, AUTH_SECONDS, wrong_time)
            .await;
        let exact_time = place_time_http
            .fresh_place_time_observation()
            .await
            .unwrap();
        assert_eq!(exact_time.observed_l2_timestamp_seconds(), AUTH_SECONDS);
        let place_result = prepared
            .finalize_place_once_for_test(
                prepared_public_identity,
                AUTH_SECONDS,
                exact_time.into_proof(),
            )
            .await;
        time_server.await.unwrap();
        let place_capture = place_result.map(|place| {
            let mut capture = MutationCapture::default();
            place.request.dispatch(&mut capture).unwrap();
            capture
        });

        let open_headers = capture_headers(http.authenticate_open_orders(timestamp()).await);
        let trade_headers = capture_headers(http.authenticate_trades(timestamp()).await);
        let balance_headers =
            capture_headers(http.authenticate_balance_allowance(timestamp()).await);
        let closed_headers = capture_headers(http.authenticate_closed_only(timestamp()).await);
        let exact_headers = http
            .authenticate_exact_owned_order(SealedExactOwnedOrderReadAuthentication::new(
                order_id,
                timestamp(),
            ))
            .await
            .map(|authenticated| {
                let (headers, authenticated_order_id, authorized_timestamp) =
                    authenticated.into_parts();
                let mut capture = HeaderCapture::default();
                headers.apply_to(&mut capture).unwrap();
                (capture, authenticated_order_id, authorized_timestamp)
            });

        let bound_open_count = http
            .bind_open_orders(open_page)
            .await
            .map(|page| page.orders().len());
        let bound_trade_count = http
            .bind_trades(trade_page)
            .await
            .map(|page| page.trades().len());
        let bound_order_id = http
            .bind_exact_order(order_detail)
            .await
            .map(|order| order.id());
        let foreign_open_result = http.bind_open_orders(foreign_open_page).await;

        let subscription_result = user_ws
            .user_subscription(condition)
            .await
            .map(|subscription| {
                let mut capture = FrameCapture::default();
                subscription.dispatch(&mut capture).unwrap();
                capture
            });
        let bound_user_count = user_ws
            .bind_user_frame(user_frame)
            .await
            .map(|frame| frame.events().len());
        let foreign_user_result = user_ws.bind_user_frame(foreign_user_frame).await;

        drop(cancel);
        let shutdown = supervisor.shutdown_bounded(normal_bounds()).await;
        let staged_absent = all_staged_absent(directory.path());

        assert!(signer_dropped_before_reply);
        assert_eq!(
            second_place_result.unwrap_err(),
            CredentialAuthorityError::PlaceAlreadyConsumed
        );
        assert!(staged_files_remain_before_prepared_or_shutdown);
        assert_eq!(
            wrong_key_removal.unwrap_err(),
            CredentialAuthorityError::PlacePreparedIdentityMismatch
        );
        assert!(private_key_remains_after_wrong_identity);
        assert_eq!(key_removal, Ok(()));
        assert!(private_key_absent_after_explicit_removal);
        assert!(l2_files_remain_after_explicit_key_removal);
        assert_eq!(
            dispatch_mismatch.unwrap_err(),
            CredentialAuthorityError::PlaceDispatchBindingMismatch
        );

        assert_eq!(prepared_public_identity, public_identity);
        let place_capture = place_capture.unwrap();
        assert_eq!(place_capture.address, SIGNER);
        assert_eq!(place_capture.timestamp, AUTH_SECONDS.to_string());
        assert_eq!(place_capture.api_key, API_KEY);
        let place_body = String::from_utf8(place_capture.body).unwrap();
        assert!(place_body.contains(&format!(r#""maker":"{PROXY}""#)));
        assert!(place_body.contains(&format!(r#""signer":"{SIGNER}""#)));
        assert!(place_body.contains(r#""signatureType":1"#));

        let headers = [
            open_headers.unwrap(),
            trade_headers.unwrap(),
            balance_headers.unwrap(),
            closed_headers.unwrap(),
        ];
        for header in &headers {
            assert_eq!(header.address, SIGNER);
            assert_eq!(header.timestamp, AUTH_SECONDS.to_string());
            assert_eq!(header.api_key, API_KEY);
            assert_eq!(header.passphrase, PASSPHRASE);
        }
        let mut signatures = headers
            .into_iter()
            .map(|header| header.signature)
            .collect::<Vec<_>>();
        let (exact_header, exact_order_id, exact_timestamp) = exact_headers.unwrap();
        assert_eq!(exact_order_id, order_id);
        assert_eq!(exact_timestamp.unix_seconds(), AUTH_SECONDS);
        signatures.push(exact_header.signature);
        signatures.sort();
        signatures.dedup();
        assert_eq!(signatures.len(), 5);
        assert_eq!(bound_open_count, Ok(1));
        assert_eq!(bound_trade_count, Ok(1));
        assert_eq!(
            bound_order_id.map(|id| id.to_string()),
            Ok(order_id.to_string())
        );
        assert_eq!(bound_user_count, Ok(1));
        assert_eq!(
            foreign_open_result.unwrap_err(),
            CredentialAuthorityError::CredentialOwnerMismatch,
        );
        assert!(matches!(
            foreign_user_result,
            Err(CredentialAuthorityError::Auth(_))
        ));

        let subscription = subscription_result.unwrap();
        let subscription = String::from_utf8(subscription.0).unwrap();
        assert!(subscription.contains(API_KEY));
        assert!(subscription.contains(CONDITION));

        let shutdown = shutdown.unwrap();
        assert!(shutdown.shutdown_requested());
        assert!(!shutdown.abort_requested());
        assert!(shutdown.task_joined());
        assert!(shutdown.task_completed_cleanly());
        assert!(shutdown.credentials_dropped());
        assert!(shutdown.staged_l2_files_removed());
        assert!(staged_absent);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn fresh_no_place_shutdown_joins_then_removes_all_four_files() {
        let directory = stage_four();
        let roles = fresh_owner(directory.path())
            .spawn(unused_place_time_finalizer())
            .unwrap();
        let (place, cancel, http, user_ws, mut supervisor) = roles.into_roles();
        let signer_was_owned = !supervisor.signer_dropped_for_test();
        let premature_key_removal = supervisor.remove_private_key_after_prepared_for_test(
            derive_place_public_request_identity(
                reap_polymarket_auth::PmClobDomain::Standard,
                proxy_order(),
            ),
        );
        let all_staged_remain_after_premature_removal =
            ["private-key", "api-key", "l2-secret", "passphrase"]
                .iter()
                .all(|name| directory.path().join(name).exists());
        drop((place, cancel, http, user_ws));
        let shutdown = supervisor.shutdown_bounded(normal_bounds()).await;
        let staged_absent = all_staged_absent(directory.path());

        assert!(signer_was_owned);
        assert_eq!(
            premature_key_removal.unwrap_err(),
            CredentialAuthorityError::SignerStillOwned
        );
        assert!(all_staged_remain_after_premature_removal);
        let shutdown = shutdown.unwrap();
        assert!(shutdown.task_completed_cleanly());
        assert!(shutdown.credentials_dropped());
        assert!(shutdown.staged_l2_files_removed());
        assert!(staged_absent);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn failed_place_consumes_and_drops_signer_then_shutdown_removes_all_files() {
        let invalid_place = sealed_place(eoa_order());
        let directory = stage_four();
        let roles = fresh_owner(directory.path())
            .spawn(unused_place_time_finalizer())
            .unwrap();
        let (place, cancel, http, user_ws, mut supervisor) = roles.into_roles();
        let place_result = place.prepare_place_once(invalid_place).await;
        let signer_dropped_before_reply = supervisor.signer_dropped_for_test();
        let key_removal = supervisor.remove_private_key_after_prepared_for_test(
            derive_place_public_request_identity(
                reap_polymarket_auth::PmClobDomain::Standard,
                proxy_order(),
            ),
        );
        drop((cancel, http, user_ws));
        let shutdown = supervisor.shutdown_bounded(normal_bounds()).await;
        let staged_absent = all_staged_absent(directory.path());

        assert_eq!(
            place_result.unwrap_err(),
            CredentialAuthorityError::PlaceProfileMismatch
        );
        assert!(signer_dropped_before_reply);
        assert_eq!(
            key_removal.unwrap_err(),
            CredentialAuthorityError::PlacePreparedIdentityUnavailable
        );
        assert!(shutdown.unwrap().task_completed_cleanly());
        assert!(staged_absent);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn recovery_authority_has_cancel_and_read_roles_but_no_signer_or_key_custody() {
        let condition = PmConditionId::parse(CONDITION).unwrap();
        let directory = stage_four();
        let roles = recovery_owner(directory.path()).spawn().unwrap();
        let (cancel, mut http, mut user_ws, supervisor) = roles.into_roles();
        let read_result = capture_headers(http.authenticate_closed_only(timestamp()).await);
        let subscription_result = user_ws.user_subscription(condition).await;
        drop((read_result, subscription_result, cancel, http, user_ws));
        let shutdown = supervisor.shutdown_bounded(normal_bounds()).await;
        let private_key_exists = directory.path().join("private-key").exists();
        let l2_absent = ["api-key", "l2-secret", "passphrase"]
            .iter()
            .all(|name| !directory.path().join(name).exists());

        let shutdown = shutdown.unwrap();
        assert!(shutdown.task_completed_cleanly());
        assert!(shutdown.credentials_dropped());
        assert!(shutdown.staged_l2_files_removed());
        assert!(private_key_exists);
        assert!(l2_absent);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn external_read_provider_traits_share_the_recovery_authority_and_fail_owner_mismatch() {
        let condition = PmConditionId::parse(CONDITION).unwrap();
        let order_id = FixedOrderId::parse(
            "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        )
        .unwrap();
        let foreign_user_frame =
            parse_live_user_frame(user_frame_json(order_id, FOREIGN_API_KEY).as_bytes()).unwrap();
        let directory = stage_four();
        let roles = recovery_owner(directory.path()).spawn().unwrap();
        let (cancel, mut http, mut user_ws, supervisor) = roles.into_roles();

        let headers = PmHttpReadAuthorityProvider::authenticate_closed_only(
            &mut http,
            L2Timestamp::from_unix_seconds(AUTH_SECONDS).unwrap(),
        )
        .await
        .unwrap();
        let mut header_capture = HeaderCapture::default();
        headers.apply_to(&mut header_capture).unwrap();
        let subscription =
            PmUserWsReadAuthorityProvider::authenticate_user_subscription(&mut user_ws, condition)
                .await
                .unwrap();
        let mut frame_capture = FrameCapture::default();
        subscription.dispatch(&mut frame_capture).unwrap();
        let foreign =
            PmUserWsReadAuthorityProvider::bind_user_frame(&mut user_ws, foreign_user_frame).await;

        assert_eq!(header_capture.address, SIGNER);
        assert_eq!(header_capture.api_key, API_KEY);
        assert!(
            String::from_utf8(frame_capture.0)
                .unwrap()
                .contains(API_KEY)
        );
        assert!(matches!(foreign, Err(PmLiveAdapterError::Auth(_))));

        drop((cancel, http, user_ws));
        assert!(
            supervisor
                .shutdown_bounded(normal_bounds())
                .await
                .unwrap()
                .credentials_dropped()
        );
        assert!(directory.path().join("private-key").exists());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn graceful_timeout_aborts_joins_then_tears_down_recovery_l2_files() {
        let directory = stage_four();
        let (credentials, teardown) =
            recovery_handoff(directory.path()).into_authority_and_teardown();
        let (shutdown, shutdown_receiver) = oneshot::channel();
        let task = tokio::spawn(async move {
            let _held_until_abort = (credentials, shutdown_receiver);
            std::future::pending::<()>().await;
        });
        let supervisor = RecoveryCredentialAuthoritySupervisor {
            task: ArmedTaskSupervisor::new(shutdown, task),
            teardown,
        };
        let bounds =
            CredentialAuthorityShutdownBounds::new(Duration::from_nanos(1), Duration::from_secs(2))
                .unwrap();
        let shutdown = supervisor.shutdown_bounded(bounds).await;
        let private_key_exists = directory.path().join("private-key").exists();
        let l2_absent = ["api-key", "l2-secret", "passphrase"]
            .iter()
            .all(|name| !directory.path().join(name).exists());

        let shutdown = shutdown.unwrap();
        assert!(shutdown.shutdown_requested());
        assert!(shutdown.abort_requested());
        assert!(shutdown.task_joined());
        assert!(!shutdown.task_completed_cleanly());
        assert!(shutdown.credentials_dropped());
        assert!(shutdown.staged_l2_files_removed());
        assert!(private_key_exists);
        assert!(l2_absent);
    }
}
