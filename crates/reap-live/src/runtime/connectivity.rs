use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use reap_core::{
    BacktestLatencyClass, Channel, ConnId, MarketEvent, NormalizedEvent, SystemEvent,
    SystemEventKind, TimeMs, Venue,
};
use reap_feed::{
    ConnectionStatus, ConnectionStatusKind, FeedOutput, FeedProcessor, SocketPlan, SupervisedFeed,
};
use reap_okx_live_adapter::OrderCommandWebsocketLifecycle;
use reap_venue::VenueAdapter;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::LiveLatencySemantics;

use super::dispatch::RuntimeEvent;
use super::{LiveRuntime, LiveRuntimeError};

pub(super) struct ConnectivityState {
    pub(super) processor: FeedProcessor,
    pub(super) feed_rx: mpsc::Receiver<RuntimeEvent>,
    pub(super) order_ws_runtimes: Vec<OrderCommandWebsocketLifecycle>,
    pub(super) order_ws_status_tasks: Vec<JoinHandle<()>>,
    pub(super) feeds: Vec<SupervisedFeed>,
    pub(super) feed_tasks: Vec<JoinHandle<()>>,
    pub(super) sources: Vec<FeedSourceState>,
    pub(super) public_feed_index: usize,
    pub(super) max_feed_age_ms: u64,
}

pub(super) struct FeedSourceState {
    pub(super) adapter: Arc<dyn VenueAdapter>,
    pub(super) account_id: Option<String>,
    expected_connections: HashSet<ConnId>,
    ready_connections: HashSet<ConnId>,
    public_subscriptions: Vec<PublicSubscriptionRoute>,
    required_private_data_channels: HashSet<Channel>,
    private_data_round: HashSet<Channel>,
    private_ready: bool,
}

struct PublicSubscriptionRoute {
    channel: Channel,
    symbol: Option<String>,
    connections: HashSet<ConnId>,
}

impl FeedSourceState {
    pub(super) fn public(adapter: Arc<dyn VenueAdapter>, plans: &[SocketPlan]) -> Self {
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
            required_private_data_channels: HashSet::new(),
            private_data_round: HashSet::new(),
            private_ready: false,
        }
    }

    pub(super) fn private(
        adapter: Arc<dyn VenueAdapter>,
        account_id: String,
        plans: &[SocketPlan],
    ) -> Self {
        let required_private_data_channels = plans
            .iter()
            .flat_map(|plan| &plan.subscriptions)
            .filter(|subscription| {
                matches!(subscription.channel, Channel::Account | Channel::Positions)
            })
            .map(|subscription| subscription.channel.clone())
            .collect();
        Self {
            adapter,
            account_id: Some(account_id),
            expected_connections: plans.iter().map(|plan| plan.conn_id.clone()).collect(),
            ready_connections: HashSet::new(),
            public_subscriptions: Vec::new(),
            required_private_data_channels,
            private_data_round: HashSet::new(),
            private_ready: false,
        }
    }

    pub(super) fn public_connectivity_ready(&self) -> Option<bool> {
        self.account_id.is_none().then(|| {
            !self.public_subscriptions.is_empty()
                && self
                    .public_subscriptions
                    .iter()
                    .all(|route| !route.connections.is_disjoint(&self.ready_connections))
        })
    }

    pub(super) fn on_status(&mut self, status: ConnectionStatus) -> Vec<SystemEvent> {
        let disconnected = matches!(
            status.kind,
            ConnectionStatusKind::Disconnected | ConnectionStatusKind::Fatal
        );
        let status_reason = status.reason.clone();
        match status.kind {
            ConnectionStatusKind::Ready | ConnectionStatusKind::Heartbeat => {
                self.ready_connections.insert(status.conn_id.clone());
            }
            ConnectionStatusKind::Disconnected | ConnectionStatusKind::Fatal => {
                self.ready_connections.remove(&status.conn_id);
                self.private_ready = false;
                self.private_data_round.clear();
            }
        }
        if let Some(account_id) = &self.account_id {
            if disconnected {
                return vec![SystemEvent {
                    ts_ms: status.ts_ms,
                    kind: SystemEventKind::PrivateStreamStale,
                    venue: Some(status.venue),
                    account_id: Some(account_id.clone()),
                    symbol: None,
                    reason: format!(
                        "private websocket set is incomplete ({}/{}): {status_reason}",
                        self.ready_connections.len(),
                        self.expected_connections.len()
                    ),
                }];
            }
            if let Some(event) = self.private_health_event(
                status.ts_ms,
                status.venue,
                "all private transports and state-data channels are healthy",
            ) {
                return vec![event];
            }
            return Vec::new();
        }

        if !disconnected {
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
                    "all redundant {:?} websocket connections are down: {status_reason}",
                    route.channel,
                ),
            })
            .collect()
    }

    pub(super) fn on_private_data(&mut self, channel: Channel, ts_ms: TimeMs) -> Vec<SystemEvent> {
        if self.account_id.is_none() || !self.required_private_data_channels.contains(&channel) {
            return Vec::new();
        }
        self.private_data_round.insert(channel);
        self.private_health_event(
            ts_ms,
            self.adapter.venue(),
            "fresh account and positions websocket payloads received",
        )
        .into_iter()
        .collect()
    }

    fn private_health_event(
        &mut self,
        ts_ms: TimeMs,
        venue: Venue,
        reason: &str,
    ) -> Option<SystemEvent> {
        if !self.expected_connections.is_subset(&self.ready_connections)
            || !self
                .required_private_data_channels
                .is_subset(&self.private_data_round)
        {
            return None;
        }
        let kind = if self.private_ready {
            SystemEventKind::PrivateStreamHeartbeat
        } else {
            SystemEventKind::PrivateStreamRecovered
        };
        self.private_ready = true;
        self.private_data_round.clear();
        Some(SystemEvent {
            ts_ms,
            kind,
            venue: Some(venue),
            account_id: self.account_id.clone(),
            symbol: None,
            reason: reason.to_string(),
        })
    }
}

pub(super) async fn handle_runtime_event(
    runtime: &mut LiveRuntime,
    event: RuntimeEvent,
) -> Result<(), LiveRuntimeError> {
    match event {
        RuntimeEvent::Connection { source_id, status } => {
            if status.kind == ConnectionStatusKind::Fatal {
                return Err(LiveRuntimeError::ConnectionPacerRuntime(format!(
                    "{}: {}",
                    status.conn_id, status.reason
                )));
            }
            if status.kind == ConnectionStatusKind::Disconnected {
                runtime
                    .composition
                    .evidence
                    .observe_disconnect(status.private);
            }
            let (events, public_connectivity) = {
                let source = runtime
                    .connectivity
                    .sources
                    .get_mut(source_id)
                    .ok_or_else(|| {
                        LiveRuntimeError::FeedAdapter("unknown feed source".to_string())
                    })?;
                let events = source.on_status(status);
                (events, source.public_connectivity_ready())
            };
            if let Some(ready) = public_connectivity {
                runtime.coordinator.mark_public_connectivity(
                    ready,
                    if ready {
                        "every public subscription has an acknowledged connection"
                    } else {
                        "one or more public subscriptions has no acknowledged connection"
                    },
                );
            }
            runtime.handle_feed_source_events(events).await?;
        }
        _ => unreachable!("non-connectivity event sent to connectivity handler"),
    }
    Ok(())
}

impl LiveRuntime {
    pub(super) async fn handle_feed_source_events(
        &mut self,
        events: Vec<SystemEvent>,
    ) -> Result<(), LiveRuntimeError> {
        for event in events {
            if event.kind == SystemEventKind::PrivateStreamRecovered {
                let account_id = event.account_id.as_deref().ok_or_else(|| {
                    LiveRuntimeError::FeedAdapter(
                        "private recovery event has no account identity".to_string(),
                    )
                })?;
                let output = self.coordinator.require_reconciliation(
                    account_id,
                    event.ts_ms,
                    "verify REST state after private websocket state recovery",
                )?;
                self.commit_output(output).await?;
            }
            let output = self
                .coordinator
                .process_event(NormalizedEvent::System(event));
            self.commit_output(output).await?;
        }
        Ok(())
    }

    pub(super) fn observe_feed_latency(
        &mut self,
        output: &FeedOutput,
        received_ns: u64,
        strategy_visible_ns: u64,
    ) {
        match output {
            FeedOutput::Event(NormalizedEvent::Market(event)) => {
                let class = match event {
                    MarketEvent::Depth(_) => BacktestLatencyClass::MarketDepth,
                    MarketEvent::Trade { .. } => BacktestLatencyClass::HistoricalTrade,
                    MarketEvent::IndexPrice { .. }
                    | MarketEvent::FundingRate { .. }
                    | MarketEvent::BurstSignal { .. }
                    | MarketEvent::PriceLimits { .. } => BacktestLatencyClass::ReferenceData,
                };
                self.composition.latency.observe_ns(
                    class,
                    event.symbol(),
                    LiveLatencySemantics::HostReceiveToStrategyVisibility,
                    received_ns,
                    strategy_visible_ns,
                );
            }
            FeedOutput::PrivateOrder { update, .. } => {
                self.composition.latency.observe_exchange_ms(
                    BacktestLatencyClass::OrderUpdate,
                    &update.symbol,
                    update.ts_ms,
                    strategy_visible_ns,
                );
            }
            FeedOutput::PrivateFill { fill, .. } => {
                self.composition.latency.observe_exchange_ms(
                    BacktestLatencyClass::OrderUpdate,
                    &fill.symbol,
                    fill.ts_ms,
                    strategy_visible_ns,
                );
            }
            FeedOutput::Event(_)
            | FeedOutput::PrivateAccount { .. }
            | FeedOutput::Duplicate(_)
            | FeedOutput::RecoveryRequired(_)
            | FeedOutput::System(_) => {}
        }
    }
}

pub(super) fn spawn_feed_forwarders(
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
