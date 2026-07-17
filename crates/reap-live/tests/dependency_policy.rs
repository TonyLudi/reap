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
        "reap-live",
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
    assert_direct(&packages, "reap-live", "reap-okx-live-adapter");
    assert_not_direct(&packages, "reap-live", "reap-okx-wire");
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
            assert_not_direct(&packages, consumer, authority);
        }
    }

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
