use std::time::Duration;

use reap_feed::{ConnectionAttemptPacer, ReconnectPolicy};
use tokio::sync::watch;

#[test]
fn legacy_reconnect_policy_public_shape_is_unchanged() {
    let policy = ReconnectPolicy {
        initial_delay: Duration::from_millis(10),
        max_delay: Duration::from_millis(25),
        multiplier: 2,
    };

    assert_eq!(
        policy.next_delay(Duration::from_millis(20)),
        Duration::from_millis(25)
    );
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn legacy_feed_pacer_retains_its_existing_state_magic() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("legacy.pacer");
    let pacer = ConnectionAttemptPacer::process_shared(Duration::from_millis(1), &path).unwrap();
    let (_shutdown_sender, mut shutdown_receiver) = watch::channel(false);

    assert!(pacer.wait_for_turn(&mut shutdown_receiver).await.unwrap());

    let state = std::fs::read_to_string(path).unwrap();
    assert!(state.starts_with("reap-okx-connect-pacer-v1 "));
}

#[tokio::test]
async fn legacy_watch_compatibility_still_fails_closed_when_sender_disappears() {
    let pacer = ConnectionAttemptPacer::new(Duration::from_secs(5));
    let (shutdown_sender, mut shutdown_receiver) = watch::channel(false);
    assert!(pacer.wait_for_turn(&mut shutdown_receiver).await.unwrap());

    let waiter = tokio::spawn(async move { pacer.wait_for_turn(&mut shutdown_receiver).await });
    drop(shutdown_sender);

    assert!(
        !tokio::time::timeout(Duration::from_millis(100), waiter)
            .await
            .unwrap()
            .unwrap()
            .unwrap()
    );
}
