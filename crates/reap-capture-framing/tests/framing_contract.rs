use reap_capture_framing::{
    JsonlVerifyError, encode_jsonl_frame_bounded, measure_jsonl_frame_bounded,
    read_bounded_regular_file, scan_jsonl_file_bounded, scan_jsonl_file_bounded_total, sha256_hex,
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
    assert_eq!(
        measure_jsonl_frame_bounded(
            &TestRecord {
                record: 7,
                label: "alpha",
            },
            1024,
        )
        .unwrap(),
        bytes.len()
    );
}

#[test]
fn frame_measurement_enforces_the_same_exact_bound_without_returning_bytes() {
    let record = TestRecord {
        record: 7,
        label: "alpha",
    };
    let exact = br#"{"record":7,"label":"alpha"}"#.len() + 1;

    assert_eq!(measure_jsonl_frame_bounded(&record, exact).unwrap(), exact);
    assert!(matches!(
        measure_jsonl_frame_bounded(&record, exact - 1),
        Err(reap_capture_framing::BoundedJsonlFrameError::FrameTooLarge {
            limit_bytes,
            ..
        }) if limit_bytes == exact - 1
    ));
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

#[test]
fn verifier_total_bound_accepts_exact_bytes_and_rejects_one_byte_less() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("bounded.jsonl");
    let bytes = b"one\ntwo\n";
    std::fs::write(&path, bytes).unwrap();

    let scan = scan_jsonl_file_bounded_total(&path, 1024, bytes.len() as u64, |_| true).unwrap();
    assert_eq!(scan.bytes, bytes.len() as u64);
    assert_eq!(scan.records, 2);

    let error =
        scan_jsonl_file_bounded_total(&path, 1024, bytes.len() as u64 - 1, |_| true).unwrap_err();
    assert!(matches!(
        error,
        JsonlVerifyError::InputTooLarge {
            actual,
            limit,
            ..
        } if actual == bytes.len() as u64 && limit == bytes.len() as u64 - 1
    ));
}

#[cfg(unix)]
#[test]
fn bounded_scanner_rejects_a_symbolic_link_input() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("capture.jsonl");
    let link = directory.path().join("capture-link.jsonl");
    std::fs::write(&path, b"{\"record\":1}\n").unwrap();
    std::os::unix::fs::symlink(&path, &link).unwrap();

    let error = scan_jsonl_file_bounded_total(&link, 1024, 1024, |_| true).unwrap_err();
    assert!(matches!(error, JsonlVerifyError::InvalidInputPath { .. }));
}
