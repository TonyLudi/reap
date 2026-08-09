use std::{collections::BTreeSet, fs, path::PathBuf};

fn package_manifest() -> toml::Value {
    toml::from_str(include_str!("../Cargo.toml")).expect("readiness Cargo.toml must parse")
}

fn workspace_manifest() -> toml::Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../Cargo.toml");
    let source = fs::read_to_string(path).expect("workspace Cargo.toml must be readable");
    toml::from_str(&source).expect("workspace Cargo.toml must parse")
}

fn dependency_edges(manifest: &toml::Value) -> Vec<(String, String, toml::Value)> {
    fn visit(value: &toml::Value, path: &str, edges: &mut Vec<(String, String, toml::Value)>) {
        let Some(table) = value.as_table() else {
            return;
        };
        for (key, child) in table {
            let child_path = if path.is_empty() {
                key.clone()
            } else {
                format!("{path}.{key}")
            };
            if matches!(
                key.as_str(),
                "dependencies" | "dev-dependencies" | "build-dependencies"
            ) {
                let dependencies = child
                    .as_table()
                    .unwrap_or_else(|| panic!("{child_path} must be a dependency table"));
                for (name, specification) in dependencies {
                    edges.push((child_path.clone(), name.clone(), specification.clone()));
                }
            } else {
                visit(child, &child_path, edges);
            }
        }
    }

    let mut edges = Vec::new();
    visit(manifest, "", &mut edges);
    edges
}

fn is_production_edge(section: &str) -> bool {
    section == "dependencies" || section.ends_with(".dependencies")
}

fn features(specification: &toml::Value) -> BTreeSet<&str> {
    specification
        .as_table()
        .and_then(|table| table.get("features"))
        .and_then(toml::Value::as_array)
        .into_iter()
        .flatten()
        .map(|feature| {
            feature
                .as_str()
                .expect("dependency features must be strings")
        })
        .collect()
}

fn forbidden_direct_dependency(name: &str) -> bool {
    matches!(
        name,
        "reap-polymarket-auth"
            | "reap-pm-live"
            | "reap-pm-live-contracts"
            | "reap-pm-state"
            | "reap-order"
            | "reap-storage"
            | "reap-durable-writer"
            | "reap-live"
            | "reap-live-contracts"
            | "reap-engine"
            | "reap-risk"
            | "reqwest"
            | "hyper"
            | "ureq"
            | "surf"
            | "isahc"
    ) || name.contains("strategy")
        || name.contains("journal")
        || name.contains("mutation")
}

#[test]
fn direct_manifest_graph_has_no_auth_product_mutation_or_http_client_edge() {
    let edges = dependency_edges(&package_manifest());
    assert!(!edges.is_empty(), "readiness dependency graph is empty");
    for (section, name, _) in &edges {
        assert!(
            !forbidden_direct_dependency(name),
            "forbidden direct readiness dependency in {section}: {name}"
        );
        if is_production_edge(section) {
            assert!(
                !matches!(name.as_str(), "tokio-tungstenite" | "tungstenite"),
                "production readiness networking must enter only through the live adapter: {name}"
            );
        }
    }
}

#[test]
fn default_live_adapter_edge_enables_no_evidence_or_mutation_feature() {
    let manifest = package_manifest();
    let edges = dependency_edges(&manifest);
    let mut production_adapter_edges = edges.iter().filter(|(section, name, _)| {
        is_production_edge(section) && name == "reap-polymarket-live-adapter"
    });
    let (_, _, adapter) = production_adapter_edges
        .next()
        .expect("one production live-adapter dependency is required");
    assert!(
        production_adapter_edges.next().is_none(),
        "production live-adapter edge must be unique"
    );
    let table = adapter
        .as_table()
        .expect("production live-adapter dependency must be an inline table");
    assert_eq!(
        table.get("workspace").and_then(toml::Value::as_bool),
        Some(true)
    );
    assert!(
        features(adapter).is_empty(),
        "default live-adapter edge must not enable any feature"
    );

    if let Some(package_features) = manifest.get("features").and_then(toml::Value::as_table) {
        for (feature, members) in package_features {
            let rendered = members.to_string();
            assert!(
                !rendered.contains("loopback-evidence") && !rendered.contains("read-only-evidence"),
                "package feature {feature} must not forward an adapter evidence feature"
            );
        }
    }

    let workspace = workspace_manifest();
    let workspace_adapter = workspace
        .get("workspace")
        .and_then(|value| value.get("dependencies"))
        .and_then(|value| value.get("reap-polymarket-live-adapter"))
        .expect("workspace live-adapter dependency must exist");
    assert!(
        features(workspace_adapter).is_empty(),
        "workspace live-adapter edge must not inject an evidence feature"
    );
}

#[test]
fn read_only_evidence_is_test_only_and_never_enables_loopback_mutation() {
    let edges = dependency_edges(&package_manifest());
    let dev_adapter = edges
        .iter()
        .find(|(section, name, _)| {
            (section == "dev-dependencies" || section.ends_with(".dev-dependencies"))
                && name == "reap-polymarket-live-adapter"
        })
        .expect("read-only loopback tests require one dev live-adapter edge");
    let enabled = features(&dev_adapter.2);
    assert_eq!(
        enabled,
        BTreeSet::from(["read-only-evidence"]),
        "dev adapter edge must enable only the read-only evidence feature"
    );
    assert!(!enabled.contains("loopback-evidence"));
}
