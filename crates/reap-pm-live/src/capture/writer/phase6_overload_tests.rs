use super::*;

#[test]
fn sealed_raw_entry_row_is_8193_one_byte_attempts_with_one_resync_failure() {
    let mut capacity = RawCaptureCapacity::default();
    for _ in 0..8_192 {
        let next = capacity.preflight(1).expect("first 8192 entries");
        capacity.commit(next);
    }
    assert!(matches!(
        capacity.preflight(1),
        Err(PmCaptureVerifyError::TooManyRawFrames)
    ));
    assert_eq!(capacity.frames, 8_192);
    assert_eq!(capacity.payload_bytes, 8_192);
    assert_eq!(std::mem::size_of_val(&capacity), 16);
}

#[test]
fn pending_capture_record_age_is_inclusive_and_fails_one_nanosecond_late() {
    let mut inclusive = PendingCaptureRecordTimes::new();
    assert_eq!(
        inclusive.reserved_capacity_bytes(),
        8_192 * std::mem::size_of::<u64>()
    );
    inclusive.push_back(100).expect("one pending record");
    assert_eq!(
        inclusive
            .preflight_depth(1, 100 + MAX_PM_PUBLIC_CAPTURE_PENDING_AGE_NS)
            .expect("the exact pending-age boundary is inclusive"),
        1
    );
    assert_eq!(inclusive.len, 1);

    let mut exceeded = PendingCaptureRecordTimes::new();
    exceeded.push_back(100).expect("one pending record");
    assert!(matches!(
        exceeded.preflight_depth(1, 100 + MAX_PM_PUBLIC_CAPTURE_PENDING_AGE_NS + 1),
        Err(PmCaptureWriteError::CaptureAged {
            observed_age_ns,
            maximum_age_ns,
        }) if observed_age_ns == MAX_PM_PUBLIC_CAPTURE_PENDING_AGE_NS + 1
            && maximum_age_ns == MAX_PM_PUBLIC_CAPTURE_PENDING_AGE_NS
    ));
    assert_eq!(exceeded.len, 1);
}
