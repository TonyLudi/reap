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
fn tracked_predarb_seed_and_provenance_are_pinned() {
    let seed = include_bytes!("../fixtures/market_book_predarb_seed.json");
    assert_eq!(
        format!("{:x}", Sha256::digest(seed)),
        "8e671f14c4b1e8137b1dc1b0bd7d39c79d9c8f961a8483daa32151df99cbdf81"
    );

    let provenance: serde_json::Value =
        serde_json::from_str(include_str!("../fixtures/provenance.json")).unwrap();
    assert_eq!(provenance["schema"], 1);
    assert_eq!(
        provenance["predarb_revision"],
        "8222273a9c72033b760e1d2fec813bc77144556d"
    );
    assert_eq!(
        provenance["tracked_sources"][0]["source_git_blob"],
        "bbb5bc143a914ba8c96d84342321b3dba30ec0fc"
    );
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
