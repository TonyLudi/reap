#[test]
fn strategy_cannot_import_venue_live_wire_or_adapter_crates() {
    let tests = trybuild::TestCases::new();
    tests.compile_fail("tests/ui/no_infrastructure_dependencies.rs");
    tests.compile_fail("tests/ui/no_forged_execution_intents.rs");
}
