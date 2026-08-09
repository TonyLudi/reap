use super::*;
use crate::journal::schema::{
    PM_MUTATION_JOURNAL_VERSION, PM_T2_PROXY_MUTATION_JOURNAL_VERSION, PmJournalSchemaError,
};

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

fn one_line_value(scope: &PmJournalScopeV1) -> serde_json::Value {
    serde_json::to_value(PmJournalLineV1::new_for_scope(
        scope,
        0,
        PmJournalRecordV1::Header(PmJournalHeaderV1::new(scope.clone())),
    ))
    .expect("encode header value")
}

fn line_bytes(value: &serde_json::Value) -> Vec<u8> {
    let mut bytes = serde_json::to_vec(value).expect("encode tampered line");
    bytes.push(b'\n');
    bytes
}

#[test]
fn envelope_version_must_match_the_expected_profile_domain() {
    let eoa = test_scope();
    let proxy = test_proxy_scope();

    let mut proxy_as_v1 = one_line_value(&proxy);
    proxy_as_v1[1] = serde_json::Value::from(PM_MUTATION_JOURNAL_VERSION);
    assert!(matches!(
        recover_lines(&mut Cursor::new(line_bytes(&proxy_as_v1)), &proxy),
        Err(PmJournalRecoveryError::ScopeVersionMismatch)
    ));

    let mut eoa_as_v2 = one_line_value(&eoa);
    eoa_as_v2[1] = serde_json::Value::from(PM_T2_PROXY_MUTATION_JOURNAL_VERSION);
    assert!(matches!(
        recover_lines(&mut Cursor::new(line_bytes(&eoa_as_v2)), &eoa),
        Err(PmJournalRecoveryError::ScopeVersionMismatch)
    ));
}

#[test]
fn cross_profile_scope_and_line_combinations_are_rejected() {
    let eoa = test_scope();
    let proxy = test_proxy_scope();

    let eoa_line = one_line_value(&eoa);
    assert!(matches!(
        recover_lines(&mut Cursor::new(line_bytes(&eoa_line)), &proxy),
        Err(PmJournalRecoveryError::ScopeVersionMismatch)
    ));

    let proxy_line = one_line_value(&proxy);
    assert!(matches!(
        recover_lines(&mut Cursor::new(line_bytes(&proxy_line)), &eoa),
        Err(PmJournalRecoveryError::ScopeVersionMismatch)
    ));

    let mut v2_without_profile = proxy_line.clone();
    v2_without_profile[4]["body"]["scope"]
        .as_object_mut()
        .expect("scope object")
        .remove("account_signature_profile");
    assert!(matches!(
        recover_lines(&mut Cursor::new(line_bytes(&v2_without_profile)), &proxy),
        Err(PmJournalRecoveryError::Schema(
            PmJournalSchemaError::WrongScopeDomain
        ))
    ));

    let mut v1_with_proxy_profile = eoa_line;
    v1_with_proxy_profile[4]["body"]["scope"]["account_signature_profile"] =
        serde_json::Value::String("proxy_type_1".to_owned());
    assert!(matches!(
        recover_lines(&mut Cursor::new(line_bytes(&v1_with_proxy_profile)), &eoa),
        Err(PmJournalRecoveryError::Schema(
            PmJournalSchemaError::WrongScopeDomain
        ))
    ));
}

#[test]
fn proxy_scope_profile_signer_funder_config_and_authority_tampering_rejects() {
    let proxy = test_proxy_scope();
    let pristine = one_line_value(&proxy);

    let mut wrong_profile = pristine.clone();
    wrong_profile[4]["body"]["scope"]["account_signature_profile"] =
        serde_json::Value::String("eoa_type_0".to_owned());
    assert!(matches!(
        recover_lines(&mut Cursor::new(line_bytes(&wrong_profile)), &proxy),
        Err(PmJournalRecoveryError::Json(_))
    ));

    for field in ["signer", "funder"] {
        let mut wrong_identity = pristine.clone();
        wrong_identity[4]["body"]["scope"]["account_scope"][field] =
            serde_json::Value::String("0x5555555555555555555555555555555555555555".to_owned());
        assert!(matches!(
            recover_lines(&mut Cursor::new(line_bytes(&wrong_identity)), &proxy),
            Err(PmJournalRecoveryError::Schema(
                PmJournalSchemaError::ScopeFingerprintMismatch
                    | PmJournalSchemaError::AccountIdentityMismatch
            ))
        ));
    }

    let mut wrong_config = pristine.clone();
    wrong_config[4]["body"]["scope"]["configuration_fingerprint"] =
        serde_json::Value::String("66".repeat(32));
    assert!(matches!(
        recover_lines(&mut Cursor::new(line_bytes(&wrong_config)), &proxy),
        Err(PmJournalRecoveryError::Schema(
            PmJournalSchemaError::ScopeFingerprintMismatch
        ))
    ));

    for authority in ["authentication_enabled", "production_authorized"] {
        let mut forbidden = pristine.clone();
        forbidden[4]["body"]["scope"][authority] = serde_json::Value::Bool(true);
        assert!(matches!(
            recover_lines(&mut Cursor::new(line_bytes(&forbidden)), &proxy),
            Err(PmJournalRecoveryError::Schema(
                PmJournalSchemaError::ForbiddenLiveAuthority
            ))
        ));
    }
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
