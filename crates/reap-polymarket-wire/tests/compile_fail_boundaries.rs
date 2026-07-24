#[test]
fn unsigned_order_is_output_only_checked_data() {
    let tests = trybuild::TestCases::new();
    tests.compile_fail("tests/ui/unsigned_order_fields_are_private.rs");
    tests.compile_fail("tests/ui/unsigned_order_has_no_deserialize.rs");
    tests.compile_fail("tests/ui/unsigned_order_has_no_signature_method.rs");
}
