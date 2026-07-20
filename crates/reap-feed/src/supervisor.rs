use std::collections::{HashMap, HashSet};
#[cfg(test)]
use std::fs::OpenOptions;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
#[cfg(test)]
use std::time::Instant;

use reap_core::{ConnId, RawEnvelope, Symbol, Venue};
use reap_transport::{
    ConnectionAttemptPacer as TransportConnectionAttemptPacer, ShutdownReceiver, ShutdownSender,
    SupervisorState, request_shutdown as signal_shutdown, shutdown_channel, shutdown_requested,
    supervision_channels,
};
pub use reap_transport::{ConnectionAttemptPacerError, ReconnectPolicy};
use reap_venue::VenueAdapter;
pub use reap_venue::okx::{
    DEFAULT_OKX_CONNECTION_ATTEMPT_PACER_PATH, OKX_MIN_CONNECTION_ATTEMPT_INTERVAL_MS,
};
use thiserror::Error;
use tokio::sync::{mpsc, watch};
use tokio::task::JoinHandle;

use crate::{
    ConnectionError, ConnectionStatus, ConnectionStatusKind, PrivateLoginBootstrap,
    RecoveryRequest, SocketPlan, prepare_connection_subscription,
    run_connection_once_with_readiness,
};

type RecoveryStreamKey = (Venue, Symbol);
type RecoveryRoute = (ConnId, watch::Sender<u64>);
type RecoveryRoutes = HashMap<RecoveryStreamKey, Vec<RecoveryRoute>>;

const SHARED_PACER_STATE_MAGIC: &str = "reap-okx-connect-pacer-v1";
#[cfg(test)]
const MAX_SHARED_RESERVATION_AHEAD: Duration = Duration::from_secs(15 * 60);
#[cfg(test)]
const MAX_SHARED_PACER_LOCK_WAIT: Duration = Duration::from_secs(1);

/// Pacing for connection handshakes across independent feed groups and, when
/// configured, every Reap process using the same state file.
#[derive(Debug, Clone)]
pub struct ConnectionAttemptPacer {
    inner: TransportConnectionAttemptPacer,
}

impl ConnectionAttemptPacer {
    pub fn new(interval: Duration) -> Self {
        Self {
            inner: TransportConnectionAttemptPacer::new(interval),
        }
    }

    pub fn process_shared(
        interval: Duration,
        path: impl AsRef<Path>,
    ) -> Result<Self, ConnectionAttemptPacerError> {
        Ok(Self {
            inner: TransportConnectionAttemptPacer::process_shared(
                interval,
                path,
                SHARED_PACER_STATE_MAGIC,
            )?,
        })
    }

    pub fn interval(&self) -> Duration {
        self.inner.interval()
    }

    pub fn is_process_shared(&self) -> bool {
        self.inner.is_process_shared()
    }

    pub async fn wait_for_turn(
        &self,
        shutdown: &mut watch::Receiver<bool>,
    ) -> Result<bool, ConnectionAttemptPacerError> {
        let (shutdown_sender, mut monotonic_shutdown) = shutdown_channel();
        if legacy_shutdown_requested(shutdown) {
            let _ = signal_shutdown(&shutdown_sender);
        }
        let wait = self.inner.wait_for_turn(&mut monotonic_shutdown);
        tokio::pin!(wait);
        loop {
            tokio::select! {
                biased;
                changed = shutdown.changed() => {
                    if changed.is_err() || *shutdown.borrow() {
                        let _ = signal_shutdown(&shutdown_sender);
                        return (&mut wait).await;
                    }
                }
                result = &mut wait => return result,
            }
        }
    }

    async fn wait_for_turn_monotonic(
        &self,
        shutdown: &mut ShutdownReceiver,
    ) -> Result<bool, ConnectionAttemptPacerError> {
        self.inner.wait_for_turn(shutdown).await
    }
}

fn legacy_shutdown_requested(shutdown: &watch::Receiver<bool>) -> bool {
    shutdown.has_changed().is_err() || *shutdown.borrow()
}

type BootstrapGenerator =
    dyn Fn(&SocketPlan) -> Result<PrivateLoginBootstrap, ConnectionError> + Send + Sync;

pub struct BootstrapFactory {
    kind: BootstrapKind,
}

enum BootstrapKind {
    None,
    Private {
        websocket_url: Arc<str>,
        generator: Arc<BootstrapGenerator>,
    },
}

impl BootstrapFactory {
    pub fn bind_private_websocket(
        websocket_url: impl Into<String>,
        generator: impl Fn(&SocketPlan) -> Result<PrivateLoginBootstrap, ConnectionError>
        + Send
        + Sync
        + 'static,
    ) -> Self {
        Self {
            kind: BootstrapKind::Private {
                websocket_url: Arc::from(websocket_url.into()),
                generator: Arc::new(generator),
            },
        }
    }

    fn generate(
        &self,
        plan: &SocketPlan,
        selected_websocket_url: &str,
    ) -> Result<Option<PrivateLoginBootstrap>, ConnectionError> {
        match &self.kind {
            BootstrapKind::None => Ok(None),
            BootstrapKind::Private { .. } if !plan.private => Ok(None),
            BootstrapKind::Private {
                websocket_url,
                generator,
            } => {
                if websocket_url.as_ref() != selected_websocket_url {
                    return Err(ConnectionError::PrivateBootstrapDestinationMismatch {
                        bound_url: websocket_url.to_string(),
                        selected_url: selected_websocket_url.to_string(),
                    });
                }
                generator(plan).map(Some)
            }
        }
    }
}

pub fn no_bootstrap() -> BootstrapFactory {
    BootstrapFactory {
        kind: BootstrapKind::None,
    }
}

pub struct SupervisedFeed {
    pub raw: mpsc::Receiver<RawEnvelope>,
    pub status: mpsc::Receiver<ConnectionStatus>,
    shutdown: ShutdownSender,
    recovery_routes: RecoveryRoutes,
    // Non-book plans have no entry in `recovery_routes`, but their receiver
    // still needs an owner for the entire supervised-feed lifetime.
    _recovery_guards: Vec<watch::Sender<u64>>,
    tasks: Vec<JoinHandle<()>>,
}

impl SupervisedFeed {
    pub fn take_raw(&mut self) -> mpsc::Receiver<RawEnvelope> {
        let (_sender, replacement) = reap_transport::bounded_channel(1);
        std::mem::replace(&mut self.raw, replacement)
    }

    pub fn take_status(&mut self) -> mpsc::Receiver<ConnectionStatus> {
        let (_sender, replacement) = reap_transport::bounded_channel(1);
        std::mem::replace(&mut self.status, replacement)
    }

    pub fn request_recovery(&self, request: &RecoveryRequest) -> usize {
        let Some(routes) = self
            .recovery_routes
            .get(&(request.stream.venue, request.stream.symbol.clone()))
        else {
            return 0;
        };
        routes
            .iter()
            .filter(|(conn_id, route)| {
                if request
                    .source_conn_id
                    .as_ref()
                    .is_some_and(|source| source != conn_id)
                {
                    return false;
                }
                let next = route.borrow().wrapping_add(1);
                route.send(next).is_ok()
            })
            .count()
    }

    pub fn request_shutdown(&self) {
        let _ = signal_shutdown(&self.shutdown);
    }

    pub async fn shutdown(mut self) {
        self.request_shutdown();
        for task in &mut self.tasks {
            let _ = task.await;
        }
        self.tasks.clear();
    }
}

impl Drop for SupervisedFeed {
    fn drop(&mut self) {
        self.request_shutdown();
        for task in &self.tasks {
            task.abort();
        }
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum SupervisedFeedSpawnError {
    #[error("supervised feed plans repeat connection id {conn_id}")]
    DuplicateConnectionId { conn_id: ConnId },
}

pub fn spawn_supervised_feed(
    adapter: Arc<dyn VenueAdapter>,
    plans: Vec<SocketPlan>,
    bootstrap: BootstrapFactory,
    channel_capacity: usize,
    connection_attempt_pacer: ConnectionAttemptPacer,
    reconnect: ReconnectPolicy,
) -> SupervisedFeed {
    try_spawn_supervised_feed(
        adapter,
        plans,
        bootstrap,
        channel_capacity,
        connection_attempt_pacer,
        reconnect,
    )
    .expect("supervised feed plans must have unique connection ids")
}

pub fn try_spawn_supervised_feed(
    adapter: Arc<dyn VenueAdapter>,
    plans: Vec<SocketPlan>,
    bootstrap: BootstrapFactory,
    channel_capacity: usize,
    connection_attempt_pacer: ConnectionAttemptPacer,
    reconnect: ReconnectPolicy,
) -> Result<SupervisedFeed, SupervisedFeedSpawnError> {
    let mut connection_ids = HashSet::new();
    for plan in &plans {
        if !connection_ids.insert(plan.conn_id.clone()) {
            return Err(SupervisedFeedSpawnError::DuplicateConnectionId {
                conn_id: plan.conn_id.clone(),
            });
        }
    }
    let reap_transport::SupervisionChannels {
        output_sender: raw_tx,
        output_receiver: raw_rx,
        health_sender: status_tx,
        health_receiver: status_rx,
        shutdown_sender: shutdown_tx,
        shutdown_receiver: shutdown_rx,
    } = supervision_channels(channel_capacity);
    let mut tasks = Vec::new();
    let mut recovery_routes = RecoveryRoutes::new();
    let mut recovery_guards = Vec::new();
    let bootstrap = Arc::new(bootstrap);
    for plan in plans {
        let (recovery_tx, recovery_rx) = watch::channel(0_u64);
        let mut routed_symbols = HashSet::new();
        for subscription in &plan.subscriptions {
            if subscription.channel.is_book()
                && let Some(symbol) = &subscription.symbol
                && routed_symbols.insert(symbol.clone())
            {
                recovery_routes
                    .entry((plan.venue, symbol.clone()))
                    .or_default()
                    .push((plan.conn_id.clone(), recovery_tx.clone()));
            }
        }
        recovery_guards.push(recovery_tx);
        let adapter = Arc::clone(&adapter);
        let output = raw_tx.clone();
        let status = status_tx.clone();
        let bootstrap = Arc::clone(&bootstrap);
        let shutdown = shutdown_rx.clone();
        let connection_attempt_pacer = connection_attempt_pacer.clone();
        let reconnect = reconnect.clone();
        tasks.push(tokio::spawn(async move {
            supervise_connection(
                adapter,
                plan,
                bootstrap,
                ConnectionChannels {
                    output,
                    status,
                    shutdown,
                    recovery: recovery_rx,
                },
                connection_attempt_pacer,
                reconnect,
            )
            .await;
        }));
    }
    drop(raw_tx);
    drop(status_tx);
    Ok(SupervisedFeed {
        raw: raw_rx,
        status: status_rx,
        shutdown: shutdown_tx,
        recovery_routes,
        _recovery_guards: recovery_guards,
        tasks,
    })
}

struct ConnectionChannels {
    output: mpsc::Sender<RawEnvelope>,
    status: mpsc::Sender<ConnectionStatus>,
    shutdown: ShutdownReceiver,
    recovery: watch::Receiver<u64>,
}

fn is_fatal_connection_error(error: &ConnectionError) -> bool {
    matches!(
        error,
        ConnectionError::InvalidSubscriptionPlan(_)
            | ConnectionError::InvalidPrivateLogin(_)
            | ConnectionError::PrivateBootstrapDestinationMismatch { .. }
            | ConnectionError::RecoveryChannelClosed
    )
}

async fn supervise_connection(
    adapter: Arc<dyn VenueAdapter>,
    plan: SocketPlan,
    bootstrap: Arc<BootstrapFactory>,
    channels: ConnectionChannels,
    connection_attempt_pacer: ConnectionAttemptPacer,
    reconnect: ReconnectPolicy,
) {
    let ConnectionChannels {
        output,
        status,
        mut shutdown,
        mut recovery,
    } = channels;
    let mut supervision = SupervisorState::new(reconnect);
    loop {
        if supervision.should_stop(&shutdown) {
            return;
        }
        match connection_attempt_pacer
            .wait_for_turn_monotonic(&mut shutdown)
            .await
        {
            Ok(true) => {}
            Ok(false) => return,
            Err(error) => {
                supervision.mark_fatal();
                tracing::error!(conn_id = %plan.conn_id, %error, "feed connection pacer failed");
                let _ = status
                    .send(ConnectionStatus {
                        conn_id: plan.conn_id.clone(),
                        venue: plan.venue,
                        private: plan.private,
                        ts_ms: crate::unix_time_ns() / 1_000_000,
                        kind: ConnectionStatusKind::Fatal,
                        reason: error.to_string(),
                    })
                    .await;
                return;
            }
        }
        let prepared_subscription = match prepare_connection_subscription(adapter.as_ref(), &plan) {
            Ok(subscription) => subscription,
            Err(error) => {
                let fatal = is_fatal_connection_error(&error);
                let _ = status
                    .send(ConnectionStatus {
                        conn_id: plan.conn_id.clone(),
                        venue: plan.venue,
                        private: plan.private,
                        ts_ms: crate::unix_time_ns() / 1_000_000,
                        kind: if fatal {
                            ConnectionStatusKind::Fatal
                        } else {
                            ConnectionStatusKind::Disconnected
                        },
                        reason: error.to_string(),
                    })
                    .await;
                if fatal {
                    supervision.mark_fatal();
                    tracing::error!(
                        conn_id = %plan.conn_id,
                        ?error,
                        "feed subscription serializer produced an invalid request"
                    );
                    return;
                }
                let delay = supervision.after_failure(false);
                tracing::warn!(
                    conn_id = %plan.conn_id,
                    ?error,
                    ?delay,
                    "feed subscription generation failed"
                );
                tokio::select! {
                    _ = tokio::time::sleep(delay) => {}
                    changed = shutdown.changed() => {
                        if changed.is_err() || shutdown_requested(&shutdown) {
                            return;
                        }
                    }
                }
                continue;
            }
        };
        let websocket_url = adapter.websocket_url(plan.private).to_string();
        let private_login = match bootstrap.generate(&plan, &websocket_url) {
            Ok(private_login) => private_login,
            Err(error) if is_fatal_connection_error(&error) => {
                supervision.mark_fatal();
                tracing::error!(
                    conn_id = %plan.conn_id,
                    ?error,
                    "feed bootstrap destination is invalid"
                );
                let _ = status
                    .send(ConnectionStatus {
                        conn_id: plan.conn_id.clone(),
                        venue: plan.venue,
                        private: plan.private,
                        ts_ms: crate::unix_time_ns() / 1_000_000,
                        kind: ConnectionStatusKind::Fatal,
                        reason: error.to_string(),
                    })
                    .await;
                return;
            }
            Err(error) => {
                let delay = supervision.after_failure(false);
                tracing::warn!(conn_id = %plan.conn_id, ?error, ?delay, "feed bootstrap generation failed");
                tokio::select! {
                    _ = tokio::time::sleep(delay) => {}
                    changed = shutdown.changed() => {
                        if changed.is_err() || shutdown_requested(&shutdown) {
                            return;
                        }
                    }
                }
                continue;
            }
        };
        let outcome = run_connection_once_with_readiness(
            &websocket_url,
            &plan,
            private_login.as_ref(),
            prepared_subscription,
            &output,
            &status,
            &mut shutdown,
            &mut recovery,
        )
        .await;
        let reached_ready = outcome.reached_ready;
        let result = outcome.result;
        if shutdown_requested(&shutdown) || matches!(result, Ok(())) {
            return;
        }
        let error = result.expect_err("non-success result must contain an error");
        let fatal = is_fatal_connection_error(&error);
        let disconnected = status.send(ConnectionStatus {
            conn_id: plan.conn_id.clone(),
            venue: plan.venue,
            private: plan.private,
            ts_ms: crate::unix_time_ns() / 1_000_000,
            kind: if fatal {
                ConnectionStatusKind::Fatal
            } else {
                ConnectionStatusKind::Disconnected
            },
            reason: error.to_string(),
        });
        tokio::select! {
            result = disconnected => {
                if result.is_err() {
                    return;
                }
            }
            changed = shutdown.changed() => {
                if changed.is_err() || shutdown_requested(&shutdown) {
                    return;
                }
            }
        }
        if fatal {
            supervision.mark_fatal();
            tracing::error!(conn_id = %plan.conn_id, ?error, "feed connection cannot recover");
            return;
        }
        if matches!(error, ConnectionError::RecoveryRequested) {
            supervision.reset_for_recovery();
            tracing::info!(conn_id = %plan.conn_id, "feed connection restarting for snapshot recovery");
            continue;
        }
        let delay = supervision.after_failure(reached_ready);
        tracing::warn!(conn_id = %plan.conn_id, ?error, ?delay, "feed connection restarting");
        if matches!(error, ConnectionError::OutputClosed) {
            return;
        }
        tokio::select! {
            _ = tokio::time::sleep(delay) => {}
            changed = shutdown.changed() => {
                if changed.is_err() || shutdown_requested(&shutdown) {
                    return;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use reap_core::{Channel, FeedPriority, Subscription};
    use reap_transport::ReconnectBackoff;
    use reap_venue::okx::OkxAdapter;

    use super::*;

    const ATTACK_BOUND_URL: &str = "wss://bound.example:8443/ws/v5/private";

    struct SubstitutingSubscriptionAdapter {
        delegate: OkxAdapter,
        websocket_calls: Arc<AtomicUsize>,
        subscription_calls: Arc<AtomicUsize>,
        payload: String,
    }

    impl VenueAdapter for SubstitutingSubscriptionAdapter {
        fn venue(&self) -> Venue {
            self.delegate.venue()
        }

        fn websocket_url(&self, private: bool) -> &str {
            self.websocket_calls.fetch_add(1, Ordering::Relaxed);
            self.delegate.websocket_url(private)
        }

        fn parse(
            &self,
            envelope: &RawEnvelope,
        ) -> Result<Vec<reap_venue::ParsedEvent>, reap_venue::VenueError> {
            self.delegate.parse(envelope)
        }

        fn is_data_frame(&self, envelope: &RawEnvelope) -> Result<bool, reap_venue::VenueError> {
            self.delegate.is_data_frame(envelope)
        }

        fn subscription_message(
            &self,
            _subscriptions: &[Subscription],
        ) -> Result<String, reap_venue::VenueError> {
            self.subscription_calls.fetch_add(1, Ordering::Relaxed);
            Ok(self.payload.clone())
        }
    }

    fn private_login(timestamp: usize) -> PrivateLoginBootstrap {
        PrivateLoginBootstrap::parse(
            serde_json::json!({
                "op": "login",
                "args": [{
                    "apiKey": "key",
                    "passphrase": "passphrase",
                    "timestamp": timestamp.to_string(),
                    "sign": "signature",
                }],
            })
            .to_string(),
        )
        .unwrap()
    }

    async fn assert_subscription_substitution_is_fatal(
        plan: SocketPlan,
        bootstrap: BootstrapFactory,
        payload: &str,
        expected_reason: &str,
    ) {
        let websocket_calls = Arc::new(AtomicUsize::new(0));
        let subscription_calls = Arc::new(AtomicUsize::new(0));
        let adapter = SubstitutingSubscriptionAdapter {
            delegate: OkxAdapter::new("wss://public.example:8443/ws/v5/public", ATTACK_BOUND_URL),
            websocket_calls: Arc::clone(&websocket_calls),
            subscription_calls: Arc::clone(&subscription_calls),
            payload: payload.to_string(),
        };
        let (output, _output_rx) = mpsc::channel(1);
        let (status, mut status_rx) = mpsc::channel(1);
        let (_shutdown_tx, shutdown) = shutdown_channel();
        let (_recovery_tx, recovery) = watch::channel(0_u64);

        supervise_connection(
            Arc::new(adapter),
            plan,
            Arc::new(bootstrap),
            ConnectionChannels {
                output,
                status,
                shutdown,
                recovery,
            },
            ConnectionAttemptPacer::new(Duration::ZERO),
            ReconnectPolicy::default(),
        )
        .await;

        let fatal = status_rx.recv().await.unwrap();
        assert_eq!(fatal.kind, ConnectionStatusKind::Fatal);
        assert!(fatal.reason.contains(expected_reason), "{}", fatal.reason);
        assert_eq!(subscription_calls.load(Ordering::Relaxed), 1);
        assert_eq!(websocket_calls.load(Ordering::Relaxed), 0);
        assert!(status_rx.recv().await.is_none());
    }

    #[test]
    fn reconnect_backoff_is_bounded() {
        let policy = ReconnectPolicy {
            initial_delay: Duration::from_millis(10),
            max_delay: Duration::from_millis(25),
            multiplier: 2,
        };
        assert_eq!(
            policy.next_delay(Duration::from_millis(10)),
            Duration::from_millis(20)
        );
        assert_eq!(
            policy.next_delay(Duration::from_millis(20)),
            Duration::from_millis(25)
        );

        let mut backoff = ReconnectBackoff::new(policy);
        assert_eq!(backoff.after_failure(false), Duration::from_millis(10));
        assert_eq!(backoff.after_failure(false), Duration::from_millis(20));
        assert_eq!(backoff.after_failure(false), Duration::from_millis(25));
        assert_eq!(
            backoff.after_failure(true),
            Duration::from_millis(10),
            "a successfully subscribed session must reset historical startup backoff"
        );
        assert_eq!(backoff.after_failure(false), Duration::from_millis(20));
        backoff.reset();
        assert_eq!(backoff.after_failure(false), Duration::from_millis(10));
    }

    #[test]
    fn invalid_plan_and_closed_recovery_route_are_fatal() {
        assert!(is_fatal_connection_error(
            &ConnectionError::InvalidSubscriptionPlan("duplicate".to_string())
        ));
        assert!(is_fatal_connection_error(
            &ConnectionError::PrivateBootstrapDestinationMismatch {
                bound_url: "wss://bound.example/ws/v5/private".to_string(),
                selected_url: "wss://selected.example/ws/v5/private".to_string(),
            }
        ));
        assert!(is_fatal_connection_error(
            &ConnectionError::RecoveryChannelClosed
        ));
        assert!(!is_fatal_connection_error(
            &ConnectionError::ConnectionTimeout
        ));
        assert!(!is_fatal_connection_error(
            &ConnectionError::PrivateBackpressureTimeout { timeout_ms: 1 }
        ));
    }

    #[tokio::test]
    async fn invalid_subscription_plan_is_fatal_without_reconnect() {
        let subscription = Subscription::public(
            Venue::Okx,
            Channel::Books,
            "BTC-USDT",
            FeedPriority::Critical,
        );
        let plan = SocketPlan {
            conn_id: reap_core::ConnId::new("duplicate-plan"),
            venue: Venue::Okx,
            private: false,
            subscriptions: vec![subscription.clone(), subscription],
        };
        let (output, _output_rx) = mpsc::channel(1);
        let (status, mut status_rx) = mpsc::channel(1);
        let (_shutdown_tx, shutdown) = shutdown_channel();
        let (_recovery_tx, recovery) = watch::channel(0_u64);

        supervise_connection(
            Arc::new(OkxAdapter::new("ws://127.0.0.1:9", "ws://127.0.0.1:9")),
            plan,
            Arc::new(no_bootstrap()),
            ConnectionChannels {
                output,
                status,
                shutdown,
                recovery,
            },
            ConnectionAttemptPacer::new(Duration::ZERO),
            ReconnectPolicy::default(),
        )
        .await;

        let fatal = status_rx.recv().await.unwrap();
        assert_eq!(fatal.kind, ConnectionStatusKind::Fatal);
        assert!(fatal.reason.contains("repeats subscription books/BTC-USDT"));
        assert!(status_rx.recv().await.is_none());
    }

    #[tokio::test]
    async fn supervised_non_book_socket_retains_its_recovery_channel_owner() {
        let plan = SocketPlan {
            conn_id: reap_core::ConnId::new("trades"),
            venue: Venue::Okx,
            private: false,
            subscriptions: vec![Subscription::public(
                Venue::Okx,
                Channel::Trades,
                "BTC-USDT",
                FeedPriority::High,
            )],
        };
        let feed = spawn_supervised_feed(
            Arc::new(OkxAdapter::new("ws://127.0.0.1:9", "ws://127.0.0.1:9")),
            vec![plan],
            no_bootstrap(),
            4,
            ConnectionAttemptPacer::new(Duration::ZERO),
            ReconnectPolicy::default(),
        );

        assert_eq!(feed._recovery_guards.len(), 1);
        assert!(!feed._recovery_guards[0].is_closed());
        feed.shutdown().await;
    }

    #[test]
    fn duplicate_connection_ids_are_rejected_before_tasks_are_spawned() {
        let plan = SocketPlan {
            conn_id: reap_core::ConnId::new("private-account-r0"),
            venue: Venue::Okx,
            private: true,
            subscriptions: vec![Subscription::private(
                Venue::Okx,
                Channel::Account,
                FeedPriority::Critical,
            )],
        };
        let result = try_spawn_supervised_feed(
            Arc::new(OkxAdapter::new("ws://127.0.0.1:9", "ws://127.0.0.1:9")),
            vec![plan.clone(), plan],
            no_bootstrap(),
            4,
            ConnectionAttemptPacer::new(Duration::ZERO),
            ReconnectPolicy::default(),
        );

        assert!(matches!(
            result,
            Err(SupervisedFeedSpawnError::DuplicateConnectionId { conn_id })
                if conn_id == reap_core::ConnId::new("private-account-r0")
        ));
    }

    #[tokio::test]
    async fn process_shared_pacers_reserve_distinct_handshake_slots() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("connect.pacer");
        let interval = Duration::from_millis(50);
        let first = ConnectionAttemptPacer::process_shared(interval, &path).unwrap();
        let second = ConnectionAttemptPacer::process_shared(interval, &path).unwrap();
        let (_first_shutdown, mut first_shutdown) = watch::channel(false);
        let (_second_shutdown, mut second_shutdown) = watch::channel(false);

        assert!(first.wait_for_turn(&mut first_shutdown).await.unwrap());
        let started = Instant::now();
        assert!(second.wait_for_turn(&mut second_shutdown).await.unwrap());

        assert!(started.elapsed() >= Duration::from_millis(40));
        let state = std::fs::read_to_string(path).unwrap();
        assert!(state.starts_with(SHARED_PACER_STATE_MAGIC));
    }

    #[tokio::test]
    async fn process_shared_pacer_wait_remains_shutdown_cancellable() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("connect.pacer");
        let interval = Duration::from_millis(250);
        let first = ConnectionAttemptPacer::process_shared(interval, &path).unwrap();
        let second = ConnectionAttemptPacer::process_shared(interval, &path).unwrap();
        let (_first_shutdown, mut first_shutdown) = watch::channel(false);
        assert!(first.wait_for_turn(&mut first_shutdown).await.unwrap());
        let (shutdown_tx, mut shutdown_rx) = watch::channel(false);

        let waiter = tokio::spawn(async move { second.wait_for_turn(&mut shutdown_rx).await });
        tokio::time::sleep(Duration::from_millis(20)).await;
        shutdown_tx.send(true).unwrap();

        assert!(!waiter.await.unwrap().unwrap());
    }

    #[tokio::test]
    async fn process_shared_pacer_does_not_reserve_after_shutdown() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("connect.pacer");
        let pacer =
            ConnectionAttemptPacer::process_shared(Duration::from_millis(400), &path).unwrap();
        let (_shutdown_tx, mut shutdown_rx) = watch::channel(true);

        assert!(!pacer.wait_for_turn(&mut shutdown_rx).await.unwrap());
        assert!(std::fs::read(path).unwrap().is_empty());
    }

    #[tokio::test]
    async fn process_shared_pacer_fails_closed_on_a_stuck_file_lock() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("connect.pacer");
        let pacer =
            ConnectionAttemptPacer::process_shared(Duration::from_millis(400), &path).unwrap();
        let blocker = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .unwrap();
        blocker.lock().unwrap();
        let (_shutdown_tx, mut shutdown_rx) = watch::channel(false);
        let started = Instant::now();

        let error = pacer.wait_for_turn(&mut shutdown_rx).await.unwrap_err();

        blocker.unlock().unwrap();
        assert!(matches!(
            error,
            ConnectionAttemptPacerError::LockTimeout { .. }
        ));
        assert!(started.elapsed() >= MAX_SHARED_PACER_LOCK_WAIT);
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[tokio::test]
    async fn process_shared_pacer_resets_state_from_another_boot() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("connect.pacer");
        let stale = format!(
            "{SHARED_PACER_STATE_MAGIC} 00000000-0000-0000-0000-000000000000 {:039}\n",
            u128::MAX
        );
        std::fs::write(&path, stale).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        }
        let pacer =
            ConnectionAttemptPacer::process_shared(Duration::from_millis(400), &path).unwrap();
        let (_shutdown_tx, mut shutdown_rx) = watch::channel(false);
        let started = Instant::now();

        assert!(pacer.wait_for_turn(&mut shutdown_rx).await.unwrap());

        assert!(started.elapsed() < Duration::from_millis(100));
    }

    #[tokio::test]
    async fn process_shared_pacer_rejects_implausibly_distant_reservations() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("connect.pacer");
        let pacer =
            ConnectionAttemptPacer::process_shared(Duration::from_millis(400), &path).unwrap();
        let (_initial_shutdown_tx, mut initial_shutdown_rx) = watch::channel(false);
        assert!(pacer.wait_for_turn(&mut initial_shutdown_rx).await.unwrap());
        let state = std::fs::read_to_string(&path).unwrap();
        let mut fields = state.split_whitespace();
        assert_eq!(fields.next(), Some(SHARED_PACER_STATE_MAGIC));
        let clock_id = fields.next().unwrap();
        let next_attempt_ns = fields.next().unwrap().parse::<u128>().unwrap();
        assert!(fields.next().is_none());
        let distant = next_attempt_ns
            + MAX_SHARED_RESERVATION_AHEAD.as_nanos()
            + Duration::from_secs(1).as_nanos();
        std::fs::write(
            &path,
            format!("{SHARED_PACER_STATE_MAGIC} {clock_id} {distant:039}\n"),
        )
        .unwrap();
        let (_shutdown_tx, mut shutdown_rx) = watch::channel(false);

        let error = pacer.wait_for_turn(&mut shutdown_rx).await.unwrap_err();

        assert!(matches!(
            error,
            ConnectionAttemptPacerError::ReservationTooFar { .. }
        ));
    }

    #[test]
    fn process_shared_pacer_rejects_malformed_or_exposed_state_files() {
        let directory = tempfile::tempdir().unwrap();
        let malformed = directory.path().join("malformed.pacer");
        std::fs::write(&malformed, b"not-reap-state\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&malformed, std::fs::Permissions::from_mode(0o600)).unwrap();
        }
        assert!(matches!(
            ConnectionAttemptPacer::process_shared(Duration::from_millis(400), &malformed),
            Err(ConnectionAttemptPacerError::InvalidState { .. })
        ));

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let exposed = directory.path().join("exposed.pacer");
            std::fs::write(&exposed, b"").unwrap();
            std::fs::set_permissions(&exposed, std::fs::Permissions::from_mode(0o640)).unwrap();
            assert!(matches!(
                ConnectionAttemptPacer::process_shared(Duration::from_millis(400), &exposed),
                Err(ConnectionAttemptPacerError::UnsafeFile { .. })
            ));
        }
    }

    #[test]
    fn recovery_routes_include_every_redundant_book_socket() {
        let mut subscription = Subscription::public(
            Venue::Okx,
            Channel::Books,
            "BTC-USDT",
            FeedPriority::Critical,
        );
        subscription.connections = 2;
        let plans = crate::partition_subscriptions(&[subscription], 10).unwrap();
        let count = plans
            .iter()
            .filter(|plan| {
                plan.subscriptions
                    .iter()
                    .any(|subscription| subscription.symbol.as_deref() == Some("BTC-USDT"))
            })
            .count();
        assert_eq!(count, 2);
    }

    #[test]
    fn unscoped_recovery_notifies_every_registered_socket() {
        let (first_route, mut first_rx) = watch::channel(0_u64);
        let (second_route, mut second_rx) = watch::channel(0_u64);
        let (_raw_tx, raw_rx) = mpsc::channel(1);
        let (_status_tx, status_rx) = mpsc::channel(1);
        let (shutdown, _shutdown_rx) = shutdown_channel();
        let feed = SupervisedFeed {
            raw: raw_rx,
            status: status_rx,
            shutdown,
            recovery_routes: HashMap::from([(
                (Venue::Okx, "BTC-USDT".to_string()),
                vec![
                    (ConnId::new("book-0"), first_route),
                    (ConnId::new("book-1"), second_route),
                ],
            )]),
            _recovery_guards: Vec::new(),
            tasks: Vec::new(),
        };
        let request = RecoveryRequest {
            stream: crate::FeedStreamId {
                venue: Venue::Okx,
                channel: Channel::Books,
                symbol: "BTC-USDT".to_string(),
            },
            source_conn_id: None,
            expected_prev: Some(10),
            received_prev: 11,
            received_seq: 12,
        };

        assert_eq!(feed.request_recovery(&request), 2);
        assert!(first_rx.has_changed().unwrap());
        assert!(second_rx.has_changed().unwrap());
        assert_eq!(*first_rx.borrow_and_update(), 1);
        assert_eq!(*second_rx.borrow_and_update(), 1);
    }

    #[test]
    fn source_scoped_recovery_only_notifies_failed_socket() {
        let (first_route, first_rx) = watch::channel(0_u64);
        let (second_route, mut second_rx) = watch::channel(0_u64);
        let (_raw_tx, raw_rx) = mpsc::channel(1);
        let (_status_tx, status_rx) = mpsc::channel(1);
        let (shutdown, _shutdown_rx) = shutdown_channel();
        let failed_source = ConnId::new("book-1");
        let feed = SupervisedFeed {
            raw: raw_rx,
            status: status_rx,
            shutdown,
            recovery_routes: HashMap::from([(
                (Venue::Okx, "BTC-USDT".to_string()),
                vec![
                    (ConnId::new("book-0"), first_route),
                    (failed_source.clone(), second_route),
                ],
            )]),
            _recovery_guards: Vec::new(),
            tasks: Vec::new(),
        };
        let request = RecoveryRequest {
            stream: crate::FeedStreamId {
                venue: Venue::Okx,
                channel: Channel::Books,
                symbol: "BTC-USDT".to_string(),
            },
            source_conn_id: Some(failed_source),
            expected_prev: Some(10),
            received_prev: 11,
            received_seq: 12,
        };

        assert_eq!(feed.request_recovery(&request), 1);
        assert!(!first_rx.has_changed().unwrap());
        assert!(second_rx.has_changed().unwrap());
        assert_eq!(*second_rx.borrow_and_update(), 1);
    }

    #[test]
    fn private_bootstrap_builds_login_per_attempt() {
        let attempt = Arc::new(AtomicUsize::new(0));
        let bound_url = "wss://private.example/ws/v5/private";
        let factory = BootstrapFactory::bind_private_websocket(bound_url, {
            let attempt = Arc::clone(&attempt);
            move |_| {
                let attempt = attempt.fetch_add(1, Ordering::Relaxed) + 1;
                Ok(private_login(attempt))
            }
        });
        let private = SocketPlan {
            conn_id: reap_core::ConnId::new("private"),
            venue: Venue::Okx,
            private: true,
            subscriptions: vec![Subscription::private(
                Venue::Okx,
                Channel::Orders,
                FeedPriority::Critical,
            )],
        };
        let public = SocketPlan {
            conn_id: reap_core::ConnId::new("public"),
            venue: Venue::Okx,
            private: false,
            subscriptions: vec![Subscription::public(
                Venue::Okx,
                Channel::Books,
                "BTC-USDT",
                FeedPriority::Critical,
            )],
        };

        assert!(factory.generate(&private, bound_url).unwrap().is_some());
        assert_eq!(attempt.load(Ordering::Relaxed), 1);
        assert!(
            factory
                .generate(&public, "wss://public.example")
                .unwrap()
                .is_none()
        );
        assert_eq!(attempt.load(Ordering::Relaxed), 1);

        assert!(factory.generate(&private, bound_url).unwrap().is_some());
        assert_eq!(attempt.load(Ordering::Relaxed), 2);
    }

    #[tokio::test]
    async fn mismatched_private_destination_is_fatal_before_bootstrap_generation() {
        let attempts = Arc::new(AtomicUsize::new(0));
        let factory =
            BootstrapFactory::bind_private_websocket("wss://bound.example:8443/ws/v5/private", {
                let attempts = Arc::clone(&attempts);
                move |_| {
                    attempts.fetch_add(1, Ordering::Relaxed);
                    Ok(private_login(1))
                }
            });
        let plan = SocketPlan {
            conn_id: ConnId::new("private-destination-mismatch"),
            venue: Venue::Okx,
            private: true,
            subscriptions: vec![Subscription::private(
                Venue::Okx,
                Channel::Orders,
                FeedPriority::Critical,
            )],
        };
        let (output, _output_rx) = mpsc::channel(1);
        let (status, mut status_rx) = mpsc::channel(1);
        let (_shutdown_tx, shutdown) = shutdown_channel();
        let (_recovery_tx, recovery) = watch::channel(0_u64);

        supervise_connection(
            Arc::new(OkxAdapter::new(
                "wss://public.example:8443/ws/v5/public",
                "wss://selected.example:8443/ws/v5/private",
            )),
            plan,
            Arc::new(factory),
            ConnectionChannels {
                output,
                status,
                shutdown,
                recovery,
            },
            ConnectionAttemptPacer::new(Duration::ZERO),
            ReconnectPolicy::default(),
        )
        .await;

        let fatal = status_rx.recv().await.unwrap();
        assert_eq!(fatal.kind, ConnectionStatusKind::Fatal);
        assert!(fatal.reason.contains("bound.example"));
        assert!(fatal.reason.contains("selected.example"));
        assert_eq!(attempts.load(Ordering::Relaxed), 0);
        assert!(status_rx.recv().await.is_none());
    }

    #[tokio::test]
    async fn raw_order_subscription_is_rejected_before_bootstrap_or_connection() {
        let bootstrap_attempts = Arc::new(AtomicUsize::new(0));
        let websocket_calls = Arc::new(AtomicUsize::new(0));
        let subscription_calls = Arc::new(AtomicUsize::new(0));
        let bound_url = "wss://bound.example:8443/ws/v5/private";
        let factory = BootstrapFactory::bind_private_websocket(bound_url, {
            let bootstrap_attempts = Arc::clone(&bootstrap_attempts);
            move |_| {
                bootstrap_attempts.fetch_add(1, Ordering::Relaxed);
                Ok(private_login(1))
            }
        });
        let adapter = SubstitutingSubscriptionAdapter {
            delegate: OkxAdapter::new("wss://public.example:8443/ws/v5/public", bound_url),
            websocket_calls: Arc::clone(&websocket_calls),
            subscription_calls: Arc::clone(&subscription_calls),
            payload: r#"{"id":"attack","op":"order","args":[{"instId":"BTC-USDT","side":"buy"}]}"#
                .to_string(),
        };
        let plan = SocketPlan {
            conn_id: ConnId::new("raw-order-subscription"),
            venue: Venue::Okx,
            private: true,
            subscriptions: vec![Subscription::private(
                Venue::Okx,
                Channel::Orders,
                FeedPriority::Critical,
            )],
        };
        let (output, _output_rx) = mpsc::channel(1);
        let (status, mut status_rx) = mpsc::channel(1);
        let (_shutdown_tx, shutdown) = shutdown_channel();
        let (_recovery_tx, recovery) = watch::channel(0_u64);

        supervise_connection(
            Arc::new(adapter),
            plan,
            Arc::new(factory),
            ConnectionChannels {
                output,
                status,
                shutdown,
                recovery,
            },
            ConnectionAttemptPacer::new(Duration::ZERO),
            ReconnectPolicy::default(),
        )
        .await;

        let fatal = status_rx.recv().await.unwrap();
        assert_eq!(fatal.kind, ConnectionStatusKind::Fatal);
        assert!(fatal.reason.contains("no subscribe operation"));
        assert_eq!(subscription_calls.load(Ordering::Relaxed), 1);
        assert_eq!(websocket_calls.load(Ordering::Relaxed), 0);
        assert_eq!(bootstrap_attempts.load(Ordering::Relaxed), 0);
        assert!(status_rx.recv().await.is_none());
    }

    #[tokio::test]
    async fn same_count_private_channel_substitution_is_rejected_before_bootstrap_or_connection() {
        let bootstrap_attempts = Arc::new(AtomicUsize::new(0));
        let factory = BootstrapFactory::bind_private_websocket(ATTACK_BOUND_URL, {
            let bootstrap_attempts = Arc::clone(&bootstrap_attempts);
            move |_| {
                bootstrap_attempts.fetch_add(1, Ordering::Relaxed);
                Ok(private_login(1))
            }
        });
        let plan = SocketPlan {
            conn_id: ConnId::new("private-channel-substitution"),
            venue: Venue::Okx,
            private: true,
            subscriptions: vec![Subscription::private(
                Venue::Okx,
                Channel::Orders,
                FeedPriority::Critical,
            )],
        };

        assert_subscription_substitution_is_fatal(
            plan,
            factory,
            r#"{"op":"subscribe","args":[{"channel":"positions","instType":"ANY"}]}"#,
            "does not exactly match the bound socket plan",
        )
        .await;
        assert_eq!(bootstrap_attempts.load(Ordering::Relaxed), 0);
    }

    #[tokio::test]
    async fn same_count_public_symbol_substitution_is_rejected_before_connection() {
        let plan = SocketPlan {
            conn_id: ConnId::new("public-symbol-substitution"),
            venue: Venue::Okx,
            private: false,
            subscriptions: vec![Subscription::public(
                Venue::Okx,
                Channel::Books,
                "BTC-USDT",
                FeedPriority::Critical,
            )],
        };

        assert_subscription_substitution_is_fatal(
            plan,
            no_bootstrap(),
            r#"{"op":"subscribe","args":[{"channel":"books","instId":"ETH-USDT"}]}"#,
            "does not exactly match the bound socket plan",
        )
        .await;
    }
}
