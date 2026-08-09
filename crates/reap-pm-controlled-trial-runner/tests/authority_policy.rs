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
            "place_sender",
            "place_receiver",
            "authenticate_place",
            "FreshPlaceAuthenticationOnce",
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
fn place_is_consume_once_cancel_is_bounded_and_supervision_is_fail_stop() {
    assert!(
        AUTHORITY_SOURCE.contains("pub(super) async fn authenticate_place_once(\n        self,")
    );
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
    assert!(AUTHORITY_SOURCE.contains("CancelAuthenticationBudgetExhausted"));
    assert!(AUTHORITY_SOURCE.contains("impl Drop for TaskSignerCustody"));
    assert!(AUTHORITY_SOURCE.contains("drop(signer_value);"));
    assert!(AUTHORITY_SOURCE.contains("impl Drop for ArmedTaskSupervisor"));
    assert!(AUTHORITY_SOURCE.contains("std::process::abort();"));
    assert!(AUTHORITY_SOURCE.contains("tokio::time::timeout(bounds.graceful_join"));
    assert!(AUTHORITY_SOURCE.contains("task.abort();"));
    assert!(AUTHORITY_SOURCE.contains("tokio::time::timeout(bounds.abort_join"));
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
    assert!(production.contains("neither validates nor mints"));
    assert!(production.contains("TODO(A3 runner integration)"));
    assert_eq!(
        AUTHORITY_SOURCE
            .matches(".remove_private_key_after_prepared(")
            .count(),
        0,
        "this slice must not treat signer destruction as durable Prepared authority",
    );
}
