use reap_capture_framing::{
    JsonlVerifyError, encode_jsonl_frame_bounded, read_bounded_regular_file,
    scan_jsonl_file_bounded, sha256_hex,
};
use serde::Serialize;

#[derive(Debug, Serialize)]
struct TestRecord {
    record: u64,
    label: &'static str,
}

#[test]
fn frame_bytes_are_canonical_json_followed_by_one_newline() {
    let bytes = encode_jsonl_frame_bounded(
        &TestRecord {
            record: 7,
            label: "alpha",
        },
        1024,
    )
    .unwrap();

    assert_eq!(
        bytes,
        br#"{"record":7,"label":"alpha"}"#.iter().copied().chain([b'\n']).collect::<Vec<_>>()
    );
    assert_eq!(
        sha256_hex(&bytes),
        "f4656ae20e597017c743cc772f697b2881e6bd67977818d93606732cd2131d60"
    );
}

#[test]
fn verifier_counts_and_hashes_a_trailing_partial_record() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("capture.jsonl");
    let bytes = b"{\"record\":1}\n{\"record\":2}";
    std::fs::write(&path, bytes).unwrap();

    let scan = scan_jsonl_file_bounded(&path, 1024, |_| true).unwrap();

    assert_eq!(scan.records, 2);
    assert_eq!(scan.bytes, bytes.len() as u64);
    assert_eq!(scan.sha256, sha256_hex(bytes));
    assert!(scan.has_trailing_partial_record);
    assert!(scan.stable_while_reading);
}

#[test]
fn verifier_rejects_oversize_frame_without_accepting_a_partial_record() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("oversize.jsonl");
    std::fs::write(&path, b"123456789\n").unwrap();

    let error = scan_jsonl_file_bounded(&path, 8, |_| true).unwrap_err();

    assert!(matches!(
        error,
        JsonlVerifyError::FrameTooLarge {
            record: 1,
            observed_at_least: 9,
            limit: 8,
            ..
        }
    ));
    assert_eq!(error.actual_bytes(), Some(9));
}

#[test]
fn verifier_rejects_symlinks_and_bounds_whole_file_reads() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("evidence.json");
    std::fs::write(&path, b"12345").unwrap();

    let error = read_bounded_regular_file(&path, 4).unwrap_err();
    assert_eq!(error.actual_bytes(), Some(5));

    #[cfg(unix)]
    {
        let link = directory.path().join("evidence-link.json");
        std::os::unix::fs::symlink(&path, &link).unwrap();
        assert!(read_bounded_regular_file(&link, 10).is_err());
    }
}
