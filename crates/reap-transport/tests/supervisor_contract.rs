use std::time::Duration;

use reap_transport::{
    ConnectionAttemptPacer, ConnectionAttemptPacerError, ConnectionStatusKind, ReconnectPolicy,
    SupervisorState, request_shutdown, shutdown_channel, shutdown_requested, supervision_channels,
};

const TEST_STATE_MAGIC: &str = "reap-transport-test-pacer-v1";

#[test]
fn supervision_state_uses_health_and_backoff_without_protocol_semantics() {
    let mut state = SupervisorState::new(ReconnectPolicy {
        initial_delay: Duration::from_millis(5),
        max_delay: Duration::from_millis(20),
        multiplier: 2,
    });

    assert_eq!(state.health(), ConnectionStatusKind::Disconnected);
    state.mark_ready();
    assert_eq!(state.health(), ConnectionStatusKind::Ready);
    assert_eq!(state.after_failure(true), Duration::from_millis(5));
    assert_eq!(state.health(), ConnectionStatusKind::Disconnected);
    assert_eq!(state.after_failure(false), Duration::from_millis(10));
    state.mark_fatal();
    assert_eq!(state.health(), ConnectionStatusKind::Fatal);
}

#[test]
fn supervision_channels_are_bounded_and_share_one_shutdown_signal() {
    let channels = supervision_channels::<u8, &'static str>(1);
    channels.output_sender.try_send(1).unwrap();
    assert!(channels.output_sender.try_send(2).is_err());
    channels.health_sender.try_send("ready").unwrap();
    assert!(channels.health_sender.try_send("extra").is_err());
    assert!(!shutdown_requested(&channels.shutdown_receiver));

    assert!(request_shutdown(&channels.shutdown_sender));
    assert!(shutdown_requested(&channels.shutdown_receiver));
}

#[tokio::test]
async fn shutdown_is_monotonic_and_sender_loss_fails_closed() {
    let (shutdown_sender, mut shutdown_receiver) = shutdown_channel();
    let observer = shutdown_receiver.clone();
    assert!(!shutdown_requested(&observer));

    assert!(request_shutdown(&shutdown_sender));
    shutdown_receiver.changed().await.unwrap();
    assert!(shutdown_requested(&shutdown_receiver));
    assert!(shutdown_requested(&observer));

    drop(shutdown_sender);
    assert!(shutdown_requested(&observer));

    let (shutdown_sender, mut shutdown_receiver) = shutdown_channel();
    drop(shutdown_sender);
    assert!(shutdown_receiver.changed().await.is_err());
    assert!(shutdown_requested(&shutdown_receiver));
}

#[tokio::test]
async fn local_attempt_pacing_is_shutdown_cancellable() {
    let pacer = ConnectionAttemptPacer::new(Duration::from_secs(5));
    let (shutdown_sender, mut shutdown_receiver) = shutdown_channel();
    assert!(pacer.wait_for_turn(&mut shutdown_receiver).await.unwrap());

    let waiter = tokio::spawn(async move { pacer.wait_for_turn(&mut shutdown_receiver).await });
    tokio::time::sleep(Duration::from_millis(10)).await;
    assert!(request_shutdown(&shutdown_sender));

    assert!(!waiter.await.unwrap().unwrap());
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn scoped_process_shared_pacer_preserves_exact_caller_owned_magic() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("connect.pacer");
    let pacer =
        ConnectionAttemptPacer::process_shared(Duration::from_millis(1), &path, TEST_STATE_MAGIC)
            .unwrap();
    let (_shutdown_sender, mut shutdown_receiver) = shutdown_channel();

    assert!(pacer.wait_for_turn(&mut shutdown_receiver).await.unwrap());

    let state = std::fs::read_to_string(path).unwrap();
    assert!(state.starts_with(&format!("{TEST_STATE_MAGIC} ")));
    assert_eq!(state.lines().count(), 1);
}

#[test]
fn scoped_process_shared_pacer_rejects_protocol_like_or_unsafe_magic() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("connect.pacer");

    assert!(matches!(
        ConnectionAttemptPacer::process_shared(
            Duration::from_millis(1),
            &path,
            "contains whitespace",
        ),
        Err(ConnectionAttemptPacerError::InvalidState { .. })
    ));
}
