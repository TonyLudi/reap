#[test]
fn venue_consumers_cannot_import_raw_or_outbound_authenticated_authority() {
    let tests = trybuild::TestCases::new();
    tests.compile_fail("tests/ui/no_outbound_ws_builders.rs");
    tests.compile_fail("tests/ui/no_raw_authentication.rs");
    tests.compile_fail("tests/ui/no_raw_transport_or_client.rs");
}
