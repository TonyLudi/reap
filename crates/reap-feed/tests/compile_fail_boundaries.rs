#[test]
fn signed_private_bootstrap_authority_is_not_publicly_callable() {
    let tests = trybuild::TestCases::new();
    tests.compile_fail("tests/ui/bootstrap_factory_is_opaque.rs");
    tests.compile_fail("tests/ui/run_connection_once_is_private.rs");
}
