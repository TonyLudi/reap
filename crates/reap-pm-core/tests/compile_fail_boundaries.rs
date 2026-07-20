#[test]
fn pm_core_checked_values_and_typed_envelopes_cannot_be_bypassed() {
    let tests = trybuild::TestCases::new();
    tests.compile_fail("tests/ui/unchecked_numeric_constructors_are_private.rs");
    tests.compile_fail("tests/ui/exact_order_amounts_cannot_be_forged.rs");
    tests.compile_fail("tests/ui/exact_order_amount_fields_are_private.rs");
    tests.compile_fail("tests/ui/client_order_id_representation_is_private.rs");
    tests.compile_fail("tests/ui/reference_mapping_fields_are_private.rs");
    tests.compile_fail("tests/ui/event_envelope_payload_cannot_be_erased.rs");
    tests.compile_fail("tests/ui/event_envelope_fields_are_private.rs");
    tests.compile_fail("tests/ui/pm_state_consumer_cannot_mint_unchecked_values.rs");
    tests.compile_fail("tests/ui/pm_strategy_consumer_cannot_access_raw_auth.rs");
    tests.compile_fail("tests/ui/price_deserialization_cannot_mint_off_grid_candidate.rs");
}
