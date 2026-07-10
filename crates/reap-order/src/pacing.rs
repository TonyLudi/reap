use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RequestKind {
    Submit,
    Cancel,
    Reconcile,
}

#[derive(Debug, Clone)]
pub struct PacingPolicy {
    pub submit_requests: usize,
    pub cancel_requests: usize,
    pub reconcile_requests: usize,
    pub window: Duration,
}

impl Default for PacingPolicy {
    fn default() -> Self {
        Self {
            submit_requests: 50,
            cancel_requests: 50,
            reconcile_requests: 50,
            window: Duration::from_secs(2),
        }
    }
}

#[derive(Debug)]
pub struct RequestPacer {
    policy: PacingPolicy,
    reservations: HashMap<(RequestKind, String), VecDeque<Instant>>,
}

impl RequestPacer {
    pub fn new(policy: PacingPolicy) -> Self {
        Self {
            policy,
            reservations: HashMap::new(),
        }
    }

    pub fn reserve_at(&mut self, kind: RequestKind, scope: &str, now: Instant) -> Duration {
        let limit = self.limit(kind).max(1);
        let queue = self
            .reservations
            .entry((kind, scope.to_string()))
            .or_default();
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

    pub async fn pace(&mut self, kind: RequestKind, scope: &str) {
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
        let mut pacer = RequestPacer::new(PacingPolicy {
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
}
