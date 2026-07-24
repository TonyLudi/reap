use super::*;
use crate::journal::schema::PmJournalSchemaError;

fn header_bytes(scope: &PmJournalScopeV1) -> Vec<u8> {
    let mut bytes = Vec::new();
    append_line(
        &mut bytes,
        scope,
        0,
        PmJournalRecordV1::Header(PmJournalHeaderV1::new(scope.clone())),
    );
    bytes
}

#[test]
fn malformed_json_after_a_valid_prefix_is_rejected() {
    let scope = test_scope();
    let mut bytes = header_bytes(&scope);
    bytes.extend_from_slice(br#"["reap-pm-mutation-journal",1,{"corrupt":]"#);
    bytes.push(b'\n');

    assert!(matches!(
        recover_lines(&mut Cursor::new(bytes), &scope),
        Err(PmJournalRecoveryError::Json(_))
    ));
}

#[test]
fn newline_missing_from_an_otherwise_complete_tail_is_rejected() {
    let scope = test_scope();
    let mut bytes = header_bytes(&scope);
    assert_eq!(bytes.pop(), Some(b'\n'));

    assert!(matches!(
        recover_lines(&mut Cursor::new(bytes), &scope),
        Err(PmJournalRecoveryError::TruncatedTail)
    ));
}

#[test]
fn duplicate_sequence_replay_is_rejected_before_duplicate_record_reduction() {
    let scope = test_scope();
    let intent = quote(&scope, 1, PmOrderSide::Buy, "1");
    let mut bytes = header_bytes(&scope);
    append_line(
        &mut bytes,
        &scope,
        1,
        PmJournalRecordV1::QuoteIntent(intent),
    );
    append_line(
        &mut bytes,
        &scope,
        1,
        PmJournalRecordV1::QuoteIntent(intent),
    );

    assert!(matches!(
        recover_lines(&mut Cursor::new(bytes), &scope),
        Err(PmJournalRecoveryError::NonContiguousSequence {
            expected: 2,
            actual: 1
        })
    ));
}

#[test]
fn noncontiguous_sequence_gap_is_rejected() {
    let scope = test_scope();
    let mut bytes = header_bytes(&scope);
    append_line(
        &mut bytes,
        &scope,
        2,
        PmJournalRecordV1::QuoteIntent(quote(&scope, 1, PmOrderSide::Buy, "1")),
    );

    assert!(matches!(
        recover_lines(&mut Cursor::new(bytes), &scope),
        Err(PmJournalRecoveryError::NonContiguousSequence {
            expected: 1,
            actual: 2
        })
    ));
}

#[test]
fn line_scope_fingerprint_outside_the_expected_lease_tuple_is_rejected() {
    let scope = test_scope();
    let wrong_scope = PmJournalFingerprintV1::from_bytes([0x99; 32]);
    let mut bytes = Vec::new();
    serde_json::to_writer(
        &mut bytes,
        &PmJournalLineV1::new(
            wrong_scope,
            0,
            PmJournalRecordV1::Header(PmJournalHeaderV1::new(scope.clone())),
        ),
    )
    .expect("encode wrong-scope line");
    bytes.push(b'\n');

    assert!(matches!(
        recover_lines(&mut Cursor::new(bytes), &scope),
        Err(PmJournalRecoveryError::ScopeMismatch)
    ));
}

#[test]
fn tampered_header_scope_fingerprint_is_rejected() {
    let scope = test_scope();
    let line = PmJournalLineV1::new(
        scope.fingerprint(),
        0,
        PmJournalRecordV1::Header(PmJournalHeaderV1::new(scope.clone())),
    );
    let mut value = serde_json::to_value(line).expect("encode header value");
    value[4]["body"]["scope"]["scope_fingerprint"] = serde_json::Value::String("00".repeat(32));
    let mut bytes = serde_json::to_vec(&value).expect("encode tampered header");
    bytes.push(b'\n');

    assert!(matches!(
        recover_lines(&mut Cursor::new(bytes), &scope),
        Err(PmJournalRecoveryError::Schema(
            PmJournalSchemaError::ScopeFingerprintMismatch
        ))
    ));
}

#[test]
fn pm_recovery_rejects_the_existing_chaos_schema_seven_envelope() {
    // This is the frozen schema-seven raw envelope asserted by
    // `reap-storage::schema_seven_codec_preserves_the_frozen_raw_envelope_bytes`.
    let chaos = br#"{"schema_version":7,"record":{"kind":"raw","data":{"account_id":null,"envelope":{"venue":"okx","conn_id":"test","channel":"books","symbol":"BTC-USDT","recv_ts_ns":1,"raw_hash":2,"payload":"{}"}}}}
"#;
    let scope = test_scope();

    assert!(matches!(
        recover_lines(&mut Cursor::new(chaos), &scope),
        Err(PmJournalRecoveryError::WrongEnvelopeShape)
    ));
}
