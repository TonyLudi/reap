#[test]
fn role_boundaries_are_enforced_by_the_type_system() {
    let tests = trybuild::TestCases::new();
    tests.compile_fail("tests/ui/public_role_has_no_mutation.rs");
    tests.compile_fail("tests/ui/private_read_roles_have_no_mutation.rs");
    tests.compile_fail("tests/ui/roles_are_not_interchangeable.rs");
    tests.compile_fail("tests/ui/owned_execution_role_is_linear.rs");
    tests.compile_fail("tests/ui/external_role_implementations_are_sealed.rs");
}
