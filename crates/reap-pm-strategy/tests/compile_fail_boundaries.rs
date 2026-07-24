#[test]
fn validated_candidate_cannot_be_field_forged() {
    let tests = trybuild::TestCases::new();
    tests.compile_fail("tests/ui/validated_candidate_fields_are_private.rs");
}
