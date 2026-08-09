const AUTHORITY: &str = include_str!("../src/controlled_trial/authority.rs");
const PRIVATE_READS: &str = include_str!("../src/controlled_trial/runtime/private_reads.rs");
const PUBLIC_BOOK: &str = include_str!("../src/controlled_trial/runtime/public_book.rs");
const USER_STREAM: &str = include_str!("../src/controlled_trial/runtime/user_stream.rs");
const RUNTIME_MOD: &str = include_str!("../src/controlled_trial/runtime/mod.rs");
const MANIFEST: &str = include_str!("../Cargo.toml");

fn production_prefix(source: &str) -> &str {
    source
        .split_once("\n#[cfg(test)]\nmod tests")
        .map_or(source, |(head, _)| head)
}

#[test]
fn runtime_is_binary_private_read_only_evidence_without_a_transport_escape() {
    assert!(RUNTIME_MOD.contains("mod private_reads;"));
    assert!(RUNTIME_MOD.contains("mod public_book;"));
    assert!(RUNTIME_MOD.contains("mod user_stream;"));
    assert!(!MANIFEST.contains("reqwest"));

    for (name, source) in [
        ("private reads", production_prefix(PRIVATE_READS)),
        ("public book", production_prefix(PUBLIC_BOOK)),
        ("user stream", production_prefix(USER_STREAM)),
    ] {
        for forbidden in [
            "reqwest",
            "send_once",
            "authenticate_place_once",
            "AuthenticatedPlaceRequest",
            "RuntimeExactBodyCommitment",
            "POST /order",
            "DELETE /order",
            "production_order_entry_authorized: true",
            "real_order_submission_authorized: true",
            "println!",
            "eprintln!",
            "dbg!",
            "serde::Serialize",
        ] {
            assert!(
                !source.contains(forbidden),
                "{name} runtime gained forbidden `{forbidden}` surface",
            );
        }
    }
}

#[test]
fn public_book_uses_one_role_bundle_source_watermarks_and_internal_clock_edges() {
    for required in [
        "struct PmControlledTrialPublicConnectivity",
        "fn from_roles(roles: PmPublicConnectivityRoles)",
        "roles: PmPublicConnectivityRoles",
        "metadata_control_begin = actor_clock.observe_control_edge()",
        "metadata_control_complete = actor_clock.observe_control_edge()",
        "RuntimeControlClock::Live(actor_clock)",
        "self.activity.generation()",
        "source_high_water != self.admitted_activity_generation",
        "self.last_heartbeat = None;",
        "struct PmPublicBookWsTask",
        "async fn run_to_exit(",
        "role: PmPublicMarketWsRole",
        "sink: PmPublicBookRuntimeSink",
        "struct PmPublicBookCoordinator",
        "book_http: PmPublicHttpRole",
        "handle: PmPublicBookRuntimeHandle",
        "self.book_http.seed_book(&mut self.handle).await",
        "self.book_http.resync_book(&mut self.handle).await",
        "invalidate_after_task_exit",
        "impl Drop for PmPublicBookRuntimeSink",
        "fn into_runtime_roles(",
        "PmPrivateReadClockBundle {",
        "PmMutationCompanionRoles {",
    ] {
        assert!(
            PUBLIC_BOOK.contains(required),
            "missing public-book pin `{required}`"
        );
    }
    assert!(!production_prefix(PUBLIC_BOOK).contains("now_monotonic_ns:"));
    assert!(!PUBLIC_BOOK.contains("pub(super) struct PmPublicBookRuntimeHandle"));
    assert!(!PUBLIC_BOOK.contains("pub(super) struct PmPublicBookRuntimeSink"));
    let before = PUBLIC_BOOK
        .find("metadata_control_begin = actor_clock.observe_control_edge()")
        .expect("pre-fetch control edge");
    let fetch = PUBLIC_BOOK
        .find(".refresh_authoritative_observation(")
        .expect("one-fetch authoritative metadata");
    let after = PUBLIC_BOOK
        .find("metadata_control_complete = actor_clock.observe_control_edge()")
        .expect("post-fetch control edge");
    assert!(before < fetch && fetch < after);
}

#[test]
fn private_rest_and_user_stream_form_one_move_only_same_authority_cut() {
    for required in [
        "PmSameCredentialUserWsInput",
        "PmSameCredentialAuthorityMarker",
        "PmUserRestCollectionStart",
        "PmSameAuthorityRestJoin",
        "Arc<PmRestCutIdentity>",
        "PmFreshAuthenticatedRestCut",
        "PmRecoveryAccountAuthenticatedRestCut",
        "PmRecoveryExactAuthenticatedRestCut",
        "begin_open_orders_observation",
        "seal_complete_open_orders",
        "begin_trades_observation",
        "seal_complete_trades",
        "fresh_read_server_time_observation",
    ] {
        assert!(
            PRIVATE_READS.contains(required),
            "missing private-read pin `{required}`"
        );
    }
    for required in [
        "begin_rest_collection",
        "finish_rest_collection",
        "consume_final_cut_ticket",
        "MAX_RETAINED_USER_BUSINESS_EVENTS",
        "struct PmUserStreamWsTask",
        "role: PmAuthenticatedUserWsRole",
        "sink: PmUserStreamSink",
        "open_order_rows,",
        "trade_rows,",
        "current_epoch_business_events",
        "rest_cut_identity",
        "impl Drop for PmUserStreamSink",
    ] {
        assert!(
            USER_STREAM.contains(required),
            "missing user-stream pin `{required}`"
        );
    }
    assert!(!PRIVATE_READS.contains("impl PmRecoveryExpectedOrderCandidate {\n    pub(super)"));
    assert!(!USER_STREAM.contains("pub(super) struct PmUserStreamSink"));
}

#[test]
fn recovery_roles_and_cuts_cannot_recover_signer_or_place_authority() {
    let recovery_roles = PRIVATE_READS
        .split_once("pub(super) struct PmRecoveryPrivateReadRuntimeRoles")
        .and_then(|(_, tail)| {
            tail.split_once("fn production_runtime_core")
                .map(|(slice, _)| slice)
        })
        .expect("recovery runtime surface markers");
    for forbidden in [
        "FreshPlaceAuthenticationOnce",
        "FixedEoaSigner",
        "authenticate_place",
        "PmFreshAuthenticatedRestCut",
    ] {
        assert!(
            !recovery_roles.contains(forbidden),
            "recovery runtime gained forbidden `{forbidden}` authority",
        );
    }
    assert!(AUTHORITY.contains("RecoveryCredentialAuthorityRoles"));
    assert!(AUTHORITY.contains("into_private_read_runtime_parts"));
    assert!(PRIVATE_READS.contains("PmRecoveryExpectedOrderCandidate"));
    assert!(PRIVATE_READS.contains("issued exact-owned carrier before enabling"));
}
