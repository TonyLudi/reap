#[test]
fn production_polygon_evidence_is_move_only() {
    let cases = trybuild::TestCases::new();
    cases.compile_fail("tests/ui/production_polygon_evidence_is_move_only.rs");
}
