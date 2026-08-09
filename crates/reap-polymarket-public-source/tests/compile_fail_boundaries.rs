#[test]
fn production_data_api_evidence_is_move_only() {
    let cases = trybuild::TestCases::new();
    cases.compile_fail("tests/ui/production_data_api_evidence_is_move_only.rs");
}
