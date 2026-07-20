use std::future::pending;
use std::time::{Duration, Instant};

use super::SchedulingState;

impl SchedulingState {
    pub(super) fn new() -> Self {
        Self {
            origin: Instant::now(),
        }
    }

    pub(super) const fn origin(&self) -> Instant {
        self.origin
    }

    pub(super) fn captured_receipt_ns(&self, received_at: Instant) -> u64 {
        duration_ns(received_at.saturating_duration_since(self.origin))
    }
}

pub(super) fn monotonic_now_ns(origin: Instant) -> u64 {
    duration_ns(Instant::now().saturating_duration_since(origin))
}

pub(super) async fn wait_until_monotonic_ns(origin: Instant, due_ns: Option<u64>) {
    let Some(due_ns) = due_ns else {
        pending::<()>().await;
        return;
    };
    let deadline = origin
        .checked_add(Duration::from_nanos(due_ns))
        .expect("bounded monotonic trade-reprice deadline must fit Instant");
    tokio::time::sleep_until(tokio::time::Instant::from_std(deadline)).await;
}

fn duration_ns(duration: Duration) -> u64 {
    u64::try_from(duration.as_nanos()).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::SchedulingState;

    #[test]
    fn captured_receipts_keep_their_exact_origin_relative_time_when_delivery_reorders() {
        let origin = Instant::now();
        let scheduling = SchedulingState { origin };
        assert_eq!(
            scheduling.captured_receipt_ns(origin + Duration::from_nanos(200)),
            200
        );
        assert_eq!(
            scheduling.captured_receipt_ns(origin + Duration::from_nanos(100)),
            100
        );
        assert_eq!(
            scheduling.captured_receipt_ns(origin + Duration::from_nanos(300)),
            300
        );
    }
}
