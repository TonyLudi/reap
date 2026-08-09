use std::{
    fmt,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use reap_pm_core::PmConditionId;
use reap_polymarket_auth::{
    AuthenticatedL2Headers, AuthenticatedPlaceRequest, AuthenticatedUserSubscription,
    CredentialOwnedUserFrame, FixedEoaSigner, L2Credentials, L2Timestamp, PmAuthError,
    derive_place_public_request_identity,
};
use reap_polymarket_wire::{
    PmClobV2SignatureType, PmLiveOpenOrderPage, PmLiveOrder, PmLiveTradePage, PmLiveUserFrame,
};
use thiserror::Error;
use tokio::{
    sync::{mpsc, oneshot},
    task::JoinHandle,
};

use super::{
    AuthenticatedExactOwnedCancel, AuthenticatedExactOwnedOrderRead, AuthorizedL2Timestamp,
    SealedExactOwnedCancelAuthentication, SealedExactOwnedOrderReadAuthentication,
    SealedFreshPlaceAuthentication, SignerDroppedAuthenticatedPlace,
};
use crate::credentials::{
    FreshPlaceCredentialHandoff, FreshPlaceCredentialTeardown, RecoveryOnlyCredentialHandoff,
    RecoveryOnlyCredentialTeardown,
};

const COMMON_AUTHORITY_CAPACITY: usize = 8;
const PLACE_AUTHORITY_CAPACITY: usize = 1;
const MAX_EXACT_CANCEL_AUTHENTICATIONS_PER_AUTHORITY: u8 = 3;
const MAX_JOIN_TIMEOUT: Duration = Duration::from_secs(60);

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
    #[error("fresh-place authentication requires the closed PM-T2 proxy signature profile")]
    PlaceProfileMismatch,
    #[error("fresh-place public identity does not match the sealed unsigned order and domain")]
    PlacePublicIdentityMismatch,
    #[error("the L2 signer does not match the sealed proxy order signer")]
    PlaceSignerMismatch,
    #[error("the fixed local exact-cancel authentication ceiling was exhausted")]
    CancelAuthenticationBudgetExhausted,
    #[error("the authenticated response owner does not match the sole L2 bundle")]
    CredentialOwnerMismatch,
    #[error("the task-local EOA signer has not yet been destroyed")]
    SignerStillOwned,
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
}

impl AuthorityIdentity {
    fn new() -> Self {
        Self {
            signer_dropped: AtomicBool::new(false),
        }
    }

    fn signer_dropped(&self) -> bool {
        self.signer_dropped.load(Ordering::Acquire)
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
    pub(super) const fn from_custody(custody: FreshPlaceCredentialHandoff) -> Self {
        Self { custody }
    }

    pub(super) fn spawn(self) -> Result<FreshCredentialAuthorityRoles, CredentialAuthorityError> {
        let runtime = tokio::runtime::Handle::try_current()
            .map_err(|_| CredentialAuthorityError::ActiveRuntimeRequired)?;
        let (signer, credentials, teardown) = self.custody.into_authorities_and_teardown();
        let identity = Arc::new(AuthorityIdentity::new());
        let (common_sender, common_receiver) = mpsc::channel(COMMON_AUTHORITY_CAPACITY);
        let (place_sender, place_receiver) = mpsc::channel(PLACE_AUTHORITY_CAPACITY);
        let (shutdown, shutdown_receiver) = oneshot::channel();
        let task = runtime.spawn(run_fresh_authority(
            TaskSignerCustody::new(signer, Arc::clone(&identity)),
            credentials,
            common_receiver,
            place_receiver,
            shutdown_receiver,
        ));
        let task = ArmedTaskSupervisor::new(shutdown, task);

        Ok(FreshCredentialAuthorityRoles {
            place: FreshPlaceAuthenticationOnce {
                sender: place_sender,
            },
            cancel: ExactOwnedCancelAuthenticationRole::new(common_sender.clone()),
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
    pub(super) const fn from_custody(custody: RecoveryOnlyCredentialHandoff) -> Self {
        Self { custody }
    }

    pub(super) fn spawn(
        self,
    ) -> Result<RecoveryCredentialAuthorityRoles, CredentialAuthorityError> {
        let runtime = tokio::runtime::Handle::try_current()
            .map_err(|_| CredentialAuthorityError::ActiveRuntimeRequired)?;
        let (credentials, teardown) = self.custody.into_authority_and_teardown();
        let (common_sender, common_receiver) = mpsc::channel(COMMON_AUTHORITY_CAPACITY);
        let (shutdown, shutdown_receiver) = oneshot::channel();
        let task = runtime.spawn(run_recovery_authority(
            credentials,
            common_receiver,
            shutdown_receiver,
        ));
        let task = ArmedTaskSupervisor::new(shutdown, task);

        Ok(RecoveryCredentialAuthorityRoles {
            cancel: ExactOwnedCancelAuthenticationRole::new(common_sender.clone()),
            http: FixedHttpAuthenticationRole {
                sender: common_sender.clone(),
            },
            user_ws: FixedUserWsAuthenticationRole {
                sender: common_sender,
            },
            supervisor: RecoveryCredentialAuthoritySupervisor { task, teardown },
        })
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
    sender: mpsc::Sender<PlaceAuthenticationRequest>,
}

impl FreshPlaceAuthenticationOnce {
    pub(super) async fn authenticate_place_once(
        self,
        request: SealedFreshPlaceAuthentication,
    ) -> Result<SignerDroppedAuthenticatedPlace, CredentialAuthorityError> {
        request_place(&self.sender, request).await
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

/// Locally bounded exact-owned cancel authenticator. Durable primary/recovery
/// budgets remain an upper journal responsibility; this independent ceiling
/// prevents an unbounded in-process caller loop.
pub(super) struct ExactOwnedCancelAuthenticationRole {
    sender: mpsc::Sender<CommonAuthorityRequest>,
    remaining_attempts: u8,
}

impl ExactOwnedCancelAuthenticationRole {
    fn new(sender: mpsc::Sender<CommonAuthorityRequest>) -> Self {
        Self {
            sender,
            remaining_attempts: MAX_EXACT_CANCEL_AUTHENTICATIONS_PER_AUTHORITY,
        }
    }

    pub(super) async fn authenticate_exact_owned_cancel(
        &mut self,
        request: SealedExactOwnedCancelAuthentication,
    ) -> Result<AuthenticatedExactOwnedCancel, CredentialAuthorityError> {
        if self.remaining_attempts == 0 {
            return Err(CredentialAuthorityError::CancelAuthenticationBudgetExhausted);
        }
        self.remaining_attempts -= 1;
        request_common(&self.sender, |response| CommonAuthorityRequest::Cancel {
            request,
            response,
        })
        .await
    }
}

impl fmt::Debug for ExactOwnedCancelAuthenticationRole {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ExactOwnedCancelAuthenticationRole(<opaque>)")
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
    /// that durable ack; it only requires proof that the task-local signer was
    /// destroyed before the place reply and reports staged-file teardown.
    ///
    /// TODO(A3 runner integration): make the private session owner consume
    /// its durable Prepared ack immediately before invoking this method.
    pub(super) fn remove_private_key_after_prepared(
        &mut self,
    ) -> Result<(), CredentialAuthorityError> {
        if !self.identity.signer_dropped() {
            return Err(CredentialAuthorityError::SignerStillOwned);
        }
        self.teardown
            .remove_private_key()
            .map_err(|_| CredentialAuthorityError::StagedCredentialTeardownFailed)?;
        self.private_key_removed = true;
        Ok(())
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

struct PlaceAuthenticationRequest {
    request: SealedFreshPlaceAuthentication,
    response: oneshot::Sender<Result<SignerDroppedAuthenticatedPlace, CredentialAuthorityError>>,
}

enum CommonAuthorityRequest {
    Cancel {
        request: SealedExactOwnedCancelAuthentication,
        response: oneshot::Sender<Result<AuthenticatedExactOwnedCancel, CredentialAuthorityError>>,
    },
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

async fn run_fresh_authority(
    mut signer: TaskSignerCustody,
    credentials: L2Credentials,
    mut common: mpsc::Receiver<CommonAuthorityRequest>,
    mut place: mpsc::Receiver<PlaceAuthenticationRequest>,
    mut shutdown: oneshot::Receiver<()>,
) {
    let mut common_open = true;
    let mut place_open = true;
    loop {
        tokio::select! {
            biased;
            _ = &mut shutdown => {
                common.close();
                place.close();
                return;
            }
            request = common.recv(), if common_open => match request {
                Some(request) => handle_common_request(&credentials, request),
                None => common_open = false,
            },
            request = place.recv(), if place_open => match request {
                Some(request) => respond_place(request, &mut signer, &credentials),
                None => place_open = false,
            },
            else => return,
        }
    }
}

// BEGIN RECOVERY_TASK
async fn run_recovery_authority(
    credentials: L2Credentials,
    mut common: mpsc::Receiver<CommonAuthorityRequest>,
    mut shutdown: oneshot::Receiver<()>,
) {
    loop {
        tokio::select! {
            biased;
            _ = &mut shutdown => {
                common.close();
                return;
            }
            request = common.recv() => {
                let Some(request) = request else {
                    return;
                };
                handle_common_request(&credentials, request);
            }
        }
    }
}
// END RECOVERY_TASK

fn respond_place(
    request: PlaceAuthenticationRequest,
    signer: &mut TaskSignerCustody,
    credentials: &L2Credentials,
) {
    let PlaceAuthenticationRequest { request, response } = request;
    let Some(signer_value) = signer.take() else {
        let _ = response.send(Err(CredentialAuthorityError::PlaceAlreadyConsumed));
        return;
    };
    let timestamp = request.timestamp.into_inner();
    let authenticated = authenticate_place(&signer_value, credentials, request, timestamp);
    drop(signer_value);
    signer.publish_signer_dropped();
    // Constructing and sending the reply happens strictly after the signer
    // value is dropped and that fact is published with Release ordering.
    let value =
        authenticated.map(|request| SignerDroppedAuthenticatedPlace::new(request, timestamp));
    let _ = response.send(value);
}

fn authenticate_place(
    signer: &FixedEoaSigner,
    credentials: &L2Credentials,
    request: SealedFreshPlaceAuthentication,
    timestamp: L2Timestamp,
) -> Result<AuthenticatedPlaceRequest, CredentialAuthorityError> {
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
    let authenticated = credentials.authenticate_place(timestamp, serialized)?;
    if authenticated.expected_order_id() != derived.expected_order_id()
        || authenticated.semantic_request_commitment() != derived.semantic_request_commitment()
    {
        return Err(CredentialAuthorityError::PlacePublicIdentityMismatch);
    }
    Ok(authenticated)
}

fn handle_common_request(credentials: &L2Credentials, request: CommonAuthorityRequest) {
    match request {
        CommonAuthorityRequest::Cancel { request, response } => {
            let timestamp = request.timestamp.into_inner();
            let value = credentials
                .serialize_owned_cancel(request.order_id)
                .and_then(|serialized| credentials.authenticate_owned_cancel(timestamp, serialized))
                .map(|authenticated| AuthenticatedExactOwnedCancel::new(authenticated, timestamp))
                .map_err(Into::into);
            let _ = response.send(value);
        }
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

async fn request_place(
    sender: &mpsc::Sender<PlaceAuthenticationRequest>,
    request: SealedFreshPlaceAuthentication,
) -> Result<SignerDroppedAuthenticatedPlace, CredentialAuthorityError> {
    let (response, receive) = oneshot::channel();
    sender
        .try_send(PlaceAuthenticationRequest { request, response })
        .map_err(classify_place_send)?;
    receive
        .await
        .map_err(|_| CredentialAuthorityError::AuthorityClosed)?
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
    error: mpsc::error::TrySendError<PlaceAuthenticationRequest>,
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

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use std::{
        convert::Infallible, fs, os::unix::fs::PermissionsExt as _, path::Path, time::Duration,
    };

    use reap_pm_core::{
        EvmAddress, PmOrderSalt, PmOrderSide, PmPrice, PmQuantity, PmTick, PmTokenId, U256,
    };
    use reap_polymarket_auth::{
        AuthenticatedUserSubscriptionSink, EoaAddress, FixedOrderId, FixedOwnedCancelRequestSink,
        FixedPlaceRequestSink, L2HeaderSink,
    };
    use reap_polymarket_wire::{
        PmUnsignedClobV2Order, parse_live_open_order_page, parse_live_order_detail,
        parse_live_trade_page, parse_live_user_frame,
    };
    use tempfile::TempDir;

    use super::*;
    use crate::{
        controlled_trial::{
            AuthorizedL2Timestamp, SealedExactOwnedCancelAuthentication,
            SealedExactOwnedOrderReadAuthentication, SealedFreshPlaceAuthentication,
        },
        credentials::{FreshPlaceCredentialFiles, RecoveryOnlyCredentialFiles},
    };

    const KEY: &str = "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";
    const SIGNER: &str = "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266";
    const PROXY: &str = "0x70997970C51812dc3A010C7d01b50e0d17dc79C8";
    const API_KEY: &str = "00000000-0000-4000-8000-000000000001";
    const FOREIGN_API_KEY: &str = "00000000-0000-4000-8000-000000000002";
    const L2_SECRET: &str = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";
    const PASSPHRASE: &str = "synthetic-passphrase";
    const CONDITION: &str = "0x1111111111111111111111111111111111111111111111111111111111111111";
    const AUTH_SECONDS: u64 = 1_780_449_126;

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
        let custody = FreshPlaceCredentialFiles::new(
            directory.to_owned(),
            "private-key".into(),
            "api-key".into(),
            "l2-secret".into(),
            "passphrase".into(),
        )
        .load(configured_eoa())
        .unwrap();
        FreshCredentialAuthorityOwner::from_custody(custody)
    }

    fn recovery_owner(directory: &Path) -> RecoveryCredentialAuthorityOwner {
        let custody = recovery_handoff(directory);
        RecoveryCredentialAuthorityOwner::from_custody(custody)
    }

    fn recovery_handoff(directory: &Path) -> RecoveryOnlyCredentialHandoff {
        RecoveryOnlyCredentialFiles::new(
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

    fn sealed_place(order: PmUnsignedClobV2Order) -> SealedFreshPlaceAuthentication {
        let domain = reap_polymarket_auth::PmClobDomain::Standard;
        let identity = derive_place_public_request_identity(domain, order);
        SealedFreshPlaceAuthentication::new(domain, order, timestamp(), identity)
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

    impl FixedOwnedCancelRequestSink for MutationCapture {
        type Output = ();
        type Error = Infallible;

        fn send_exact_owned_cancel(
            &mut self,
            poly_address: &str,
            _poly_signature: &str,
            poly_timestamp: &str,
            poly_api_key: &str,
            _poly_passphrase: &str,
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

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn fresh_authority_places_once_then_reads_binds_and_cancels_with_the_same_l2() {
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

        let directory = stage_four();
        let roles = fresh_owner(directory.path()).spawn().unwrap();
        let (place, mut cancel, mut http, mut user_ws, supervisor) = roles.into_roles();
        let duplicate_place = place.duplicate_for_task_gate();

        let place_result = place
            .authenticate_place_once(sealed_place(proxy_order()))
            .await;
        let signer_dropped_before_reply = supervisor.signer_dropped_for_test();
        let second_place_result = duplicate_place
            .authenticate_place_once(sealed_place(proxy_order()))
            .await;
        let staged_files_remain_before_prepared_or_shutdown =
            directory.path().join("private-key").exists()
                && directory.path().join("api-key").exists()
                && directory.path().join("l2-secret").exists()
                && directory.path().join("passphrase").exists();

        let place_capture = place_result.map(|place| {
            let (request, authorized_timestamp) = place.into_parts();
            let mut capture = MutationCapture::default();
            request.dispatch(&mut capture).unwrap();
            (capture, authorized_timestamp)
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

        let first_cancel = cancel
            .authenticate_exact_owned_cancel(SealedExactOwnedCancelAuthentication::new(
                order_id,
                timestamp(),
            ))
            .await
            .map(|cancel| {
                let (request, authorized_timestamp) = cancel.into_parts();
                let mut capture = MutationCapture::default();
                request.dispatch(&mut capture).unwrap();
                (capture, authorized_timestamp)
            });
        let second_cancel = cancel
            .authenticate_exact_owned_cancel(SealedExactOwnedCancelAuthentication::new(
                order_id,
                timestamp(),
            ))
            .await;
        let third_cancel = cancel
            .authenticate_exact_owned_cancel(SealedExactOwnedCancelAuthentication::new(
                order_id,
                timestamp(),
            ))
            .await;
        let fourth_cancel = cancel
            .authenticate_exact_owned_cancel(SealedExactOwnedCancelAuthentication::new(
                order_id,
                timestamp(),
            ))
            .await;
        drop(second_cancel);
        drop(third_cancel);

        let shutdown = supervisor.shutdown_bounded(normal_bounds()).await;
        let staged_absent = all_staged_absent(directory.path());

        assert!(signer_dropped_before_reply);
        assert_eq!(
            second_place_result.unwrap_err(),
            CredentialAuthorityError::PlaceAlreadyConsumed
        );
        assert!(staged_files_remain_before_prepared_or_shutdown);

        let (place_capture, place_timestamp) = place_capture.unwrap();
        assert_eq!(place_timestamp.unix_seconds(), AUTH_SECONDS);
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

        let (cancel_capture, cancel_timestamp) = first_cancel.unwrap();
        assert_eq!(cancel_timestamp.unix_seconds(), AUTH_SECONDS);
        assert_eq!(cancel_capture.address, SIGNER);
        assert_eq!(cancel_capture.api_key, API_KEY);
        assert_eq!(
            cancel_capture.body,
            format!(r#"{{"orderID":"{order_id}"}}"#).as_bytes(),
        );
        assert_eq!(
            fourth_cancel.unwrap_err(),
            CredentialAuthorityError::CancelAuthenticationBudgetExhausted
        );

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
        let roles = fresh_owner(directory.path()).spawn().unwrap();
        let (place, cancel, http, user_ws, supervisor) = roles.into_roles();
        let signer_was_owned = !supervisor.signer_dropped_for_test();
        drop((place, cancel, http, user_ws));
        let shutdown = supervisor.shutdown_bounded(normal_bounds()).await;
        let staged_absent = all_staged_absent(directory.path());

        assert!(signer_was_owned);
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
        let roles = fresh_owner(directory.path()).spawn().unwrap();
        let (place, cancel, http, user_ws, supervisor) = roles.into_roles();
        let place_result = place.authenticate_place_once(invalid_place).await;
        let signer_dropped_before_reply = supervisor.signer_dropped_for_test();
        drop((cancel, http, user_ws));
        let shutdown = supervisor.shutdown_bounded(normal_bounds()).await;
        let staged_absent = all_staged_absent(directory.path());

        assert_eq!(
            place_result.unwrap_err(),
            CredentialAuthorityError::PlaceProfileMismatch
        );
        assert!(signer_dropped_before_reply);
        assert!(shutdown.unwrap().task_completed_cleanly());
        assert!(staged_absent);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn recovery_authority_has_l2_cancel_and_read_roles_but_leaves_unowned_key_untouched() {
        let order_id = FixedOrderId::from(
            derive_place_public_request_identity(
                reap_polymarket_auth::PmClobDomain::Standard,
                proxy_order(),
            )
            .expected_order_id(),
        );
        let condition = PmConditionId::parse(CONDITION).unwrap();
        let directory = stage_four();
        let roles = recovery_owner(directory.path()).spawn().unwrap();
        let (mut cancel, mut http, mut user_ws, supervisor) = roles.into_roles();
        let cancel_result = cancel
            .authenticate_exact_owned_cancel(SealedExactOwnedCancelAuthentication::new(
                order_id,
                timestamp(),
            ))
            .await;
        let read_result = capture_headers(http.authenticate_closed_only(timestamp()).await);
        let subscription_result = user_ws.user_subscription(condition).await;
        drop((
            cancel_result,
            read_result,
            subscription_result,
            cancel,
            http,
            user_ws,
        ));
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
