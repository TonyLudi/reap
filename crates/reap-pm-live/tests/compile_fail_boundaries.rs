#[test]
fn composition_boundaries_are_enforced_by_the_type_system() {
    let tests = trybuild::TestCases::new();
    tests.compile_fail("tests/ui/public_capture_has_no_mutation.rs");
    tests.compile_fail("tests/ui/read_only_monitor_has_no_mutation.rs");
    tests.compile_fail("tests/ui/product_has_no_okx_private_or_order.rs");
    tests.compile_fail("tests/ui/product_requires_explicit_model.rs");
    tests.compile_fail("tests/ui/observation_lane_cannot_be_selected.rs");
    tests.compile_fail("tests/ui/service_rank_cannot_be_forged.rs");
    tests.compile_fail("tests/ui/observation_key_cannot_be_replayed.rs");
}
