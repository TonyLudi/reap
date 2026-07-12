use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use reap_core::{
    Channel, ConnId, FeedPriority, NormalizedEvent, OrderStatus, Subscription, SystemEvent,
    SystemEventKind, TimerEvent, Venue,
};
use reap_feed::{
    ConnectionStatus, ConnectionStatusKind, FeedOutput, FeedProcessor, ReconnectPolicy, SocketPlan,
    SupervisedFeed, okx_login_bootstrap, partition_subscriptions, spawn_supervised_feed,
};
use reap_order::{CancelOutcome, OkxOrderGateway, SubmitOutcome, SubmitPreparation, reconcile};
use reap_storage::{
    BootstrapRecord, OrderOperation, OrderRequestRecord, StorageConfig, StorageError,
    StorageRecord, StorageRuntime, StorageSink, recover_jsonl, start_jsonl_storage,
};
use reap_venue::okx::{OkxAdapter, OkxRestClient, OkxSigner, ReqwestTransport};
use reap_venue::{PrivateOrderState, PrivateOrderUpdate, RemoteFill, RemoteOrder, VenueAdapter};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::{
    AccountBootstrapSnapshot, CancelAction, CoordinatorError, CoordinatorOutput, LiveAction,
    LiveConfig, LiveConfigError, LiveCoordinator, ReadinessSnapshot, ReconcileAction,
    ReconciliationResult, StartupGate, SubmitAction, TradingEnvironment, VerifiedBootstrap,
    okx_instrument_type, verify_bootstrap,
};

type LiveGateway = OkxOrderGateway<ReqwestTransport>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LiveMode {
    Validate,
    Observe,
    Demo,
}

#[derive(Debug, Clone)]
pub struct LiveRunOptions {
    pub mode: LiveMode,
    pub demo_confirmed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LiveRunReport {
    pub mode: LiveMode,
    pub readiness: ReadinessSnapshot,
    pub dropped_storage_records: u64,
}

#[derive(Debug, Error)]
pub enum LiveRuntimeError {
    #[error(transparent)]
    Config(#[from] LiveConfigError),
    #[error("demo order entry requires explicit confirmation")]
    DemoConfirmationRequired,
    #[error("demo mode refuses production exchange configuration")]
    DemoRequiresSimulatedTrading,
    #[error("account {account_id} bootstrap failed: {message}")]
    Bootstrap { account_id: String, message: String },
    #[error("bootstrap verification failed: {0}")]
    BootstrapVerification(String),
    #[error("checkpoint identity mismatch for account {account_id}: {message}")]
    CheckpointIdentity { account_id: String, message: String },
    #[error("feed subscription planning failed: {0}")]
    Subscription(String),
    #[error("order gateway setup failed for account {account_id}: {message}")]
    GatewaySetup { account_id: String, message: String },
    #[error(transparent)]
    Coordinator(#[from] CoordinatorError),
    #[error(transparent)]
    Storage(#[from] StorageError),
    #[error("runtime event channel closed")]
    EventChannelClosed,
    #[error("order command queue for account {0} is unavailable or full")]
    OrderQueueUnavailable(String),
    #[error("book recovery request had no matching public socket for {0}")]
    MissingRecoveryRoute(String),
    #[error("live readiness timed out after {0}ms")]
    ReadinessTimeout(u64),
    #[error("gateway task failed: {0}")]
    GatewayTask(String),
    #[error(
        "shutdown timed out with {active_orders} active local orders and {unreconciled_accounts} unreconciled accounts"
    )]
    ShutdownUnresolved {
        active_orders: usize,
        unreconciled_accounts: usize,
    },
    #[error("feed adapter failed: {0}")]
    FeedAdapter(String),
    #[error("runtime task failed: {0}")]
    Join(#[from] tokio::task::JoinError),
    #[error("shutdown signal failed: {0}")]
    Signal(#[from] std::io::Error),
}

pub async fn run_live_path(
    path: impl AsRef<Path>,
    options: LiveRunOptions,
) -> Result<LiveRunReport, LiveRuntimeError> {
    let config = LiveConfig::load(path)?;
    run_live(config, options).await
}

pub async fn run_live(
    config: LiveConfig,
    options: LiveRunOptions,
) -> Result<LiveRunReport, LiveRuntimeError> {
    config.ensure_valid()?;
    if options.mode == LiveMode::Validate {
        return Ok(LiveRunReport {
            mode: options.mode,
            readiness: StartupGate::new(&config).snapshot(),
            dropped_storage_records: 0,
        });
    }
    if options.mode == LiveMode::Demo {
        if !options.demo_confirmed {
            return Err(LiveRuntimeError::DemoConfirmationRequired);
        }
        if config.venue.environment != TradingEnvironment::Demo {
            return Err(LiveRuntimeError::DemoRequiresSimulatedTrading);
        }
    }
    let runtime = LiveRuntime::build(config, options.mode).await?;
    runtime.run().await
}

struct AccountSeed {
    account_id: String,
    signer: OkxSigner,
    gateway: LiveGateway,
}

async fn bootstrap_accounts(
    config: &LiveConfig,
    restored_orders: &HashMap<String, Vec<reap_core::OrderUpdate>>,
) -> Result<
    (
        VerifiedBootstrap,
        Vec<AccountSeed>,
        HashMap<String, AccountBootstrapSnapshot>,
    ),
    LiveRuntimeError,
> {
    let mut snapshots = HashMap::new();
    let mut seeds = Vec::new();
    for account in &config.accounts {
        let credentials = account.credentials_from_env()?;
        let signer = OkxSigner::new(credentials, config.venue.environment.is_demo());
        let transport = ReqwestTransport::with_timeouts(
            &config.venue.rest_url,
            Duration::from_millis(config.runtime.rest_connect_timeout_ms),
            Duration::from_millis(config.runtime.rest_request_timeout_ms),
        )
        .map_err(|error| LiveRuntimeError::Bootstrap {
            account_id: account.id.clone(),
            message: error.to_string(),
        })?;
        let client = OkxRestClient::new(transport, signer.clone());
        let account_config = client
            .account_config()
            .await
            .map_err(|error| bootstrap_error(&account.id, "account config", error.to_string()))?;
        let balance = client
            .account_balance()
            .await
            .map_err(|error| bootstrap_error(&account.id, "account balance", error.to_string()))?;
        let positions = client
            .account_positions(None, None)
            .await
            .map_err(|error| {
                bootstrap_error(&account.id, "account positions", error.to_string())
            })?;
        let mut open_orders = client
            .open_orders(None, None)
            .await
            .map_err(|error| bootstrap_error(&account.id, "open orders", error.to_string()))?;
        let recent_fills = client
            .fills(None, None)
            .await
            .map_err(|error| bootstrap_error(&account.id, "recent fills", error.to_string()))?;
        let mut remote_ids = open_orders
            .iter()
            .map(remote_order_id)
            .collect::<HashSet<_>>();
        for restored in restored_orders.get(&account.id).into_iter().flatten() {
            if remote_ids.contains(&restored.order_id) {
                continue;
            }
            let details = match client
                .order_details(&restored.symbol, None, Some(&restored.order_id))
                .await
            {
                Ok(details) => details,
                Err(error)
                    if error.is_order_not_found()
                        && unix_time_ms().saturating_sub(restored.ts_ms)
                            < config.runtime.ambiguous_submit_grace_ms =>
                {
                    continue;
                }
                Err(error) if error.is_order_not_found() => RemoteOrder {
                    exchange_order_id: String::new(),
                    client_order_id: restored.order_id.clone(),
                    symbol: restored.symbol.clone(),
                    side: restored.side,
                    state: PrivateOrderState::Rejected,
                    price: restored.price,
                    qty: restored.qty,
                    cumulative_filled_qty: restored.filled_qty,
                    average_fill_price: restored.avg_fill_price,
                    update_time_ms: unix_time_ms(),
                },
                Err(error) => {
                    return Err(bootstrap_error(
                        &account.id,
                        &format!("order details {}", restored.order_id),
                        error.to_string(),
                    ));
                }
            };
            remote_ids.insert(remote_order_id(&details));
            open_orders.push(details);
        }
        let mut instruments = HashMap::new();
        for instrument in config.instruments_for_account(&account.id) {
            let metadata = client
                .account_instruments(
                    okx_instrument_type(instrument.kind),
                    Some(&instrument.symbol),
                )
                .await
                .map_err(|error| {
                    bootstrap_error(
                        &account.id,
                        &format!("instrument {}", instrument.symbol),
                        error.to_string(),
                    )
                })?
                .into_iter()
                .find(|metadata| metadata.symbol == instrument.symbol)
                .ok_or_else(|| LiveRuntimeError::Bootstrap {
                    account_id: account.id.clone(),
                    message: format!("exchange returned no metadata for {}", instrument.symbol),
                })?;
            instruments.insert(instrument.symbol.clone(), metadata);
        }
        snapshots.insert(
            account.id.clone(),
            AccountBootstrapSnapshot {
                account_config,
                instruments,
                balance,
                positions,
                open_orders,
                recent_fills,
            },
        );
        let trade_modes = account
            .trade_modes
            .iter()
            .map(|(symbol, mode)| (symbol.clone(), (*mode).into()))
            .collect();
        let gateway = OkxOrderGateway::new(
            client,
            account.id_prefix.clone(),
            account.node_id,
            trade_modes,
            config.runtime.pacing_policy(),
        )
        .map_err(|error| LiveRuntimeError::GatewaySetup {
            account_id: account.id.clone(),
            message: error.to_string(),
        })?;
        seeds.push(AccountSeed {
            account_id: account.id.clone(),
            signer,
            gateway,
        });
    }
    let verified = verify_bootstrap(config, &snapshots)
        .map_err(|error| LiveRuntimeError::BootstrapVerification(error.to_string()))?;
    Ok((verified, seeds, snapshots))
}

fn bootstrap_error(account_id: &str, operation: &str, message: String) -> LiveRuntimeError {
    LiveRuntimeError::Bootstrap {
        account_id: account_id.to_string(),
        message: format!("{operation}: {message}"),
    }
}

fn remote_order_id(order: &RemoteOrder) -> String {
    if order.client_order_id.is_empty() {
        order.exchange_order_id.clone()
    } else {
        order.client_order_id.clone()
    }
}

fn private_update_from_remote(order: RemoteOrder) -> PrivateOrderUpdate {
    PrivateOrderUpdate {
        ts_ms: if order.update_time_ms == 0 {
            unix_time_ms()
        } else {
            order.update_time_ms
        },
        exchange_order_id: order.exchange_order_id,
        client_order_id: order.client_order_id,
        symbol: order.symbol,
        side: order.side,
        state: order.state,
        price: order.price,
        qty: order.qty,
        cumulative_filled_qty: order.cumulative_filled_qty,
        average_fill_price: order.average_fill_price,
        last_fill_qty: 0.0,
        last_fill_price: 0.0,
        liquidity: None,
        fill_id: None,
        reject_reason: if order.state == PrivateOrderState::Rejected {
            "order not present during restart reconciliation".to_string()
        } else {
            String::new()
        },
    }
}

struct LiveRuntime {
    mode: LiveMode,
    coordinator: LiveCoordinator,
    processor: FeedProcessor,
    storage: Option<StorageRuntime>,
    storage_sink: StorageSink,
    control_rx: mpsc::Receiver<RuntimeEvent>,
    feed_rx: mpsc::Receiver<RuntimeEvent>,
    order_senders: HashMap<String, mpsc::Sender<OrderTaskCommand>>,
    order_tasks: Vec<JoinHandle<()>>,
    feeds: Vec<SupervisedFeed>,
    feed_tasks: Vec<JoinHandle<()>>,
    sources: Vec<FeedSourceState>,
    public_feed_index: usize,
    reconcile_inflight: HashSet<String>,
    cancel_inflight: HashSet<(String, String)>,
    last_reconcile_attempt: HashMap<String, Instant>,
    readiness_timeout_ms: u64,
    timer_interval_ms: u64,
    max_feed_age_ms: u64,
    shutdown_timeout_ms: u64,
}

impl LiveRuntime {
    async fn build(config: LiveConfig, mode: LiveMode) -> Result<Self, LiveRuntimeError> {
        let config_fingerprint = config.fingerprint()?;
        let recovered = recover_jsonl(&config.storage.path)?;
        for (account_id, (strategy_name, fingerprint)) in &recovered.bootstrap_identities {
            if config.account(account_id).is_none() {
                return Err(LiveRuntimeError::CheckpointIdentity {
                    account_id: account_id.clone(),
                    message: "checkpoint account is not present in the live config".to_string(),
                });
            }
            if strategy_name != &config.strategy.strategy_name || fingerprint != &config_fingerprint
            {
                return Err(LiveRuntimeError::CheckpointIdentity {
                    account_id: account_id.clone(),
                    message: "strategy name or live config fingerprint changed; rotate the storage path after reconciling all orders".to_string(),
                });
            }
        }
        let recovered_orders = recovered
            .latest_orders
            .values()
            .filter(|update| {
                matches!(
                    update.status,
                    OrderStatus::PendingNew | OrderStatus::Live | OrderStatus::PartiallyFilled
                )
            })
            .cloned()
            .collect::<Vec<_>>();
        let mut restored_by_account: HashMap<String, Vec<reap_core::OrderUpdate>> = HashMap::new();
        for update in &recovered_orders {
            let account_id = config
                .account_for_symbol(&update.symbol)
                .map(|account| account.id.clone())
                .ok_or_else(|| {
                    LiveRuntimeError::BootstrapVerification(format!(
                        "recovered order {} has unmapped symbol {}",
                        update.order_id, update.symbol
                    ))
                })?;
            restored_by_account
                .entry(account_id)
                .or_default()
                .push(update.clone());
        }
        let (mut verified, seeds, snapshots) =
            bootstrap_accounts(&config, &restored_by_account).await?;
        let mut bootstrap_records = Vec::new();
        for account in &config.accounts {
            let exchange_baseline = verified
                .baseline_fill_ids
                .get(&account.id)
                .cloned()
                .unwrap_or_default();
            let mut fill_ids = recovered
                .baseline_fill_ids
                .get(&account.id)
                .cloned()
                .unwrap_or_else(|| exchange_baseline.clone());
            for fill in &recovered.fills {
                let fill_account_id = fill.account_id.as_deref().or_else(|| {
                    config
                        .account_for_symbol(&fill.symbol)
                        .map(|owner| owner.id.as_str())
                });
                if fill_account_id == Some(account.id.as_str()) {
                    fill_ids.insert(fill.fill_id.clone());
                }
            }
            verified
                .baseline_fill_ids
                .insert(account.id.clone(), fill_ids);
            if !recovered.baseline_fill_ids.contains_key(&account.id) {
                let mut baseline_fill_ids = exchange_baseline.into_iter().collect::<Vec<_>>();
                baseline_fill_ids.sort();
                bootstrap_records.push(StorageRecord::Bootstrap(BootstrapRecord {
                    ts_ms: unix_time_ms(),
                    account_id: account.id.clone(),
                    strategy_name: config.strategy.strategy_name.clone(),
                    config_fingerprint: config_fingerprint.clone(),
                    baseline_fill_ids,
                }));
            }
        }
        let session_id = format!("{:x}", unix_time_ns());
        let mut coordinator =
            LiveCoordinator::new(config.clone(), verified, mode == LiveMode::Demo, session_id)?;
        let mut initial_outputs = vec![CoordinatorOutput {
            actions: Vec::new(),
            records: bootstrap_records,
        }];
        for update in recovered_orders {
            let account_id = config
                .account_for_symbol(&update.symbol)
                .map(|account| account.id.clone())
                .ok_or_else(|| {
                    LiveRuntimeError::BootstrapVerification(format!(
                        "recovered order {} has unmapped symbol {}",
                        update.order_id, update.symbol
                    ))
                })?;
            initial_outputs.push(coordinator.restore_order(&account_id, update)?);
        }
        for account in &config.accounts {
            let snapshot = snapshots.get(&account.id).ok_or_else(|| {
                LiveRuntimeError::BootstrapVerification(format!(
                    "missing reconciliation snapshot for {}",
                    account.id
                ))
            })?;
            for fill in &snapshot.recent_fills {
                let order_id = if fill.client_order_id.is_empty() {
                    &fill.exchange_order_id
                } else {
                    &fill.client_order_id
                };
                let should_apply = coordinator.private_state(&account.id).is_some_and(|state| {
                    state.order_reducer().contains_order(order_id)
                        && !state.seen_fill_ids().contains(&fill.fill_id)
                });
                if should_apply {
                    initial_outputs.push(coordinator.process_feed(FeedOutput::PrivateFill {
                        account_id: Some(account.id.clone()),
                        fill: fill.clone(),
                    })?);
                }
            }
            for remote in &snapshot.open_orders {
                let order_id = remote_order_id(remote);
                let known = coordinator
                    .private_state(&account.id)
                    .is_some_and(|state| state.order_reducer().contains_order(&order_id));
                if known {
                    initial_outputs.push(coordinator.process_feed(FeedOutput::PrivateOrder {
                        account_id: Some(account.id.clone()),
                        update: private_update_from_remote(remote.clone()),
                    })?);
                }
            }
            initial_outputs.push(coordinator.process_feed(FeedOutput::PrivateAccount {
                account_id: Some(account.id.clone()),
                update: snapshot.scoped_account_update(&account.id),
            })?);
            let state = coordinator
                .private_state(&account.id)
                .ok_or_else(|| CoordinatorError::UnknownAccount(account.id.clone()))?;
            let report = reconcile(
                state.order_reducer(),
                state.seen_fill_ids(),
                &snapshot.open_orders,
                &snapshot.recent_fills,
            );
            initial_outputs.push(coordinator.on_reconciliation(ReconciliationResult {
                account_id: account.id.clone(),
                ts_ms: unix_time_ms(),
                clean: report.is_clean(),
                local_live_orders: report.local_live_orders,
                remote_live_orders: report.remote_live_orders,
                remote_recent_fills: report.remote_fills,
                reason: if report.is_clean() {
                    "startup REST reconciliation is clean".to_string()
                } else {
                    format!("startup reconciliation drift: {:?}", report.issues)
                },
            })?);
        }
        let storage = start_jsonl_storage(StorageConfig {
            path: config.storage.path.clone(),
            channel_capacity: config.storage.channel_capacity,
            flush_every_records: config.storage.flush_every_records,
        })
        .await?;
        let storage_sink = storage.sink();
        coordinator.mark_storage_ready(true, "storage file opened");

        let (control_tx, control_rx) = mpsc::channel(config.runtime.event_channel_capacity);
        let (feed_tx, feed_rx) = mpsc::channel(config.runtime.event_channel_capacity);
        let mut feeds = Vec::new();
        let mut feed_tasks = Vec::new();
        let mut sources = Vec::new();

        let public_subscriptions = public_subscriptions(&config);
        let public_plans = partition_subscriptions(
            &public_subscriptions,
            config.runtime.max_subscriptions_per_socket,
        )
        .map_err(|error| LiveRuntimeError::Subscription(error.to_string()))?;
        let public_adapter: Arc<dyn VenueAdapter> = Arc::new(OkxAdapter::new(
            &config.venue.public_ws_url,
            &config.venue.private_ws_url,
        ));
        let mut public_feed = spawn_supervised_feed(
            Arc::clone(&public_adapter),
            public_plans.clone(),
            reap_feed::no_bootstrap(),
            config.runtime.feed_channel_capacity,
            ReconnectPolicy::default(),
        );
        let public_source_id = sources.len();
        sources.push(FeedSourceState::public(public_adapter, &public_plans));
        spawn_feed_forwarders(
            public_source_id,
            &mut public_feed,
            &feed_tx,
            &mut feed_tasks,
        );
        feeds.push(public_feed);
        let public_feed_index = 0;

        let mut order_senders = HashMap::new();
        let mut order_tasks = Vec::new();
        for seed in seeds {
            let private_subscriptions =
                private_subscriptions(config.venue.enable_vip_fills_channel);
            let private_plans = partition_subscriptions(
                &private_subscriptions,
                config.runtime.max_subscriptions_per_socket,
            )
            .map_err(|error| LiveRuntimeError::Subscription(error.to_string()))?;
            let private_adapter: Arc<dyn VenueAdapter> = Arc::new(
                OkxAdapter::new(&config.venue.public_ws_url, &config.venue.private_ws_url)
                    .with_account_id(&seed.account_id),
            );
            let mut private_feed = spawn_supervised_feed(
                Arc::clone(&private_adapter),
                private_plans.clone(),
                okx_login_bootstrap(seed.signer),
                config.runtime.feed_channel_capacity,
                ReconnectPolicy::default(),
            );
            let source_id = sources.len();
            sources.push(FeedSourceState::private(
                private_adapter,
                seed.account_id.clone(),
                &private_plans,
            ));
            spawn_feed_forwarders(source_id, &mut private_feed, &feed_tx, &mut feed_tasks);
            feeds.push(private_feed);

            let (order_tx, order_rx) = mpsc::channel(config.runtime.order_channel_capacity);
            order_senders.insert(seed.account_id.clone(), order_tx);
            order_tasks.push(tokio::spawn(run_order_task(
                seed.account_id,
                seed.gateway,
                order_rx,
                control_tx.clone(),
                config.runtime.ambiguous_submit_grace_ms,
            )));
        }

        let mut runtime = Self {
            mode,
            coordinator,
            processor: FeedProcessor::new(
                config.runtime.dedup_capacity_per_stream,
                config.runtime.max_sequence_buffer,
            ),
            storage: Some(storage),
            storage_sink,
            control_rx,
            feed_rx,
            order_senders,
            order_tasks,
            feeds,
            feed_tasks,
            sources,
            public_feed_index,
            reconcile_inflight: HashSet::new(),
            cancel_inflight: HashSet::new(),
            last_reconcile_attempt: HashMap::new(),
            readiness_timeout_ms: config.runtime.readiness_timeout_ms,
            timer_interval_ms: config.runtime.timer_interval_ms,
            max_feed_age_ms: config.risk.max_feed_age_ms,
            shutdown_timeout_ms: config.runtime.shutdown_timeout_ms,
        };
        for output in initial_outputs {
            runtime.commit_output(output)?;
        }
        Ok(runtime)
    }

    async fn run(mut self) -> Result<LiveRunReport, LiveRuntimeError> {
        let result = self.run_loop().await;
        let readiness = self.coordinator.readiness();
        let dropped_storage_records = self.storage_sink.dropped_records();
        let shutdown_result = self.shutdown().await;
        result?;
        shutdown_result?;
        Ok(LiveRunReport {
            mode: self.mode,
            readiness,
            dropped_storage_records,
        })
    }

    async fn run_loop(&mut self) -> Result<(), LiveRuntimeError> {
        let started = Instant::now();
        let mut reached_ready = false;
        let mut last_phase = self.coordinator.readiness().phase;
        let mut timer = tokio::time::interval(Duration::from_millis(self.timer_interval_ms));
        timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let shutdown = shutdown_signal();
        tokio::pin!(shutdown);

        loop {
            tokio::select! {
                biased;
                signal = &mut shutdown => {
                    signal?;
                    return self.graceful_stop().await;
                }
                event = self.control_rx.recv() => {
                    let event = event.ok_or(LiveRuntimeError::EventChannelClosed)?;
                    self.handle_runtime_event(event)?;
                }
                _ = timer.tick() => {
                    let now_ms = unix_time_ms();
                    for event in self.processor.mark_stale(
                        now_ms,
                        self.coordinator_risk_max_feed_age(),
                    ) {
                        let output = self.coordinator.process_event(NormalizedEvent::System(event));
                        self.commit_output(output)?;
                    }
                    let output = self.coordinator.process_event(NormalizedEvent::Timer(TimerEvent {
                        ts_ms: now_ms,
                        name: "live_tick".to_string(),
                    }));
                    self.commit_output(output)?;
                    self.retry_reconciliation(now_ms)?;
                }
                event = self.feed_rx.recv() => {
                    let event = event.ok_or(LiveRuntimeError::EventChannelClosed)?;
                    self.handle_runtime_event(event)?;
                }
            }

            let readiness = self.coordinator.readiness();
            if readiness.phase != last_phase {
                tracing::info!(from = ?last_phase, to = ?readiness.phase, ?readiness, "live readiness changed");
                last_phase = readiness.phase;
            }
            if readiness.is_ready() {
                reached_ready = true;
            } else if !reached_ready
                && started.elapsed() > Duration::from_millis(self.readiness_timeout_ms)
            {
                return Err(LiveRuntimeError::ReadinessTimeout(
                    self.readiness_timeout_ms,
                ));
            }
        }
    }

    fn coordinator_risk_max_feed_age(&self) -> u64 {
        self.max_feed_age_ms
    }

    async fn graceful_stop(&mut self) -> Result<(), LiveRuntimeError> {
        if self.mode != LiveMode::Demo {
            return Ok(());
        }
        let now_ms = unix_time_ms();
        let output = self
            .coordinator
            .process_event(NormalizedEvent::System(SystemEvent {
                ts_ms: now_ms,
                kind: SystemEventKind::KillSwitchActivated,
                venue: None,
                account_id: None,
                symbol: None,
                reason: "operator shutdown".to_string(),
            }));
        self.commit_output(output)?;

        let deadline = tokio::time::sleep(Duration::from_millis(self.shutdown_timeout_ms));
        tokio::pin!(deadline);
        let mut retry = tokio::time::interval(Duration::from_millis(100));
        retry.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            let readiness = self.coordinator.readiness();
            if self.coordinator.active_order_count() == 0
                && readiness.missing_reconciliation.is_empty()
            {
                return Ok(());
            }
            tokio::select! {
                biased;
                _ = &mut deadline => {
                    return Err(LiveRuntimeError::ShutdownUnresolved {
                        active_orders: self.coordinator.active_order_count(),
                        unreconciled_accounts: self
                            .coordinator
                            .readiness()
                            .missing_reconciliation
                            .len(),
                    });
                }
                event = self.control_rx.recv() => {
                    let event = event.ok_or(LiveRuntimeError::EventChannelClosed)?;
                    self.handle_runtime_event(event)?;
                }
                event = self.feed_rx.recv() => {
                    let event = event.ok_or(LiveRuntimeError::EventChannelClosed)?;
                    self.handle_runtime_event(event)?;
                }
                _ = retry.tick() => {
                    self.retry_reconciliation(unix_time_ms())?;
                }
            }
        }
    }

    fn handle_runtime_event(&mut self, event: RuntimeEvent) -> Result<(), LiveRuntimeError> {
        match event {
            RuntimeEvent::Raw {
                source_id,
                envelope,
            } => {
                let source = self.sources.get(source_id).ok_or_else(|| {
                    LiveRuntimeError::FeedAdapter("unknown feed source".to_string())
                })?;
                self.storage_sink.try_record(StorageRecord::Raw {
                    account_id: source.account_id.clone(),
                    envelope: envelope.clone(),
                })?;
                let parsed = source
                    .adapter
                    .parse(&envelope)
                    .map_err(|error| LiveRuntimeError::FeedAdapter(error.to_string()))?;
                for event in parsed {
                    for output in self.processor.process(event) {
                        self.observe_feed_output(&output);
                        let output = self.coordinator.process_feed(output)?;
                        self.commit_output(output)?;
                    }
                }
            }
            RuntimeEvent::Connection { source_id, status } => {
                let (events, public_connectivity) = {
                    let source = self.sources.get_mut(source_id).ok_or_else(|| {
                        LiveRuntimeError::FeedAdapter("unknown feed source".to_string())
                    })?;
                    let events = source.on_status(status);
                    (events, source.public_connectivity_ready())
                };
                if let Some(ready) = public_connectivity {
                    self.coordinator.mark_public_connectivity(
                        ready,
                        if ready {
                            "every public subscription has an acknowledged connection"
                        } else {
                            "one or more public subscriptions has no acknowledged connection"
                        },
                    );
                }
                for event in events {
                    let output = self
                        .coordinator
                        .process_event(NormalizedEvent::System(event));
                    self.commit_output(output)?;
                }
            }
            RuntimeEvent::SubmitComplete {
                account_id,
                outcome,
                ts_ms,
            } => {
                let output = self
                    .coordinator
                    .on_submit_outcome(&account_id, outcome, ts_ms)?;
                self.commit_output(output)?;
            }
            RuntimeEvent::SubmitFailed {
                account_id,
                client_order_id,
                ts_ms,
                ambiguous,
                reason,
            } => {
                let output = self.coordinator.on_submit_error(
                    &account_id,
                    &client_order_id,
                    ts_ms,
                    ambiguous,
                    reason,
                )?;
                self.commit_output(output)?;
            }
            RuntimeEvent::CancelComplete {
                account_id,
                outcome,
                ts_ms,
            } => {
                let output = self
                    .coordinator
                    .on_cancel_outcome(&account_id, outcome, ts_ms)?;
                self.commit_output(output)?;
            }
            RuntimeEvent::CancelFailed {
                account_id,
                client_order_id,
                ts_ms,
                ambiguous,
                reason,
            } => {
                let output = self.coordinator.on_cancel_error(
                    &account_id,
                    &client_order_id,
                    ts_ms,
                    ambiguous,
                    reason,
                )?;
                self.commit_output(output)?;
            }
            RuntimeEvent::RemoteState {
                account_id,
                remote_orders,
                remote_fills,
                ts_ms,
            } => {
                self.reconcile_inflight.remove(&account_id);
                self.cancel_inflight
                    .retain(|(cancel_account, _)| cancel_account != &account_id);
                self.apply_remote_recovery(&account_id, &remote_orders, &remote_fills)?;
                let state = self
                    .coordinator
                    .private_state(&account_id)
                    .ok_or_else(|| CoordinatorError::UnknownAccount(account_id.clone()))?;
                let report = reconcile(
                    state.order_reducer(),
                    state.seen_fill_ids(),
                    &remote_orders,
                    &remote_fills,
                );
                let reason = if report.is_clean() {
                    "REST and canonical private state agree".to_string()
                } else {
                    format!("{:?}", report.issues)
                };
                let output = self.coordinator.on_reconciliation(ReconciliationResult {
                    account_id,
                    ts_ms,
                    clean: report.is_clean(),
                    local_live_orders: report.local_live_orders,
                    remote_live_orders: report.remote_live_orders,
                    remote_recent_fills: report.remote_fills,
                    reason,
                })?;
                self.commit_output(output)?;
            }
            RuntimeEvent::ReconcileFailed {
                account_id,
                ts_ms,
                reason,
            } => {
                self.reconcile_inflight.remove(&account_id);
                let output = self.coordinator.on_reconciliation(ReconciliationResult {
                    account_id,
                    ts_ms,
                    clean: false,
                    local_live_orders: 0,
                    remote_live_orders: 0,
                    remote_recent_fills: 0,
                    reason: format!("REST reconciliation request failed: {reason}"),
                })?;
                self.commit_output(output)?;
            }
            RuntimeEvent::Fatal(message) => return Err(LiveRuntimeError::GatewayTask(message)),
        }
        Ok(())
    }

    fn observe_feed_output(&mut self, output: &FeedOutput) {
        if let FeedOutput::PrivateOrder {
            account_id: Some(account_id),
            update,
        } = output
            && matches!(
                update.state,
                PrivateOrderState::Filled
                    | PrivateOrderState::Cancelled
                    | PrivateOrderState::Rejected
            )
        {
            let order_id = if update.client_order_id.is_empty() {
                &update.exchange_order_id
            } else {
                &update.client_order_id
            };
            self.cancel_inflight
                .remove(&(account_id.clone(), order_id.clone()));
        }
    }

    fn apply_remote_recovery(
        &mut self,
        account_id: &str,
        remote_orders: &[RemoteOrder],
        remote_fills: &[RemoteFill],
    ) -> Result<(), LiveRuntimeError> {
        for fill in remote_fills {
            let order_id = if fill.client_order_id.is_empty() {
                &fill.exchange_order_id
            } else {
                &fill.client_order_id
            };
            let should_apply = self
                .coordinator
                .private_state(account_id)
                .is_some_and(|state| {
                    state.order_reducer().contains_order(order_id)
                        && !state.seen_fill_ids().contains(&fill.fill_id)
                });
            if should_apply {
                let output = self.coordinator.process_feed(FeedOutput::PrivateFill {
                    account_id: Some(account_id.to_string()),
                    fill: fill.clone(),
                })?;
                self.commit_output(output)?;
            }
        }
        for remote in remote_orders {
            let order_id = remote_order_id(remote);
            let known = self
                .coordinator
                .private_state(account_id)
                .is_some_and(|state| state.order_reducer().contains_order(&order_id));
            if known {
                let output = self.coordinator.process_feed(FeedOutput::PrivateOrder {
                    account_id: Some(account_id.to_string()),
                    update: private_update_from_remote(remote.clone()),
                })?;
                self.commit_output(output)?;
            }
        }
        Ok(())
    }

    fn commit_output(&mut self, output: CoordinatorOutput) -> Result<(), LiveRuntimeError> {
        for record in output.records {
            self.storage_sink.try_record(record)?;
        }
        for action in output.actions {
            self.dispatch_action(action)?;
        }
        Ok(())
    }

    fn dispatch_action(&mut self, action: LiveAction) -> Result<(), LiveRuntimeError> {
        match action {
            LiveAction::Submit(action) => {
                self.storage_sink
                    .try_record(StorageRecord::OrderRequest(OrderRequestRecord {
                        ts_ms: action.ts_ms,
                        account_id: action.account_id.clone(),
                        operation: OrderOperation::Submit,
                        idempotency_key: Some(action.idempotency_key.clone()),
                        client_order_id: Some(action.client_order_id.clone()),
                        exchange_order_id: None,
                        symbol: action.order.symbol.clone(),
                    }))?;
                self.order_sender(&action.account_id)?
                    .try_send(OrderTaskCommand::Submit(action))
                    .map_err(|_| {
                        LiveRuntimeError::OrderQueueUnavailable("submit account queue".to_string())
                    })?;
            }
            LiveAction::Cancel(action) => {
                let cancel_key = (action.account_id.clone(), action.client_order_id.clone());
                if !self.cancel_inflight.insert(cancel_key) {
                    return Ok(());
                }
                self.storage_sink
                    .try_record(StorageRecord::OrderRequest(OrderRequestRecord {
                        ts_ms: action.ts_ms,
                        account_id: action.account_id.clone(),
                        operation: OrderOperation::Cancel,
                        idempotency_key: None,
                        client_order_id: Some(action.client_order_id.clone()),
                        exchange_order_id: None,
                        symbol: action.symbol.clone(),
                    }))?;
                self.order_sender(&action.account_id)?
                    .try_send(OrderTaskCommand::Cancel(action))
                    .map_err(|_| {
                        LiveRuntimeError::OrderQueueUnavailable("cancel account queue".to_string())
                    })?;
            }
            LiveAction::RecoverBook(request) => {
                let routes = self.feeds[self.public_feed_index].request_recovery(&request);
                if routes == 0 {
                    return Err(LiveRuntimeError::MissingRecoveryRoute(
                        request.stream.symbol,
                    ));
                }
            }
            LiveAction::Reconcile(action) => self.dispatch_reconcile(action)?,
        }
        Ok(())
    }

    fn dispatch_reconcile(&mut self, action: ReconcileAction) -> Result<(), LiveRuntimeError> {
        if !self.reconcile_inflight.insert(action.account_id.clone()) {
            return Ok(());
        }
        self.last_reconcile_attempt
            .insert(action.account_id.clone(), Instant::now());
        let state = self
            .coordinator
            .private_state(&action.account_id)
            .ok_or_else(|| CoordinatorError::UnknownAccount(action.account_id.clone()))?;
        let orders = state
            .order_reducer()
            .orders()
            .filter(|(_, order)| {
                matches!(
                    order.status,
                    OrderStatus::PendingNew | OrderStatus::Live | OrderStatus::PartiallyFilled
                )
            })
            .map(|(order_id, order)| ReconcileOrderRef {
                order_id: order_id.to_string(),
                symbol: order.symbol.clone(),
                side: order.side,
                price: order.price,
                qty: order.qty,
                filled_qty: order.filled_qty,
                average_fill_price: order.avg_fill_price,
                last_update_ms: state.last_order_update_ms(order_id).unwrap_or(0),
            })
            .collect();
        self.order_sender(&action.account_id)?
            .try_send(OrderTaskCommand::Reconcile(orders))
            .map_err(|_| {
                self.reconcile_inflight.remove(&action.account_id);
                LiveRuntimeError::OrderQueueUnavailable(action.account_id)
            })
    }

    fn retry_reconciliation(&mut self, ts_ms: u64) -> Result<(), LiveRuntimeError> {
        let readiness = self.coordinator.readiness();
        let accounts = readiness
            .missing_reconciliation
            .into_iter()
            .filter(|account_id| !self.reconcile_inflight.contains(account_id))
            .filter(|account_id| {
                self.last_reconcile_attempt
                    .get(account_id)
                    .is_none_or(|last| last.elapsed() >= Duration::from_secs(2))
            })
            .collect::<Vec<_>>();
        for account_id in accounts {
            self.dispatch_reconcile(ReconcileAction {
                ts_ms,
                account_id,
                reason: "retry degraded reconciliation".to_string(),
            })?;
        }
        Ok(())
    }

    fn order_sender(
        &self,
        account_id: &str,
    ) -> Result<&mpsc::Sender<OrderTaskCommand>, LiveRuntimeError> {
        self.order_senders
            .get(account_id)
            .ok_or_else(|| LiveRuntimeError::OrderQueueUnavailable(account_id.to_string()))
    }

    async fn shutdown(&mut self) -> Result<(), LiveRuntimeError> {
        for sender in self.order_senders.values() {
            let _ = sender.try_send(OrderTaskCommand::Shutdown);
        }
        self.order_senders.clear();
        self.control_rx.close();
        self.feed_rx.close();
        for feed in self.feeds.drain(..) {
            feed.shutdown().await;
        }
        for task in self.feed_tasks.drain(..) {
            task.await?;
        }
        for task in self.order_tasks.drain(..) {
            task.await?;
        }
        if let Some(storage) = self.storage.take() {
            storage.shutdown().await?;
        }
        Ok(())
    }
}

enum RuntimeEvent {
    Raw {
        source_id: usize,
        envelope: reap_core::RawEnvelope,
    },
    Connection {
        source_id: usize,
        status: ConnectionStatus,
    },
    SubmitComplete {
        account_id: String,
        outcome: SubmitOutcome,
        ts_ms: u64,
    },
    SubmitFailed {
        account_id: String,
        client_order_id: String,
        ts_ms: u64,
        ambiguous: bool,
        reason: String,
    },
    CancelComplete {
        account_id: String,
        outcome: CancelOutcome,
        ts_ms: u64,
    },
    CancelFailed {
        account_id: String,
        client_order_id: String,
        ts_ms: u64,
        ambiguous: bool,
        reason: String,
    },
    RemoteState {
        account_id: String,
        remote_orders: Vec<RemoteOrder>,
        remote_fills: Vec<RemoteFill>,
        ts_ms: u64,
    },
    ReconcileFailed {
        account_id: String,
        ts_ms: u64,
        reason: String,
    },
    Fatal(String),
}

enum OrderTaskCommand {
    Submit(SubmitAction),
    Cancel(CancelAction),
    Reconcile(Vec<ReconcileOrderRef>),
    Shutdown,
}

#[derive(Debug, Clone)]
struct ReconcileOrderRef {
    order_id: String,
    symbol: String,
    side: reap_core::Side,
    price: f64,
    qty: f64,
    filled_qty: f64,
    average_fill_price: f64,
    last_update_ms: u64,
}

async fn run_order_task(
    account_id: String,
    mut gateway: LiveGateway,
    mut commands: mpsc::Receiver<OrderTaskCommand>,
    events: mpsc::Sender<RuntimeEvent>,
    ambiguous_submit_grace_ms: u64,
) {
    while let Some(command) = commands.recv().await {
        match command {
            OrderTaskCommand::Submit(action) => {
                let client_order_id = action.client_order_id;
                let preparation = match gateway.prepare_registered_submit(
                    action.idempotency_key,
                    action.order,
                    client_order_id.clone(),
                ) {
                    Ok(preparation) => preparation,
                    Err(error) => {
                        if events
                            .send(RuntimeEvent::Fatal(format!(
                                "account {account_id} submit preparation failed: {error}"
                            )))
                            .await
                            .is_err()
                        {
                            return;
                        }
                        continue;
                    }
                };
                let SubmitPreparation::Ready(prepared) = preparation else {
                    let SubmitPreparation::Complete(outcome) = preparation else {
                        unreachable!()
                    };
                    if events
                        .send(RuntimeEvent::SubmitComplete {
                            account_id: account_id.clone(),
                            outcome,
                            ts_ms: unix_time_ms(),
                        })
                        .await
                        .is_err()
                    {
                        return;
                    }
                    continue;
                };
                match gateway.execute_submit(prepared).await {
                    Ok(outcome) => {
                        if events
                            .send(RuntimeEvent::SubmitComplete {
                                account_id: account_id.clone(),
                                outcome,
                                ts_ms: unix_time_ms(),
                            })
                            .await
                            .is_err()
                        {
                            return;
                        }
                    }
                    Err(error) => {
                        if events
                            .send(RuntimeEvent::SubmitFailed {
                                account_id: account_id.clone(),
                                client_order_id,
                                ts_ms: unix_time_ms(),
                                ambiguous: error.is_ambiguous(),
                                reason: error.to_string(),
                            })
                            .await
                            .is_err()
                        {
                            return;
                        }
                    }
                }
            }
            OrderTaskCommand::Cancel(action) => {
                match gateway
                    .cancel(&action.symbol, None, Some(action.client_order_id.clone()))
                    .await
                {
                    Ok(outcome) => {
                        if events
                            .send(RuntimeEvent::CancelComplete {
                                account_id: account_id.clone(),
                                outcome,
                                ts_ms: unix_time_ms(),
                            })
                            .await
                            .is_err()
                        {
                            return;
                        }
                    }
                    Err(error) => {
                        if events
                            .send(RuntimeEvent::CancelFailed {
                                account_id: account_id.clone(),
                                client_order_id: action.client_order_id,
                                ts_ms: unix_time_ms(),
                                ambiguous: error.is_ambiguous(),
                                reason: error.to_string(),
                            })
                            .await
                            .is_err()
                        {
                            return;
                        }
                    }
                }
            }
            OrderTaskCommand::Reconcile(restored_orders) => {
                match gateway.fetch_remote_state(None, None).await {
                    Ok((mut remote_orders, remote_fills)) => {
                        let mut remote_ids = remote_orders
                            .iter()
                            .map(remote_order_id)
                            .collect::<HashSet<_>>();
                        let mut failed = None;
                        for restored in restored_orders {
                            if remote_ids.contains(&restored.order_id) {
                                continue;
                            }
                            let details = match gateway
                                .fetch_order_details(&restored.symbol, &restored.order_id)
                                .await
                            {
                                Ok(details) => details,
                                Err(error)
                                    if error.is_order_not_found()
                                        && unix_time_ms()
                                            .saturating_sub(restored.last_update_ms)
                                            < ambiguous_submit_grace_ms =>
                                {
                                    failed = Some(format!(
                                        "order {} is not visible within the ambiguous-submit grace period",
                                        restored.order_id
                                    ));
                                    break;
                                }
                                Err(error) if error.is_order_not_found() => RemoteOrder {
                                    exchange_order_id: String::new(),
                                    client_order_id: restored.order_id.clone(),
                                    symbol: restored.symbol,
                                    side: restored.side,
                                    state: PrivateOrderState::Rejected,
                                    price: restored.price,
                                    qty: restored.qty,
                                    cumulative_filled_qty: restored.filled_qty,
                                    average_fill_price: restored.average_fill_price,
                                    update_time_ms: unix_time_ms(),
                                },
                                Err(error) => {
                                    failed = Some(error.to_string());
                                    break;
                                }
                            };
                            remote_ids.insert(remote_order_id(&details));
                            remote_orders.push(details);
                        }
                        if let Some(reason) = failed {
                            if events
                                .send(RuntimeEvent::ReconcileFailed {
                                    account_id: account_id.clone(),
                                    ts_ms: unix_time_ms(),
                                    reason,
                                })
                                .await
                                .is_err()
                            {
                                return;
                            }
                            continue;
                        }
                        if events
                            .send(RuntimeEvent::RemoteState {
                                account_id: account_id.clone(),
                                remote_orders,
                                remote_fills,
                                ts_ms: unix_time_ms(),
                            })
                            .await
                            .is_err()
                        {
                            return;
                        }
                    }
                    Err(error) => {
                        if events
                            .send(RuntimeEvent::ReconcileFailed {
                                account_id: account_id.clone(),
                                ts_ms: unix_time_ms(),
                                reason: error.to_string(),
                            })
                            .await
                            .is_err()
                        {
                            return;
                        }
                    }
                }
            }
            OrderTaskCommand::Shutdown => return,
        }
    }
}

struct FeedSourceState {
    adapter: Arc<dyn VenueAdapter>,
    account_id: Option<String>,
    expected_connections: HashSet<ConnId>,
    ready_connections: HashSet<ConnId>,
    public_subscriptions: Vec<PublicSubscriptionRoute>,
    private_ready: bool,
}

struct PublicSubscriptionRoute {
    channel: Channel,
    symbol: Option<String>,
    connections: HashSet<ConnId>,
}

impl FeedSourceState {
    fn public(adapter: Arc<dyn VenueAdapter>, plans: &[SocketPlan]) -> Self {
        let mut public_subscriptions: HashMap<(Channel, Option<String>), HashSet<ConnId>> =
            HashMap::new();
        for plan in plans {
            for subscription in &plan.subscriptions {
                public_subscriptions
                    .entry((subscription.channel.clone(), subscription.symbol.clone()))
                    .or_default()
                    .insert(plan.conn_id.clone());
            }
        }
        Self {
            adapter,
            account_id: None,
            expected_connections: plans.iter().map(|plan| plan.conn_id.clone()).collect(),
            ready_connections: HashSet::new(),
            public_subscriptions: public_subscriptions
                .into_iter()
                .map(|((channel, symbol), connections)| PublicSubscriptionRoute {
                    channel,
                    symbol,
                    connections,
                })
                .collect(),
            private_ready: false,
        }
    }

    fn private(adapter: Arc<dyn VenueAdapter>, account_id: String, plans: &[SocketPlan]) -> Self {
        Self {
            adapter,
            account_id: Some(account_id),
            expected_connections: plans.iter().map(|plan| plan.conn_id.clone()).collect(),
            ready_connections: HashSet::new(),
            public_subscriptions: Vec::new(),
            private_ready: false,
        }
    }

    fn public_connectivity_ready(&self) -> Option<bool> {
        self.account_id.is_none().then(|| {
            !self.public_subscriptions.is_empty()
                && self
                    .public_subscriptions
                    .iter()
                    .all(|route| !route.connections.is_disjoint(&self.ready_connections))
        })
    }

    fn on_status(&mut self, status: ConnectionStatus) -> Vec<SystemEvent> {
        match status.kind {
            ConnectionStatusKind::Ready | ConnectionStatusKind::Heartbeat => {
                self.ready_connections.insert(status.conn_id.clone());
            }
            ConnectionStatusKind::Disconnected => {
                self.ready_connections.remove(&status.conn_id);
            }
        }
        if let Some(account_id) = &self.account_id {
            let ready = self.expected_connections.is_subset(&self.ready_connections);
            let transition = ready != self.private_ready;
            self.private_ready = ready;
            if ready {
                return vec![SystemEvent {
                    ts_ms: status.ts_ms,
                    kind: if transition {
                        SystemEventKind::PrivateStreamRecovered
                    } else {
                        SystemEventKind::PrivateStreamHeartbeat
                    },
                    venue: Some(status.venue),
                    account_id: Some(account_id.clone()),
                    symbol: None,
                    reason: if transition {
                        "all required private websocket channels are connected".to_string()
                    } else {
                        status.reason
                    },
                }];
            }
            if transition || status.kind == ConnectionStatusKind::Disconnected {
                return vec![SystemEvent {
                    ts_ms: status.ts_ms,
                    kind: SystemEventKind::PrivateStreamStale,
                    venue: Some(status.venue),
                    account_id: Some(account_id.clone()),
                    symbol: None,
                    reason: format!(
                        "private websocket set is incomplete ({}/{})",
                        self.ready_connections.len(),
                        self.expected_connections.len()
                    ),
                }];
            }
            return Vec::new();
        }

        if status.kind != ConnectionStatusKind::Disconnected {
            return Vec::new();
        }
        self.public_subscriptions
            .iter()
            .filter(|route| route.connections.is_disjoint(&self.ready_connections))
            .map(|route| SystemEvent {
                ts_ms: status.ts_ms,
                kind: SystemEventKind::FeedStale,
                venue: Some(status.venue),
                account_id: None,
                symbol: route.symbol.clone(),
                reason: format!(
                    "all redundant {:?} websocket connections are down",
                    route.channel
                ),
            })
            .collect()
    }
}

fn spawn_feed_forwarders(
    source_id: usize,
    feed: &mut SupervisedFeed,
    events: &mpsc::Sender<RuntimeEvent>,
    tasks: &mut Vec<JoinHandle<()>>,
) {
    let mut raw = feed.take_raw();
    let raw_events = events.clone();
    tasks.push(tokio::spawn(async move {
        while let Some(envelope) = raw.recv().await {
            if raw_events
                .send(RuntimeEvent::Raw {
                    source_id,
                    envelope,
                })
                .await
                .is_err()
            {
                return;
            }
        }
    }));
    let mut status = feed.take_status();
    let status_events = events.clone();
    tasks.push(tokio::spawn(async move {
        while let Some(status) = status.recv().await {
            if status_events
                .send(RuntimeEvent::Connection { source_id, status })
                .await
                .is_err()
            {
                return;
            }
        }
    }));
}

fn public_subscriptions(config: &LiveConfig) -> Vec<Subscription> {
    let mut subscriptions = Vec::new();
    let mut seen = HashSet::new();
    for instrument in &config.strategy.instruments {
        push_public_subscription(
            &mut subscriptions,
            &mut seen,
            Channel::Books,
            &instrument.symbol,
            FeedPriority::Critical,
            config.runtime.public_connections_per_subscription,
        );
        push_public_subscription(
            &mut subscriptions,
            &mut seen,
            Channel::Trades,
            &instrument.symbol,
            FeedPriority::High,
            config.runtime.public_connections_per_subscription,
        );
        if instrument.kind.is_derivative() {
            for channel in ["funding-rate", "mark-price", "price-limit"] {
                push_public_subscription(
                    &mut subscriptions,
                    &mut seen,
                    Channel::Custom(channel.to_string()),
                    &instrument.symbol,
                    FeedPriority::High,
                    config.runtime.public_connections_per_subscription,
                );
            }
        }
        if let Some(index_symbol) = &instrument.index_symbol {
            push_public_subscription(
                &mut subscriptions,
                &mut seen,
                Channel::Custom("index-tickers".to_string()),
                index_symbol,
                FeedPriority::High,
                config.runtime.public_connections_per_subscription,
            );
        }
    }
    subscriptions
}

fn push_public_subscription(
    subscriptions: &mut Vec<Subscription>,
    seen: &mut HashSet<(Channel, String)>,
    channel: Channel,
    symbol: &str,
    priority: FeedPriority,
    connections: usize,
) {
    if !seen.insert((channel.clone(), symbol.to_string())) {
        return;
    }
    let mut subscription = Subscription::public(Venue::Okx, channel, symbol, priority);
    subscription.connections = connections;
    subscriptions.push(subscription);
}

fn private_subscriptions(enable_vip_fills_channel: bool) -> Vec<Subscription> {
    let mut channels = vec![Channel::Orders, Channel::Account, Channel::Positions];
    if enable_vip_fills_channel {
        channels.push(Channel::Fills);
    }
    channels
        .into_iter()
        .map(|channel| Subscription::private(Venue::Okx, channel, FeedPriority::Critical))
        .collect()
}

fn unix_time_ns() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
        .min(u64::MAX as u128) as u64
}

fn unix_time_ms() -> u64 {
    unix_time_ns() / 1_000_000
}

#[cfg(unix)]
async fn shutdown_signal() -> Result<(), std::io::Error> {
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    tokio::select! {
        result = tokio::signal::ctrl_c() => result,
        _ = terminate.recv() => Ok(()),
    }
}

#[cfg(not(unix))]
async fn shutdown_signal() -> Result<(), std::io::Error> {
    tokio::signal::ctrl_c().await
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use reap_risk::RiskLimits;
    use reap_strategy::ChaosConfig;
    use reap_venue::okx::{OkxAccountLevel, OkxPositionMode};

    use crate::{
        LiveAccountConfig, LiveStorageConfig, OkxTradeModeConfig, OkxVenueConfig, RuntimeConfig,
    };

    use super::*;

    fn status(conn_id: &str, kind: ConnectionStatusKind) -> ConnectionStatus {
        ConnectionStatus {
            conn_id: ConnId::new(conn_id),
            venue: Venue::Okx,
            private: true,
            ts_ms: 1,
            kind,
            reason: "test".to_string(),
        }
    }

    fn plan(conn_id: &str, private: bool, channel: Channel, symbol: Option<&str>) -> SocketPlan {
        SocketPlan {
            conn_id: ConnId::new(conn_id),
            venue: Venue::Okx,
            private,
            subscriptions: vec![Subscription {
                venue: Venue::Okx,
                channel,
                symbol: symbol.map(str::to_string),
                priority: FeedPriority::Critical,
                connections: 1,
            }],
        }
    }

    fn config() -> LiveConfig {
        let mut strategy: ChaosConfig =
            toml::from_str(include_str!("../../../examples/iarb2-basic.toml")).unwrap();
        strategy.risk_groups[0].account_id = Some("main".to_string());
        LiveConfig {
            strategy,
            risk: RiskLimits::default(),
            venue: OkxVenueConfig::default(),
            runtime: RuntimeConfig::default(),
            storage: LiveStorageConfig::default(),
            accounts: vec![LiveAccountConfig {
                id: "main".to_string(),
                api_key_env: "KEY".to_string(),
                secret_key_env: "SECRET".to_string(),
                passphrase_env: "PASS".to_string(),
                expected_account_level: OkxAccountLevel::SingleCurrencyMargin,
                expected_position_mode: OkxPositionMode::NetMode,
                id_prefix: "reap".to_string(),
                node_id: 1,
                trade_modes: HashMap::from([
                    ("BTC-USDT".to_string(), OkxTradeModeConfig::Cash),
                    ("BTC-PERP".to_string(), OkxTradeModeConfig::Cross),
                ]),
            }],
        }
    }

    #[test]
    fn private_account_is_ready_only_when_every_channel_is_connected() {
        let plans = vec![
            plan("orders", true, Channel::Orders, None),
            plan("fills", true, Channel::Fills, None),
            plan("account", true, Channel::Account, None),
            plan("positions", true, Channel::Positions, None),
        ];
        let adapter: Arc<dyn VenueAdapter> = Arc::new(OkxAdapter::default());
        let mut source = FeedSourceState::private(adapter, "main".to_string(), &plans);

        assert!(
            source
                .on_status(status("orders", ConnectionStatusKind::Ready))
                .is_empty()
        );
        assert!(
            source
                .on_status(status("fills", ConnectionStatusKind::Ready))
                .is_empty()
        );
        assert!(
            source
                .on_status(status("account", ConnectionStatusKind::Ready))
                .is_empty()
        );
        let ready = source.on_status(status("positions", ConnectionStatusKind::Ready));
        assert_eq!(ready[0].kind, SystemEventKind::PrivateStreamRecovered);

        let stale = source.on_status(status("fills", ConnectionStatusKind::Disconnected));
        assert_eq!(stale[0].kind, SystemEventKind::PrivateStreamStale);
    }

    #[test]
    fn one_redundant_book_disconnect_does_not_mark_feed_stale() {
        let plans = vec![
            plan("book-1", false, Channel::Books, Some("BTC-USDT")),
            plan("book-2", false, Channel::Books, Some("BTC-USDT")),
        ];
        let adapter: Arc<dyn VenueAdapter> = Arc::new(OkxAdapter::default());
        let mut source = FeedSourceState::public(adapter, &plans);
        source.on_status(status("book-1", ConnectionStatusKind::Ready));
        source.on_status(status("book-2", ConnectionStatusKind::Ready));
        assert_eq!(source.public_connectivity_ready(), Some(true));

        assert!(
            source
                .on_status(status("book-1", ConnectionStatusKind::Disconnected))
                .is_empty()
        );
        assert_eq!(source.public_connectivity_ready(), Some(true));
        let stale = source.on_status(status("book-2", ConnectionStatusKind::Disconnected));
        assert_eq!(source.public_connectivity_ready(), Some(false));
        assert_eq!(stale[0].kind, SystemEventKind::FeedStale);
        assert_eq!(stale[0].symbol.as_deref(), Some("BTC-USDT"));
    }

    #[test]
    fn public_plan_replicates_every_required_subscription() {
        let config = config();
        let subscriptions = public_subscriptions(&config);

        assert!(subscriptions.iter().all(|subscription| {
            subscription.connections == config.runtime.public_connections_per_subscription
        }));
        assert_eq!(private_subscriptions(false).len(), 3);
        assert_eq!(private_subscriptions(true).len(), 4);
    }

    #[tokio::test]
    async fn demo_mode_requires_confirmation_and_simulated_environment() {
        let error = run_live(
            config(),
            LiveRunOptions {
                mode: LiveMode::Demo,
                demo_confirmed: false,
            },
        )
        .await
        .unwrap_err();
        assert!(matches!(error, LiveRuntimeError::DemoConfirmationRequired));

        let mut production = config();
        production.venue.environment = TradingEnvironment::Production;
        let error = run_live(
            production,
            LiveRunOptions {
                mode: LiveMode::Demo,
                demo_confirmed: true,
            },
        )
        .await
        .unwrap_err();
        assert!(matches!(
            error,
            LiveRuntimeError::DemoRequiresSimulatedTrading
        ));
    }
}
