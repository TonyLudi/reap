#[test]
fn plan_boundaries_are_enforced_by_the_type_system() {
    let tests = trybuild::TestCases::new();
    tests.compile_fail("tests/ui/model_cannot_request_private_or_mutation.rs");
    tests.compile_fail("tests/ui/plan_entries_cannot_be_forged.rs");
    tests.compile_fail("tests/ui/fake_profile_cannot_select_order_features.rs");
}
