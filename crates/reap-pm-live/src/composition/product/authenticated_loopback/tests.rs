use std::any::type_name;

use super::root::{PmAuthenticatedLoopbackCompositionError, PmAuthenticatedLoopbackProduct};
use super::run::PmAuthenticatedLoopbackRun;
use super::startup::{PmAuthenticatedLoopbackReady, validate_public_ws_policy};
use super::supervision::{PmAbortableTask, PmMutationTask};
use crate::composition::PmProductRun;

fn assert_send<T: Send>() {}

#[test]
fn authenticated_root_is_static_distinct_and_task_movable() {
    assert_send::<PmAuthenticatedLoopbackProduct<()>>();
    assert_send::<PmAuthenticatedLoopbackReady<reap_pm_strategy::PmFixtureQuoteModel>>();
    assert_ne!(
        type_name::<PmAuthenticatedLoopbackProduct<()>>(),
        type_name::<PmProductRun<reap_pm_strategy::PmFixtureQuoteModel>>(),
    );
    assert_eq!(
        PmAuthenticatedLoopbackCompositionError::PublicScopeMismatch.to_string(),
        "authenticated loopback public roles do not match the exact product wire scope",
    );
}

#[test]
fn authenticated_static_start_has_one_ordered_scope_and_durability_gate() {
    let source = include_str!("startup.rs");
    let ordered = [
        "public_connectivity.into_roles().into_roles()",
        ".refresh(&mut metadata_sink)",
        ".start(capture_path, authoritative, session_policy, provenance)",
        "PmCoordinator::prepare_assembly",
        "private_connectivity.split()",
        "PmLoopbackAuthenticatedExecutionStage::start",
        ".activate_after_goal_f_validation(",
        "PmCoordinator::assemble_with_mutation",
    ];
    let mut prior = 0;
    for needle in ordered {
        let position = source
            .find(needle)
            .unwrap_or_else(|| panic!("static start omitted {needle}"));
        assert!(position >= prior, "static start reordered {needle}");
        prior = position;
    }
    for required in [
        "PmAuthoritativeMetadata::join_live_clob_v2_raw",
        "authenticated_user_ws.with_clock_source(user_ws_clock)",
        "goal_f_recovery",
        "authenticated_recovery",
        "PmLoopbackAuthenticatedExecutionShutdown",
    ] {
        assert!(
            source.contains(required),
            "static ready owner omitted {required}",
        );
    }
    for forbidden in [
        "tokio::spawn",
        "Arc<Mutex",
        "PmFixtureEffectExecutor",
        "SystemPublicWsClock",
        "SystemUserWsClock",
        "Option<PmFixedPlaceLoopbackRole>",
    ] {
        assert!(
            !source.contains(forbidden),
            "static start contains forbidden shape {forbidden}",
        );
    }
}

#[test]
fn public_transport_policy_must_match_the_canonical_session_before_io() {
    use std::time::Duration;

    use reap_pm_core::{ConnectionEpoch, PmConditionId, PmMarketId, PmTokenId, U256};
    use reap_pm_state::PmBookFreshness;
    use reap_polymarket_adapter::PmPublicHeartbeatConfig;
    use reap_polymarket_live_adapter::PmPublicWsConfig;
    use reap_polymarket_wire::PmWireScope;

    use crate::capture::{PmCaptureReconnectPolicy, PmCaptureSessionPolicy};

    let scope = PmWireScope::new(
        PmConditionId::parse(&format!("0x{}", "1".repeat(64))).unwrap(),
        PmMarketId::parse(&format!("0x{}", "2".repeat(64))).unwrap(),
        PmTokenId::new(U256::from_u64(7)).unwrap(),
    );
    let transport = PmPublicWsConfig::loopback_evidence(
        "ws://127.0.0.1:32123/ws/market",
        scope,
        Duration::from_millis(100),
        Duration::from_secs(2),
        Duration::from_millis(100),
        Duration::from_millis(50),
        4_096,
        1,
        Duration::from_millis(10),
        16,
        ConnectionEpoch::new(1),
    )
    .unwrap()
    .transport_policy();
    let reconnect =
        PmCaptureReconnectPolicy::new(Duration::from_millis(1), Duration::from_millis(4), 2)
            .unwrap();
    let policy = |ping_ns| {
        PmCaptureSessionPolicy::new(
            ConnectionEpoch::new(1),
            None,
            reconnect,
            PmPublicHeartbeatConfig::new(ping_ns, 50_000_000).unwrap(),
            PmBookFreshness::new(1_000_000_000, 1_000_000_000).unwrap(),
            1,
            reconnect,
        )
        .unwrap()
    };

    assert!(validate_public_ws_policy(transport, policy(100_000_000)).is_ok());
    assert!(validate_public_ws_policy(transport, policy(1_000_000_000)).is_err());

    let startup = include_str!("startup.rs");
    assert!(
        startup.find("validate_public_ws_policy").unwrap()
            < startup.find(".refresh(&mut metadata_sink)").unwrap(),
        "transport/session contradiction must be rejected before endpoint I/O",
    );
}

#[test]
fn every_post_capture_start_failure_has_bounded_owned_cleanup() {
    let source = include_str!("startup.rs");
    assert!(source.matches("public.finish().await").count() >= 5);
    let assembly_failure = source
        .split("Err(failure) => {")
        .nth(1)
        .expect("retaining coordinator assembly failure branch")
        .split("return Err(PmAuthenticatedLoopbackStartError::CoordinatorAssembly")
        .next()
        .expect("bounded assembly cleanup body");
    for required in [
        "failure.into_parts()",
        "mutation.shutdown().await",
        "public.finish().await",
        "execution.shutdown().await",
    ] {
        assert!(
            assembly_failure.contains(required),
            "assembly failure dropped started owner instead of cleaning {required}",
        );
    }
    assert!(source.contains("public_cleanup: Option<Box<PmPublicCaptureRunError>>"));
    assert!(source.contains("PmAuthenticatedLoopbackCleanupErrors"));
}

#[test]
fn public_metadata_and_private_journal_start_revalidate_the_exact_configuration() {
    let root = include_str!("root.rs");
    for exact_public_identity in [
        "public_scope.condition() != expected.condition()",
        "public_scope.market() != expected.market()",
        "public_scope.token() != expected.outcome().token()",
    ] {
        assert!(root.contains(exact_public_identity));
    }

    let startup = include_str!("startup.rs");
    for exact_metadata_identity in [
        "pair.scope().condition() != self.expected.condition()",
        "pair.scope().market() != self.expected.market()",
        "pair.scope().token() != self.expected.outcome().token()",
    ] {
        assert!(startup.contains(exact_metadata_identity));
    }
    let execution = include_str!("../../../coordinator/authenticated_execution.rs");
    let validation = execution
        .find("validate_connectivity_binding(config, &connectivity_binding)")
        .expect("exact private connectivity validation");
    let journal_start = execution
        .find("PmAuthenticatedJournalRuntime::start")
        .expect("authenticated journal start");
    assert!(
        validation < journal_start,
        "private config must be rejected before the authenticated journal opens",
    );
    for exact_private_identity in [
        "connectivity_fingerprint != config.public().configuration_fingerprint()",
        "binding.account() != config.account().account_scope()",
        "binding.instrument() != config.account().instrument()",
        "binding.trading_domain() != config.account().trading_domain()",
    ] {
        assert!(execution.contains(exact_private_identity));
    }
}

#[test]
fn authenticated_root_has_no_runtime_backend_or_detached_task_escape() {
    let root = include_str!("root.rs");
    for forbidden in [
        "Arc<Mutex",
        "PmFixtureEffectExecutor",
        "Option<PmFixedPlaceLoopbackRole>",
        "Option<PmExactOwnedCancelLoopbackRole>",
        "PmPublicHttpRole",
        "PmPublicMetadataHttpRole",
        "PmPublicMarketWsRole",
        "SystemPublicWsClock",
        "SystemUserWsClock",
        "tokio::spawn",
        "JoinHandle",
    ] {
        assert!(
            !root.contains(forbidden),
            "authenticated root contains forbidden ownership shape {forbidden}",
        );
    }
    for required in [
        "PmLoopbackMutationConnectivityOwner",
        "PmFixedPlaceLoopbackRole",
        "PmExactOwnedCancelLoopbackRole",
        "PmPublicConnectivityOwner",
        "PmMutationPreparationRole",
    ] {
        assert!(
            root.contains(required),
            "authenticated root omitted required static authority {required}",
        );
    }
    let authenticated_fields = root
        .split("pub(super) struct PmAuthenticatedLoopbackProduct<M> {")
        .nth(1)
        .expect("authenticated root declaration")
        .split('}')
        .next()
        .expect("authenticated root fields");
    assert!(!authenticated_fields.contains("PmProduct<M>"));
    assert!(!authenticated_fields.contains("fixture"));
}

#[tokio::test]
async fn supervised_task_drop_aborts_instead_of_detaching_the_child() {
    struct DropSignal(Option<tokio::sync::oneshot::Sender<()>>);

    impl Drop for DropSignal {
        fn drop(&mut self) {
            if let Some(signal) = self.0.take() {
                let _ = signal.send(());
            }
        }
    }

    let (started, wait_started) = tokio::sync::oneshot::channel();
    let (dropped, wait_dropped) = tokio::sync::oneshot::channel();
    let task = tokio::spawn(async move {
        let _drop_signal = DropSignal(Some(dropped));
        let _ = started.send(());
        std::future::pending::<()>().await;
    });
    let supervised = PmAbortableTask::new(task);
    wait_started.await.unwrap();
    drop(supervised);
    tokio::time::timeout(std::time::Duration::from_secs(2), wait_dropped)
        .await
        .expect("aborted child did not destroy its owned state")
        .expect("child drop signal was lost");
}

#[test]
fn mutation_task_control_surface_has_no_controlled_abort() {
    fn assert_send<T: Send>() {}
    assert_send::<PmMutationTask<()>>();
    let source = include_str!("supervision.rs");
    let mutation_impl = source
        .split("impl<T> PmMutationTask<T> {")
        .nth(1)
        .expect("mutation task impl")
        .split("impl<T> Drop for PmMutationTask<T>")
        .next()
        .expect("mutation task impl body");
    assert!(mutation_impl.contains("async fn join"));
    assert!(!mutation_impl.contains("abort_and_join"));
    assert!(source.contains("impl<T> PmAbortableTask<T>"));
    assert!(source.contains("abort_and_join"));
}

#[tokio::test]
async fn mutation_task_gracefully_joins_a_delayed_owned_operation() {
    let (release, wait_release) = tokio::sync::oneshot::channel();
    let task = tokio::spawn(async move {
        wait_release.await.expect("test release retained");
        41_u64
    });
    let mut owned = PmMutationTask::new(task);
    assert!(!owned.is_finished());
    release.send(()).unwrap();
    assert_eq!(owned.join().await.unwrap(), 41);
}

#[tokio::test]
async fn cancelled_join_future_retains_the_post_dispatch_task_owner() {
    let (release, wait_release) = tokio::sync::oneshot::channel();
    let task = tokio::spawn(async move {
        wait_release.await.expect("test release retained");
        43_u64
    });
    let mut owned = PmMutationTask::new(task);
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(10), owned.join())
            .await
            .is_err(),
        "join unexpectedly completed before the retained operation was released",
    );
    assert!(
        !owned.is_finished(),
        "cancelled join aborted the owned task"
    );
    release.send(()).unwrap();
    assert_eq!(owned.join().await.unwrap(), 43);
}

#[tokio::test]
async fn delayed_writer_ack_is_bounded_by_deadline_not_immediate_poll_speed() {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };

    let acknowledged = Arc::new(AtomicBool::new(false));
    let writer_ack = Arc::clone(&acknowledged);
    let writer = tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        writer_ack.store(true, Ordering::Release);
    });
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
    let mut pending_polls = 0_usize;
    while !acknowledged.load(Ordering::Acquire) {
        pending_polls += 1;
        assert!(pending_polls < 1_000, "delayed writer exhausted poll cap");
        assert!(
            tokio::time::Instant::now() < deadline,
            "delayed writer exceeded the explicit durability deadline"
        );
        tokio::time::sleep(std::time::Duration::from_millis(1)).await;
    }
    writer.await.unwrap();
    assert!(
        pending_polls > 1,
        "writer delay did not exercise pending polls"
    );
}

#[test]
fn authenticated_mutation_run_orders_capacity_admission_and_both_barriers() {
    assert_send::<PmAuthenticatedLoopbackRun<reap_pm_strategy::PmFixtureQuoteModel>>();
    let source = include_str!("run.rs");
    for method in ["start_place", "start_cancel"] {
        let body = source
            .split(&format!("fn {method}"))
            .nth(1)
            .expect("purpose start method")
            .split("pub(super) fn")
            .next()
            .expect("bounded purpose start body");
        let poll_time = body.find("poll_").unwrap();
        let request_time = body.find("request_").unwrap();
        let validate_time = body.find(".authorize_loopback_").unwrap();
        let worker_take = body.find("prepare_").unwrap();
        let spawn = body.find("tokio::spawn(task.run())").unwrap();
        assert!(poll_time < request_time);
        assert!(request_time < validate_time);
        assert!(validate_time < worker_take);
        assert!(worker_take < spawn);
        assert!(!body.contains("tokio::select!"));
    }

    for method in ["admit_pending_place", "admit_pending_cancel"] {
        let body = source
            .split(&format!("fn {method}"))
            .nth(1)
            .expect("purpose completion admission")
            .split("\n    fn ")
            .next()
            .expect("bounded admission body");
        let permit = body.find("reserve_authenticated_completion").unwrap();
        let occurrence = body.find("issue_mutation_completion").unwrap();
        let moved_completion = body.find(".take()").unwrap();
        assert!(permit < occurrence);
        assert!(occurrence < moved_completion);
    }

    for required in [
        "issue_persistence_poll",
        "poll_persistence_live",
        "take_authenticated_bridge_applied",
        "confirm_goal_f_bridge",
        "MAX_AUTHENTICATED_BRIDGE_SERVICE_TURNS",
        "goal_f_bridge_timeout",
        "PmMutationFinishOutcome::PendingBridge",
    ] {
        assert!(source.contains(required), "bridge omitted {required}");
    }
}

#[test]
fn place_and_cancel_have_independent_tasks_and_clear_the_actual_applied_purpose() {
    let source = include_str!("run.rs");
    let fields = source
        .split("struct PmAuthenticatedLoopbackRun")
        .nth(1)
        .unwrap()
        .split("}\n")
        .next()
        .unwrap();
    assert!(fields.contains("place_task: Option<PmMutationTask"));
    assert!(fields.contains("cancel_task: Option<PmMutationTask"));
    assert!(fields.contains("place_bridge_pending: bool"));
    assert!(fields.contains("cancel_bridge_pending: bool"));

    let confirmation = source
        .split("fn confirm_available_bridge")
        .nth(1)
        .unwrap()
        .split("/// Controlled shutdown")
        .next()
        .unwrap();
    assert!(confirmation.contains("let actual_place = applied.is_place()"));
    assert!(confirmation.contains("if actual_place"));
    assert!(confirmation.contains("self.place_bridge_pending = false"));
    assert!(confirmation.contains("self.cancel_bridge_pending = false"));
    assert!(confirmation.contains("actual_place == expected_place"));
}

#[test]
fn controlled_shutdown_joins_mutations_without_abort_or_select_escape() {
    let source = include_str!("run.rs");
    let shutdown = source
        .split("pub(super) async fn shutdown")
        .nth(1)
        .unwrap()
        .split("#[derive(Debug, Error)]")
        .next()
        .unwrap();
    assert!(source.contains("self.finish_place().await"));
    assert!(source.contains("self.finish_cancel().await"));
    assert!(shutdown.contains("self.ready.shutdown().await"));
    assert!(!shutdown.contains(".abort("));
    assert!(!source.contains("tokio::select!"));
    assert!(!source.contains("abort_and_join"));
    assert!(!source.contains("PmFixtureCompletionOccurrence"));
    assert!(source.contains("place_task_terminal_failure"));
    assert!(source.contains("cancel_task_terminal_failure"));
    assert!(source.contains("place_execution_failure"));
    assert!(source.contains("cancel_execution_failure"));

    let bridge = source
        .split("fn advance_goal_f_bridge")
        .nth(1)
        .unwrap()
        .split("fn confirm_available_bridge")
        .next()
        .unwrap();
    assert!(
        bridge.find("confirm_available_bridge").unwrap()
            < bridge.find("BridgeDurabilityTimeout").unwrap(),
        "an already-arrived applied proof must win over an expired deadline",
    );

    let cleanup = source
        .split("async fn drain_mutation_work_after_failure")
        .nth(1)
        .unwrap()
        .split("/// Controlled shutdown never aborts mutation tasks")
        .next()
        .unwrap();
    for required in [
        "drive_controlled_shutdown_safety_until",
        "pending_cancel_time",
        "prepared_cancel",
        "may_have_sent_unbound",
        "venue_safety_unsettled",
        "evidence.place.get_or_insert",
        "evidence.cancel.get_or_insert",
        "evidence.read.get_or_insert",
        "evidence.goal_f.get_or_insert",
    ] {
        assert!(cleanup.contains(required), "cleanup omitted {required}");
    }
    for forbidden in [
        "let _ = self.finish_place()",
        "let _ = self.finish_cancel()",
        "let _ = self.pump_shutdown_read_once()",
        "let _ = self.pump_goal_f_once()",
    ] {
        assert!(
            !cleanup.contains(forbidden),
            "cleanup discarded typed secondary via {forbidden}",
        );
    }

    let bound = source
        .split("fn controlled_shutdown_bound_error")
        .nth(1)
        .unwrap()
        .split("async fn drain_mutation_work_after_failure")
        .next()
        .unwrap();
    assert!(
        bound.find("retained_terminal_failure()").unwrap()
            < bound.find("MayHaveSentPlaceUnresolved").unwrap(),
        "a retained typed task/execution failure must remain the shutdown primary",
    );

    let bridge_failure = source
        .split("fn reject_any_bridge_failure")
        .nth(1)
        .unwrap()
        .split("fn confirm_available_bridge")
        .next()
        .unwrap();
    let dequeue = bridge_failure
        .find("take_authenticated_bridge_failure")
        .unwrap();
    let reject = bridge_failure
        .find("return Err(PmAuthenticatedLoopbackRunError::BridgePersistence")
        .unwrap();
    assert!(
        dequeue < reject,
        "an opposite-purpose bridge writer failure must terminate immediately",
    );

    let shutdown_pump = source
        .split("fn pump_goal_f_once")
        .nth(1)
        .unwrap()
        .split("async fn pump_shutdown_read_once")
        .next()
        .unwrap();
    let service = shutdown_pump.find("service_turn").unwrap();
    let service_reject = shutdown_pump[service..]
        .find("finish_goal_f_boundary")
        .map(|offset| service + offset)
        .unwrap();
    let poll = shutdown_pump.find("poll_persistence_live").unwrap();
    let poll_reject = shutdown_pump[poll..]
        .find("finish_goal_f_boundary")
        .map(|offset| poll + offset)
        .unwrap();
    assert!(
        service < service_reject && service_reject < poll && poll < poll_reject,
        "shutdown must classify a cancel bridge failure before same-cycle read service",
    );

    let boundary = source
        .split("fn finish_goal_f_boundary")
        .nth(1)
        .unwrap()
        .split("fn retain_terminal_secondary")
        .next()
        .unwrap();
    assert!(
        boundary.find("reject_any_goal_f_failure").unwrap()
            < boundary.find("result.map_err").unwrap(),
        "typed Goal-F writer/bridge evidence must win over a later boundary error",
    );

    let terminal = source
        .split("fn shutdown_safety_reached_terminal_projection")
        .nth(1)
        .unwrap()
        .split("/// Controlled shutdown never aborts mutation tasks")
        .next()
        .unwrap();
    assert!(terminal.contains("owned_shutdown_safety_settled()"));
    assert!(terminal.contains("owned_shutdown_venue_safety_settled()"));
    assert!(terminal.contains("owned_shutdown_has_may_have_sent_unbound()"));

    let shutdown = source
        .split("pub(super) async fn shutdown")
        .nth(1)
        .unwrap()
        .split("#[derive(Debug, Clone, Copy, PartialEq, Eq)]")
        .next()
        .unwrap();
    let retained_at_entry = shutdown.find("retained_safety_at_entry").unwrap();
    let drive = shutdown.find("drive_controlled_shutdown_safety()").unwrap();
    let retained_after_drive = shutdown
        .find("or_else(|| self.retained_terminal_failure()")
        .unwrap();
    let aggregate = shutdown.find("safety_secondary").unwrap();
    assert!(
        retained_at_entry < drive
            && drive < retained_after_drive
            && retained_after_drive < aggregate
    );

    let read_shutdown = shutdown.find("read.shutdown").unwrap();
    let post_read_projection = shutdown[read_shutdown..]
        .find("reject_any_goal_f_failure")
        .map(|offset| read_shutdown + offset)
        .unwrap();
    let final_quiescence = shutdown[post_read_projection..]
        .find("drive_final_goal_f_quiescence")
        .map(|offset| post_read_projection + offset)
        .unwrap();
    let displaced_secondary = shutdown[final_quiescence..]
        .find("self.terminal_secondary.take()")
        .map(|offset| final_quiescence + offset)
        .unwrap();
    let time_shutdown = shutdown.find("time.shutdown").unwrap();
    let owner_shutdown = shutdown.find("self.ready.shutdown().await").unwrap();
    assert!(
        read_shutdown < post_read_projection
            && post_read_projection < final_quiescence
            && final_quiescence < displaced_secondary
            && displaced_secondary < time_shutdown
            && time_shutdown < owner_shutdown,
        "final Goal-F lanes must drain after read producers and before durability owners",
    );

    let final_drain = source
        .split("async fn drive_final_goal_f_quiescence")
        .nth(1)
        .unwrap()
        .split("async fn pump_shutdown_read_once")
        .next()
        .unwrap();
    let first_projection = final_drain.find("reject_any_goal_f_failure").unwrap();
    let quiescent = final_drain.find("goal_f_shutdown_quiescent").unwrap();
    let pump = final_drain.find("pump_goal_f_once").unwrap();
    let second_projection = final_drain[pump..]
        .find("reject_any_goal_f_failure")
        .map(|offset| pump + offset)
        .unwrap();
    let deadline = final_drain.find("GoalFShutdownQuiescenceTimeout").unwrap();
    assert!(
        first_projection < quiescent
            && quiescent < pump
            && pump < second_projection
            && second_projection < deadline,
        "final Goal-F drain must classify each boundary before declaring idle or timed out",
    );
    assert!(final_drain.contains("first_boundary_error"));
    assert!(final_drain.contains("GoalFWriter(_)"));
    assert!(final_drain.contains("BridgePersistence(_)"));
    assert!(
        !final_drain.contains("self.pump_goal_f_once()?"),
        "retryable retained persistence admission must not escape the final drain",
    );
    assert!(
        shutdown.find("cleanup.boundary_secondary").unwrap()
            < shutdown.find("goal_f_shutdown_unresolved_counts").unwrap(),
        "post-read shutdown must retain both displaced errors and unresolved fixed-count evidence",
    );
}

#[test]
fn read_shutdown_preserves_typed_exits_and_late_refresh_obligations() {
    let source = include_str!("read_ingress/mod.rs");
    let shutdown = source
        .split("pub(super) async fn shutdown")
        .nth(1)
        .unwrap()
        .split("fn merge_book_demand")
        .next()
        .unwrap();
    for required in [
        "PmReadIngressServiceError::HttpTaskExited(failure)",
        "PmReadIngressServiceError::BookTaskExited(failure)",
        "actor.drain_shutdown_copied_effects",
        "self.dispatch_next_refresh(actor)",
        "refresh_quiescent",
        "self.pending_book_demand.is_some()",
        "book_shutdown_ready(",
        "http_shutdown_ready(",
        "read_shutdown_unresolved_counts",
        "actor.service_after_ingress()",
        "read_unresolved_counts",
        "refresh_unresolved",
    ] {
        assert!(
            shutdown.contains(required),
            "read shutdown omitted {required}"
        );
    }
    assert!(!shutdown.contains("unreachable!(\"controlled shutdown joins"));

    let projection = include_str!("../../../coordinator/product/service/live_ingress.rs");
    let read_counts = projection
        .split("fn read_shutdown_unresolved_counts")
        .nth(1)
        .unwrap()
        .split("\n    }")
        .next()
        .unwrap();
    assert!(read_counts.contains("public_lane"));
    assert!(read_counts.contains("read_input_lane_depths"));
    assert!(read_counts.contains("retained_private_admission"));
    assert!(read_counts.contains("retained_reconciliation_admission"));
}
