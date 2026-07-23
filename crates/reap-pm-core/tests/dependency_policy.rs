use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::Path;
use std::process::Command;

use serde_json::Value;

#[derive(Debug)]
struct Package {
    id: String,
    production_dependencies: BTreeSet<String>,
    binary_targets: BTreeSet<String>,
}

#[test]
fn pm_core_is_a_pure_leaf_domain_crate() {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("reap-pm-core must be inside the workspace crates directory");
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

    let pm = packages
        .get("reap-pm-core")
        .expect("reap-pm-core must be a workspace package");
    assert_eq!(
        pm.production_dependencies,
        BTreeSet::from([
            "reap-core".to_string(),
            "serde".to_string(),
            "sha2".to_string(),
            "thiserror".to_string(),
        ])
    );
    assert!(
        pm.binary_targets.is_empty(),
        "the pure PM domain must not provide a binary"
    );

    let resolved = resolved_production_edges(&metadata, &packages);
    for forbidden in [
        "reap-venue",
        "reap-feed",
        "reap-order",
        "reap-storage",
        "reap-telemetry",
        "reap-live-contracts",
        "reap-live",
        "reap-okx-wire",
        "reap-okx-live-adapter",
        "reap-okx-evidence-adapter",
        "reap-okx-emergency-adapter",
    ] {
        assert_not_reachable(&packages, &resolved, "reap-pm-core", forbidden);
    }

    for name in [
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
        let package = packages
            .get(name)
            .unwrap_or_else(|| panic!("missing Phase 0 package {name}"));
        assert!(
            !package.production_dependencies.contains("reap-pm-core"),
            "Phase 1 does not authorize existing package {name} to depend on reap-pm-core"
        );
    }
}

#[test]
fn pm_core_source_has_no_runtime_or_authority_escape_hatch() {
    let source_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut sources = std::fs::read_dir(&source_root)
        .expect("read pm-core source directory")
        .map(|entry| entry.expect("source entry").path())
        .filter(|path| path.extension().and_then(|value| value.to_str()) == Some("rs"))
        .collect::<Vec<_>>();
    sources.sort();

    for path in sources {
        let source = std::fs::read_to_string(&path).expect("read pm-core source");
        for forbidden in [
            "f64",
            "serde_json::Value",
            "std::fs",
            "std::net",
            "std::time",
            "tokio",
            "reqwest",
            "tungstenite",
            "ApiKey",
            "PrivateKey",
            "Credential",
            "SignedRequest",
            "Arc<Mutex",
            "Arc<RwLock",
            "Box<dyn",
            "Vec<",
            "HashMap",
            "wrapping_",
            "saturating_",
            "unsafe ",
        ] {
            assert!(
                !source.contains(forbidden),
                "{} contains forbidden pure-domain token {forbidden:?}",
                path.display()
            );
        }
    }
}

fn resolved_production_edges(
    metadata: &Value,
    packages: &BTreeMap<String, Package>,
) -> BTreeMap<String, BTreeSet<String>> {
    let names_by_id = packages
        .iter()
        .map(|(name, package)| (package.id.clone(), name.clone()))
        .collect::<BTreeMap<_, _>>();
    let nodes = metadata["resolve"]["nodes"]
        .as_array()
        .expect("resolved dependency nodes");
    let mut edges = BTreeMap::new();
    for node in nodes {
        let Some(source) = node["id"]
            .as_str()
            .and_then(|id| names_by_id.get(id))
            .cloned()
        else {
            continue;
        };
        let targets = node["deps"]
            .as_array()
            .expect("resolved dependency entries")
            .iter()
            .filter(|dependency| {
                let dep_kinds = dependency["dep_kinds"]
                    .as_array()
                    .expect("dependency kinds");
                dep_kinds
                    .iter()
                    .any(|kind| kind["kind"].is_null() || kind["kind"].as_str() == Some("normal"))
            })
            .filter_map(|dependency| {
                dependency["pkg"]
                    .as_str()
                    .and_then(|id| names_by_id.get(id))
                    .cloned()
            })
            .collect();
        edges.insert(source, targets);
    }
    edges
}

fn assert_not_reachable(
    packages: &BTreeMap<String, Package>,
    edges: &BTreeMap<String, BTreeSet<String>>,
    source: &str,
    target: &str,
) {
    assert!(packages.contains_key(source), "missing package {source}");
    assert!(packages.contains_key(target), "missing package {target}");
    let mut seen = BTreeSet::new();
    let mut queue = VecDeque::from([source.to_string()]);
    while let Some(package) = queue.pop_front() {
        if !seen.insert(package.clone()) {
            continue;
        }
        if package == target {
            panic!("{source} reaches forbidden package {target}");
        }
        if let Some(dependencies) = edges.get(&package) {
            queue.extend(dependencies.iter().cloned());
        }
    }
}
