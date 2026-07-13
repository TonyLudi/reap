use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use reap_core::{RawEnvelope, Symbol, Venue};
use reap_venue::{VenueAdapter, okx::OkxSigner};
use tokio::sync::{mpsc, watch};
use tokio::task::JoinHandle;

use crate::{
    ConnectionError, ConnectionStatus, ConnectionStatusKind, RecoveryRequest, SocketPlan,
    run_connection_once,
};

#[derive(Debug, Clone)]
pub struct ReconnectPolicy {
    pub initial_delay: Duration,
    pub max_delay: Duration,
    pub multiplier: u32,
}

pub type BootstrapFactory =
    Arc<dyn Fn(&SocketPlan) -> Result<Vec<String>, ConnectionError> + Send + Sync>;

pub fn no_bootstrap() -> BootstrapFactory {
    Arc::new(|_| Ok(Vec::new()))
}

pub fn okx_login_bootstrap(signer: OkxSigner) -> BootstrapFactory {
    Arc::new(move |plan| {
        if !plan.private {
            return Ok(Vec::new());
        }
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
            .to_string();
        signer
            .websocket_login(&timestamp)
            .map(|message| vec![message])
            .map_err(|error| ConnectionError::LoginFailed(error.to_string()))
    })
}

impl Default for ReconnectPolicy {
    fn default() -> Self {
        Self {
            initial_delay: Duration::from_millis(250),
            max_delay: Duration::from_secs(30),
            multiplier: 2,
        }
    }
}

impl ReconnectPolicy {
    pub fn next_delay(&self, current: Duration) -> Duration {
        current
            .saturating_mul(self.multiplier.max(1))
            .min(self.max_delay)
    }
}

pub struct SupervisedFeed {
    pub raw: mpsc::Receiver<RawEnvelope>,
    pub status: mpsc::Receiver<ConnectionStatus>,
    shutdown: watch::Sender<bool>,
    recovery_routes: HashMap<(Venue, Symbol), Vec<watch::Sender<u64>>>,
    tasks: Vec<JoinHandle<()>>,
}

impl SupervisedFeed {
    pub fn take_raw(&mut self) -> mpsc::Receiver<RawEnvelope> {
        let (_sender, replacement) = mpsc::channel(1);
        std::mem::replace(&mut self.raw, replacement)
    }

    pub fn take_status(&mut self) -> mpsc::Receiver<ConnectionStatus> {
        let (_sender, replacement) = mpsc::channel(1);
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
            .filter(|route| {
                let next = route.borrow().wrapping_add(1);
                route.send(next).is_ok()
            })
            .count()
    }

    pub async fn shutdown(self) {
        let _ = self.shutdown.send(true);
        for task in self.tasks {
            let _ = task.await;
        }
    }
}

pub fn spawn_supervised_feed(
    adapter: Arc<dyn VenueAdapter>,
    plans: Vec<SocketPlan>,
    bootstrap: BootstrapFactory,
    channel_capacity: usize,
    reconnect: ReconnectPolicy,
) -> SupervisedFeed {
    let (raw_tx, raw_rx) = mpsc::channel(channel_capacity.max(1));
    let (status_tx, status_rx) = mpsc::channel(channel_capacity.max(1));
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let mut tasks = Vec::new();
    let mut recovery_routes: HashMap<(Venue, Symbol), Vec<watch::Sender<u64>>> = HashMap::new();
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
                    .push(recovery_tx.clone());
            }
        }
        let adapter = Arc::clone(&adapter);
        let output = raw_tx.clone();
        let status = status_tx.clone();
        let bootstrap = Arc::clone(&bootstrap);
        let shutdown = shutdown_rx.clone();
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
                reconnect,
            )
            .await;
        }));
    }
    drop(raw_tx);
    drop(status_tx);
    SupervisedFeed {
        raw: raw_rx,
        status: status_rx,
        shutdown: shutdown_tx,
        recovery_routes,
        tasks,
    }
}

struct ConnectionChannels {
    output: mpsc::Sender<RawEnvelope>,
    status: mpsc::Sender<ConnectionStatus>,
    shutdown: watch::Receiver<bool>,
    recovery: watch::Receiver<u64>,
}

async fn supervise_connection(
    adapter: Arc<dyn VenueAdapter>,
    plan: SocketPlan,
    bootstrap: BootstrapFactory,
    channels: ConnectionChannels,
    reconnect: ReconnectPolicy,
) {
    let ConnectionChannels {
        output,
        status,
        mut shutdown,
        mut recovery,
    } = channels;
    let mut delay = reconnect.initial_delay;
    loop {
        if *shutdown.borrow() {
            return;
        }
        let bootstrap_messages = match bootstrap(&plan) {
            Ok(messages) => messages,
            Err(error) => {
                tracing::warn!(conn_id = %plan.conn_id, ?error, ?delay, "feed bootstrap generation failed");
                tokio::select! {
                    _ = tokio::time::sleep(delay) => {}
                    changed = shutdown.changed() => {
                        if changed.is_err() || *shutdown.borrow() {
                            return;
                        }
                    }
                }
                delay = reconnect.next_delay(delay);
                continue;
            }
        };
        let result = run_connection_once(
            adapter.as_ref(),
            &plan,
            &bootstrap_messages,
            &output,
            &status,
            &mut shutdown,
            &mut recovery,
        )
        .await;
        if *shutdown.borrow() || matches!(result, Ok(())) {
            return;
        }
        let error = result.expect_err("non-success result must contain an error");
        let _ = status.try_send(ConnectionStatus {
            conn_id: plan.conn_id.clone(),
            venue: plan.venue,
            private: plan.private,
            ts_ms: crate::unix_time_ns() / 1_000_000,
            kind: ConnectionStatusKind::Disconnected,
            reason: error.to_string(),
        });
        if matches!(error, ConnectionError::RecoveryRequested) {
            delay = reconnect.initial_delay;
            tracing::info!(conn_id = %plan.conn_id, "feed connection restarting for snapshot recovery");
            continue;
        }
        tracing::warn!(conn_id = %plan.conn_id, ?error, ?delay, "feed connection restarting");
        if matches!(error, ConnectionError::OutputClosed) {
            return;
        }
        tokio::select! {
            _ = tokio::time::sleep(delay) => {}
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() {
                    return;
                }
            }
        }
        delay = reconnect.next_delay(delay);
    }
}

#[cfg(test)]
mod tests {
    use reap_core::{Channel, FeedPriority, Subscription};
    use reap_venue::okx::OkxCredentials;

    use super::*;

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
    fn recovery_request_notifies_registered_socket() {
        let (route, mut route_rx) = watch::channel(0_u64);
        let (_raw_tx, raw_rx) = mpsc::channel(1);
        let (_status_tx, status_rx) = mpsc::channel(1);
        let (shutdown, _shutdown_rx) = watch::channel(false);
        let feed = SupervisedFeed {
            raw: raw_rx,
            status: status_rx,
            shutdown,
            recovery_routes: HashMap::from([((Venue::Okx, "BTC-USDT".to_string()), vec![route])]),
            tasks: Vec::new(),
        };
        let request = RecoveryRequest {
            stream: crate::FeedStreamId {
                venue: Venue::Okx,
                channel: Channel::Books,
                symbol: "BTC-USDT".to_string(),
            },
            expected_prev: Some(10),
            received_prev: 11,
            received_seq: 12,
        };

        assert_eq!(feed.request_recovery(&request), 1);
        assert!(route_rx.has_changed().unwrap());
        assert_eq!(*route_rx.borrow_and_update(), 1);
    }

    #[test]
    fn okx_private_bootstrap_builds_login_per_attempt() {
        let factory = okx_login_bootstrap(OkxSigner::new(
            OkxCredentials::new("key", "secret", "pass"),
            true,
        ));
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

        let login: serde_json::Value =
            serde_json::from_str(&factory(&private).unwrap()[0]).unwrap();
        assert_eq!(login["op"], "login");
        assert!(login["args"][0]["timestamp"].as_str().is_some());
        assert!(factory(&public).unwrap().is_empty());
    }
}
