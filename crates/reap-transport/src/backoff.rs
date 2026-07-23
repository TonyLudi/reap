use std::time::Duration;

/// Bounded reconnect delays for a supervised session.
#[derive(Debug, Clone)]
pub struct ReconnectPolicy {
    pub initial_delay: Duration,
    pub max_delay: Duration,
    pub multiplier: u32,
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
    #[must_use]
    pub fn next_delay(&self, current: Duration) -> Duration {
        current
            .saturating_mul(self.multiplier.max(1))
            .min(self.max_delay)
    }
}

/// Stateful reconnect history owned by one supervised session.
#[derive(Debug)]
pub struct ReconnectBackoff {
    policy: ReconnectPolicy,
    next_delay: Duration,
}

impl ReconnectBackoff {
    #[must_use]
    pub fn new(policy: ReconnectPolicy) -> Self {
        let next_delay = policy.initial_delay;
        Self { policy, next_delay }
    }

    pub fn reset(&mut self) {
        self.next_delay = self.policy.initial_delay;
    }

    #[must_use]
    pub fn preview_after_failure(&self, reached_ready: bool) -> Duration {
        if reached_ready {
            self.policy.initial_delay
        } else {
            self.next_delay
        }
    }

    /// Records one failed attempt and returns the delay before the next one.
    ///
    /// A session that reached ready state resets historical startup failures
    /// before its new failure is charged.
    pub fn after_failure(&mut self, reached_ready: bool) -> Duration {
        if reached_ready {
            self.reset();
        }
        let delay = self.next_delay;
        self.next_delay = self.policy.next_delay(delay);
        delay
    }
}
