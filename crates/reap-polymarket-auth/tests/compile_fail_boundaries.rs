#[test]
fn auth_authorities_are_linear_and_purpose_scoped() {
    let cases = trybuild::TestCases::new();
    cases.compile_fail("tests/ui/secret_authorities_are_not_clone_or_serialize.rs");
    cases.compile_fail("tests/ui/no_generic_signing_or_request_escape.rs");
    cases.compile_fail("tests/ui/serialized_mutations_are_sealed_and_linear.rs");
    cases.compile_fail("tests/ui/credential_owned_user_frame_cannot_be_forged.rs");
    cases.compile_fail("tests/ui/l1_credential_derivation_is_sealed_and_linear.rs");
    cases.compile_fail("tests/ui/l1_credential_derivation_response_is_opaque_and_linear.rs");
    cases.compile_fail("tests/ui/l1_matched_closed_only_is_opaque_and_linear.rs");
    cases.compile_fail("tests/ui/l1_matched_closed_only_fields_are_private.rs");
    cases.compile_fail("tests/ui/l1_matched_closed_only_has_no_recovery_methods.rs");
    cases.compile_fail("tests/ui/l1_matched_closed_only_has_no_trait_escape.rs");
    cases.compile_fail("tests/ui/l1_matched_closed_only_request_cannot_retry.rs");
}
