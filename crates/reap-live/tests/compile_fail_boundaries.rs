#[test]
fn live_consumers_cannot_import_broad_authenticated_authority() {
    let tests = trybuild::TestCases::new();
    tests.compile_fail("tests/ui/no_broad_authenticated_authority.rs");
    tests.compile_fail("tests/ui/no_public_regular_mutation_actions.rs");
}
