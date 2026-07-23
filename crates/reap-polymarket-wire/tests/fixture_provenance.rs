use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use serde::Deserialize;
use sha2::{Digest, Sha256};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FixtureManifest {
    schema: u64,
    fixtures: Vec<FixtureEntry>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FixtureEntry {
    path: String,
    sha256: String,
    provenance: String,
    purpose: String,
    expected: String,
}

#[test]
fn tracked_predarb_seeds_and_provenance_are_pinned() {
    let provenance: serde_json::Value =
        serde_json::from_str(include_str!("../fixtures/provenance.json")).unwrap();
    assert_eq!(provenance["schema"], 1);
    assert_eq!(
        provenance["predarb_revision"],
        "8222273a9c72033b760e1d2fec813bc77144556d"
    );

    let tracked_sources = provenance["tracked_sources"].as_array().unwrap();
    let tracked_names = tracked_sources
        .iter()
        .map(|entry| entry["fixture"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert!(
        tracked_names.windows(2).all(|pair| pair[0] < pair[1]),
        "tracked provenance stays sorted by fixture"
    );

    let fixture_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures");
    for (fixture, source, blob, sha256, bytes) in [
        (
            "market_book_predarb_seed.json",
            "crates/venue-polymarket/tests/fixtures/market_book.json",
            "bbb5bc143a914ba8c96d84342321b3dba30ec0fc",
            "8e671f14c4b1e8137b1dc1b0bd7d39c79d9c8f961a8483daa32151df99cbdf81",
            190_u64,
        ),
        (
            "predarb_balance_allowance_seed.json",
            "crates/venue-polymarket/tests/fixtures/balance_allowance.json",
            "2f5fd6cd5d447d988d6eecd854afee5ae7196810",
            "7e1f683ac5032b137d8a2afdfafccce389198bb5d3a33ba6eb3cb478455fab96",
            53,
        ),
        (
            "predarb_open_order_seed.json",
            "crates/venue-polymarket/tests/fixtures/open_order.json",
            "a544ba4378c8cd5fdbf6fafc890846746906867f",
            "d0998ca29cf47ce4bcb1fb4d7183d1e895a044d859235230a6ebef464295baf2",
            247,
        ),
        (
            "predarb_user_order_seed.json",
            "crates/venue-polymarket/tests/fixtures/user_order.json",
            "a4e89e7dada536937e1e8c8f77fac82d9d4bbb24",
            "e4c3cd7975b7dc16c4c8d014444fc2a96d927cf1b9089b33875a5450b4ff99fa",
            295,
        ),
        (
            "predarb_user_trade_seed.json",
            "crates/venue-polymarket/tests/fixtures/user_trade.json",
            "d8084942ba3a5fa88518d18b8f06e26c9d0422ad",
            "042998055ec5dec2c69065d002b2619d8497faabd9bfcc36c27a1bcf7cfe224c",
            278,
        ),
    ] {
        let entry = tracked_sources
            .iter()
            .find(|entry| entry["fixture"] == fixture)
            .unwrap();
        assert_eq!(entry["source"], source, "{fixture}");
        assert_eq!(entry["source_git_blob"], blob, "{fixture}");
        assert_eq!(entry["source_sha256"], sha256, "{fixture}");
        assert_eq!(entry["source_bytes"], bytes, "{fixture}");

        let seed = std::fs::read(fixture_root.join(fixture)).unwrap();
        assert_eq!(seed.len() as u64, bytes, "{fixture}");
        assert_eq!(format!("{:x}", Sha256::digest(seed)), sha256, "{fixture}");
    }
}

#[test]
fn every_fixture_payload_is_declared_and_sha256_pinned() {
    let manifest: FixtureManifest =
        serde_json::from_str(include_str!("../fixtures/fixture_manifest.json")).unwrap();
    assert_eq!(manifest.schema, 1);

    let fixture_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures");
    let actual_paths = std::fs::read_dir(&fixture_root)
        .unwrap()
        .map(|entry| entry.unwrap())
        .filter(|entry| entry.file_type().unwrap().is_file())
        .map(|entry| entry.file_name().into_string().unwrap())
        .filter(|path| path != "fixture_manifest.json")
        .collect::<BTreeSet<_>>();

    let mut declared = BTreeMap::new();
    let mut previous = None::<String>;
    for entry in manifest.fixtures {
        assert_eq!(
            Path::new(&entry.path).components().count(),
            1,
            "fixture paths remain local to the checked-in fixture directory"
        );
        assert_ne!(entry.path, "fixture_manifest.json");
        assert_eq!(entry.sha256.len(), 64);
        assert!(
            entry
                .sha256
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        );
        assert!(!entry.provenance.trim().is_empty());
        assert!(!entry.purpose.trim().is_empty());
        assert!(!entry.expected.trim().is_empty());
        if let Some(previous) = &previous {
            assert!(
                previous.as_str() < entry.path.as_str(),
                "manifest paths stay sorted"
            );
        }
        previous = Some(entry.path.clone());

        let bytes = std::fs::read(fixture_root.join(&entry.path)).unwrap();
        assert_eq!(
            format!("{:x}", Sha256::digest(bytes)),
            entry.sha256,
            "{}",
            entry.path
        );
        assert!(
            declared.insert(entry.path, entry.expected).is_none(),
            "fixture manifest paths are unique"
        );
    }

    assert_eq!(
        declared.keys().cloned().collect::<BTreeSet<_>>(),
        actual_paths,
        "new fixture payloads cannot bypass provenance, purpose, expected outcome, and SHA-256 review"
    );
}
