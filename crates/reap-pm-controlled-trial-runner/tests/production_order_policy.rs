const MAIN: &str = include_str!("../src/main.rs");
const AUTHORITY: &str = include_str!("../src/controlled_trial/authority/production_order_v1.rs");

#[test]
fn production_command_is_explicit_capped_and_one_shot() {
    for required in [
        "ProductionPlaceThenExactCancel",
        "authorization_phrase",
        "I_ACCEPT_TOTAL_LOSS_AND_ONE_REAL_POLYMARKET_ORDER",
        "credential_env",
        "state_directory",
    ] {
        assert!(MAIN.contains(required), "production CLI lost `{required}`");
    }
    for required in [
        "const MAX_TEST_QUANTITY: &str = \"5\";",
        "const TEST_PRICE: &str = \"0.01\";",
        "const MINIMUM_FAR_DISTANCE_UNITS: u32 = 200_000;",
        "PmBtcFiveMinuteMarketSource",
        "fetch_fresh_book",
        "PmDataApiCurrentPositionSource",
        "position_unchanged",
        "MAX_FILL_POSITION_RECONCILIATION_ATTEMPTS",
        "begin_exact_scope_trades(timestamp)",
        "PmTradesCutProgress::Complete",
        "apply_polled_fills",
        "fill_position_reconciliation_report",
        "fills_match_order_cumulative",
        "fill_ledger_reconciled",
        "venue_position_matches_fill_based",
        "authoritative_minus_fill_based_position",
        "ProductionOwnedOrderState::pending",
        "PmOwnedQuoteAdmission::Admitted",
        "PmOwnedSubmitResult::AmbiguousOwned",
        "PmOwnedOrderProgressObservation::new",
        "canonical_order_state_consistent",
        "exact_order_state_reconciliation",
        "reconcile_predarb_exact_order_v1",
        "exact_local_order_detail(timestamp, order_id)",
        ".create_new(true)",
        "placement_resumption_allowed\": false",
        "place_dispatch_allowance\": 1",
        "PmFixedPlaceProductionRole",
        "PmExactOwnedCancelProductionRole",
        "serialize_owned_cancel(expected_order_id.into())",
        "PmMutationClassification::AcknowledgementUnknown",
        "PmMutationClassification::OutOfProfile",
    ] {
        assert!(AUTHORITY.contains(required), "authority lost `{required}`");
    }
    assert_eq!(
        AUTHORITY.matches("place_transport.send(retained)").count(),
        1
    );
    assert_eq!(
        AUTHORITY
            .matches("cancel_transport.send(retained_cancel)")
            .count(),
        1
    );
    assert!(
        AUTHORITY
            .find("ProductionOwnedOrderState::pending")
            .unwrap()
            < AUTHORITY.find("place_transport.send(retained)").unwrap(),
        "canonical PM PendingNew ownership must exist before place IO"
    );
    assert!(
        AUTHORITY.find("request_cancel().ok()").unwrap()
            < AUTHORITY
                .find("cancel_transport.send(retained_cancel)")
                .unwrap(),
        "canonical PM cancel-pending state must exist before cancel IO"
    );
    for forbidden in ["cancel_all(", "cancel_market(", "send_batch(", ".retry("] {
        assert!(
            !AUTHORITY.contains(forbidden),
            "production one-shot authority gained `{forbidden}`"
        );
    }
}

#[test]
fn credential_source_is_protected_and_never_serialized() {
    for required in [
        "libc::O_NOFOLLOW",
        "metadata.mode() & 0o777 != 0o600",
        "metadata.nlink() != 1",
        "Zeroizing<String>",
        "POLYMARKET_PRIVATE_KEY",
        "POLYMARKET_API_SECRET",
    ] {
        assert!(
            AUTHORITY.contains(required),
            "credential custody lost `{required}`"
        );
    }
    for forbidden in [
        "private_key\":",
        "api_secret\":",
        "api_passphrase\":",
        "println!",
        "dbg!",
        "std::env::set_var",
    ] {
        assert!(
            !AUTHORITY.contains(forbidden),
            "credential source gained forbidden `{forbidden}`"
        );
    }
}
