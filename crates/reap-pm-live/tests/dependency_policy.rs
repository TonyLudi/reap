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

    for source in [
        "reap-pm-strategy",
        "reap-pm-live-contracts",
        "reap-polymarket-adapter",
        "reap-pm-live",
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
            "base64",
        ] {
            assert!(
                !reachable(&edges, source, forbidden),
                "{source} reaches forbidden dependency {forbidden}"
            );
        }
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
    let composition = std::fs::read_to_string(root.join("composition.rs")).unwrap();
    let capture = std::fs::read_to_string(root.join("capture.rs")).unwrap();
    let fake_effect = std::fs::read_to_string(root.join("fake_effect.rs")).unwrap();
    let lanes = std::fs::read_to_string(root.join("lanes.rs")).unwrap();
    let bounded = std::fs::read_to_string(root.join("lanes/bounded.rs")).unwrap();

    assert!(!composition.contains("PmPublicRole::new"));
    assert!(!composition.contains("PmFixtureOwnedExecution::new"));
    assert!(capture.contains("PmPublicRole::new"));
    assert!(fake_effect.contains("PmFixtureOwnedExecution::new"));
    assert!(bounded.contains("BinaryHeap::with_capacity"));
    assert!(bounded.contains("HashSet::with_capacity"));
    assert!(!lanes.contains("Vec::new"));
}

#[test]
fn pm_production_modules_stay_below_the_review_size_limit() {
    for relative in [
        "../reap-pm-strategy/src",
        "../reap-pm-live-contracts/src",
        "../reap-polymarket-adapter/src",
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
