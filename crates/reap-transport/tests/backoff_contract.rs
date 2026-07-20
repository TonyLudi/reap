use std::time::Duration;

use reap_transport::{ReconnectBackoff, ReconnectPolicy};

#[test]
fn reconnect_backoff_is_bounded_and_ready_resets_history() {
    let policy = ReconnectPolicy {
        initial_delay: Duration::from_millis(10),
        max_delay: Duration::from_millis(25),
        multiplier: 2,
    };
    let mut backoff = ReconnectBackoff::new(policy);

    assert_eq!(backoff.after_failure(false), Duration::from_millis(10));
    assert_eq!(backoff.after_failure(false), Duration::from_millis(20));
    assert_eq!(backoff.after_failure(false), Duration::from_millis(25));
    assert_eq!(backoff.after_failure(true), Duration::from_millis(10));
    assert_eq!(backoff.after_failure(false), Duration::from_millis(20));
    backoff.reset();
    assert_eq!(backoff.after_failure(false), Duration::from_millis(10));
}

#[test]
fn reconnect_policy_is_independent_from_order_request_pacing() {
    let policy = ReconnectPolicy::default();

    assert_eq!(policy.initial_delay, Duration::from_millis(250));
    assert_eq!(policy.max_delay, Duration::from_secs(30));
    assert_eq!(policy.multiplier, 2);
    assert_eq!(
        policy.next_delay(Duration::from_secs(20)),
        Duration::from_secs(30)
    );
}
