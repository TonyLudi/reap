use std::time::Duration;

/// Shared request-rate limits consumed by order pacing implementations.
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
