use std::collections::BTreeSet;
use std::path::Path;
use std::process::Command;

use serde_json::Value;

#[test]
fn durable_writer_is_schema_and_product_neutral() {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .unwrap();
    let output = Command::new(env!("CARGO"))
        .args(["metadata", "--locked", "--format-version=1"])
        .current_dir(workspace)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let metadata: Value = serde_json::from_slice(&output.stdout).unwrap();
    let package = metadata["packages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|package| package["name"].as_str() == Some("reap-durable-writer"))
        .unwrap();
    let production_dependencies = package["dependencies"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|dependency| dependency["kind"].as_str() != Some("dev"))
        .map(|dependency| dependency["name"].as_str().unwrap())
        .collect::<BTreeSet<_>>();
    assert_eq!(
        production_dependencies,
        BTreeSet::from(["thiserror", "tokio"])
    );

    let source_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let modules = std::fs::read_dir(&source_root)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().and_then(|value| value.to_str()) == Some("rs"))
        .map(|path| path.file_stem().unwrap().to_str().unwrap().to_string())
        .collect::<BTreeSet<_>>();
    assert_eq!(
        modules,
        BTreeSet::from([
            "bounded".to_string(),
            "lease".to_string(),
            "lib".to_string(),
            "progress".to_string(),
            "writer".to_string(),
        ])
    );
    for entry in std::fs::read_dir(source_root).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().and_then(|value| value.to_str()) != Some("rs") {
            continue;
        }
        let source = std::fs::read_to_string(&path).unwrap();
        let production_source = source
            .split_once("#[cfg(test)]")
            .map_or(source.as_str(), |(production_source, _)| production_source);
        for forbidden in [
            "StorageRecord",
            "Pm",
            "Polymarket",
            "Venue",
            "OrderIntent",
            "Credential",
            "ApiKey",
            "PrivateKey",
            "Box<dyn",
            "serde_json",
        ] {
            assert!(
                !production_source.contains(forbidden),
                "{} contains forbidden product/schema token {forbidden}",
                path.display()
            );
        }
    }
}
