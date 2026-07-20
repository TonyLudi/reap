use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

#[derive(Debug)]
struct Package {
    id: String,
    production_dependencies: BTreeSet<String>,
    binary_targets: BTreeSet<String>,
}

#[test]
fn authenticated_okx_authority_obeys_the_workspace_dependency_policy() {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("reap-live must be inside the workspace crates directory");
    let output = Command::new(env!("CARGO"))
        .args(["metadata", "--locked", "--format-version=1"])
        .current_dir(workspace)
        .output()
        .expect("cargo metadata must execute");
    assert!(
        output.status.success(),
        "cargo metadata failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let metadata: Value = serde_json::from_slice(&output.stdout).expect("valid cargo metadata");
    let members = metadata["workspace_members"]
        .as_array()
        .expect("workspace members")
        .iter()
        .map(|id| id.as_str().expect("member id").to_string())
        .collect::<BTreeSet<_>>();

    let mut packages = BTreeMap::new();
    for package in metadata["packages"].as_array().expect("packages") {
        let id = package["id"].as_str().expect("package id").to_string();
        if !members.contains(&id) {
            continue;
        }
        let name = package["name"].as_str().expect("package name").to_string();
        let production_dependencies = package["dependencies"]
            .as_array()
            .expect("package dependencies")
            .iter()
            .filter(|dependency| dependency["kind"].as_str() != Some("dev"))
            .map(|dependency| {
                dependency["name"]
                    .as_str()
                    .expect("dependency name")
                    .to_string()
            })
            .collect();
        let binary_targets = package["targets"]
            .as_array()
            .expect("package targets")
            .iter()
            .filter(|target| {
                target["kind"]
                    .as_array()
                    .is_some_and(|kinds| kinds.iter().any(|kind| kind.as_str() == Some("bin")))
            })
            .map(|target| target["name"].as_str().expect("target name").to_string())
            .collect();
        assert!(
            packages
                .insert(
                    name,
                    Package {
                        id,
                        production_dependencies,
                        binary_targets,
                    },
                )
                .is_none(),
            "workspace package names must be unique"
        );
    }

    for required in [
        "reap-backtest",
        "reap-live",
        "reap-live-contracts",
        "reap-strategy",
        "reap-feed",
        "reap-order",
        "reap-venue",
        "reap-okx-wire",
        "reap-okx-live-adapter",
        "reap-emergency-core",
        "reap-emergency-runner",
        "reap-evidence-core",
        "reap-okx-evidence-adapter",
        "reap-okx-emergency-adapter",
    ] {
        assert!(
            packages.contains_key(required),
            "missing policy package {required}"
        );
    }

    let resolved = resolved_production_edges(&metadata, &packages);
    assert_direct(&packages, "reap-backtest", "reap-live-contracts");
    assert_not_reachable(&packages, &resolved, "reap-backtest", "reap-live");

    assert_eq!(
        packages["reap-live-contracts"].production_dependencies,
        BTreeSet::from([
            "reap-core".to_string(),
            "reap-risk".to_string(),
            "reap-strategy".to_string(),
            "reap-venue".to_string(),
            "serde".to_string(),
            "serde_ignored".to_string(),
            "serde_json".to_string(),
            "sha2".to_string(),
            "thiserror".to_string(),
            "toml".to_string(),
            "url".to_string(),
        ])
    );
    assert!(
        packages["reap-live-contracts"].binary_targets.is_empty(),
        "reap-live-contracts must not provide a runtime executable"
    );
    for forbidden in [
        "reap-live",
        "reap-feed",
        "reap-order",
        "reap-storage",
        "reap-telemetry",
        "reap-evidence-core",
        "reap-emergency-core",
        "reap-emergency-runner",
        "reap-okx-wire",
        "reap-okx-live-adapter",
        "reap-okx-evidence-adapter",
        "reap-okx-emergency-adapter",
    ] {
        assert_not_reachable(&packages, &resolved, "reap-live-contracts", forbidden);
    }

    assert_eq!(
        packages["reap-order"].production_dependencies,
        BTreeSet::from([
            "async-trait".to_string(),
            "reap-core".to_string(),
            "reap-risk".to_string(),
            "reap-storage".to_string(),
            "reap-strategy".to_string(),
            "reap-venue".to_string(),
            "serde".to_string(),
            "thiserror".to_string(),
            "tokio".to_string(),
        ])
    );
    assert!(
        packages["reap-order"].binary_targets.is_empty(),
        "reap-order must not provide a runtime executable"
    );
    for forbidden in [
        "reap-live",
        "reap-live-contracts",
        "reap-feed",
        "reap-telemetry",
        "reap-evidence-core",
        "reap-emergency-core",
        "reap-emergency-runner",
        "reap-okx-wire",
        "reap-okx-live-adapter",
        "reap-okx-evidence-adapter",
        "reap-okx-emergency-adapter",
    ] {
        assert_not_reachable(&packages, &resolved, "reap-order", forbidden);
    }

    assert_direct(&packages, "reap-live", "reap-okx-live-adapter");
    assert_not_direct(&packages, "reap-live", "reap-okx-wire");
    assert_not_direct(&packages, "reap-live", "tokio-tungstenite");
    assert_direct(&packages, "reap-okx-live-adapter", "tokio-tungstenite");
    for forbidden in [
        "reap-emergency-core",
        "reap-emergency-runner",
        "reap-okx-emergency-adapter",
        "reap-okx-evidence-adapter",
    ] {
        assert_not_reachable(&packages, &resolved, "reap-live", forbidden);
    }

    assert_direct(&packages, "reap-emergency-runner", "reap-emergency-core");
    assert!(
        packages["reap-emergency-runner"]
            .binary_targets
            .contains("reap-emergency"),
        "reap-emergency-runner must provide the dedicated reap-emergency executable"
    );
    assert_direct(
        &packages,
        "reap-emergency-runner",
        "reap-okx-emergency-adapter",
    );
    for forbidden in [
        "reap-live",
        "reap-okx-live-adapter",
        "reap-okx-evidence-adapter",
    ] {
        assert_not_reachable(&packages, &resolved, "reap-emergency-runner", forbidden);
    }
    assert_not_direct(&packages, "reap-emergency-runner", "reap-okx-wire");
    assert_not_direct(&packages, "reap-cli", "reap-emergency-runner");
    assert_not_reachable(
        &packages,
        &resolved,
        "reap-cli",
        "reap-okx-emergency-adapter",
    );
    for core in ["reap-emergency-core", "reap-evidence-core"] {
        for authority in [
            "reap-okx-wire",
            "reap-okx-live-adapter",
            "reap-okx-emergency-adapter",
            "reap-okx-evidence-adapter",
        ] {
            assert_not_reachable(&packages, &resolved, core, authority);
        }
    }

    let wire_dependents = packages
        .iter()
        .filter(|(_, package)| package.production_dependencies.contains("reap-okx-wire"))
        .map(|(name, _)| name.as_str())
        .collect::<BTreeSet<_>>();
    assert_eq!(
        wire_dependents,
        BTreeSet::from([
            "reap-okx-emergency-adapter",
            "reap-okx-evidence-adapter",
            "reap-okx-live-adapter",
        ])
    );

    for adapter in [
        "reap-okx-live-adapter",
        "reap-okx-emergency-adapter",
        "reap-okx-evidence-adapter",
    ] {
        assert_direct(&packages, adapter, "reap-okx-wire");
        for bypass in ["base64", "hmac", "reqwest", "sha2"] {
            assert_not_direct(&packages, adapter, bypass);
        }
        for peer in [
            "reap-okx-live-adapter",
            "reap-okx-emergency-adapter",
            "reap-okx-evidence-adapter",
        ] {
            if peer != adapter {
                assert_not_direct(&packages, adapter, peer);
            }
        }
    }
    assert_direct(
        &packages,
        "reap-okx-emergency-adapter",
        "reap-emergency-core",
    );
    assert_direct(&packages, "reap-okx-evidence-adapter", "reap-evidence-core");

    for forbidden in ["base64", "chrono", "hmac", "reqwest", "sha2", "url"] {
        assert_not_direct(&packages, "reap-venue", forbidden);
    }
    for consumer in ["reap-feed", "reap-order"] {
        for authority in [
            "reap-okx-wire",
            "reap-okx-live-adapter",
            "reap-okx-emergency-adapter",
            "reap-okx-evidence-adapter",
        ] {
            assert_not_reachable(&packages, &resolved, consumer, authority);
        }
    }

    let normal_regular_mutation_composers = packages
        .iter()
        .filter(|(_, package)| {
            package.production_dependencies.contains("reap-order")
                && package
                    .production_dependencies
                    .contains("reap-okx-live-adapter")
        })
        .map(|(name, _)| name.as_str())
        .collect::<BTreeSet<_>>();
    assert_eq!(
        normal_regular_mutation_composers,
        BTreeSet::from(["reap-live"]),
        "only reap-live may directly compose regular approval with the authenticated live adapter"
    );

    let strategy_workspace_dependencies = packages["reap-strategy"]
        .production_dependencies
        .iter()
        .filter(|dependency| packages.contains_key(*dependency))
        .cloned()
        .collect::<BTreeSet<_>>();
    assert_eq!(
        strategy_workspace_dependencies,
        BTreeSet::from(["reap-core".to_string()])
    );
    for forbidden in [
        "reap-venue",
        "reap-feed",
        "reap-order",
        "reap-live",
        "reap-okx-wire",
        "reap-okx-live-adapter",
        "reap-okx-emergency-adapter",
        "reap-okx-evidence-adapter",
    ] {
        assert_not_reachable(&packages, &resolved, "reap-strategy", forbidden);
    }
}

#[test]
fn regular_order_authority_construction_is_source_allowlisted() {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("reap-live must be inside the workspace crates directory");
    let mut sources = Vec::new();
    collect_rust_sources(&workspace.join("crates"), &mut sources);

    let profile_type = ["RegularExecution", "Profile"].concat();
    let profile_constructor = ["RegularExecutionPolicy", "::from_profile"].concat();
    let profile_binding = ["bind_", "profiles"].concat();
    let ownership_registry = ["OwnedRegular", "Orders"].concat();
    let local_reservation = ["reserve_", "local("].concat();
    let recovered_registration = ["register_", "recovered("].concat();
    let recovered_storage_proof = ["ProvenRegular", "SubmitRequest"].concat();
    let approval_scope_take = ["take_approval_", "scope("].concat();
    let command_dispatcher_take = ["take_command_", "dispatcher("].concat();
    let order_transport_install = ["set_order_", "transport("].concat();
    let private_bootstrap_binding = ["BootstrapFactory", "::bind_private_websocket("].concat();
    let private_login_validation = ["PrivateLoginBootstrap", "::parse("].concat();
    let mut profile_mentions = BTreeSet::new();
    let mut profile_constructors = BTreeSet::new();
    let mut profile_bindings = BTreeSet::new();
    let mut ownership_mentions = BTreeSet::new();
    let mut local_reservations = BTreeSet::new();
    let mut recovered_registrations = BTreeSet::new();
    let mut recovered_storage_proof_mentions = BTreeSet::new();
    let mut approval_scope_takes = BTreeSet::new();
    let mut command_dispatcher_takes = BTreeSet::new();
    let mut order_transport_installs = BTreeSet::new();
    let mut private_bootstrap_bindings = BTreeSet::new();
    let mut private_login_validations = BTreeSet::new();

    for path in &sources {
        let relative = workspace_relative(workspace, path);
        if !relative.contains("/src/") {
            continue;
        }
        let source = fs::read_to_string(path)
            .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()));
        let compact = production_rust_source(&source)
            .chars()
            .filter(|character| !character.is_whitespace())
            .collect::<String>();
        if compact.contains(&profile_type) {
            profile_mentions.insert(relative.clone());
        }
        if compact.contains(&profile_constructor) {
            profile_constructors.insert(relative.clone());
        }
        if compact.contains(&profile_binding) {
            profile_bindings.insert(relative.clone());
        }
        if compact.contains(&ownership_registry) {
            ownership_mentions.insert(relative.clone());
        }
        if compact.contains(&local_reservation) {
            local_reservations.insert(relative.clone());
        }
        if compact.contains(&recovered_registration) {
            recovered_registrations.insert(relative.clone());
        }
        if compact.contains(&recovered_storage_proof) {
            recovered_storage_proof_mentions.insert(relative.clone());
        }
        if compact.contains(&approval_scope_take) {
            approval_scope_takes.insert(relative.clone());
        }
        if compact.contains(&command_dispatcher_take) {
            command_dispatcher_takes.insert(relative.clone());
        }
        if compact.contains(&order_transport_install) {
            order_transport_installs.insert(relative.clone());
        }
        if compact.contains(&private_bootstrap_binding) {
            private_bootstrap_bindings.insert(relative.clone());
        }
        if compact.contains(&private_login_validation) {
            private_login_validations.insert(relative);
        }
    }

    assert_eq!(
        profile_mentions,
        BTreeSet::from([
            "crates/reap-live/src/regular_execution.rs".to_string(),
            "crates/reap-order/src/authority.rs".to_string(),
            "crates/reap-order/src/lib.rs".to_string(),
        ]),
        "regular execution profiles must stay inside their policy owner and the one live composition seam"
    );
    assert_eq!(
        profile_constructors,
        BTreeSet::from(["crates/reap-live/src/regular_execution.rs".to_string()]),
        "only the live verified-bootstrap seam may construct the regular execution policy"
    );
    assert_eq!(
        profile_bindings,
        BTreeSet::from([
            "crates/reap-live/src/regular_execution.rs".to_string(),
            "crates/reap-order/src/authority.rs".to_string(),
        ]),
        "gateway-bound profiles must be assembled only at the verified live composition seam"
    );
    assert_eq!(
        ownership_mentions,
        BTreeSet::from([
            "crates/reap-live/src/coordinator.rs".to_string(),
            "crates/reap-order/src/authority.rs".to_string(),
            "crates/reap-order/src/lib.rs".to_string(),
        ]),
        "owned-order cancellation authority must stay inside reap-order and the single live coordinator"
    );
    assert_eq!(
        local_reservations,
        BTreeSet::from([
            "crates/reap-live/src/coordinator/routing.rs".to_string(),
            "crates/reap-order/src/authority.rs".to_string(),
        ]),
        "local ownership reservation must stay inside its definition and the single live coordinator"
    );
    assert_eq!(
        recovered_registrations,
        BTreeSet::from([
            "crates/reap-live/src/coordinator.rs".to_string(),
            "crates/reap-order/src/authority.rs".to_string(),
        ]),
        "recovered ownership registration must stay inside its definition and the storage-proof validating coordinator"
    );
    assert_eq!(
        recovered_storage_proof_mentions,
        BTreeSet::from([
            "crates/reap-live/src/coordinator.rs".to_string(),
            "crates/reap-live/src/runtime/recovery.rs".to_string(),
            "crates/reap-order/src/authority.rs".to_string(),
            "crates/reap-storage/src/lib.rs".to_string(),
        ]),
        "durable regular-submit ownership proof must stay inside storage, order policy, and the one live recovery path"
    );
    assert_eq!(
        approval_scope_takes,
        BTreeSet::from([
            "crates/reap-live/src/runtime/startup.rs".to_string(),
            "crates/reap-okx-live-adapter/src/lib.rs".to_string(),
            "crates/reap-order/src/gateway.rs".to_string(),
        ]),
        "account approval scopes must transfer only through the sealed adapter bundle into live composition"
    );
    assert_eq!(
        command_dispatcher_takes,
        BTreeSet::from([
            "crates/reap-live/src/runtime/dispatch.rs".to_string(),
            "crates/reap-order/src/gateway.rs".to_string(),
        ]),
        "the command role must transfer only from the gateway into the bounded order task"
    );
    assert_eq!(
        order_transport_installs,
        BTreeSet::new(),
        "no public gateway transport installation seam may exist; the authenticated adapter owns its private once-only command slot"
    );
    assert_eq!(
        private_bootstrap_bindings,
        BTreeSet::from(["crates/reap-okx-live-adapter/src/lib.rs".to_string()]),
        "only the authenticated live adapter may bind a private feed-login factory in production source"
    );
    assert_eq!(
        private_login_validations,
        BTreeSet::from(["crates/reap-okx-live-adapter/src/lib.rs".to_string()]),
        "only the authenticated live adapter may create a validated private feed-login payload in production source"
    );

    let order_lib = fs::read_to_string(workspace.join("crates/reap-order/src/lib.rs"))
        .expect("reap-order lib source");
    let compact_order_lib = order_lib
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect::<String>();
    assert!(
        !compact_order_lib.contains("pubuseclient_id::*")
            && !compact_order_lib.contains("pubusegateway::*")
            && !compact_order_lib.contains("pubusepacing::*")
            && !compact_order_lib.contains("pubuseprivate::*")
            && !compact_order_lib.contains("pubusereconcile::*")
            && !compact_order_lib.contains("pubusetransport::*"),
        "reap-order must use an explicit public export surface"
    );

    let order_authority = fs::read_to_string(workspace.join("crates/reap-order/src/authority.rs"))
        .expect("reap-order authority source");
    let order_authority = production_rust_source(&order_authority)
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect::<String>();
    assert!(
        !order_authority.contains("recover_jsonl"),
        "reap-order may accept the opaque durable-submit proof type but must not own storage recovery IO"
    );
}

#[test]
fn raw_regular_order_dtos_are_constructed_only_by_the_live_adapter() {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("reap-live must be inside the workspace crates directory");
    let place_constructor = ["OkxPlaceOrder", "{symbol:"].concat();
    let cancel_constructor = ["OkxCancelOrder", "{symbol:"].concat();
    let mut live_bundle_starts = BTreeMap::<String, usize>::new();

    let mut live_sources = Vec::new();
    collect_rust_sources(&workspace.join("crates/reap-live/src"), &mut live_sources);
    for path in live_sources {
        let source = fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()));
        let compact = production_rust_source(&source)
            .chars()
            .filter(|character| !character.is_whitespace())
            .collect::<String>();
        let relative = workspace_relative(workspace, &path);
        assert!(
            !compact.contains(&place_constructor) && !compact.contains(&cancel_constructor),
            "{} constructs a raw OKX regular-order DTO",
            relative
        );
        for forbidden in [
            "tokio_tungstenite",
            "tungstenite::",
            "OrderCommandWebsocketTransport",
            "OrderCommandTransportSlot",
            "OrderSessionOperations",
            "OkxWsOrderOperation",
            "start_order_command_websocket(",
            "set_order_transport(",
            "order_transport.install(",
        ] {
            assert!(
                !compact.contains(forbidden),
                "{relative} owns forbidden raw order-websocket authority token {forbidden}"
            );
        }
        let starts = compact.matches(".start_and_install(").count();
        if starts != 0 {
            live_bundle_starts.insert(relative, starts);
        }
    }
    assert_eq!(
        live_bundle_starts,
        BTreeMap::from([("crates/reap-live/src/runtime/startup.rs".to_string(), 1,)]),
        "the live runtime must consume exactly one sealed gateway/session bundle start seam"
    );

    for relative in [
        "crates/reap-order/src/gateway.rs",
        "crates/reap-order/src/transport.rs",
    ] {
        let source = fs::read_to_string(workspace.join(relative))
            .unwrap_or_else(|error| panic!("failed to read {relative}: {error}"));
        let compact = source
            .chars()
            .filter(|character| !character.is_whitespace())
            .collect::<String>();
        assert!(
            !compact.contains(&place_constructor) && !compact.contains(&cancel_constructor),
            "{relative} constructs a raw OKX regular-order DTO"
        );
    }

    let mut adapter_sources = Vec::new();
    collect_rust_sources(
        &workspace.join("crates/reap-okx-live-adapter/src"),
        &mut adapter_sources,
    );
    let mut adapter_production = String::new();
    let mut websocket_transport_owners = BTreeSet::new();
    let mut tungstenite_owners = BTreeSet::new();
    let mut transport_install_owners = BTreeSet::new();
    for path in adapter_sources {
        let relative = workspace_relative(workspace, &path);
        let source = fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()));
        let compact = production_rust_source(&source)
            .chars()
            .filter(|character| !character.is_whitespace())
            .collect::<String>();
        if compact.contains("OrderCommandWebsocketTransport") {
            websocket_transport_owners.insert(relative.clone());
        }
        if compact.contains("tokio_tungstenite") {
            tungstenite_owners.insert(relative.clone());
        }
        if compact.contains("order_transport.install(") {
            transport_install_owners.insert(relative);
        }
        adapter_production.push_str(&compact);
    }
    assert_eq!(
        adapter_production.matches(&place_constructor).count(),
        1,
        "the live adapter must have one regular place DTO mapper"
    );
    assert_eq!(
        adapter_production.matches(&cancel_constructor).count(),
        1,
        "the live adapter must have one regular cancel DTO mapper"
    );
    assert_eq!(
        websocket_transport_owners,
        BTreeSet::from([
            "crates/reap-okx-live-adapter/src/lib.rs".to_string(),
            "crates/reap-okx-live-adapter/src/order_ws.rs".to_string(),
        ]),
        "the adapter's private command transport may span only its role root and websocket module"
    );
    assert_eq!(
        tungstenite_owners,
        BTreeSet::from(["crates/reap-okx-live-adapter/src/order_ws.rs".to_string()]),
        "only the adapter order-websocket module may own the raw websocket dependency"
    );
    assert_eq!(
        transport_install_owners,
        BTreeSet::from(["crates/reap-okx-live-adapter/src/order_ws.rs".to_string()]),
        "only the sealed bundle start may install the adapter's private command transport"
    );
    assert_eq!(
        adapter_production
            .matches("pubfnstart_and_install(")
            .count(),
        1,
        "the adapter must expose exactly one consuming bound gateway/session start seam"
    );
    assert!(
        !adapter_production.contains("pubstructOrderCommandWebsocketTransport")
            && !adapter_production.contains("pubtraitOrderSessionOperations")
            && !adapter_production.contains("pubfninstall("),
        "raw order-websocket transport, protocol, and installation authority must remain private"
    );
}

#[test]
fn normal_regular_submit_numeric_lowering_uses_only_prepared_canonical_values() {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("reap-live must be inside the workspace crates directory");
    let adapter_path = workspace.join("crates/reap-okx-live-adapter/src/lib.rs");
    let source = fs::read_to_string(&adapter_path)
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", adapter_path.display()));
    let compact = production_rust_source(&source)
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect::<String>();

    for forbidden in [
        "order.price.to_string()",
        "order.qty.to_string()",
        "prepared.order().price.to_string()",
        "prepared.order().qty.to_string()",
        "price:order.price",
        "qty:order.qty",
    ] {
        assert!(
            !compact.contains(forbidden),
            "normal regular-submit px/sz lowering derives bytes from raw f64 via {forbidden}"
        );
    }
    assert!(
        compact.contains(
            "price:*prepared.canonical_price(),qty:*prepared.canonical_qty(),client_order_id:"
        ),
        "regular_place_order must assign both numeric fields directly from the opaque prepared canonical payload"
    );
}

#[test]
fn storage_progress_telemetry_is_a_narrow_read_only_contract() {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("reap-live must be inside the workspace crates directory");
    let storage_path = workspace.join("crates/reap-storage/src/lib.rs");
    let storage = compact_production_rust_source(
        &fs::read_to_string(&storage_path)
            .unwrap_or_else(|error| panic!("failed to read {}: {error}", storage_path.display())),
    );

    let snapshot_marker = "pubstructStorageProgressSnapshot{";
    assert_eq!(
        storage.matches(snapshot_marker).count(),
        1,
        "storage progress must expose exactly one public snapshot type"
    );
    let snapshot = compact_rust_item(&storage, snapshot_marker);
    assert_eq!(
        snapshot,
        concat!(
            "pubstructStorageProgressSnapshot{",
            "pubrecords_enqueued:u64,",
            "pubrecords_written:u64,",
            "pubdurable_sync_completions:u64,",
            "pubwrite_failures:u64,",
            "pubsync_failures:u64,",
            "pubdropped_records:u64,",
            "pubrecords_outstanding:usize,",
            "pubqueue_capacity:usize,",
            "pubqueue_depth:usize,",
            "pubqueue_high_water:usize,",
            "publast_writer_progress_ns:u64,",
            "publast_writer_progress_age_ns:u64,",
            "}"
        ),
        "storage telemetry must remain a fixed numeric observation payload"
    );

    let snapshot_start = storage.find(snapshot_marker).expect("snapshot declaration");
    let derive_start = storage[..snapshot_start]
        .rfind("#[derive(")
        .expect("snapshot derive");
    assert_eq!(
        &storage[derive_start..snapshot_start],
        "#[derive(Debug,Clone,Copy,PartialEq,Eq)]",
        "storage telemetry must not acquire Serialize, Deserialize, or another behavioral derive"
    );
    assert!(
        !storage.contains("implStorageProgressSnapshot")
            && !storage.contains("forStorageProgressSnapshot"),
        "the storage snapshot must remain a passive data value without methods or trait authority"
    );

    let public_reader_marker = "pubfnprogress_snapshot(&self)->StorageProgressSnapshot{";
    assert_eq!(
        storage.matches(public_reader_marker).count(),
        1,
        "StorageSink must expose exactly one immutable progress reader"
    );
    let public_reader = compact_rust_item(&storage, public_reader_marker);
    assert!(
        public_reader.contains("letsnapshot=self.inner.progress_snapshot();")
            && public_reader.contains("StorageProgressSnapshot{")
            && public_reader.ends_with("}}"),
        "StorageSink::progress_snapshot must remain a parameter-free read-only facade over neutral writer progress"
    );

    for forbidden_mutation_or_authority in [
        ".store(",
        ".swap(",
        ".fetch_",
        ".compare_exchange",
        ".send(",
        ".try_send(",
        ".reserve(",
        ".try_reserve(",
        "request_shutdown",
        "stop_writer",
        "tokio::spawn",
        ".await",
        "unsafe{",
    ] {
        assert!(
            !public_reader.contains(forbidden_mutation_or_authority),
            "storage progress reader contains mutation/control operation `{forbidden_mutation_or_authority}`"
        );
    }
}

#[test]
fn runtime_health_stays_private_observation_only_and_bounded() {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("reap-live must be inside the workspace crates directory");
    let runtime_path = workspace.join("crates/reap-live/src/runtime.rs");
    let health_path = workspace.join("crates/reap-live/src/runtime/health.rs");
    let lib_path = workspace.join("crates/reap-live/src/lib.rs");

    let runtime = compact_production_rust_source(
        &fs::read_to_string(&runtime_path)
            .unwrap_or_else(|error| panic!("failed to read {}: {error}", runtime_path.display())),
    );
    let health = compact_production_rust_source(
        &fs::read_to_string(&health_path)
            .unwrap_or_else(|error| panic!("failed to read {}: {error}", health_path.display())),
    );
    let live_lib = compact_production_rust_source(
        &fs::read_to_string(&lib_path)
            .unwrap_or_else(|error| panic!("failed to read {}: {error}", lib_path.display())),
    );

    assert_eq!(
        runtime.matches("modhealth;").count(),
        1,
        "runtime health must remain one private runtime responsibility module"
    );
    for public_module in [
        "pubmodhealth;",
        "pub(crate)modhealth;",
        "pub(super)modhealth;",
    ] {
        assert!(
            !runtime.contains(public_module),
            "runtime health module visibility broadened through `{public_module}`"
        );
    }
    assert!(
        health.contains("pub(super)structRuntimeHealthSnapshot{"),
        "the private schema payload must remain visible only to its parent runtime"
    );
    for public_snapshot in [
        "pubstructRuntimeHealthSnapshot{",
        "pub(crate)structRuntimeHealthSnapshot{",
    ] {
        assert!(
            !health.contains(public_snapshot),
            "runtime health snapshot visibility broadened through `{public_snapshot}`"
        );
    }
    for public_api in ["RuntimeHealthSnapshot", "RuntimeHealthState"] {
        assert!(
            !live_lib.contains(public_api),
            "reap-live public exports must not expose private health type `{public_api}`"
        );
    }

    let report = compact_rust_item(&runtime, "pubstructLiveRunReport{");
    for forbidden_report_field in [
        "RuntimeHealthSnapshot",
        "runtime_health:",
        "runtime_health_snapshot:",
        "health_snapshot:",
        "health_heartbeat:",
    ] {
        assert!(
            !report.contains(forbidden_report_field),
            "runtime health must not change the public LiveRunReport schema via `{forbidden_report_field}`"
        );
    }

    let mut contract_sources = Vec::new();
    collect_rust_sources(
        &workspace.join("crates/reap-live-contracts/src"),
        &mut contract_sources,
    );
    for path in contract_sources {
        let relative = workspace_relative(workspace, &path);
        let source = fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()));
        let compact = compact_production_rust_source(&source);
        for forbidden_config_surface in [
            "RuntimeHealthSnapshot",
            "RuntimeHealthState",
            "runtime_health:",
            "health_emission_interval",
            "health_interval",
            "health_cadence",
            "health_listener",
            "health_endpoint",
            "health_socket",
            "health_server",
            "health_port",
            "heartbeat_output",
        ] {
            assert!(
                !compact.contains(forbidden_config_surface),
                "{relative} adds forbidden runtime-health config/API surface `{forbidden_config_surface}`"
            );
        }
    }

    for forbidden_io_or_authority in [
        "TcpListener",
        "UnixListener",
        "UdpSocket",
        "TcpStream",
        "UnixStream",
        "hyper::",
        "axum::",
        "warp::",
        "tonic::",
        "oneshot::Sender",
        "watch::Sender",
        "broadcast::Sender",
        "OrderIntent",
        "ChaosExecutionIntent",
        "LiveAction",
        "SubmitAction",
        "CancelAction",
        "ApprovedRegular",
        "ReservedRegular",
        "PreparedRegular",
        "RegularApprovalScope",
        "OwnedRegularOrders",
        "OkxOrderGateway",
        "OrderTaskCommand",
        "OkxPlaceOrder",
        "OkxCancelOrder",
        "RawEnvelope",
        "reap_okx_wire",
        "Emergency",
        "emergency::",
        "Signer",
        "Credential",
        "SecretKey",
        "Deserialize",
        "LiveConfig",
        "RuntimeConfig",
    ] {
        assert!(
            !health.contains(forbidden_io_or_authority),
            "runtime health must remain observation-only and contains `{forbidden_io_or_authority}`"
        );
    }

    let progress_start = health
        .find("pub(super)fnset_connectivity_expected(")
        .expect("health progress methods must begin at fixed connectivity setup");
    let snapshot_start = health
        .find("pub(super)fnperiodic_snapshot(")
        .expect("snapshot assembly must remain separate from progress updates");
    assert!(
        progress_start < snapshot_start,
        "health snapshot assembly moved into the progress-update section"
    );
    let progress_updates = &health[progress_start..snapshot_start];
    let progress_clock_update = compact_rust_item(&health, "fnobserve_at(");
    let saturating_counter_update = compact_rust_item(&health, "fnatomic_saturating_add(");
    let saturating_depth_increment = compact_rust_item(&health, "fnatomic_saturating_increment(");
    let saturating_depth_decrement = compact_rust_item(&health, "fnatomic_saturating_sub(");
    let counter_classification = compact_rust_item(&health, "pub(super)fnfrom_storage_record(");
    let queue_lane_state = compact_rust_item(&health, "fnstate(&self)->&QueueLaneState{");
    let queue_enqueue = compact_rust_item(&health, "fnenqueue(&self)->u64{");
    let queue_enqueue_at = compact_rust_item(&health, "fnenqueue_at(&self,enqueued_at_ns:u64){");
    let queue_begin_remove = compact_rust_item(&health, "fnbegin_remove(&self)->QueueRemoval{");
    let queue_finish_remove =
        compact_rust_item(&health, "fnfinish_remove(&self,removal:QueueRemoval){");
    let queue_dequeue = compact_rust_item(&health, "fndequeue(&self,enqueued_at_ns:u64){");
    let queue_discard = compact_rust_item(&health, "fndiscard(&self){");
    let queue_full_attempt = compact_rust_item(&health, "fnobserve_full_attempt(&self){");
    let queued_new = compact_rust_item(&health, "fnnew(value:T,lane:Arc<QueueLane>)->Self{");
    let queued_consume = compact_rust_item(&health, "fnconsume(mutself)->T{");
    let queued_drop = compact_rust_item(&health, "impl<T>DropforQueued<T>{");
    let sender_send = compact_rust_item(&health, "pub(super)asyncfnsend(&self,value:T)");
    let sender_try_send = compact_rust_item(&health, "pub(super)fntry_send(&self,value:T)");
    let receiver_recv = compact_rust_item(&health, "pub(super)asyncfnrecv(&mutself)");
    let receiver_try_recv = compact_rust_item(&health, "pub(super)fntry_recv(&mutself)");
    let receiver_consume = compact_rust_item(&health, "fnconsume(&self,queued:Queued<T>)->T{");
    for production_progress in [
        progress_updates,
        progress_clock_update,
        saturating_counter_update,
        saturating_depth_increment,
        saturating_depth_decrement,
        counter_classification,
        queue_lane_state,
        queue_enqueue,
        queue_enqueue_at,
        queue_begin_remove,
        queue_finish_remove,
        queue_dequeue,
        queue_discard,
        queue_full_attempt,
        queued_new,
        queued_consume,
        queued_drop,
        sender_send,
        sender_try_send,
        receiver_recv,
        receiver_try_recv,
        receiver_consume,
    ] {
        for forbidden_progress_operation in [
            "HealthRegistry",
            ".set(",
            "Mutex",
            "RwLock",
            "parking_lot",
            ".lock(",
            ".read(",
            ".write(",
            "HashMap",
            "BTreeMap",
            "HashSet",
            "BTreeSet",
            ".register(",
            "register(",
            ".insert(",
            ".entry(",
            "String",
            "&str",
            "format!",
            "format_args!",
            "write!",
            ".to_string(",
            ".to_owned(",
            ".clone(",
            "Symbol",
            "Vec<",
            "Vec::",
            "vec![",
            "Box<",
            "Box::",
            "Arc::new(",
            "Rc::new(",
            ".collect(",
            "collect::<",
            "serde_json",
            "tracing::",
        ] {
            assert!(
                !production_progress.contains(forbidden_progress_operation),
                "runtime health progress path contains forbidden operation `{forbidden_progress_operation}`"
            );
        }
    }

    let mut live_sources = Vec::new();
    collect_rust_sources(&workspace.join("crates/reap-live/src"), &mut live_sources);
    for path in live_sources {
        let relative = workspace_relative(workspace, &path);
        let source = fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()));
        let compact = compact_production_rust_source(&source);
        assert!(
            !compact.contains("HealthRegistry"),
            "{relative} routes production progress through the allocating HealthRegistry"
        );
    }
}

fn resolved_production_edges(
    metadata: &Value,
    packages: &BTreeMap<String, Package>,
) -> BTreeMap<String, BTreeSet<String>> {
    let names_by_id = packages
        .iter()
        .map(|(name, package)| (package.id.as_str(), name.as_str()))
        .collect::<BTreeMap<_, _>>();
    let mut edges = BTreeMap::new();
    for node in metadata["resolve"]["nodes"]
        .as_array()
        .expect("resolve nodes")
    {
        let Some(source) = node["id"]
            .as_str()
            .and_then(|id| names_by_id.get(id).copied())
        else {
            continue;
        };
        let targets = node["deps"]
            .as_array()
            .expect("resolved dependencies")
            .iter()
            .filter(|dependency| {
                dependency["dep_kinds"]
                    .as_array()
                    .expect("dependency kinds")
                    .iter()
                    .any(|kind| kind["kind"].as_str() != Some("dev"))
            })
            .filter_map(|dependency| dependency["pkg"].as_str())
            .filter_map(|id| names_by_id.get(id).copied())
            .map(str::to_string)
            .collect();
        edges.insert(source.to_string(), targets);
    }
    edges
}

fn assert_direct(packages: &BTreeMap<String, Package>, source: &str, target: &str) {
    assert!(
        packages[source].production_dependencies.contains(target),
        "required production edge {source} -> {target} is absent"
    );
}

fn assert_not_direct(packages: &BTreeMap<String, Package>, source: &str, target: &str) {
    assert!(
        !packages[source].production_dependencies.contains(target),
        "forbidden production edge {source} -> {target} is present"
    );
}

fn assert_not_reachable(
    packages: &BTreeMap<String, Package>,
    edges: &BTreeMap<String, BTreeSet<String>>,
    source: &str,
    target: &str,
) {
    let mut visited = BTreeSet::new();
    let mut pending = VecDeque::from([source.to_string()]);
    while let Some(current) = pending.pop_front() {
        if !visited.insert(current.clone()) {
            continue;
        }
        if current == target {
            panic!("forbidden production dependency path {source} -> ... -> {target} is present");
        }
        pending.extend(
            edges
                .get(&current)
                .into_iter()
                .flatten()
                .filter(|dependency| packages.contains_key(*dependency))
                .cloned(),
        );
    }
}

fn collect_rust_sources(directory: &Path, sources: &mut Vec<PathBuf>) {
    let mut entries = fs::read_dir(directory)
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", directory.display()))
        .collect::<Result<Vec<_>, _>>()
        .unwrap_or_else(|error| panic!("failed to enumerate {}: {error}", directory.display()));
    entries.sort_by_key(|entry| entry.path());
    for entry in entries {
        let path = entry.path();
        if path.is_dir() {
            collect_rust_sources(&path, sources);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            sources.push(path);
        }
    }
}

fn workspace_relative(workspace: &Path, path: &Path) -> String {
    path.strip_prefix(workspace)
        .expect("source must be inside workspace")
        .to_string_lossy()
        .replace('\\', "/")
}

const INLINE_TEST_MODULE_MARKER: &str = "#[cfg(test)]\nmod tests";
const EXTERNAL_TEST_MODULE_MARKERS: [&str; 5] = [
    "#[cfg(test)]\n#[path = \"../tests/runner_unit/mod.rs\"]\nmod tests;",
    "#[cfg(test)]\n#[path = \"../tests/coordinator_unit/mod.rs\"]\nmod tests;",
    "#[cfg(test)]\n#[path = \"../tests/runtime_unit/mod.rs\"]\nmod tests;",
    "#[cfg(test)]\n#[path = \"../tests/economic_statement_unit/mod.rs\"]\nmod tests;",
    "#[cfg(test)]\n#[path = \"../tests/production_evidence_unit/mod.rs\"]\nmod tests;",
];

fn production_rust_source(source: &str) -> &str {
    if let Some((production, _)) = source.split_once(INLINE_TEST_MODULE_MARKER) {
        return production;
    }

    let source_without_final_newline = source.strip_suffix('\n').unwrap_or(source);
    for marker in EXTERNAL_TEST_MODULE_MARKERS {
        if let Some(production) = source_without_final_newline.strip_suffix(marker) {
            return production;
        }
    }
    source
}

fn compact_production_rust_source(source: &str) -> String {
    production_rust_source(source)
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect()
}

fn compact_rust_item<'a>(compact_source: &'a str, marker: &str) -> &'a str {
    let item_start = compact_source
        .find(marker)
        .unwrap_or_else(|| panic!("missing compact Rust item marker `{marker}`"));
    let body_start = compact_source[item_start..]
        .find('{')
        .map(|offset| item_start + offset)
        .unwrap_or_else(|| panic!("compact Rust item `{marker}` has no body"));
    let mut depth = 0_usize;
    for (offset, byte) in compact_source.as_bytes()[body_start..]
        .iter()
        .copied()
        .enumerate()
    {
        match byte {
            b'{' => depth += 1,
            b'}' => {
                depth = depth.checked_sub(1).unwrap_or_else(|| {
                    panic!("compact Rust item `{marker}` has unbalanced braces")
                });
                if depth == 0 {
                    return &compact_source[item_start..=body_start + offset];
                }
            }
            _ => {}
        }
    }
    panic!("compact Rust item `{marker}` has no closing brace")
}

#[test]
fn external_test_markers_cannot_hide_later_production_source() {
    const PRODUCTION: &str = "fn visible_production() {}\n";

    for marker in EXTERNAL_TEST_MODULE_MARKERS {
        let terminal = [PRODUCTION, marker].concat();
        assert_eq!(production_rust_source(&terminal), PRODUCTION);

        let terminal_with_newline = [PRODUCTION, marker, "\n"].concat();
        assert_eq!(production_rust_source(&terminal_with_newline), PRODUCTION);

        let bypass = [
            PRODUCTION,
            marker,
            "\nfn production_after_test_marker() {}\n",
        ]
        .concat();
        assert_eq!(
            production_rust_source(&bypass),
            bypass,
            "a non-terminal external test marker must not hide later production"
        );
    }

    let inline = [
        PRODUCTION,
        INLINE_TEST_MODULE_MARKER,
        " { fn existing_inline_test() {} }\n",
    ]
    .concat();
    assert_eq!(
        production_rust_source(&inline),
        PRODUCTION,
        "the existing inline test-module truncation must remain unchanged"
    );
}
