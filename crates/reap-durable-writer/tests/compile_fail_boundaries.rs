#[test]
fn durable_authority_is_take_once_and_unforgeable() {
    let tests = trybuild::TestCases::new();
    tests.compile_fail("tests/ui/durable_reservation_is_move_only.rs");
    tests.compile_fail("tests/ui/durable_receipt_is_move_only.rs");
    tests.compile_fail("tests/ui/durable_acknowledgement_is_move_only.rs");
    tests.compile_fail("tests/ui/durable_acknowledgement_cannot_be_forged.rs");
}
