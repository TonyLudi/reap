#[test]
fn regular_order_authority_cannot_be_forged_or_bypassed() {
    let tests = trybuild::TestCases::new();
    tests.compile_fail("tests/ui/regular_authority_tokens_are_opaque.rs");
    tests.compile_fail("tests/ui/regular_authority_token_fields_are_private.rs");
    tests.compile_fail("tests/ui/no_raw_regular_gateway.rs");
    tests.compile_fail("tests/ui/generated_client_order_id_is_linear.rs");
    tests.compile_fail("tests/ui/recovered_ownership_requires_storage_proof.rs");
}
