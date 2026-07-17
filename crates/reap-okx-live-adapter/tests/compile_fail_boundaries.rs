#[test]
fn live_adapter_compile_fail_boundaries() {
    let tests = trybuild::TestCases::new();
    tests.compile_fail("tests/ui/observe_roles_no_mutation_authority.rs");
    tests.compile_fail("tests/ui/roles_no_raw_authority.rs");
    tests.compile_fail("tests/ui/roles_no_unsupported_mutations.rs");
}
