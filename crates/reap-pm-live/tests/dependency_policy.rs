use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::Path;
use std::process::Command;

use serde_json::Value;

#[test]
fn pm_contracts_do_not_import_existing_okx_authority_or_raw_clients() {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("crate must be inside workspace");
    let output = Command::new(env!("CARGO"))
        .args(["metadata", "--locked", "--format-version=1"])
        .current_dir(workspace)
        .output()
        .expect("cargo metadata");
    assert!(
        output.status.success(),
        "cargo metadata failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let metadata: Value = serde_json::from_slice(&output.stdout).unwrap();
    let edges = workspace_edges(&metadata);

    let live_dependencies = edges.get("reap-pm-live").unwrap();
    assert!(live_dependencies.contains("reap-pm-core"));
    assert!(live_dependencies.contains("reap-transport"));
    assert!(live_dependencies.contains("reap-pm-state"));
    assert!(live_dependencies.contains("reap-capture-framing"));
    assert!(live_dependencies.contains("reap-okx-public-source"));
    assert!(live_dependencies.contains("base64"));
    assert!(!live_dependencies.contains("reap-core"));
    assert!(!live_dependencies.contains("reap-polymarket-wire"));

    let state_dependencies = edges.get("reap-pm-state").unwrap();
    assert_eq!(
        state_dependencies,
        &BTreeSet::from(["reap-pm-core".to_owned(), "thiserror".to_owned()]),
        "the pure PM reducer crate has exactly core types plus typed errors"
    );

    for source in [
        "reap-pm-strategy",
        "reap-pm-live-contracts",
        "reap-polymarket-adapter",
        "reap-pm-live",
        "reap-pm-state",
        "reap-polymarket-wire",
    ] {
        for forbidden in [
            "reap-order",
            "reap-live",
            "reap-live-contracts",
            "reap-venue",
            "reap-okx-wire",
            "reap-okx-live-adapter",
            "reap-okx-evidence-adapter",
            "reap-okx-emergency-adapter",
            "reqwest",
            "hmac",
        ] {
            assert!(
                !reachable(&edges, source, forbidden),
                "{source} reaches forbidden dependency {forbidden}"
            );
        }
    }

    for source in [
        "reap-pm-strategy",
        "reap-pm-live-contracts",
        "reap-polymarket-adapter",
        "reap-polymarket-wire",
        "reap-pm-state",
    ] {
        assert!(
            !reachable(&edges, source, "base64"),
            "{source} reaches capture-only base64 schema encoding"
        );
    }

    for legacy in [
        "reap-backtest",
        "reap-book",
        "reap-capture",
        "reap-cli",
        "reap-core",
        "reap-emergency-core",
        "reap-emergency-runner",
        "reap-engine",
        "reap-evidence-core",
        "reap-fault",
        "reap-feed",
        "reap-live",
        "reap-live-contracts",
        "reap-okx-emergency-adapter",
        "reap-okx-evidence-adapter",
        "reap-okx-live-adapter",
        "reap-okx-wire",
        "reap-order",
        "reap-risk",
        "reap-storage",
        "reap-strategy",
        "reap-telemetry",
        "reap-venue",
    ] {
        for pm in [
            "reap-pm-core",
            "reap-pm-strategy",
            "reap-pm-live-contracts",
            "reap-polymarket-adapter",
            "reap-pm-live",
        ] {
            assert!(
                !reachable(&edges, legacy, pm),
                "legacy crate {legacy} reaches PM crate {pm}"
            );
        }
    }
}

#[test]
fn pm_contract_sources_have_no_dynamic_or_authority_escape_hatch() {
    let crates = [
        "../reap-pm-strategy/src",
        "../reap-pm-live-contracts/src",
        "../reap-polymarket-adapter/src",
        "../reap-pm-state/src",
        "../reap-polymarket-wire/src",
        "src",
    ];
    for relative in crates {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join(relative);
        for path in rust_sources(&root) {
            let source = std::fs::read_to_string(&path).unwrap();
            for forbidden in [
                "Box<dyn",
                "Arc<Mutex",
                "Arc<RwLock",
                "unbounded_channel",
                "std::sync::mpsc",
                "tokio::sync::mpsc",
                "reqwest",
                "tungstenite",
                "Credentials",
                "PrivateKey",
                "ApiKey",
                "SignedRequest",
                "cancel_all",
                "arbitrary_command",
            ] {
                assert!(
                    !source.contains(forbidden),
                    "{} contains forbidden token {forbidden}",
                    path.display()
                );
            }
        }
    }
}

#[test]
fn pm_state_remains_a_pure_reducer_crate() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../reap-pm-state/src");
    for path in rust_sources(&root) {
        let source = std::fs::read_to_string(&path).unwrap();
        let source = production_extent(&source);
        for forbidden in [
            "async fn",
            ".await",
            "std::fs",
            "std::io",
            "std::net",
            "std::process",
            "std::thread",
            "tokio",
            "reqwest",
            "tungstenite",
            "serde",
            "base64",
            "reap_polymarket",
            "reap_transport",
            "reap_pm_live",
            "reap_pm_strategy",
        ] {
            assert!(
                !source.contains(forbidden),
                "{} contains non-reducer dependency token {forbidden}",
                path.display()
            );
        }
    }
}

#[test]
fn read_only_private_monitor_has_no_lane_model_mutation_or_wire_escape() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/private_monitor.rs");
    let source = std::fs::read_to_string(&path).unwrap();
    for forbidden in [
        "crate::lanes",
        "crate::capture",
        "crate::fake_effect",
        "crate::schedule",
        "PmQuoteModel",
        "journal",
        "reap_polymarket_wire",
        "tokio",
        "reqwest",
        "Authenticated",
        "PrivateKey",
        "ApiKey",
        "arbitrary_command",
    ] {
        assert!(
            !source.contains(forbidden),
            "{} contains forbidden monitor token {forbidden}",
            path.display()
        );
    }
}

#[test]
fn base64_is_confined_to_capture_schema_and_replay_modules() {
    let source_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let capture = source_root.join("capture.rs");
    let replay = source_root.join("replay.rs");
    for path in rust_sources(&source_root) {
        let source = std::fs::read_to_string(&path).unwrap();
        if source.contains("base64") {
            assert!(
                path == capture || path == replay,
                "{} uses capture-only base64 outside schema/replay",
                path.display()
            );
        }
    }
    for relative in [
        "../reap-pm-strategy/src",
        "../reap-pm-live-contracts/src",
        "../reap-polymarket-adapter/src",
        "../reap-polymarket-wire/src",
        "../reap-pm-state/src",
    ] {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join(relative);
        for path in rust_sources(&root) {
            assert!(
                !std::fs::read_to_string(&path).unwrap().contains("base64"),
                "{} uses capture-only base64",
                path.display()
            );
        }
    }
}

#[test]
fn legacy_polymarket_mentions_are_an_exact_fail_closed_allowlist() {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .unwrap();
    let mut actual = Vec::new();
    for relative in [
        "crates/reap-backtest",
        "crates/reap-book",
        "crates/reap-capture",
        "crates/reap-cli",
        "crates/reap-emergency-core",
        "crates/reap-emergency-runner",
        "crates/reap-engine",
        "crates/reap-evidence-core",
        "crates/reap-fault",
        "crates/reap-live",
        "crates/reap-live-contracts",
        "crates/reap-okx-emergency-adapter",
        "crates/reap-okx-evidence-adapter",
        "crates/reap-okx-live-adapter",
        "crates/reap-okx-wire",
        "crates/reap-order",
        "crates/reap-feed",
        "crates/reap-risk",
        "crates/reap-storage",
        "crates/reap-strategy",
        "crates/reap-telemetry",
        "crates/reap-venue",
    ] {
        let root = workspace.join(relative).join("src");
        for path in rust_sources(&root) {
            let source = std::fs::read_to_string(&path).unwrap();
            for line in source.lines() {
                let count = line.to_ascii_lowercase().matches("polymarket").count();
                for _ in 0..count {
                    actual.push(format!(
                        "{}|{}",
                        path.strip_prefix(workspace).unwrap().display(),
                        line.trim()
                    ));
                }
            }
        }
    }
    actual.sort();

    let mut expected = vec![
        "crates/reap-feed/src/connection.rs|ConnectionError::UnsupportedVenue(Venue::Polymarket)",
        "crates/reap-feed/src/connection.rs|Venue::Polymarket => return Err(ConnectionError::UnsupportedVenue(Venue::Polymarket)),",
        "crates/reap-feed/src/connection.rs|Venue::Polymarket => return Err(ConnectionError::UnsupportedVenue(Venue::Polymarket)),",
        "crates/reap-feed/src/connection.rs|Venue::Polymarket,",
        "crates/reap-feed/src/connection.rs|conn_id: ConnId::new(\"unsupported-polymarket\"),",
        "crates/reap-feed/src/connection.rs|fn old_connection_path_rejects_polymarket_explicitly() {",
        "crates/reap-feed/src/connection.rs|panic!(\"the old OKX feed path admitted a Polymarket socket plan\");",
        "crates/reap-feed/src/connection.rs|venue: Venue::Polymarket,",
        "crates/reap-feed/src/subscription.rs|Err(PartitionError::UnsupportedVenue(Venue::Polymarket))",
        "crates/reap-feed/src/subscription.rs|Err(PartitionError::UnsupportedVenue(Venue::Polymarket))",
        "crates/reap-feed/src/subscription.rs|Err(PartitionError::UnsupportedVenue(Venue::Polymarket))",
        "crates/reap-feed/src/subscription.rs|Venue::Polymarket => Err(PartitionError::UnsupportedVenue(Venue::Polymarket)),",
        "crates/reap-feed/src/subscription.rs|Venue::Polymarket => Err(PartitionError::UnsupportedVenue(Venue::Polymarket)),",
        "crates/reap-feed/src/subscription.rs|Venue::Polymarket => Err(PartitionError::UnsupportedVenue(Venue::Polymarket)),",
        "crates/reap-feed/src/subscription.rs|Venue::Polymarket => Err(PartitionError::UnsupportedVenue(Venue::Polymarket)),",
        "crates/reap-feed/src/subscription.rs|Venue::Polymarket,",
        "crates/reap-feed/src/subscription.rs|fn old_subscription_partitioning_rejects_polymarket_explicitly() {",
        "crates/reap-feed/src/subscription.rs|venue_key(Venue::Polymarket),",
        "crates/reap-feed/src/subscription.rs|venue_label(Venue::Polymarket),",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect::<Vec<_>>();
    expected.sort();
    assert_eq!(actual, expected);

    let mut core_actual = Vec::new();
    for path in rust_sources(&workspace.join("crates/reap-core/src")) {
        let source = std::fs::read_to_string(&path).unwrap();
        for line in source.lines() {
            let count = line.to_ascii_lowercase().matches("polymarket").count();
            for _ in 0..count {
                core_actual.push(format!(
                    "{}|{}",
                    path.strip_prefix(workspace).unwrap().display(),
                    line.trim()
                ));
            }
        }
    }
    core_actual.sort();
    let mut core_expected = [
        r#"crates/reap-core/src/types.rs|#[serde(rename = "polymarket")]"#,
        "crates/reap-core/src/types.rs|Polymarket,",
        "crates/reap-core/src/types.rs|Venue::Polymarket",
        r##"crates/reap-core/src/types.rs|r#""polymarket""#"##,
        r##"crates/reap-core/src/types.rs|serde_json::from_str::<Venue>(r#""polymarket""#).unwrap(),"##,
        "crates/reap-core/src/types.rs|serde_json::to_string(&Venue::Polymarket).unwrap(),",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect::<Vec<_>>();
    core_expected.sort();
    assert_eq!(
        core_actual, core_expected,
        "the foundational common venue identity is the only separately pinned legacy exception"
    );
}

#[test]
fn constructor_ownership_and_preallocated_lane_shape_are_pinned() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let composition = rust_module_source(&root, "composition");
    let capture_roles = std::fs::read_to_string(root.join("capture_roles.rs")).unwrap();
    let replay = std::fs::read_to_string(root.join("replay.rs")).unwrap();
    let fake_effect = std::fs::read_to_string(root.join("fake_effect.rs")).unwrap();
    let lane_entry = std::fs::read_to_string(root.join("lanes.rs")).unwrap();
    let lane_service = std::fs::read_to_string(root.join("lanes/service.rs")).unwrap();
    let lanes = rust_module_source(&root, "lanes");
    let bounded = std::fs::read_to_string(root.join("lanes/bounded.rs")).unwrap();

    assert!(!composition.contains("PmPublicRole::from_expected_metadata"));
    assert!(!composition.contains(".observation_grant()"));
    assert!(!composition.contains("PmFixtureOwnedExecution::new"));
    assert!(capture_roles.contains("PmPublicRole::from_expected_metadata"));
    assert!(capture_roles.contains("config.observation_grant()"));
    assert!(replay.contains("PmPublicRole::from_expected_metadata"));
    assert!(replay.contains("scope.observation_grant()?"));
    assert!(fake_effect.contains("PmFixtureOwnedExecution::new"));
    assert!(bounded.contains("BinaryHeap::with_capacity"));
    assert!(bounded.contains("HashSet::with_capacity"));
    assert!(!lanes.contains("Vec::new"));
    assert!(lane_entry.contains("mod service;"));
    assert!(!lane_entry.contains("pub trait PmLaneService"));
    assert!(lane_service.contains("pub trait PmPublicLaneService"));
    assert!(lane_service.contains("impl PmPublicLaneState"));
    assert!(!lanes.contains("pub struct PmLaneSet"));
    assert!(!lane_service.contains("pub trait PmLaneService"));
}

#[test]
fn pm_mutation_authority_remains_internal_and_composition_confined() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let library = std::fs::read_to_string(root.join("lib.rs")).unwrap();
    let product = rust_module_source(&root, "composition/product");
    let authority = std::fs::read_to_string(root.join("coordinator/authority.rs")).unwrap();

    for hidden in [
        "PmMutationOwner",
        "PmQuoteMutationRequest",
        "PmCancelMutationRequest",
        "PmFakeEffectRole",
        "PmFixtureOwnedExecution",
        "PmFakePlaceCommand",
        "PmFakeCancelCommand",
    ] {
        assert!(
            !library.contains(hidden),
            "crate root exports internal PM mutation authority {hidden}"
        );
    }

    for forbidden in [
        "AuthenticatedClient",
        "AuthenticatedHttpSession",
        "AuthenticatedWsSession",
        "LiveSigner",
        "RequestExecutor",
        "arbitrary_request",
        "pub signer:",
        "pub authenticated_client:",
        "pub authenticated_http_session:",
        "pub authenticated_ws_session:",
        "pub request_executor:",
        "pub mutation:",
        "pub fake_effect:",
        "pub fn signer(",
        "pub fn authenticated_http_session(",
        "pub fn authenticated_ws_session(",
        "pub fn request_executor(",
        "pub fn mutation_owner(",
        "pub fn quote_mutation_request(",
        "pub fn cancel_mutation_request(",
    ] {
        assert!(
            !product.contains(forbidden),
            "PmProduct exposes forbidden live authority token {forbidden}"
        );
    }

    for gate in [
        "approve_pm_quote",
        "prepare_pm_quote",
        "consume_prepared_quote",
        "approve_pm_cancel",
        "prepare_pm_cancel",
        "consume_prepared_cancel",
    ] {
        assert!(
            authority.contains(&format!("pub(crate) fn {gate}")),
            "PM mutation gate {gate} is no longer crate-confined"
        );
        assert!(
            !authority.contains(&format!("pub fn {gate}")),
            "PM mutation gate {gate} became publicly constructible"
        );
    }
}

#[test]
fn integrated_product_run_embeds_the_sole_public_capture_owner() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let product = rust_module_source(&root, "composition/product");
    let complete = std::fs::read_to_string(root.join("lanes/complete.rs")).unwrap();
    let coordinator_service =
        std::fs::read_to_string(root.join("coordinator/product/service.rs")).unwrap();

    assert!(product.contains("pub async fn start("));
    assert!(product.contains("PmCoordinator::start("));
    assert!(product.contains("pub struct PmProductRun"));
    assert!(
        !product.contains("key: PmScheduledActionKey"),
        "the public product Run must not accept caller-stamped account/instrument schedule scope"
    );
    assert!(product.contains("side: PmOrderSide"));
    assert!(product.contains("kind: PmScheduledActionKind"));
    assert!(
        coordinator_service
            .contains("PmScheduledActionKey::new(self.account_scope, self.instrument, side, kind)")
    );
    assert!(complete.contains("Capture(Box<PmPublicCaptureRun>)"));
    assert!(complete.contains("run.service_lane_turn(monotonic_now_ns, consumer)"));
    assert!(complete.contains("Bare(PmPublicLaneState)"));
    assert!(
        !complete.contains("pub(crate) fn new(public: PmPublicLaneState"),
        "production complete scheduling must not accept a sibling bare public lane"
    );
}

#[test]
fn public_lane_requires_capability_specific_route_deliveries() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let routes = std::fs::read_to_string(root.join("public_routes.rs")).unwrap();
    let lanes = rust_module_source(&root, "lanes");
    let composition = rust_module_source(&root, "composition");
    let capture_roles = std::fs::read_to_string(root.join("capture_roles.rs")).unwrap();
    let adapter_session =
        std::fs::read_to_string(root.join("../../reap-polymarket-adapter/src/public_session.rs"))
            .unwrap();

    for delivery in [
        "PmPublicMetadataDelivery",
        "PmPublicBookDelivery",
        "OkxPublicReferenceDelivery",
        "PmPublicUnavailableDelivery",
        "OkxPublicUnavailableDelivery",
    ] {
        assert!(routes.contains(&format!("opaque_delivery!({delivery},")));
    }
    assert!(!routes.contains("pub fn into_envelope"));
    assert!(routes.contains("pub(crate) struct PmPublicRoutes"));
    assert!(!routes.contains("#[derive(Debug, Clone, PartialEq, Eq)]"));
    assert!(
        !std::fs::read_to_string(root.join("lib.rs"))
            .unwrap()
            .contains("PmPublicRouteError, PmPublicRoutes")
    );
    assert!(!routes.contains("ConfiguredPublicRouteProof"));
    assert!(!routes.contains("PublicRouteProof<"));
    assert!(routes.contains("session.issue_metadata_occurrence(local_wall_receive_ns)?"));
    assert!(adapter_session.contains("pub struct PmPublicMetadataOccurrence"));
    assert!(adapter_session.contains("pub fn issue_metadata_occurrence"));
    assert!(
        !adapter_session
            .contains("#[derive(Debug, Clone, PartialEq, Eq)]\npub struct PmPublicBookDelivery")
    );
    assert!(!capture_roles.contains("pm_and_routes"));
    assert!(!capture_roles.contains("okx_and_routes"));
    assert!(capture_roles.contains("classify_and_route_pm"));
    assert!(capture_roles.contains("classify_and_route_okx"));

    for admission in [
        "enqueue_pm_metadata",
        "enqueue_okx_reference",
        "enqueue_pm_unavailable",
        "enqueue_okx_unavailable",
    ] {
        assert!(lanes.contains(&format!("pub(crate) fn {admission}")));
        assert!(composition.contains(&format!("pub(crate) fn {admission}")));
        assert!(!composition.contains(&format!("pub fn {admission}")));
    }
    assert!(lanes.contains("pub(crate) fn enqueue_pm_book"));
    assert!(!composition.contains("pub fn enqueue_pm_book"));
    assert!(composition.contains("pub fn issue_and_enqueue_pm_metadata"));
    assert!(composition.contains("pub async fn capture_okx_public"));
    assert!(composition.contains("pub fn reduce_then_enqueue_pm_book"));
    assert!(composition.contains("pub fn commit_then_enqueue_pm_snapshot"));
    assert!(composition.contains("fn issue_pm_metadata"));
    assert!(!composition.contains("pub fn issue_pm_metadata"));
    assert!(composition.contains("async fn capture_okx_public_routed"));
    assert!(!composition.contains("pub async fn capture_okx_public_routed"));
    assert!(!composition.contains("pub fn commit_pm_snapshot"));
    assert!(!composition.contains("pub fn commit_pm_book_update"));
    assert!(lanes.contains("enum PmPublicInput"));
    assert!(!lanes.contains("PmPublicInput::Signal"));
    assert!(lanes.contains("pub struct PmPublicLaneEnqueueError"));
    assert!(lanes.contains("struct PmPublicLaneFailureProof"));
    assert!(lanes.contains("PmLaneAuthorityId(<opaque>)"));
    assert!(lanes.contains("lane_generation: u64"));
    assert!(lanes.contains("lane_capacity: usize"));
    assert!(lanes.contains("pub(crate) fn authenticate_lane_failure"));
    assert!(lanes.contains("pub(crate) fn authenticate_aged_failure"));
    assert!(lanes.contains("delivery: D"));
    assert!(lanes.contains("pub struct PmAgedLaneFailure"));
    assert!(lanes.contains("pub struct PmAgedDeliveryEvidence"));
    assert!(lanes.contains("observed_now_ns: u64"));
    assert!(lanes.contains("source_kind_rank: u8"));
    assert!(lanes.contains("source_scope_ordinal: u16"));
    assert!(lanes.contains("pub enum PmServiceSourceKind"));
    assert!(lanes.contains("pub fn source_kind(self) -> PmServiceSourceKind"));
    assert!(lanes.contains("trait PmObservedEvent"));
    assert!(!lanes.contains("pub trait PmObservedEvent"));
}

#[test]
fn pm_production_modules_stay_below_the_review_size_limit() {
    for relative in [
        "../reap-benchmark-allocator/src",
        "../reap-capture-framing/src",
        "../reap-durable-writer/src",
        "../reap-okx-public-source/src",
        "../reap-pm-core/src",
        "../reap-pm-strategy/src",
        "../reap-pm-live-contracts/src",
        "../reap-polymarket-adapter/src",
        "../reap-pm-state/src",
        "../reap-polymarket-wire/src",
        "../reap-transport/src",
        "src",
    ] {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join(relative);
        for path in rust_sources(&root) {
            let line_count = std::fs::read_to_string(&path).unwrap().lines().count();
            assert!(
                line_count <= 1_500,
                "{} has {line_count} lines; split responsibilities before it exceeds 1,500",
                path.display()
            );
        }
    }
}

#[test]
fn only_the_active_run_can_construct_or_mutate_capture_artifacts() {
    let crate_root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let library = std::fs::read_to_string(crate_root.join("src/lib.rs")).unwrap();
    let writer = std::fs::read_to_string(crate_root.join("src/capture/writer.rs")).unwrap();

    assert!(
        !library.contains("PmPublicCaptureWriter"),
        "the low-level writer must not be re-exported"
    );
    assert!(writer.contains("pub(crate) struct PmPublicCaptureWriter"));
    for method in [
        "pub(crate) async fn start",
        "pub(crate) async fn capture_raw_before_parse",
        "pub(crate) async fn capture_okx_raw_before_parse",
        "pub(crate) async fn record_lifecycle",
        "pub(crate) async fn record_okx_lifecycle",
        "pub(crate) async fn record_freshness_timer",
        "pub(crate) async fn finish",
    ] {
        assert!(
            writer.contains(method),
            "capture writer authority widened or method disappeared: {method}"
        );
    }
}

#[test]
fn active_run_never_accepts_or_returns_a_raw_book_reducer() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let composition = rust_module_source(&root, "composition");
    assert!(composition.contains("pm_reducer: PmBookReducer"));
    assert!(!composition.contains("pub pm_reducer: PmBookReducer"));

    let mut public_signature = String::new();
    let mut collecting = false;
    for line in composition.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("pub fn ")
            || trimmed.starts_with("pub async fn ")
            || trimmed.starts_with("pub const fn ")
        {
            public_signature.clear();
            collecting = true;
        }
        if collecting {
            public_signature.push_str(trimmed);
            public_signature.push(' ');
            if trimmed.contains('{') || trimmed.ends_with(';') {
                assert!(
                    !public_signature.contains("PmBookReducer"),
                    "raw reducer escaped through public signature: {public_signature}"
                );
                collecting = false;
            }
        }
    }
    assert!(!collecting, "unterminated public signature source scan");
}

fn rust_sources(root: &Path) -> Vec<std::path::PathBuf> {
    let mut pending = vec![root.to_path_buf()];
    let mut sources = Vec::new();
    while let Some(directory) = pending.pop() {
        for entry in std::fs::read_dir(directory).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                pending.push(path);
            } else if path.extension().and_then(|value| value.to_str()) == Some("rs") {
                sources.push(path);
            }
        }
    }
    sources.sort();
    sources
}

fn rust_module_source(root: &Path, module: &str) -> String {
    let mut paths = vec![root.join(format!("{module}.rs"))];
    paths.extend(rust_sources(&root.join(module)));
    paths
        .into_iter()
        .map(|path| std::fs::read_to_string(path).unwrap())
        .collect::<Vec<_>>()
        .join("\n")
}

fn production_extent(source: &str) -> &str {
    let mut previous_start = 0;
    let mut previous = "";
    let mut offset = 0;
    for line in source.split_inclusive('\n') {
        if previous.trim() == "#[cfg(test)]" && line.trim_start().starts_with("mod tests {") {
            return &source[..previous_start];
        }
        previous_start = offset;
        previous = line;
        offset += line.len();
    }
    source
}

fn workspace_edges(metadata: &Value) -> BTreeMap<String, BTreeSet<String>> {
    let packages = metadata["packages"].as_array().unwrap();
    let names = packages
        .iter()
        .map(|package| {
            (
                package["id"].as_str().unwrap().to_string(),
                package["name"].as_str().unwrap().to_string(),
            )
        })
        .collect::<BTreeMap<_, _>>();
    metadata["resolve"]["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|node| {
            let source = names.get(node["id"].as_str()?)?.clone();
            let targets = node["deps"]
                .as_array()?
                .iter()
                .filter(|dependency| {
                    dependency["dep_kinds"]
                        .as_array()
                        .is_some_and(|kinds| kinds.iter().any(|kind| kind["kind"].is_null()))
                })
                .filter_map(|dependency| names.get(dependency["pkg"].as_str()?).cloned())
                .collect::<BTreeSet<_>>();
            Some((source, targets))
        })
        .collect()
}

fn reachable(edges: &BTreeMap<String, BTreeSet<String>>, source: &str, target: &str) -> bool {
    let mut queue = VecDeque::from([source.to_string()]);
    let mut seen = BTreeSet::new();
    while let Some(current) = queue.pop_front() {
        if !seen.insert(current.clone()) {
            continue;
        }
        for next in edges.get(&current).into_iter().flatten() {
            if next == target {
                return true;
            }
            queue.push_back(next.clone());
        }
    }
    false
}
