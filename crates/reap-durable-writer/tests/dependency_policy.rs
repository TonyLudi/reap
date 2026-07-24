use std::collections::BTreeSet;
use std::path::Path;
use std::process::Command;

use serde_json::Value;

fn rust_item<'a>(source: &'a str, marker: &str) -> &'a str {
    let start = source
        .find(marker)
        .expect("item marker must remain present");
    let source = &source[start..];
    let open = source.find('{').expect("item must have a body");
    let mut depth = 0_usize;
    for (offset, byte) in source.as_bytes()[open..].iter().copied().enumerate() {
        match byte {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return &source[..=open + offset];
                }
            }
            _ => {}
        }
    }
    panic!("item body must be balanced");
}

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

#[test]
fn durable_reservation_commit_and_receipt_poll_never_block_the_caller() {
    let source = include_str!("../src/bounded.rs");
    for marker in [
        "pub fn try_reserve_durable(",
        "pub fn commit(self, record: Record) -> DurableReceipt",
        "pub fn try_result(mut self) -> DurableReceiptPoll",
    ] {
        let item = rust_item(source, marker);
        for forbidden in [
            ".await",
            "block_on(",
            "spawn(",
            "sleep(",
            "reserve().",
            "send().await",
        ] {
            assert!(
                !item.contains(forbidden),
                "{marker} must not contain blocking operation {forbidden}"
            );
        }
    }
}
