const AUTHORITY_SOURCE: &str = include_str!("../src/controlled_trial/authority.rs");
const PARENT_SOURCE: &str = include_str!("../src/controlled_trial/mod.rs");
const MAIN_SOURCE: &str = include_str!("../src/main.rs");
const MANIFEST_SOURCE: &str = include_str!("../Cargo.toml");

fn between<'a>(source: &'a str, start: &str, end: &str) -> &'a str {
    source
        .split_once(start)
        .and_then(|(_, tail)| tail.split_once(end).map(|(slice, _)| slice))
        .expect("source policy markers must remain paired")
}

fn declaration_prefix<'a>(source: &'a str, declaration: &str) -> &'a str {
    source
        .split_once(declaration)
        .map(|(prefix, _)| prefix.rsplit("\n\n").next().unwrap_or(prefix))
        .expect("declared authority type must remain present")
}

#[test]
fn recovery_surface_and_task_cannot_acquire_fresh_place_or_signer_authority() {
    let role = between(
        AUTHORITY_SOURCE,
        "// BEGIN RECOVERY_ROLE_SURFACE",
        "// END RECOVERY_ROLE_SURFACE",
    );
    let task = between(
        AUTHORITY_SOURCE,
        "// BEGIN RECOVERY_TASK",
        "// END RECOVERY_TASK",
    );
    for (name, slice) in [("role", role), ("task", task)] {
        for forbidden in [
            "FixedEoaSigner",
            "TaskSignerCustody",
            "PlaceAuthenticationRequest",
            "PlaceAuthorityRequest",
            "place_sender",
            "place_receiver",
            "authenticate_place",
            "prepare_place",
            "finalize_place",
            "FreshPlaceAuthenticationOnce",
            "SignerDroppedPlacePreparation",
            "PlaceHmacAdmission",
            "PmPlaceMutationTimeFinalizer",
            "PmPlaceMutationTimeProof",
            "OpaqueAuthenticatedPlaceRequest",
        ] {
            assert!(
                !slice.contains(forbidden),
                "recovery {name} slice gained forbidden `{forbidden}` authority",
            );
        }
    }
}

#[test]
fn authority_is_binary_private_and_has_no_raw_secret_or_generic_signing_escape() {
    assert!(MAIN_SOURCE.contains("mod controlled_trial;"));
    assert!(!MAIN_SOURCE.contains("pub mod controlled_trial"));
    assert!(MANIFEST_SOURCE.contains("[[bin]]"));
    assert!(!MANIFEST_SOURCE.contains("[lib]"));
    assert!(!MANIFEST_SOURCE.contains("reqwest"));
    for source in [AUTHORITY_SOURCE, PARENT_SOURCE] {
        for line in source.lines() {
            let line = line.trim_start();
            assert!(
                !line.starts_with("pub struct")
                    && !line.starts_with("pub enum")
                    && !line.starts_with("pub fn")
                    && !line.starts_with("pub async fn"),
                "runner authority gained an externally public item: {line}",
            );
        }
        for forbidden in [
            "fn credentials(",
            "fn l2_credentials(",
            "fn private_key(",
            "fn signer(",
            "sign_message",
            "sign_typed_data",
            "generic_request",
        ] {
            assert!(
                !source.contains(forbidden),
                "runner authority gained forbidden escape `{forbidden}`",
            );
        }
    }
}

#[test]
fn place_is_two_stage_consume_once_cancel_is_bounded_and_supervision_is_fail_stop() {
    assert!(AUTHORITY_SOURCE.contains("pub(super) async fn prepare_place_once(\n        self,"));
    assert!(AUTHORITY_SOURCE.contains("pub(super) async fn finalize_place_once(\n        self,"));
    assert!(!AUTHORITY_SOURCE.contains("authenticate_place_once"));
    let code = AUTHORITY_SOURCE
        .lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n");
    assert_eq!(code.matches("signer: Option<FixedEoaSigner>,").count(), 1);
    assert!(AUTHORITY_SOURCE.contains("const PLACE_AUTHORITY_CAPACITY: usize = 1;"));
    assert!(
        AUTHORITY_SOURCE.contains("const MAX_EXACT_CANCEL_AUTHENTICATIONS_PER_AUTHORITY: u8 = 3;")
    );
    assert!(AUTHORITY_SOURCE.contains("PlaceAlreadyConsumed"));
    assert!(AUTHORITY_SOURCE.contains("PlaceDispatchBindingMismatch"));
    assert!(AUTHORITY_SOURCE.contains("CancelAuthenticationBudgetExhausted"));
    assert!(AUTHORITY_SOURCE.contains("impl Drop for TaskSignerCustody"));
    assert!(AUTHORITY_SOURCE.contains("drop(signer_value);"));
    assert!(AUTHORITY_SOURCE.contains("struct RetainedPreparedPlace"));
    assert!(AUTHORITY_SOURCE.contains("serialized: SerializedPlaceRequest,"));
    assert!(AUTHORITY_SOURCE.contains("place_time_finalizer: PmPlaceMutationTimeFinalizer,"));
    assert!(AUTHORITY_SOURCE.contains("mut place_time_finalizer: PmPlaceMutationTimeFinalizer,"));
    assert!(AUTHORITY_SOURCE.contains("&mut place_time_finalizer,"));
    assert!(AUTHORITY_SOURCE.contains("struct AdmittedPlaceRequestGuard"));
    assert!(AUTHORITY_SOURCE.contains("impl Drop for AdmittedPlaceRequestGuard"));
    assert_eq!(AUTHORITY_SOURCE.matches("request_place(&").count(), 2);
    assert!(
        AUTHORITY_SOURCE.contains("let mut admitted = AdmittedPlaceRequestGuard { armed: true };")
    );
    assert!(AUTHORITY_SOURCE.contains("Err(_) => std::process::abort(),"));
    assert!(
        AUTHORITY_SOURCE.contains("if self.armed {\n            // Dropping the caller future")
    );
    assert!(AUTHORITY_SOURCE.contains("impl Drop for ArmedTaskSupervisor"));
    assert!(AUTHORITY_SOURCE.contains("std::process::abort();"));
    assert!(AUTHORITY_SOURCE.contains("tokio::time::timeout(bounds.graceful_join"));
    assert!(AUTHORITY_SOURCE.contains("task.abort();"));
    assert!(AUTHORITY_SOURCE.contains("tokio::time::timeout(bounds.abort_join"));
    for required in [
        "admitted_prepare_and_finalize_future_cancellation_abort_the_process",
        "status.signal(),\n                Some(libc::SIGABRT)",
        "dropping_an_unpolled_prepare_future_preserves_the_sole_task_authority",
        "PlaceRequestTestPause",
    ] {
        assert!(
            AUTHORITY_SOURCE.contains(required),
            "behavioral cancellation gate is missing `{required}`",
        );
    }
}

#[test]
fn place_preparation_returns_only_public_identity_and_final_hmac_needs_private_proof() {
    let production = AUTHORITY_SOURCE
        .split_once("#[cfg(all(test, target_os = \"linux\"))]")
        .map(|(production, _)| production)
        .expect("unit tests must remain after the production authority");
    for required in [
        "struct SealedPmT2ProxyPlacePreparation",
        "struct SignerDroppedPlacePreparation",
        "pub(super) const fn public_identity(&self) -> PlacePublicRequestIdentity",
        "dispatch: &PmRevalidatedPhaseAPlaceLiveDispatchOwnerV1",
        "struct PlaceHmacAdmission",
        "proof: PmPlaceMutationTimeProof",
        "expected_l2_timestamp_seconds: profile.l2_timestamp_seconds()",
        "struct OpaqueAuthenticatedPlaceRequest",
        ".authenticate_exact_place(",
        "authorization.expected_l2_timestamp_seconds",
    ] {
        assert!(
            production.contains(required),
            "two-stage place seam is missing `{required}`",
        );
    }
    assert_eq!(production.matches(".authenticate_exact_place(").count(), 1);
    assert_eq!(production.matches(".authenticate_place(").count(), 0);
    assert!(production.contains("let Some(prepared) = retained_place.take()"));
    assert!(!production.contains("pub(super) fn new_place_hmac_admission"));
    assert!(!production.contains("pub(super) fn place_hmac_admission"));
    let finalizer = between(
        production,
        "pub(super) async fn finalize_place_once(",
        "async fn finalize_with_admission(",
    );
    assert_eq!(
        finalizer.matches("PlaceHmacAdmission {").count(),
        1,
        "production admission must be minted only from the borrowed durable owner",
    );
    assert!(!production.contains("impl Clone for SignerDroppedPlacePreparation"));
    assert!(!production.contains("impl Clone for PlaceHmacAdmission"));
    assert!(!production.contains("impl Clone for PmPlaceMutationTimeProof"));
    assert!(!production.contains("impl Clone for OpaqueAuthenticatedPlaceRequest"));
    for name in [
        "pub(super) struct FreshPlaceAuthenticationOnce",
        "pub(super) struct SignerDroppedPlacePreparation",
        "struct PlaceHmacAdmission",
        "pub(super) struct OpaqueAuthenticatedPlaceRequest",
    ] {
        let attributes = declaration_prefix(production, name);
        assert!(
            !attributes.contains("#[derive"),
            "move-only authority `{name}` gained a derive that could add Clone/Copy",
        );
    }
    let admission = between(
        production,
        "struct PlaceHmacAdmission {",
        "impl PlaceHmacAdmission",
    );
    assert!(admission.contains("proof: PmPlaceMutationTimeProof,"));
    assert!(!admission.contains("AuthorizedL2Timestamp"));
    assert!(!admission.contains("timestamp: L2Timestamp"));
    let opaque = between(
        production,
        "pub(super) struct OpaqueAuthenticatedPlaceRequest {",
        "impl fmt::Debug for OpaqueAuthenticatedPlaceRequest",
    );
    assert_eq!(
        opaque
            .lines()
            .filter(|line| line.trim() == "request: AuthenticatedPlaceRequest,")
            .count(),
        1,
    );
    assert!(
        opaque
            .lines()
            .all(|line| !line.trim_start().starts_with("pub")),
        "opaque authenticated request field became visible to sibling composition",
    );
    for forbidden in [
        "pub(super) fn sender(",
        "pub(super) fn timestamp(",
        "pub(super) fn credentials(",
        "pub(super) fn serialized(",
        "pub(super) fn request(",
        "pub(super) fn runtime_exact_body_commitment(",
        "pub(super) fn proof(",
        "pub(super) fn into_proof(",
        "pub(super) fn into_request(",
        "pub(super) fn into_parts(",
        "pub(super) fn dispatch(",
        "pub(super) fn decompose(",
        "pub(super) fn authenticated_request(",
        "impl OpaqueAuthenticatedPlaceRequest",
        "impl FixedPlaceRequestSink",
    ] {
        assert!(
            !production.contains(forbidden),
            "two-stage place seam gained forbidden escape `{forbidden}`",
        );
    }
}

#[test]
fn sole_task_routes_every_fixed_http_user_ws_binding_and_exact_cancel_operation() {
    assert_eq!(
        AUTHORITY_SOURCE
            .matches("credentials: L2Credentials")
            .count(),
        2
    );
    assert!(AUTHORITY_SOURCE.contains("common_sender.clone()"));
    for required in [
        "authenticate_open_orders",
        "authenticate_trades",
        "authenticate_balance_allowance",
        "authenticate_closed_only",
        "authenticate_order_detail",
        "serialize_owned_cancel",
        "authenticate_owned_cancel",
        "bind_open_orders",
        "bind_trades",
        "bind_exact_order",
        "user_subscription",
        "bind_user_stream_frame",
    ] {
        assert!(
            AUTHORITY_SOURCE.contains(required),
            "sole authority task is missing `{required}`",
        );
    }
    assert!(AUTHORITY_SOURCE.contains("struct FixedHttpAuthenticationRole"));
    assert!(AUTHORITY_SOURCE.contains("struct FixedUserWsAuthenticationRole"));
    assert!(AUTHORITY_SOURCE.contains("SealedExactOwnedOrderReadAuthentication"));
}

#[test]
fn external_live_read_seam_is_purpose_closed_and_recovery_implements_no_place_provider() {
    assert!(MANIFEST_SOURCE.contains("reap-polymarket-live-adapter.workspace = true"));
    assert!(MANIFEST_SOURCE.contains("reap-pm-controlled-trial-live.workspace = true"));
    assert!(!MANIFEST_SOURCE.contains("reqwest"));
    for required in [
        "impl PmHttpReadAuthorityProvider for FixedHttpAuthenticationRole",
        "impl PmUserWsReadAuthorityProvider for FixedUserWsAuthenticationRole",
        "AuthorizedL2Timestamp::new(timestamp)",
        "SealedExactOwnedOrderReadAuthentication::new(",
        "sealed_order_id != order_id || sealed_timestamp != timestamp",
        "map_external_read_error",
    ] {
        assert!(
            AUTHORITY_SOURCE.contains(required),
            "external read seam is missing `{required}`",
        );
    }
    let recovery = between(
        AUTHORITY_SOURCE,
        "// BEGIN RECOVERY_ROLE_SURFACE",
        "// END RECOVERY_ROLE_SURFACE",
    );
    assert!(recovery.contains("FixedHttpAuthenticationRole"));
    assert!(recovery.contains("FixedUserWsAuthenticationRole"));
    assert!(!recovery.contains("FreshPlaceAuthenticationOnce"));
    assert!(!recovery.contains("PmFixedPlace"));
}

#[test]
fn production_authority_has_no_persistence_logging_or_raw_secret_escape() {
    let production = AUTHORITY_SOURCE
        .split_once("#[cfg(all(test, target_os = \"linux\"))]")
        .map(|(production, _)| production)
        .expect("unit tests must remain after the production authority");
    for (name, source) in [("authority", production), ("parent", PARENT_SOURCE)] {
        for forbidden in [
            "serde",
            "std::fs",
            "OpenOptions",
            "write_all",
            "sync_all",
            "create_dir",
            "println!",
            "eprintln!",
            "dbg!",
            "tracing",
            "log::",
            "runtime_exact_body_commitment",
            "runtime_only_bytes",
            "fn api_key(",
            "fn passphrase(",
            "fn secret(",
            "fn exact_body(",
            "fn body(",
            "as_bytes(",
        ] {
            assert!(
                !source.contains(forbidden),
                "production {name} gained forbidden surface `{forbidden}`",
            );
        }
    }
    assert!(production.contains("prepared: &PmDurablePlacePreparedAckV1,"));
    assert!(production.contains("let durable = prepared.preparation();"));
    assert!(production.contains("durable.expected_order_id()"));
    assert!(production.contains("durable.semantic_request_commitment()"));
    assert!(production.contains("prepared_public_identity: OnceLock<PlacePublicRequestIdentity>"));
    assert!(production.contains("PlacePreparedIdentityUnavailable"));
    assert!(production.contains("PlacePreparedIdentityMismatch"));
    assert_eq!(
        production
            .matches(".remove_private_key_after_prepared(")
            .count(),
        0,
        "this slice must not treat signer destruction as durable Prepared authority",
    );
}
