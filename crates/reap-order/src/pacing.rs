use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub use reap_core::PacingPolicy;

type ReservationQueues = HashMap<(RequestKind, String), VecDeque<Instant>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RequestKind {
    Submit,
    Cancel,
    Reconcile,
}

#[derive(Debug, Clone)]
pub struct RequestPacer {
    policy: PacingPolicy,
    reservations: Arc<Mutex<ReservationQueues>>,
}

impl RequestPacer {
    pub fn new(policy: PacingPolicy) -> Self {
        Self {
            policy,
            reservations: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn reserve_at(&self, kind: RequestKind, scope: &str, now: Instant) -> Duration {
        let limit = self.limit(kind).max(1);
        let mut reservations = self
            .reservations
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let queue = reservations.entry((kind, scope.to_string())).or_default();
        while queue
            .front()
            .is_some_and(|reserved| *reserved + self.policy.window <= now)
        {
            queue.pop_front();
        }
        let reserved = if queue.len() < limit {
            now
        } else {
            let earliest = queue.pop_front().expect("full pacing queue has a head");
            (earliest + self.policy.window).max(now)
        };
        queue.push_back(reserved);
        reserved.saturating_duration_since(now)
    }

    pub async fn pace(&self, kind: RequestKind, scope: &str) {
        let delay = self.reserve_at(kind, scope, Instant::now());
        if !delay.is_zero() {
            tokio::time::sleep(delay).await;
        }
    }

    fn limit(&self, kind: RequestKind) -> usize {
        match kind {
            RequestKind::Submit => self.policy.submit_requests,
            RequestKind::Cancel => self.policy.cancel_requests,
            RequestKind::Reconcile => self.policy.reconcile_requests,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn per_scope_requests_are_scheduled_inside_policy() {
        let pacer = RequestPacer::new(PacingPolicy {
            submit_requests: 2,
            cancel_requests: 1,
            reconcile_requests: 1,
            window: Duration::from_secs(1),
        });
        let now = Instant::now();

        assert_eq!(
            pacer.reserve_at(RequestKind::Submit, "BTC-USDT", now),
            Duration::ZERO
        );
        assert_eq!(
            pacer.reserve_at(RequestKind::Submit, "BTC-USDT", now),
            Duration::ZERO
        );
        assert_eq!(
            pacer.reserve_at(RequestKind::Submit, "BTC-USDT", now),
            Duration::from_secs(1)
        );
        assert_eq!(
            pacer.reserve_at(RequestKind::Submit, "ETH-USDT", now),
            Duration::ZERO
        );
    }

    #[test]
    fn cloned_pacers_share_reservations() {
        let pacer = RequestPacer::new(PacingPolicy {
            submit_requests: 1,
            cancel_requests: 1,
            reconcile_requests: 1,
            window: Duration::from_secs(1),
        });
        let clone = pacer.clone();
        let now = Instant::now();

        assert_eq!(
            pacer.reserve_at(RequestKind::Reconcile, "account", now),
            Duration::ZERO
        );
        assert_eq!(
            clone.reserve_at(RequestKind::Reconcile, "account", now),
            Duration::from_secs(1)
        );
    }
}
