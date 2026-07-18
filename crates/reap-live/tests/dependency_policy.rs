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
            "crates/reap-live/src/coordinator.rs".to_string(),
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
        BTreeMap::from([("crates/reap-live/src/runtime.rs".to_string(), 1)]),
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

fn production_rust_source(source: &str) -> &str {
    for marker in [
        "#[cfg(test)]\nmod tests",
        "#[cfg(test)]\n#[path = \"../tests/runtime_unit/mod.rs\"]\nmod tests",
    ] {
        if let Some((production, _)) = source.split_once(marker) {
            return production;
        }
    }
    source
}
