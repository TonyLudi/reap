use std::{fmt, time::Duration};

use async_trait::async_trait;
use reap_pm_core::EvmAddress;
use reap_polymarket_auth::{
    AuthenticatedL2Headers, AuthenticatedUserSubscription, CredentialOwnedUserFrame, FixedOrderId,
    L2Credentials, L2Timestamp,
};
use reap_polymarket_wire::{PmLiveOpenOrderPage, PmLiveOrder, PmLiveTradePage, PmLiveUserFrame};
use tokio::{
    sync::{mpsc, oneshot},
    task::JoinHandle,
};

use crate::{
    PmAuthenticatedHttpOwner, PmLiveAdapterError, PmPrivateHttpConfig, PmReadOnlySignatureType,
    PmUserWsConfig,
    private_http::PmPrivateHttpTransport,
    read_authority::{PmHttpReadAuthorityProvider, PmUserWsReadAuthorityProvider},
    user_ws::PmAuthenticatedUserWsRole,
};

const CREDENTIAL_AUTHORITY_CAPACITY: usize = 32;
const MAX_CREDENTIAL_AUTHORITY_JOIN_TIMEOUT: Duration = Duration::from_secs(60);

/// Fixed shutdown bounds used by ordinary composition teardown.
pub const PM_CREDENTIAL_AUTHORITY_DEFAULT_SHUTDOWN_BOUNDS: PmCredentialAuthorityShutdownBounds =
    PmCredentialAuthorityShutdownBounds::fixed(Duration::from_secs(30), Duration::from_secs(5));

/// Short, fixed shutdown bounds used by the credentialed read-only smoke.
pub const PM_CREDENTIAL_AUTHORITY_READ_ONLY_SHUTDOWN_BOUNDS: PmCredentialAuthorityShutdownBounds =
    PmCredentialAuthorityShutdownBounds::fixed(Duration::from_secs(5), Duration::from_secs(5));

/// Validated upper bounds for graceful credential-authority join and the
/// follow-up join after abort is requested.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PmCredentialAuthorityShutdownBounds {
    graceful_join_timeout: Duration,
    abort_join_timeout: Duration,
}

impl PmCredentialAuthorityShutdownBounds {
    const fn fixed(graceful_join_timeout: Duration, abort_join_timeout: Duration) -> Self {
        Self {
            graceful_join_timeout,
            abort_join_timeout,
        }
    }

    pub fn new(
        graceful_join_timeout: Duration,
        abort_join_timeout: Duration,
    ) -> Result<Self, PmLiveAdapterError> {
        if graceful_join_timeout.is_zero()
            || abort_join_timeout.is_zero()
            || graceful_join_timeout > MAX_CREDENTIAL_AUTHORITY_JOIN_TIMEOUT
            || abort_join_timeout > MAX_CREDENTIAL_AUTHORITY_JOIN_TIMEOUT
        {
            return Err(PmLiveAdapterError::InvalidConfiguration(
                "credential-authority join timeouts must each be within 1ns..=60s",
            ));
        }
        Ok(Self::fixed(graceful_join_timeout, abort_join_timeout))
    }

    #[must_use]
    pub const fn graceful_join_timeout(self) -> Duration {
        self.graceful_join_timeout
    }

    #[must_use]
    pub const fn abort_join_timeout(self) -> Duration {
        self.abort_join_timeout
    }
}

/// Truthful result of bounded credential-authority teardown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PmCredentialAuthorityShutdownOutcome {
    shutdown_requested: bool,
    abort_requested: bool,
    task_joined: bool,
    task_completed_cleanly: bool,
    credentials_dropped: bool,
}

impl PmCredentialAuthorityShutdownOutcome {
    #[must_use]
    pub const fn shutdown_requested(self) -> bool {
        self.shutdown_requested
    }

    #[must_use]
    pub const fn abort_requested(self) -> bool {
        self.abort_requested
    }

    #[must_use]
    pub const fn task_joined(self) -> bool {
        self.task_joined
    }

    #[must_use]
    pub const fn task_completed_cleanly(self) -> bool {
        self.task_completed_cleanly
    }

    /// A completed join, including a cancelled or panicked task, proves that
    /// the secret-owning future and its credential bundle were dropped.
    #[must_use]
    pub const fn credentials_dropped(self) -> bool {
        self.credentials_dropped
    }
}

/// Sole owner of one account's L2 credential bundle and its exact private
/// transport configuration.
///
/// Splitting consumes this value exactly once. The resulting roles share only
/// a bounded typed request channel; credentials never leave the authority
/// task. The returned supervisor must be retained and explicitly joined so
/// secret destruction is part of composition teardown evidence.
pub struct PmPrivateConnectivityOwner {
    http_config: PmPrivateHttpConfig,
    user_ws_config: PmUserWsConfig,
    credentials: L2Credentials,
    expected_order_maker: EvmAddress,
    balance_signature_type: PmReadOnlySignatureType,
}

impl PmPrivateConnectivityOwner {
    pub fn new(
        http_config: PmPrivateHttpConfig,
        user_ws_config: PmUserWsConfig,
        credentials: L2Credentials,
    ) -> Result<Self, PmLiveAdapterError> {
        if http_config.exact_order_scope().condition() != user_ws_config.condition() {
            return Err(PmLiveAdapterError::InvalidConfiguration(
                "private HTTP and user WebSocket must bind the same condition",
            ));
        }
        let expected_order_maker = credentials.address().as_core();
        Ok(Self {
            http_config,
            user_ws_config,
            credentials,
            expected_order_maker,
            balance_signature_type: PmReadOnlySignatureType::Eoa,
        })
    }

    /// Construct a read-only proxy-account owner. L2 requests remain bound to
    /// the signer address held by `credentials`; exact order identity and
    /// downstream normalization instead use the distinct proxy funder.
    pub(crate) fn new_proxy_read_only(
        http_config: PmPrivateHttpConfig,
        user_ws_config: PmUserWsConfig,
        expected_order_maker: EvmAddress,
        credentials: L2Credentials,
    ) -> Result<Self, PmLiveAdapterError> {
        if http_config.exact_order_scope().condition() != user_ws_config.condition() {
            return Err(PmLiveAdapterError::InvalidConfiguration(
                "private HTTP and user WebSocket must bind the same condition",
            ));
        }
        if credentials.address().as_core() == expected_order_maker {
            return Err(PmLiveAdapterError::InvalidConfiguration(
                "proxy read profile requires distinct signer and funder",
            ));
        }
        Ok(Self {
            http_config,
            user_ws_config,
            credentials,
            expected_order_maker,
            balance_signature_type: PmReadOnlySignatureType::Proxy,
        })
    }

    /// Consume the sole credential owner into distinct, non-clone role
    /// handles. A Tokio runtime must be active because the authority is an
    /// isolated task rather than a lent secret reference.
    pub fn split(self) -> Result<PmPrivateConnectivityRoles, PmLiveAdapterError> {
        let l2_signer_address = self.credentials.address();
        let transport = PmPrivateHttpTransport::new(&self.http_config, l2_signer_address)?;
        let (sender, supervisor) = spawn_credential_authority(self.credentials)?;

        let http = PmAuthenticatedHttpOwner::from_authority_with_account_profile(
            transport,
            self.http_config.exact_order_scope(),
            l2_signer_address,
            self.expected_order_maker,
            self.balance_signature_type,
            PmHttpCredentialRole {
                sender: sender.clone(),
            },
        );
        let user_ws = PmAuthenticatedUserWsRole::from_authority(
            self.user_ws_config,
            PmUserWsCredentialRole { sender },
        );
        Ok(PmPrivateConnectivityRoles {
            http,
            user_ws,
            supervisor,
        })
    }
}

#[cfg(test)]
pub(crate) fn test_http_credential_role(
    credentials: L2Credentials,
) -> Result<(PmHttpCredentialRole, PmCredentialAuthoritySupervisor), PmLiveAdapterError> {
    let (sender, supervisor) = spawn_credential_authority(credentials)?;
    Ok((PmHttpCredentialRole { sender }, supervisor))
}

#[cfg(test)]
pub(crate) fn test_read_credential_roles(
    credentials: L2Credentials,
) -> Result<
    (
        PmHttpCredentialRole,
        PmUserWsCredentialRole,
        PmCredentialAuthoritySupervisor,
    ),
    PmLiveAdapterError,
> {
    let (sender, supervisor) = spawn_credential_authority(credentials)?;
    Ok((
        PmHttpCredentialRole {
            sender: sender.clone(),
        },
        PmUserWsCredentialRole { sender },
        supervisor,
    ))
}

pub(crate) fn account_http_credential_role(
    credentials: L2Credentials,
) -> Result<(PmHttpCredentialRole, PmCredentialAuthoritySupervisor), PmLiveAdapterError> {
    let (sender, supervisor) = spawn_credential_authority(credentials)?;
    Ok((PmHttpCredentialRole { sender }, supervisor))
}

impl fmt::Debug for PmPrivateConnectivityOwner {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PmPrivateConnectivityOwner([REDACTED])")
    }
}

/// The one-time split result. Each role and the task supervisor are move-only;
/// they have no route, raw header, client, credential, or generic signing
/// escape.
pub struct PmPrivateConnectivityRoles {
    http: PmAuthenticatedHttpOwner,
    user_ws: PmAuthenticatedUserWsRole,
    supervisor: PmCredentialAuthoritySupervisor,
}

impl PmPrivateConnectivityRoles {
    #[must_use]
    pub fn into_read_roles(
        self,
    ) -> (
        PmAuthenticatedHttpOwner,
        PmAuthenticatedUserWsRole,
        PmCredentialAuthoritySupervisor,
    ) {
        (self.http, self.user_ws, self.supervisor)
    }
}

impl fmt::Debug for PmPrivateConnectivityRoles {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PmPrivateConnectivityRoles([REDACTED])")
    }
}

/// Move-only lifecycle owner for the sole secret-custody task.
///
/// Composition must retain this value alongside both read roles and await
/// [`Self::shutdown`] during teardown. Dropping it early aborts the task so a
/// secret-owning task can never become detached.
pub struct PmCredentialAuthoritySupervisor {
    shutdown: Option<oneshot::Sender<()>>,
    task: Option<JoinHandle<()>>,
}

struct PmCredentialAuthorityShutdownFailStop {
    armed: bool,
}

impl PmCredentialAuthorityShutdownFailStop {
    const fn armed() -> Self {
        Self { armed: true }
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for PmCredentialAuthorityShutdownFailStop {
    fn drop(&mut self) {
        if self.armed {
            // Cancelling the bounded shutdown future would otherwise detach
            // the sole task that owns the L2 credential bundle.
            std::process::abort();
        }
    }
}

impl PmCredentialAuthoritySupervisor {
    pub(crate) fn from_task(shutdown: oneshot::Sender<()>, task: JoinHandle<()>) -> Self {
        Self {
            shutdown: Some(shutdown),
            task: Some(task),
        }
    }

    /// Request graceful shutdown, then join through two finite waits. If the
    /// graceful wait expires the task is aborted and joined for the second
    /// finite interval. No path awaits a task after the abort bound expires.
    pub async fn shutdown_bounded(
        mut self,
        bounds: PmCredentialAuthorityShutdownBounds,
    ) -> PmCredentialAuthorityShutdownOutcome {
        let shutdown_requested = self.shutdown.take().is_some_and(|shutdown| {
            let _ = shutdown.send(());
            true
        });
        let Some(task) = self.task.as_mut() else {
            return PmCredentialAuthorityShutdownOutcome {
                shutdown_requested,
                abort_requested: false,
                task_joined: false,
                task_completed_cleanly: false,
                credentials_dropped: false,
            };
        };
        let mut cancellation_fail_stop = PmCredentialAuthorityShutdownFailStop::armed();

        let outcome = match tokio::time::timeout(bounds.graceful_join_timeout(), &mut *task).await {
            Ok(join) => PmCredentialAuthorityShutdownOutcome {
                shutdown_requested,
                abort_requested: false,
                task_joined: true,
                task_completed_cleanly: join.is_ok(),
                credentials_dropped: true,
            },
            Err(_) => {
                task.abort();
                match tokio::time::timeout(bounds.abort_join_timeout(), &mut *task).await {
                    Ok(join) => PmCredentialAuthorityShutdownOutcome {
                        shutdown_requested,
                        abort_requested: true,
                        task_joined: true,
                        task_completed_cleanly: join.is_ok(),
                        credentials_dropped: true,
                    },
                    // Returning would detach the still secret-owning task.
                    // Fail-stop the process so the OS closes sockets and
                    // destroys the credential address space instead.
                    Err(_) => std::process::abort(),
                }
            }
        };
        drop(self.task.take());
        cancellation_fail_stop.disarm();
        outcome
    }

    pub async fn shutdown(self) -> Result<(), PmLiveAdapterError> {
        let outcome = self
            .shutdown_bounded(PM_CREDENTIAL_AUTHORITY_DEFAULT_SHUTDOWN_BOUNDS)
            .await;
        if outcome.shutdown_requested()
            && !outcome.abort_requested()
            && outcome.task_joined()
            && outcome.task_completed_cleanly()
            && outcome.credentials_dropped()
        {
            Ok(())
        } else {
            Err(PmLiveAdapterError::CredentialAuthorityTaskFailed)
        }
    }
}

impl Drop for PmCredentialAuthoritySupervisor {
    fn drop(&mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

impl fmt::Debug for PmCredentialAuthoritySupervisor {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PmCredentialAuthoritySupervisor([REDACTED])")
    }
}

pub(crate) struct PmHttpCredentialRole {
    sender: mpsc::Sender<CredentialRequest>,
}

impl PmHttpCredentialRole {
    #[cfg(any(test, feature = "loopback-evidence"))]
    pub(crate) const fn from_sender(sender: mpsc::Sender<CredentialRequest>) -> Self {
        Self { sender }
    }

    pub(crate) async fn authenticate_open_orders(
        &mut self,
        timestamp: L2Timestamp,
    ) -> Result<AuthenticatedL2Headers, PmLiveAdapterError> {
        request(&self.sender, |response| CredentialRequest::OpenOrders {
            timestamp,
            response,
        })
        .await
    }

    pub(crate) async fn authenticate_trades(
        &mut self,
        timestamp: L2Timestamp,
    ) -> Result<AuthenticatedL2Headers, PmLiveAdapterError> {
        request(&self.sender, |response| CredentialRequest::Trades {
            timestamp,
            response,
        })
        .await
    }

    pub(crate) async fn authenticate_balance(
        &mut self,
        timestamp: L2Timestamp,
    ) -> Result<AuthenticatedL2Headers, PmLiveAdapterError> {
        request(&self.sender, |response| CredentialRequest::Balance {
            timestamp,
            response,
        })
        .await
    }

    pub(crate) async fn authenticate_closed_only(
        &mut self,
        timestamp: L2Timestamp,
    ) -> Result<AuthenticatedL2Headers, PmLiveAdapterError> {
        request(&self.sender, |response| CredentialRequest::ClosedOnly {
            timestamp,
            response,
        })
        .await
    }

    pub(crate) async fn authenticate_exact_order(
        &mut self,
        timestamp: L2Timestamp,
        order_id: FixedOrderId,
    ) -> Result<AuthenticatedL2Headers, PmLiveAdapterError> {
        request(&self.sender, |response| CredentialRequest::ExactOrder {
            timestamp,
            order_id,
            response,
        })
        .await
    }

    pub(crate) async fn bind_open_orders(
        &mut self,
        page: PmLiveOpenOrderPage,
    ) -> Result<PmLiveOpenOrderPage, PmLiveAdapterError> {
        request(&self.sender, |response| CredentialRequest::BindOpenOrders {
            page,
            response,
        })
        .await
    }

    pub(crate) async fn bind_trades(
        &mut self,
        page: PmLiveTradePage,
    ) -> Result<PmLiveTradePage, PmLiveAdapterError> {
        request(&self.sender, |response| CredentialRequest::BindTrades {
            page,
            response,
        })
        .await
    }

    pub(crate) async fn bind_exact_order(
        &mut self,
        order: PmLiveOrder,
    ) -> Result<PmLiveOrder, PmLiveAdapterError> {
        request(&self.sender, |response| CredentialRequest::BindExactOrder {
            order: Box::new(order),
            response,
        })
        .await
    }
}

#[async_trait]
impl PmHttpReadAuthorityProvider for PmHttpCredentialRole {
    async fn authenticate_open_orders(
        &mut self,
        timestamp: L2Timestamp,
    ) -> Result<AuthenticatedL2Headers, PmLiveAdapterError> {
        Self::authenticate_open_orders(self, timestamp).await
    }

    async fn authenticate_trades(
        &mut self,
        timestamp: L2Timestamp,
    ) -> Result<AuthenticatedL2Headers, PmLiveAdapterError> {
        Self::authenticate_trades(self, timestamp).await
    }

    async fn authenticate_balance_allowance(
        &mut self,
        timestamp: L2Timestamp,
    ) -> Result<AuthenticatedL2Headers, PmLiveAdapterError> {
        self.authenticate_balance(timestamp).await
    }

    async fn authenticate_closed_only(
        &mut self,
        timestamp: L2Timestamp,
    ) -> Result<AuthenticatedL2Headers, PmLiveAdapterError> {
        Self::authenticate_closed_only(self, timestamp).await
    }

    async fn authenticate_exact_order(
        &mut self,
        timestamp: L2Timestamp,
        order_id: FixedOrderId,
    ) -> Result<AuthenticatedL2Headers, PmLiveAdapterError> {
        Self::authenticate_exact_order(self, timestamp, order_id).await
    }

    async fn bind_open_orders(
        &mut self,
        page: PmLiveOpenOrderPage,
    ) -> Result<PmLiveOpenOrderPage, PmLiveAdapterError> {
        Self::bind_open_orders(self, page).await
    }

    async fn bind_trades(
        &mut self,
        page: PmLiveTradePage,
    ) -> Result<PmLiveTradePage, PmLiveAdapterError> {
        Self::bind_trades(self, page).await
    }

    async fn bind_exact_order(
        &mut self,
        order: PmLiveOrder,
    ) -> Result<PmLiveOrder, PmLiveAdapterError> {
        Self::bind_exact_order(self, order).await
    }
}

pub(crate) struct PmUserWsCredentialRole {
    sender: mpsc::Sender<CredentialRequest>,
}

impl PmUserWsCredentialRole {
    #[cfg(any(test, feature = "loopback-evidence"))]
    pub(crate) const fn from_sender(sender: mpsc::Sender<CredentialRequest>) -> Self {
        Self { sender }
    }

    pub(crate) async fn fresh_subscription(
        &mut self,
        condition: reap_pm_core::PmConditionId,
    ) -> Result<AuthenticatedUserSubscription, PmLiveAdapterError> {
        request(&self.sender, |response| {
            CredentialRequest::UserSubscription {
                condition,
                response,
            }
        })
        .await
    }

    pub(crate) async fn bind_frame(
        &mut self,
        frame: PmLiveUserFrame,
    ) -> Result<CredentialOwnedUserFrame, PmLiveAdapterError> {
        request(&self.sender, |response| CredentialRequest::BindUserFrame {
            frame,
            response,
        })
        .await
    }
}

#[async_trait]
impl PmUserWsReadAuthorityProvider for PmUserWsCredentialRole {
    async fn authenticate_user_subscription(
        &mut self,
        condition: reap_pm_core::PmConditionId,
    ) -> Result<AuthenticatedUserSubscription, PmLiveAdapterError> {
        self.fresh_subscription(condition).await
    }

    async fn bind_user_frame(
        &mut self,
        frame: PmLiveUserFrame,
    ) -> Result<CredentialOwnedUserFrame, PmLiveAdapterError> {
        self.bind_frame(frame).await
    }
}

pub(crate) enum CredentialRequest {
    OpenOrders {
        timestamp: L2Timestamp,
        response: oneshot::Sender<Result<AuthenticatedL2Headers, PmLiveAdapterError>>,
    },
    Trades {
        timestamp: L2Timestamp,
        response: oneshot::Sender<Result<AuthenticatedL2Headers, PmLiveAdapterError>>,
    },
    Balance {
        timestamp: L2Timestamp,
        response: oneshot::Sender<Result<AuthenticatedL2Headers, PmLiveAdapterError>>,
    },
    ClosedOnly {
        timestamp: L2Timestamp,
        response: oneshot::Sender<Result<AuthenticatedL2Headers, PmLiveAdapterError>>,
    },
    ExactOrder {
        timestamp: L2Timestamp,
        order_id: FixedOrderId,
        response: oneshot::Sender<Result<AuthenticatedL2Headers, PmLiveAdapterError>>,
    },
    BindOpenOrders {
        page: PmLiveOpenOrderPage,
        response: oneshot::Sender<Result<PmLiveOpenOrderPage, PmLiveAdapterError>>,
    },
    BindTrades {
        page: PmLiveTradePage,
        response: oneshot::Sender<Result<PmLiveTradePage, PmLiveAdapterError>>,
    },
    BindExactOrder {
        order: Box<PmLiveOrder>,
        response: oneshot::Sender<Result<PmLiveOrder, PmLiveAdapterError>>,
    },
    UserSubscription {
        condition: reap_pm_core::PmConditionId,
        response: oneshot::Sender<Result<AuthenticatedUserSubscription, PmLiveAdapterError>>,
    },
    BindUserFrame {
        frame: PmLiveUserFrame,
        response: oneshot::Sender<Result<CredentialOwnedUserFrame, PmLiveAdapterError>>,
    },
}

async fn run_credential_authority(
    credentials: L2Credentials,
    mut requests: mpsc::Receiver<CredentialRequest>,
    mut shutdown: oneshot::Receiver<()>,
) {
    loop {
        let request = tokio::select! {
            biased;
            _ = &mut shutdown => {
                requests.close();
                return;
            }
            request = requests.recv() => {
                let Some(request) = request else {
                    return;
                };
                request
            }
        };
        handle_credential_request(&credentials, request);
    }
}

pub(crate) fn handle_credential_request(credentials: &L2Credentials, request: CredentialRequest) {
    match request {
        CredentialRequest::OpenOrders {
            timestamp,
            response,
        } => respond(
            response,
            credentials
                .authenticate_open_orders(timestamp)
                .map_err(Into::into),
        ),
        CredentialRequest::Trades {
            timestamp,
            response,
        } => respond(
            response,
            credentials
                .authenticate_trades(timestamp)
                .map_err(Into::into),
        ),
        CredentialRequest::Balance {
            timestamp,
            response,
        } => respond(
            response,
            credentials
                .authenticate_balance_allowance(timestamp)
                .map_err(Into::into),
        ),
        CredentialRequest::ClosedOnly {
            timestamp,
            response,
        } => respond(
            response,
            credentials
                .authenticate_closed_only(timestamp)
                .map_err(Into::into),
        ),
        CredentialRequest::ExactOrder {
            timestamp,
            order_id,
            response,
        } => respond(
            response,
            credentials
                .authenticate_order_detail(timestamp, order_id)
                .map_err(Into::into),
        ),
        CredentialRequest::BindOpenOrders { page, response } => {
            let matches = page
                .orders()
                .iter()
                .all(|order| credentials.matches_credential_owner(order.owner()));
            respond(response, owner_match(matches).map(|()| page));
        }
        CredentialRequest::BindTrades { page, response } => {
            let matches = page
                .trades()
                .iter()
                .all(|trade| credentials.matches_credential_owner(trade.owner()));
            respond(response, owner_match(matches).map(|()| page));
        }
        CredentialRequest::BindExactOrder { order, response } => respond(
            response,
            owner_match(credentials.matches_credential_owner(order.owner())).map(|()| *order),
        ),
        CredentialRequest::UserSubscription {
            condition,
            response,
        } => respond(
            response,
            credentials
                .user_subscription(condition)
                .map_err(PmLiveAdapterError::from),
        ),
        CredentialRequest::BindUserFrame { frame, response } => respond(
            response,
            credentials
                .bind_user_stream_frame(frame)
                .map_err(PmLiveAdapterError::from),
        ),
    }
}

fn spawn_credential_authority(
    credentials: L2Credentials,
) -> Result<
    (
        mpsc::Sender<CredentialRequest>,
        PmCredentialAuthoritySupervisor,
    ),
    PmLiveAdapterError,
> {
    let (sender, receiver) = mpsc::channel(CREDENTIAL_AUTHORITY_CAPACITY);
    let (shutdown, shutdown_receiver) = oneshot::channel();
    let task = tokio::runtime::Handle::try_current()
        .map_err(|_| {
            PmLiveAdapterError::InvalidConfiguration(
                "private credential authority requires an active Tokio runtime",
            )
        })?
        .spawn(run_credential_authority(
            credentials,
            receiver,
            shutdown_receiver,
        ));
    Ok((
        sender,
        PmCredentialAuthoritySupervisor::from_task(shutdown, task),
    ))
}

fn owner_match(matches: bool) -> Result<(), PmLiveAdapterError> {
    if matches {
        Ok(())
    } else {
        Err(PmLiveAdapterError::CredentialOwnerMismatch)
    }
}

fn respond<T>(
    response: oneshot::Sender<Result<T, PmLiveAdapterError>>,
    value: Result<T, PmLiveAdapterError>,
) {
    let _ = response.send(value);
}

async fn request<T>(
    sender: &mpsc::Sender<CredentialRequest>,
    make: impl FnOnce(oneshot::Sender<Result<T, PmLiveAdapterError>>) -> CredentialRequest,
) -> Result<T, PmLiveAdapterError> {
    let (response, receive) = oneshot::channel();
    sender
        .send(make(response))
        .await
        .map_err(|_| PmLiveAdapterError::CredentialAuthorityClosed)?;
    receive
        .await
        .map_err(|_| PmLiveAdapterError::CredentialAuthorityClosed)?
}

#[cfg(test)]
mod tests {
    use reap_polymarket_auth::{L2CredentialInput, L2Timestamp};

    use super::*;

    const ADDRESS: &str = "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266";
    const API_KEY: &str = "00000000-0000-4000-8000-000000000001";
    const API_SECRET: &str = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";
    const PASSPHRASE: &str = "synthetic-passphrase";

    fn credentials() -> L2Credentials {
        L2Credentials::bind(
            ADDRESS,
            L2CredentialInput::new(API_KEY.into(), API_SECRET.into(), PASSPHRASE.into()),
        )
        .unwrap()
    }

    #[tokio::test]
    async fn explicit_shutdown_joins_authority_and_closes_remaining_roles() {
        let (sender, supervisor) = spawn_credential_authority(credentials()).unwrap();
        let mut role = PmHttpCredentialRole { sender };
        let debug = format!("{supervisor:?}");
        assert!(debug.contains("[REDACTED]"));
        assert!(!debug.contains(API_KEY));
        assert!(!debug.contains(API_SECRET));
        assert!(!debug.contains(PASSPHRASE));

        supervisor.shutdown().await.unwrap();
        assert!(matches!(
            role.authenticate_open_orders(L2Timestamp::from_unix_seconds(1_700_000_000).unwrap())
                .await,
            Err(PmLiveAdapterError::CredentialAuthorityClosed)
        ));
    }

    #[tokio::test]
    async fn bounded_shutdown_reports_a_clean_graceful_join() {
        let (_sender, supervisor) = spawn_credential_authority(credentials()).unwrap();
        let outcome = supervisor
            .shutdown_bounded(PM_CREDENTIAL_AUTHORITY_READ_ONLY_SHUTDOWN_BOUNDS)
            .await;

        assert!(outcome.shutdown_requested());
        assert!(!outcome.abort_requested());
        assert!(outcome.task_joined());
        assert!(outcome.task_completed_cleanly());
        assert!(outcome.credentials_dropped());
    }

    #[tokio::test]
    async fn bounded_shutdown_aborts_and_joins_a_stuck_task() {
        let (shutdown, _shutdown_receiver) = oneshot::channel();
        let task = tokio::spawn(std::future::pending::<()>());
        let supervisor = PmCredentialAuthoritySupervisor::from_task(shutdown, task);
        let bounds = PmCredentialAuthorityShutdownBounds::new(
            Duration::from_millis(1),
            Duration::from_secs(1),
        )
        .unwrap();

        let outcome = supervisor.shutdown_bounded(bounds).await;

        assert!(outcome.shutdown_requested());
        assert!(outcome.abort_requested());
        assert!(outcome.task_joined());
        assert!(!outcome.task_completed_cleanly());
        assert!(outcome.credentials_dropped());
    }

    #[test]
    fn shutdown_bounds_reject_zero_or_unbounded_waits() {
        assert!(
            PmCredentialAuthorityShutdownBounds::new(Duration::ZERO, Duration::from_secs(1))
                .is_err()
        );
        assert!(
            PmCredentialAuthorityShutdownBounds::new(
                Duration::from_secs(1),
                Duration::from_secs(61),
            )
            .is_err()
        );
    }
}
