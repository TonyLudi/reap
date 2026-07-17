#[test]
fn live_adapter_compile_fail_boundaries() {
    let tests = trybuild::TestCases::new();
    tests.compile_fail("tests/ui/observe_roles_no_mutation_authority.rs");
    tests.compile_fail("tests/ui/roles_no_raw_authority.rs");
    tests.compile_fail("tests/ui/roles_no_unsupported_mutations.rs");
    tests.compile_fail("tests/ui/order_command_config_has_no_raw_transport_or_shards.rs");
    tests.compile_fail("tests/ui/internal_order_authority_is_private.rs");
    tests.compile_fail("tests/ui/mutation_authorities_are_linear.rs");
}
