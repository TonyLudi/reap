#[test]
fn authenticated_roles_and_owner_are_capability_narrow() {
    let cases = trybuild::TestCases::new();
    cases.compile_fail("tests/ui/authenticated_roles_cannot_cross_call.rs");
    cases.compile_fail("tests/ui/authenticated_owner_is_move_only.rs");
    cases.compile_fail("tests/ui/private_transport_is_not_public.rs");
    cases.compile_fail("tests/ui/incomplete_cut_cannot_expose_pages.rs");
    cases.compile_fail("tests/ui/pagination_authority_is_take_once.rs");
    cases.compile_fail("tests/ui/metadata_pair_cannot_be_forged.rs");
    cases.compile_fail("tests/ui/public_ws_has_no_socket_escape.rs");
    cases.compile_fail("tests/ui/server_time_proofs_are_move_only.rs");
    #[cfg(feature = "loopback-evidence")]
    cases.compile_fail("tests/ui/loopback_binding_is_move_only.rs");
}
