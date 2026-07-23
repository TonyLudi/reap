mod support;

use reap_polymarket_wire::{
    PmWireError, SnapshotHash, compute_snapshot_hash, verify_snapshot_hash,
};

#[test]
fn pinned_official_vector_uses_exact_raw_lexemes_and_array_order() {
    let raw = include_bytes!("../fixtures/official_snapshot_hash_vector.json");
    let expected = SnapshotHash::parse_hex("0458ea5755c9f73d64a14636fa5c36ed460ec394").unwrap();

    assert_eq!(compute_snapshot_hash(raw).unwrap(), expected);
    assert_eq!(verify_snapshot_hash(raw).unwrap(), expected);
}

#[test]
fn lexical_or_array_order_changes_are_hash_visible() {
    let original = include_str!("../fixtures/official_snapshot_hash_vector.json");
    let lexical = original.replace("\"0.3\"", "\"0.30\"");
    let reordered = original.replace(
        r#"{"price": "0.6", "size": "100"},
    {"price": "0.7", "size": "100"}"#,
        r#"{"price": "0.7", "size": "100"},
    {"price": "0.6", "size": "100"}"#,
    );

    assert_ne!(
        compute_snapshot_hash(lexical.as_bytes()).unwrap(),
        compute_snapshot_hash(original.as_bytes()).unwrap()
    );
    assert_ne!(
        compute_snapshot_hash(reordered.as_bytes()).unwrap(),
        compute_snapshot_hash(original.as_bytes()).unwrap()
    );
}

#[test]
fn malformed_or_noncanonical_snapshot_hash_fails_closed() {
    let raw = include_str!("../fixtures/official_snapshot_hash_vector.json");
    let wrong = raw.replace(
        "0458ea5755c9f73d64a14636fa5c36ed460ec394",
        "1458ea5755c9f73d64a14636fa5c36ed460ec394",
    );
    assert!(matches!(
        verify_snapshot_hash(wrong.as_bytes()),
        Err(PmWireError::SnapshotHashMismatch { .. })
    ));

    let uppercase = raw.replace(
        "0458ea5755c9f73d64a14636fa5c36ed460ec394",
        "0458EA5755C9F73D64A14636FA5C36ED460EC394",
    );
    assert_eq!(
        verify_snapshot_hash(uppercase.as_bytes()),
        Err(PmWireError::NonCanonicalSnapshotHash)
    );
}
