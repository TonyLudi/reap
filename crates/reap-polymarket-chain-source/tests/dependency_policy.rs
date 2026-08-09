use std::collections::BTreeSet;
use std::path::PathBuf;

#[test]
fn dependency_edges_are_exactly_the_read_only_chain_source_set() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let manifest = std::fs::read_to_string(root.join("Cargo.toml")).unwrap();

    let production = manifest
        .split("[dependencies]")
        .nth(1)
        .unwrap()
        .split("[dev-dependencies]")
        .next()
        .unwrap()
        .lines()
        .filter_map(|line| {
            line.split_once('.')
                .map(|(name, _)| name.trim().to_string())
        })
        .collect::<BTreeSet<_>>();
    assert_eq!(
        production,
        BTreeSet::from([
            "reap-pm-core".to_string(),
            "reqwest".to_string(),
            "serde".to_string(),
            "serde_json".to_string(),
            "thiserror".to_string(),
            "url".to_string(),
        ])
    );
    assert!(manifest.contains("default = []"));
    assert!(manifest.contains("loopback-evidence = []"));
    for forbidden in [
        "reap-polymarket-auth",
        "reap-polymarket-wire",
        "reap-polymarket-live-adapter",
        "k256",
        "hmac",
        "sha3",
        "zeroize",
    ] {
        assert!(!manifest.contains(forbidden), "forbidden edge {forbidden}");
    }
}
