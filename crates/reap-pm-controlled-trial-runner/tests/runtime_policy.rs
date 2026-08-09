const AUTHORITY: &str = include_str!("../src/controlled_trial/authority.rs");
const CURRENT_RUNTIME: &str = include_str!("../src/controlled_trial/runtime/current_runtime.rs");
const LINUX_EGRESS_LOCAL_FACTS: &str =
    include_str!("../src/controlled_trial/runtime/linux_egress_local_facts.rs");
const ONLINE_PREFLIGHT: &str = include_str!("../src/controlled_trial/runtime/online_preflight.rs");
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

fn between<'a>(source: &'a str, start: &str, end: &str) -> &'a str {
    source
        .split_once(start)
        .and_then(|(_, tail)| tail.split_once(end).map(|(slice, _)| slice))
        .expect("runtime source policy markers must remain ordered")
}

#[test]
fn runtime_is_binary_private_read_only_evidence_without_a_transport_escape() {
    assert!(RUNTIME_MOD.contains("mod current_runtime;"));
    assert!(RUNTIME_MOD.contains("mod linux_egress_local_facts;"));
    assert!(RUNTIME_MOD.contains("mod online_preflight;"));
    assert!(RUNTIME_MOD.contains("mod private_reads;"));
    assert!(RUNTIME_MOD.contains("mod public_book;"));
    assert!(RUNTIME_MOD.contains("mod user_stream;"));
    assert!(!MANIFEST.contains("reqwest"));

    for (name, source) in [
        ("current runtime", production_prefix(CURRENT_RUNTIME)),
        (
            "Linux local-egress facts",
            production_prefix(LINUX_EGRESS_LOCAL_FACTS),
        ),
        ("online preflight", production_prefix(ONLINE_PREFLIGHT)),
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
fn online_preflight_is_an_inseparable_denied_partial_candidate() {
    let production = production_prefix(ONLINE_PREFLIGHT);
    for required in [
        "pub(super) struct PmDeniedOnlinePreflightCandidate",
        "book: PmPublicBookLease",
        "rest: PmFreshAuthenticatedRestCut",
        "user: PmUserOnlinePreflightLease",
        "status: PmProductionStatusAnnouncementObservation",
        "health: PmProductionClobLivenessHealthObservation",
        "polygon: PmProductionPolygonFinalizedAuthorizationCut",
        "position: PmProductionDataApiPositionObservation",
        "candidate_manifest: PmOnlinePreflightCandidateManifest",
        "PhantomData<Rc<()>>",
        "OfflineAuthorizationState::DENIED",
        "OuterWindowRuntimeAndSelectedEgressNotIntegrated",
        "minimum_source_wall_edge_ns",
        "maximum_source_wall_edge_ns",
        "market_end_time",
        "take_only_delay_enabled_reported",
        "b\"take_only_delay_enabled_reported\"",
        "market.take_only_delay_enabled != Some(false)",
        "market.cancel_book_on_start != Some(false)",
        "market.rfq_enabled != Some(false)",
        "market.bonding_curve_enabled != Some(false)",
    ] {
        assert!(production.contains(required), "missing `{required}`");
    }
    for forbidden in [
        "impl Clone for PmDeniedOnlinePreflightCandidate",
        "fn into_parts(",
        "fn into_dispatch",
        "fn into_permit",
        "fn runtime_binding",
        "fn online_runtime_binding",
        "PmPhaseAOnlinePreflightEvidenceManifestV2",
        "AuthenticatedPlaceRequest",
        "RuntimeExactBodyCommitment",
    ] {
        assert!(
            !production.contains(forbidden),
            "online preflight gained forbidden `{forbidden}` surface",
        );
    }
}

#[test]
fn online_preflight_requires_explicit_false_optional_market_flags() {
    let preflight = production_prefix(ONLINE_PREFLIGHT);
    let public_book = production_prefix(PUBLIC_BOOK);
    for required in [
        "take_only_delay_enabled: Option<bool>",
        "clob.take_only_delay_enabled_reported()",
        "fn take_only_delay_enabled_reported(&self) -> Option<bool>",
    ] {
        assert!(
            public_book.contains(required),
            "public-book projection lost `{required}`",
        );
    }
    for permissive in [
        "market.cancel_book_on_start == Some(true)",
        "market.rfq_enabled == Some(true)",
        "market.bonding_curve_enabled == Some(true)",
    ] {
        assert!(
            !preflight.contains(permissive),
            "online preflight regained permissive optional flag check `{permissive}`",
        );
    }
}

#[test]
fn online_preflight_extrema_cover_every_committed_user_stream_clock() {
    let production = production_prefix(ONLINE_PREFLIGHT);
    let source_join = between(
        production,
        "impl JoinedFacts {",
        "fn append_current_user_source_wall_clocks(",
    );
    for required in [
        "user.connection_open_clock()",
        "user.subscription_clock()",
        "user.ping_clock()",
        "user.correlated_pong_clock()",
        "for reconnect in user.reconnect_history()",
        "reconnect.connection_open_generation()",
        "reconnect.connection_open_clock()",
        "reconnect.subscription_generation()",
        "reconnect.subscription_clock()",
        "reconnect.latest_ping_generation()",
        "reconnect.latest_ping_clock()",
        "reconnect.correlated_pong_generation()",
        "reconnect.correlated_pong_clock()",
        "reconnect.retirement_clock()",
        "reconnect.reconnect_clock()",
    ] {
        assert!(
            source_join.contains(required),
            "candidate extrema lost user source edge `{required}`",
        );
    }
    for required in [
        "fn append_optional_user_source_wall_clock(",
        "(Some(_), Some(clock))",
        "(None, None) => Ok(())",
        "UserReconnectHistory",
        "user.reconnect_count != user.reconnect_history_count",
        "user.reconnect_count != 0",
        "user.initial_connection_epoch != user.current_connection_epoch",
        "source_wall_extrema(&facts.source_wall_clocks_ns)",
    ] {
        assert!(
            production.contains(required),
            "missing clock pin `{required}`"
        );
    }
    for committed_reconnect_clock in [
        "reconnect.connection_open_clock()",
        "reconnect.subscription_clock()",
        "reconnect.latest_ping_clock()",
        "reconnect.correlated_pong_clock()",
        "reconnect.retirement_clock()",
        "reconnect.reconnect_clock()",
    ] {
        assert!(
            production.matches(committed_reconnect_clock).count() >= 2,
            "`{committed_reconnect_clock}` must feed both digest and candidate extrema",
        );
    }
}

#[test]
fn current_runtime_is_source_owned_move_only_and_consumingly_rechecked() {
    let production = production_prefix(CURRENT_RUNTIME);
    let observe_signature = between(
        production,
        "pub(super) fn observe_phase_a_place(",
        ") -> Result<PmPhaseAPlaceCurrentRuntimeWitness",
    );
    for required in [
        "&mut self",
        "config: &CanonicalTrialConfig",
        "authorization: &CanonicalAuthorization",
        "geoblock: PmProductionGeoblockObservation",
    ] {
        assert!(observe_signature.contains(required));
    }
    for forbidden in [
        "path:",
        "sha256:",
        "length:",
        "host_identity:",
        "runtime_user:",
        "egress_identity:",
        "blocked:",
        "now:",
        "clock:",
        "origin:",
    ] {
        assert!(
            !observe_signature.contains(forbidden),
            "current-runtime observer gained caller fact `{forbidden}`",
        );
    }
    for required in [
        "pub(super) struct PmCurrentRuntimeObserver",
        "pub(super) fn system() -> Self",
        "pub(super) fn observe_phase_a_place(",
        "geoblock: PmProductionGeoblockObservation",
        "pub(super) fn recheck_phase_a_place(",
        "witness: PmPhaseAPlaceCurrentRuntimeWitness",
        "pub(super) struct PmPhaseAPlaceCurrentRuntimeWitness",
        "pub(super) struct PmRevalidatedPhaseAPlaceCurrentRuntimeWitness",
        "pub(super) struct PmCurrentRuntimeFactsView<'a>",
        "evidence: &'a PmCurrentRuntimeEvidence",
        "PmCurrentRuntimeFactsView<'_>",
        "impl fmt::Debug for PmCurrentRuntimeObserver",
        "impl fmt::Debug for PmPhaseAPlaceCurrentRuntimeWitness",
        "impl fmt::Debug for PmRevalidatedPhaseAPlaceCurrentRuntimeWitness",
        "impl fmt::Debug for PmCurrentRuntimeFactsView<'_>",
        "<move-only; runtime-and-geoblock-bound>",
        "<move-only; non-authoritative; future-consuming-recheck-required>",
        "explicitly not mutation, dispatch, freshness, or transport authority",
        "cannot be decomposed into custody",
        "permit/transport-side consuming recheck immediately before dispatch",
        "point-in-time check does not close",
        "future permit/transport boundary must consume this",
        "does not extend the point-in-time check's freshness",
        "<borrowed; nonsecret; non-authority>",
        "PmPhaseAPlaceCurrentRuntimeWitness { mut evidence }",
        "effective_user_id: local.effective_user_id",
        "geoblock_source_monotonic_provenance_ns",
        "TrialPhase::APlaceCancel",
        "validate_age_window(",
        "checked_duration_since(first_monotonic)",
        "duration_since(first_wall)",
        "evidence.checked_wall_complete = local.wall_completed",
        "evidence.checked_monotonic_complete = local.monotonic_completed",
        "checked_wall_capture_unix_ns: unix_nanoseconds(local.wall_completed)?",
        "evidence.checked_wall_capture_unix_ns = unix_nanoseconds(local.wall_completed)?",
        "checked_add(cleanup_budget)",
        "required_cleanup_boundary > cleanup_wall",
        "const PROC_ROOT_PATH: &str = \"/proc\"",
        "const PROC_SELF_EXE_ENTRY: &str = \"self/exe\"",
        "rustix::fs::open(",
        "rustix::fs::openat(",
        "PROC_SELF_EXE_ENTRY",
        "rustix::fs::fstatfs(&proc)",
        "PROC_SUPER_MAGIC",
        "linux_uts_nodename",
        "linux_boot_identity",
        "rustix::system::uname()",
        "rustix::process::geteuid().as_raw()",
        "const GETENT_PATH: &str = \"/usr/bin/getent\"",
        "Command::new(GETENT_PATH)",
        ".env_clear()",
        ".stdout(Stdio::piped())",
        "stdout.take((MAX_GETENT_STDOUT_BYTES + 1) as u64)",
        "SystemTime::now()",
        "Instant::now()",
        "Sha256::new()",
    ] {
        assert!(
            production.contains(required),
            "current runtime is missing source pin `{required}`",
        );
    }
    for borrowed_fact in [
        "canonical_config_sha256(&self) -> &str",
        "canonical_config_length(&self) -> u64",
        "canonical_config_fingerprint(&self) -> &str",
        "trial_plan_fingerprint(&self) -> &str",
        "authorization_id(&self) -> &str",
        "authorization_fingerprint(&self) -> &str",
        "release_binary_sha256(&self) -> &str",
        "release_binary_length(&self) -> u64",
        "host_identity(&self) -> &str",
        "boot_identity(&self) -> &str",
        "runtime_user(&self) -> &str",
        "linux_effective_user_id(&self) -> u32",
        "borrowed V2 evidence fact and is never authority or a V1 schema field",
        "authorized_egress_identity(&self) -> IpAddr",
        "geoblock_reported_ip(&self) -> IpAddr",
        "geoblock_blocked(&self) -> bool",
        "geoblock_country(&self) -> &str",
        "geoblock_region(&self) -> &str",
        "geoblock_commitment(&self) -> PmGeoblockObservationCommitment",
        "geoblock_wall_receive_ns(&self) -> u64",
        "geoblock_source_monotonic_provenance_ns(&self) -> u64",
        "checked_at_utc(&self) -> &str",
        "checked_wall_capture_unix_ns(&self) -> u64",
        "borrowed V2 evidence fact is not external UTC or freshness proof",
        "maximum_age_ms(&self) -> u64",
        "cleanup_not_after_utc(&self) -> &str",
    ] {
        assert!(
            production.contains(borrowed_fact),
            "current-runtime facts view lost `{borrowed_fact}`",
        );
    }
    for (name, declaration) in [
        (
            "PmCurrentRuntimeObserver",
            "pub(super) struct PmCurrentRuntimeObserver",
        ),
        (
            "PmPhaseAPlaceCurrentRuntimeWitness",
            "pub(super) struct PmPhaseAPlaceCurrentRuntimeWitness",
        ),
        (
            "PmRevalidatedPhaseAPlaceCurrentRuntimeWitness",
            "pub(super) struct PmRevalidatedPhaseAPlaceCurrentRuntimeWitness",
        ),
        (
            "PmCurrentRuntimeFactsView",
            "pub(super) struct PmCurrentRuntimeFactsView<'a>",
        ),
    ] {
        let attributes = production
            .split_once(declaration)
            .expect("current-runtime type declaration")
            .0
            .rsplit("\n\n")
            .next()
            .expect("current-runtime type attributes");
        for forbidden in ["Clone", "Copy", "Serialize", "Deserialize"] {
            assert!(
                !attributes.contains(forbidden),
                "current runtime `{name}` gained forbidden `{forbidden}` derive",
            );
            assert!(
                !production.contains(&format!("{forbidden} for {name}")),
                "current runtime `{name}` gained forbidden `{forbidden}` impl",
            );
        }
    }
    for forbidden in [
        "std::env::",
        "current_exe(",
        "Command::new(\"getent\")",
        "Command::new(\"uname\")",
        "getlogin",
        "unsafe",
        "reqwest",
        "POST /order",
        "DELETE /order",
        "same_egress_as_clob",
        "production_order_entry_authorized: true",
        "real_order_submission_authorized: true",
        "pub(super) fn from_",
        "pub(super) fn into_",
        "PmGeoblockObservation {",
        "PmHttpReceiveClock {",
        "DispatchAuthorized",
        "SignedClobV2Order",
    ] {
        assert!(
            !production.contains(forbidden),
            "current runtime gained forbidden escape `{forbidden}`",
        );
    }
    assert!(MANIFEST.contains("chrono.workspace = true"));
    assert!(MANIFEST.contains("reap-pm-controlled-trial.workspace = true"));
    assert!(MANIFEST.contains(
        "rustix = { version = \"=1.1.4\", features = [\"fs\", \"net\", \"process\", \"system\", \"thread\"] }"
    ));
    assert!(MANIFEST.contains("sha2.workspace = true"));
    assert_eq!(
        production.matches("Command::new(").count(),
        1,
        "current runtime must use only the one fixed NSS helper",
    );
    assert_eq!(
        production
            .matches("Ok(PmPhaseAPlaceCurrentRuntimeWitness {")
            .count(),
        1,
        "initial runtime witness must have one source-owned construction",
    );
    assert_eq!(
        production
            .matches("Ok(PmRevalidatedPhaseAPlaceCurrentRuntimeWitness { evidence })")
            .count(),
        1,
        "final runtime witness must have one consuming construction",
    );
    assert!(
        production
            .matches(
                "validate_place_cancel_phase(config.value().phase, authorization.value().phase)?;"
            )
            .count()
            >= 2,
        "initial observation and consuming recheck must both reject non-Phase-A inputs",
    );

    let final_witness_impl = between(
        production,
        "impl PmRevalidatedPhaseAPlaceCurrentRuntimeWitness {",
        "impl fmt::Debug for PmRevalidatedPhaseAPlaceCurrentRuntimeWitness",
    );
    assert!(final_witness_impl.contains("pub(super) const fn facts(&self)"));
    for forbidden in [
        "fn into_",
        "fn authorize",
        "fn permit",
        "fn dispatch",
        "fn transport",
        "fn evidence",
        "fn inner",
    ] {
        assert!(
            !final_witness_impl.contains(forbidden),
            "non-authoritative runtime witness gained decomposition/authority `{forbidden}`",
        );
    }
}

#[test]
fn online_runtime_window_and_consumption_are_source_owned_inseparable_and_closed() {
    let production = production_prefix(CURRENT_RUNTIME);
    for required in [
        "pub(super) struct PmPhaseAOnlinePreflightWindow",
        "pub(super) struct PmFinishedPhaseAOnlinePreflightWindow",
        "pub(super) struct PmSelectedEgressGeoblockObservation<T>",
        "owned_custody: T",
        "observation: PmProductionGeoblockObservation",
        "thread_confinement: Rc<()>",
        "There is deliberately no production constructor in this slice",
        "stable selected actor/client generation",
        "initial observation, Prepared transition, Basis transition, A3",
        "actor-private generation and client custody through every",
        "this type must gain no constructor",
        "pub(super) struct PmPhaseAOnlineCurrentRuntimeWitness",
        "struct PmRevalidatedPhaseAOnlineCurrentRuntimeWitness",
        "pub(super) struct PmPhaseAOnlinePreparedConsumptionPair",
        "pub(super) struct PmPendingPhaseAOnlineRuntimeBurnV2",
        "pub(super) struct PmPhaseAOnlineRuntimeBurnedV2",
        "pub(super) fn begin_phase_a_online_preflight(",
        "pub(super) fn finish_phase_a_online_preflight(",
        "pub(super) fn observe_phase_a_online_current_runtime(",
        "pub(super) fn prepare_phase_a_online_consumptions(",
        "pub(super) fn create_phase_a_online_preflight_basis(",
        "pub(super) fn burn_phase_a_online_preflight_a3(",
        "Duration::from_millis(maximum_age_ms)",
        ".min(policy_value.maximum_observation_age_ms)",
        "validate_candidate_inside_outer_window(",
        "source_minimum < outer_started_ns",
        "source_maximum > outer_completed_ns",
        "market_end <= cleanup",
        "full_online_preflight_manifest_identity(",
        "complete-online-preflight-manifest.v2",
        "exact_consumption_runtime_bindings(",
        "prepare_online_authorization_consumption_v2(",
        "prepare_authorization_consumption(",
        "create_phase_a_online_preflight_basis_v2(",
        ".burn_and_record_a3(",
        "abandon_to_definitely_not_dispatched",
        "final-recheck-required",
        "actively re-observe the live book/user handles",
        "snapshot equality and cannot detect subsequent live source changes",
    ] {
        assert!(
            production.contains(required),
            "missing online runtime pin `{required}`"
        );
    }
    let v2_prepare = production
        .find("let mut v2_consumption = prepare_online_authorization_consumption_v2(")
        .expect("V2 Prepared transition");
    let v1_prepare = production
        .find("prepare_authorization_consumption(config, v1_authorization, &v1_runtime)")
        .expect("V1 Prepared transition");
    assert!(v2_prepare < v1_prepare, "V2 must become Prepared before V1");
    for forbidden in [
        "Ok(PmSelectedEgressGeoblockObservation",
        "fn selected_egress_geoblock",
        "fn runtime_binding(&self)",
        "fn online_runtime_binding(&self)",
        "pub(super) fn runtime_bindings",
        "pub(super) fn into_dispatch",
        "pub(super) fn into_parts",
        "revalidate_phase_a_online_preflight_v2_for_network_dispatch",
        "AuthenticatedPlaceRequest",
        "RuntimeExactBodyCommitment",
    ] {
        assert!(
            !production.contains(forbidden),
            "online runtime gained forbidden open/escape surface `{forbidden}`",
        );
    }
    for owner in [
        "PmPhaseAOnlinePreflightWindow",
        "PmFinishedPhaseAOnlinePreflightWindow",
        "PmSelectedEgressGeoblockObservation",
        "PmPhaseAOnlineCurrentRuntimeWitness",
        "PmPhaseAOnlinePreparedConsumptionPair",
        "PmPhaseAOnlineRuntimeBurnedV2",
    ] {
        for forbidden in ["Clone", "Serialize", "Deserialize"] {
            assert!(!production.contains(&format!("impl {forbidden} for {owner}")));
        }
    }
}

#[test]
fn linux_egress_local_facts_are_fd_held_source_facts_without_a_stale_positive_bearer() {
    let production = production_prefix(LINUX_EGRESS_LOCAL_FACTS);
    let capture_signature = between(
        production,
        "pub(super) fn capture(",
        ") -> Result<Self, PmLinuxEgressLocalFactError>",
    );
    for required in [
        "authorization: &CanonicalOnlineAuthorizationV2",
        "reviewed_nonsecret_profile_path: &Path",
    ] {
        assert!(capture_signature.contains(required));
    }
    for forbidden in [
        "network_namespace_device:",
        "network_namespace_inode:",
        "interface_name:",
        "interface_index:",
        "local_source_ip:",
        "profile_sha256:",
        "process_id:",
        "thread_id:",
        "now:",
        "clock:",
    ] {
        assert!(
            !capture_signature.contains(forbidden),
            "local-egress capture gained caller fact `{forbidden}`",
        );
    }

    for required in [
        "pub(super) struct PmLinuxEgressLocalFactCustody",
        "held_network_namespace: File",
        "network_namespace_device: u64",
        "network_namespace_inode: u64",
        "interface_name: Box<str>",
        "interface_index: u32",
        "local_source_ip: IpAddr",
        "reviewed_profile: HeldReviewedEgressProfile",
        "online_authorization_fingerprint: Box<str>",
        "source_thread_id: u32",
        "effective_user_id: u32",
        "wall_started: SystemTime",
        "wall_completed: SystemTime",
        "monotonic_started: Instant",
        "monotonic_completed: Instant",
        "thread_confinement: Rc<()>",
        "thread_confinement: Rc::new(())",
        "path_text != reviewed_reference",
        "!path.is_absolute()",
        "components.next() != Some(Component::RootDir)",
        "let Component::Normal(segment) = component else",
        "if normalized != path_text",
        "const PROC_THREAD_NET_NAMESPACE_ENTRY: &str = \"thread-self/ns/net\"",
        "const NSFS_MAGIC: rustix::fs::FsWord = 0x6e73_6673",
        "rustix::fs::fstatfs(&descriptor)",
        "filesystem.f_type != NSFS_MAGIC",
        "rustix::thread::gettid().as_raw_pid()",
        "let effective_user_id = rustix::process::geteuid().as_raw()",
        "authorization.value().host.linux_euid",
        "let final_effective_user_id = rustix::process::geteuid().as_raw()",
        "final_effective_user_id != effective_user_id",
        "rustix::net::netdevice::name_to_index(&ioctl, expected_name)",
        "rustix::net::netdevice::index_to_name(&ioctl, expected_index)",
        "getifaddrs()",
        "InterfaceFlags::IFF_UP",
        "InterfaceFlags::IFF_LOOPBACK",
        "as_sockaddr_in()",
        "as_sockaddr_in6()",
        "ResolveFlags::NO_SYMLINKS | ResolveFlags::NO_MAGICLINKS",
        "regular_file: metadata.file_type().is_file()",
        "if !identity.regular_file",
        "identity.mode != 0o600",
        "identity.link_count != 1",
        "lower_hex(&sha256) != expected_sha256",
        "online_authorization_fingerprint: authorization.fingerprint().into()",
        "<move-only; held-local-facts; non-authoritative>",
        "observes no",
        "route, destination, DNS answer, connected socket, geoblock response, NAT",
        "cannot construct an online runtime binding",
        "must never become a stale positive bearer on its own",
        "current-runtime gate can consume this",
        "capture on its own dedicated OS thread",
        "structurally neither `Send` nor `Sync`",
        "repeats process,",
        "profile-identity, and profile-",
        "same-local-egress",
        "pub(super) fn validate_captured_window(",
        "pub(super) fn revalidate_for_current_runtime(",
        "self.reviewed_profile.revalidate(",
        "hash_profile_descriptor(&mut self.file)?",
        "pub(super) struct PmLinuxEgressLocalFactsView<'a>",
        "wall_completed: SystemTime",
        "monotonic_completed: Instant",
        "effective_user_id: final_sample.effective_user_id",
        "network_namespace_device: final_sample.current_namespace.device",
        "network_namespace_inode: final_sample.current_namespace.inode",
        "interface_name: final_sample.interface.name",
        "interface_index: final_sample.interface.index",
        "local_source_ip: final_sample.interface.local_source_ip",
    ] {
        assert!(
            production.contains(required),
            "Linux local-egress source lost `{required}`",
        );
    }

    let final_interface = production
        .find("let final_interface = observe_exact_assigned_interface(")
        .expect("final exact-interface observation");
    let final_namespace = production
        .find("let final_namespace = observe_thread_network_namespace_identity()?")
        .expect("post-interface namespace observation");
    let final_thread = production
        .find("current_thread_id()? != source_thread_id")
        .expect("post-interface thread observation");
    let final_effective_user = production
        .find("let final_effective_user_id = rustix::process::geteuid().as_raw()")
        .expect("post-interface effective-user observation");
    assert!(
        final_interface < final_namespace
            && final_namespace < final_thread
            && final_thread < final_effective_user,
        "final interface read must be followed by namespace, thread, and EUID checks",
    );

    let custody_impl = between(
        production,
        "impl PmLinuxEgressLocalFactCustody {",
        "impl fmt::Debug for PmLinuxEgressLocalFactCustody",
    );
    let consuming_recheck = between(
        custody_impl,
        "pub(super) fn revalidate_for_current_runtime(",
        "    fn validate_authorization_binding(",
    );
    let profile_rehash = consuming_recheck
        .find("self.reviewed_profile.revalidate(")
        .expect("consuming profile rehash");
    let post_rehash_interface = consuming_recheck
        .rfind("let final_interface = observe_exact_assigned_interface(")
        .expect("post-rehash exact interface observation");
    let post_rehash_namespace = consuming_recheck
        .find("let final_namespace = observe_thread_network_namespace_identity()?")
        .expect("post-rehash current namespace observation");
    let post_rehash_held_namespace = consuming_recheck
        .find("let final_held_namespace = namespace_file_identity(&self.held_network_namespace)?")
        .expect("post-rehash held namespace observation");
    let post_rehash_process = consuming_recheck
        .find("let final_process_id = std::process::id()")
        .expect("post-rehash PID observation");
    let post_rehash_thread = consuming_recheck
        .find("let final_thread_id = current_thread_id()?")
        .expect("post-rehash TID observation");
    let post_rehash_euid = consuming_recheck
        .find("let final_effective_user_id = rustix::process::geteuid().as_raw()")
        .expect("post-rehash EUID observation");
    let final_validation = consuming_recheck
        .find("let final_sample = validate_final_local_source_sample(")
        .expect("post-rehash exact sample validation");
    let final_euid_binding = consuming_recheck
        .find("validate_effective_user_binding(\n                final_sample.effective_user_id,")
        .expect("post-rehash exact authorization EUID validation");
    let completion_wall = consuming_recheck
        .find("let wall_completed = SystemTime::now()")
        .expect("consuming completion wall edge");
    let completion_monotonic = consuming_recheck
        .find("let monotonic_completed = Instant::now()")
        .expect("consuming completion monotonic edge");
    assert!(
        profile_rehash < post_rehash_interface
            && post_rehash_interface < post_rehash_namespace
            && post_rehash_namespace < post_rehash_held_namespace
            && post_rehash_held_namespace < post_rehash_process
            && post_rehash_process < post_rehash_thread
            && post_rehash_thread < post_rehash_euid
            && post_rehash_euid < final_validation
            && final_validation < final_euid_binding
            && final_euid_binding < completion_wall
            && completion_wall < completion_monotonic,
        "consuming recheck must close every mutable source after profile rehash and before clocks",
    );
    assert!(
        production
            .matches("authorization.value().host.linux_euid")
            .count()
            >= 2,
        "capture and consuming rechecks must bind EUID to authorization",
    );

    let attributes = production
        .split_once("pub(super) struct PmLinuxEgressLocalFactCustody")
        .expect("local-egress custody declaration")
        .0
        .rsplit("\n\n")
        .next()
        .expect("local-egress custody attributes");
    for forbidden in ["Clone", "Copy", "Serialize", "Deserialize"] {
        assert!(!attributes.contains(forbidden));
        assert!(!production.contains(&format!("{forbidden} for PmLinuxEgressLocalFactCustody")));
    }

    let custody_fields = between(
        production,
        "pub(super) struct PmLinuxEgressLocalFactCustody {\n",
        "\n}\n\nimpl PmLinuxEgressLocalFactCustody",
    );
    for field in custody_fields
        .lines()
        .filter(|line| !line.trim().is_empty())
    {
        assert!(
            !field.trim_start().starts_with("pub"),
            "local-egress custody field visibility escaped: `{field}`",
        );
    }

    assert_eq!(
        custody_impl.matches("pub(super) fn ").count(),
        3,
        "custody must expose only capture, window validation, and consuming recheck",
    );
    assert_eq!(
        custody_impl.matches("fn ").count(),
        4,
        "custody must add only its private authorization-binding helper",
    );
    assert_eq!(
        custody_impl.matches("Self {").count(),
        1,
        "only capture may construct local-egress custody",
    );
    for forbidden in [
        "fn facts",
        "fn into_",
        "fn consume",
        "fn recheck",
        "fn refresh",
        "fn authorize",
        "fn permit",
        "fn dispatch",
        "fn transport",
        "fn route",
        "fn socket",
    ] {
        assert!(
            !custody_impl.contains(forbidden),
            "local-egress custody gained stale or authoritative method `{forbidden}`",
        );
    }
    for forbidden in [
        "pub(crate) fn ",
        "\npub fn ",
        "-> PmLinuxEgressLocalFactCustody",
        "Result<PmLinuxEgressLocalFactCustody",
        "impl From<",
        "impl TryFrom<",
        "impl Into<",
        "impl TryInto<",
        "impl AsRef<",
        "impl Deref",
        "impl std::ops::Deref",
        "impl Borrow<",
        "impl std::borrow::Borrow",
        "impl std::convert::",
        "unsafe impl",
        "impl Send for PmLinuxEgressLocalFactCustody",
        "impl Sync for PmLinuxEgressLocalFactCustody",
    ] {
        assert!(
            !production.contains(forbidden),
            "local-egress custody gained constructor, conversion, or confinement escape `{forbidden}`",
        );
    }
    let custody_impl_headers: Vec<_> = production
        .lines()
        .map(str::trim)
        .filter(|line| line.starts_with("impl ") && line.contains("PmLinuxEgressLocalFactCustody"))
        .collect();
    assert_eq!(
        custody_impl_headers,
        [
            "impl PmLinuxEgressLocalFactCustody {",
            "impl fmt::Debug for PmLinuxEgressLocalFactCustody {",
        ],
        "local-egress custody gained an unreviewed trait or inherent impl",
    );
    assert_eq!(
        production
            .matches("PmLinuxEgressLocalFactCustody {")
            .count(),
        3,
        "local-egress custody gained a free construction site",
    );
    for forbidden in [
        "reqwest",
        "tokio_tungstenite",
        "getaddrinfo",
        "lookup_host",
        "OnlineAuthorizationRuntimeBindingV2",
        "prepare_online_authorization_consumption_v2",
        "PmProductionGeoblockObservation",
        "AuthenticatedPlaceRequest",
        "PmRetainedPlaceRequest",
        ".connect(",
        ".bind(",
        ".send(",
        ".recv(",
        "POST /order",
        "DELETE /order",
    ] {
        assert!(
            !production.contains(forbidden),
            "local-egress facts gained forbidden network/authority surface `{forbidden}`",
        );
    }

    assert!(MANIFEST.contains(
        "nix = { version = \"=0.30.1\", default-features = false, features = [\"net\", \"socket\"] }"
    ));
    assert!(MANIFEST.contains(
        "rustix = { version = \"=1.1.4\", features = [\"fs\", \"net\", \"process\", \"system\", \"thread\"] }"
    ));
}

#[test]
fn linux_egress_local_facts_have_no_network_test_matrix() {
    for required in [
        "reviewed_profile_path_must_be_exact_absolute_utf8",
        "numeric_effective_user_must_match_authorization",
        "consuming_recheck_rejects_post_rehash_interface_or_ip_drift",
        "validate_final_local_source_sample(expected(), drifted)",
        "current_thread_namespace_source_is_descriptor_pinned_and_stable",
        "profile_source_holds_exact_nonsecret_bytes_and_rejects_wrong_hash",
        "capture_clock_window_rejects_regression_and_expiry",
        "debug_output_is_redacted_and_non_authoritative",
        "for noncanonical in [",
        "\"/run/reap/./reviewed-egress.json\"",
        "\"/run/reap/../reap/reviewed-egress.json\"",
        "\"/run//reap/reviewed-egress.json\"",
        "\"/run/reap/reviewed-egress.json/\"",
        "validate_exact_reviewed_profile_path(Path::new(noncanonical), noncanonical,)",
    ] {
        assert!(
            LINUX_EGRESS_LOCAL_FACTS.contains(required),
            "Linux local-egress test matrix lost `{required}`",
        );
    }
}

#[test]
fn current_runtime_has_deterministic_drift_and_boundary_tests() {
    for required in [
        "getent_passwd_parser_requires_one_exact_uid_record",
        "local_sources_capture_only_the_current_linux_runtime",
        "executable_recheck_rejects_identity_hash_and_length_drift",
        "local_runtime_recheck_rejects_each_identity_drift",
        "clock_window_rejects_regression_and_expires_after_exact_boundary",
        "geoblock_values_reject_blocked_future_stale_and_wrong_egress",
        "config_authorization_pins_reject_each_swap",
        "cleanup_runway_accepts_exact_boundary_and_rejects_shortfall",
        "current_runtime_rejects_every_non_place_cancel_phase_pair",
        "debug_is_redacted",
    ] {
        assert!(
            CURRENT_RUNTIME.contains(required),
            "current-runtime test matrix lost `{required}`",
        );
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
fn phase_a_market_projection_cannot_be_detached_from_move_only_source_evidence() {
    let production = production_prefix(PUBLIC_BOOK);
    let projection = production
        .split_once("pub(super) struct PmPhaseAMarketProjection")
        .expect("Phase-A market projection")
        .0
        .rsplit_once("#[derive(")
        .expect("projection derive")
        .1
        .split_once(")]")
        .expect("projection derive terminator")
        .0;
    assert!(!projection.contains("Clone"));
    assert!(!projection.contains("Copy"));
    assert!(!projection.contains("Serialize"));
    assert!(!projection.contains("Deserialize"));
    assert!(!production.contains("impl Clone for PmPhaseAMarketProjection"));
    assert!(!production.contains("impl Copy for PmPhaseAMarketProjection"));
    assert!(!production.contains("Serialize for PmPhaseAMarketProjection"));
    assert!(!production.contains("Deserialize for PmPhaseAMarketProjection"));
    assert!(production.contains("fn duplicate_for_seal(&self) -> Self"));
    assert!(!production.contains("pub fn duplicate_for_seal"));
    assert!(!production.contains("pub(super) fn duplicate_for_seal"));
    assert!(!production.contains("pub(crate) fn duplicate_for_seal"));
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
fn user_online_preflight_lease_is_move_only_source_owned_and_borrow_inspectable() {
    let production = production_prefix(USER_STREAM);
    let declaration = between(
        production,
        "pub(super) struct PmUserOnlinePreflightLease {",
        "impl PmUserOnlinePreflightLease {",
    );
    assert!(declaration.contains("ticket: FinalCutTicket,"));
    assert!(
        declaration
            .lines()
            .all(|line| !line.trim_start().starts_with("pub")),
        "online lease field became visible to sibling composition",
    );
    let declaration_prefix = production
        .split_once("pub(super) struct PmUserOnlinePreflightLease {")
        .expect("online lease declaration")
        .0
        .rsplit("\n\n")
        .next()
        .expect("online lease attributes");
    for forbidden in ["Clone", "Copy", "Serialize", "Deserialize"] {
        assert!(
            !declaration_prefix.contains(forbidden),
            "online lease gained forbidden `{forbidden}` derive",
        );
        assert!(
            !production.contains(&format!("impl {forbidden} for PmUserOnlinePreflightLease")),
            "online lease gained forbidden `{forbidden}` implementation",
        );
    }

    let lease_impl = between(
        production,
        "impl PmUserOnlinePreflightLease {",
        "impl fmt::Debug for PmUserOnlinePreflightLease",
    );
    for required in [
        "pub(super) const fn scope(&self) -> PmWireScope",
        "pub(super) const fn proxy_maker(&self) -> EvmAddress",
        "pub(super) const fn stream_revision(&self) -> u64",
        "pub(super) const fn initial_connection_epoch(&self) -> ConnectionEpoch",
        "pub(super) const fn current_connection_epoch(&self) -> ConnectionEpoch",
        "pub(super) const fn connection_open_generation(&self) -> u64",
        "pub(super) const fn connection_open_clock(&self) -> PmUserWsEdgeClock",
        "pub(super) const fn subscription_generation(&self) -> u64",
        "pub(super) const fn subscription_clock(&self) -> PmUserWsEdgeClock",
        "pub(super) const fn ping_generation(&self) -> u64",
        "pub(super) const fn ping_clock(&self) -> PmUserWsEdgeClock",
        "pub(super) const fn correlated_pong_generation(&self) -> u64",
        "pub(super) const fn correlated_pong_clock(&self) -> PmUserWsEdgeClock",
        "pub(super) const fn admitted_activity_generation(&self) -> u64",
        "pub(super) const fn reconnect_count(&self) -> u8",
        "pub(super) fn reconnect_history(&self) -> &[PmUserStreamReconnectEvidence]",
        "pub(super) const fn business_basis(&self) -> &FinalCutBusinessBasis",
        "pub(super) fn business_events(&self) -> &[PmUserStreamBusinessEventProjection]",
        ") -> &[PmUserStreamBusinessEventProjection]",
        "pub(super) fn matches_fresh_rest_cut(&self, cut: &PmFreshAuthenticatedRestCut) -> bool",
        "pub(super) const fn open_order_rows(&self) -> usize",
        "pub(super) const fn trade_rows(&self) -> usize",
    ] {
        assert!(
            lease_impl.contains(required),
            "online lease is missing borrowed/typed view `{required}`",
        );
    }
    for forbidden in [
        "fn new(",
        "fn from_",
        "into_ticket",
        "into_parts",
        "into_rest_cut_identity",
        "into_business_events",
        "fn rest_cut_identity(",
        "PmRestCutIdentity",
        "pub(super) fn marker(",
        "pub(super) fn core(",
        "PmLiveUserEvent",
        "CredentialOwnedUserFrame",
        "PmRecoveryAccountAuthenticatedRestCut",
        "PmRecoveryExactAuthenticatedRestCut",
        "Arc::clone",
    ] {
        assert!(
            !lease_impl.contains(forbidden),
            "online lease gained forbidden constructor/escape `{forbidden}`",
        );
    }

    assert_eq!(
        production
            .matches("PmUserOnlinePreflightLease { ticket: self }")
            .count(),
        1,
        "online lease must have one ticket-consuming construction",
    );
    assert!(
        production.contains(
            "pub(super) fn into_online_preflight_lease(self) -> PmUserOnlinePreflightLease"
        )
    );
    assert!(
        production
            .contains("self.consume_online_preflight_lease(ticket.into_online_preflight_lease())")
    );
    assert!(production.contains(
        "lease: PmUserOnlinePreflightLease,\n    ) -> Result<PmUserOnlinePreflightLease"
    ));
    assert!(
        production
            .contains("lease: PmUserOnlinePreflightLease,\n    ) -> Result<FinalCutJoinFields")
    );
    assert!(production.contains("self.shared.recheck_online_preflight_core(&ticket.core)"));
    assert!(lease_impl.contains("cut.matches_rest_cut_identity(&self.ticket.rest_cut_identity)"));
    assert!(PRIVATE_READS.contains("Arc::ptr_eq(&self.rest_cut_identity, identity)"));
    assert!(production.contains("state.validate_final_core(core, before)?;"));
    assert!(production.contains("state.validate_final_core(core, after)?;"));
    assert!(production.contains("if self.activity.generation() != core.activity_generation"));

    let final_fields_impl = between(
        production,
        "impl FinalCutJoinFields {",
        "impl fmt::Debug for FinalCutJoinFields",
    );
    assert!(final_fields_impl.contains(
        "pub(super) fn matches_fresh_rest_cut(&self, cut: &PmFreshAuthenticatedRestCut) -> bool"
    ));
    assert!(final_fields_impl.contains("cut.matches_rest_cut_identity(&self.rest_cut_identity)"));
    for forbidden in [
        "fn rest_cut_identity(",
        "-> &Arc<PmRestCutIdentity>",
        "PmRecoveryAccountAuthenticatedRestCut",
        "PmRecoveryExactAuthenticatedRestCut",
    ] {
        assert!(
            !production.contains(forbidden),
            "user-stream production surface retained forbidden identity escape `{forbidden}`",
        );
    }
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
