use std::{fs, path::Path};

const AUTHORITY_SOURCE: &str = include_str!("../src/controlled_trial/authority.rs");
const CREDENTIAL_CUSTODY_SOURCE: &str =
    include_str!("../src/controlled_trial/authority/credential_custody.rs");
const LOCAL_OPERATOR_COOPERATIVE_CUSTODY_V1_SOURCE: &str =
    include_str!("../src/controlled_trial/authority/local_operator_cooperative_custody_v1.rs");
const PARENT_SOURCE: &str = include_str!("../src/controlled_trial/mod.rs");
const MAIN_SOURCE: &str = include_str!("../src/main.rs");
const MANIFEST_SOURCE: &str = include_str!("../Cargo.toml");

fn between<'a>(source: &'a str, start: &str, end: &str) -> &'a str {
    source
        .split_once(start)
        .and_then(|(_, tail)| tail.split_once(end).map(|(slice, _)| slice))
        .expect("source policy markers must remain paired")
}

fn without_between(source: &str, start: &str, end: &str) -> String {
    let (prefix, tail) = source
        .split_once(start)
        .expect("source policy start marker must remain present");
    let (_, suffix) = tail
        .split_once(end)
        .expect("source policy end marker must remain present");
    format!("{prefix}{suffix}")
}

fn declaration_prefix<'a>(source: &'a str, declaration: &str) -> &'a str {
    source
        .split_once(declaration)
        .map(|(prefix, _)| prefix.rsplit("\n\n").next().unwrap_or(prefix))
        .expect("declared authority type must remain present")
}

fn function_signature<'a>(source: &'a str, declaration: &str) -> &'a str {
    source
        .split_once(declaration)
        .and_then(|(_, tail)| tail.split_once('{').map(|(signature, _)| signature))
        .expect("source-policy function declaration must remain present")
}

fn assert_no_raw_credential_authority_outside(
    directory: &Path,
    source_root: &Path,
    allowed: &[&str],
) {
    let mut entries = fs::read_dir(directory)
        .expect("runner source directory must remain readable")
        .collect::<Result<Vec<_>, _>>()
        .expect("runner source directory entries must remain readable");
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let path = entry.path();
        if path.is_dir() {
            assert_no_raw_credential_authority_outside(&path, source_root, allowed);
            continue;
        }
        if path.extension().and_then(|extension| extension.to_str()) != Some("rs") {
            continue;
        }
        let relative = path
            .strip_prefix(source_root)
            .expect("runner source must remain below its source root")
            .to_string_lossy()
            .replace('\\', "/");
        if allowed.contains(&relative.as_str()) {
            continue;
        }
        let source = fs::read_to_string(&path).expect("runner Rust source must remain readable");
        for forbidden in [
            "FixedEoaSigner",
            "L2Credentials",
            "EoaPrivateKeyInput",
            "L2CredentialInput",
            "into_authorities_and_teardown",
            "into_authority_and_teardown",
        ] {
            assert!(
                !source.contains(forbidden),
                "raw credential authority `{forbidden}` escaped into {relative}",
            );
        }
    }
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
    assert!(role.contains("ExactOwnedCancelAuthenticationRole"));
    assert!(task.contains("mut cancel_time_finalizer: PmCancelMutationTimeFinalizer"));
    assert!(task.contains("CancelAuthorityMode::RecoveryOnly"));
    assert!(task.contains("mut cancel: mpsc::Receiver<CancelAuthorityRequest>"));
}

#[test]
fn authority_is_binary_private_and_has_no_raw_secret_or_generic_signing_escape() {
    assert!(MAIN_SOURCE.contains("mod controlled_trial;"));
    assert!(!MAIN_SOURCE.contains("mod credentials;"));
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
fn credential_custody_is_a_private_authority_child_with_no_raw_sibling_escape() {
    assert!(
        AUTHORITY_SOURCE
            .lines()
            .any(|line| line == "mod credential_custody;")
    );
    assert!(!AUTHORITY_SOURCE.contains("pub mod credential_custody;"));
    assert!(!AUTHORITY_SOURCE.contains("pub(crate) mod credential_custody;"));
    assert!(!AUTHORITY_SOURCE.contains("pub(super) mod credential_custody;"));
    assert!(!AUTHORITY_SOURCE.contains("pub(in "));
    assert!(!PARENT_SOURCE.contains("mod credential_custody;"));
    assert!(!MAIN_SOURCE.contains("mod credentials;"));

    let production = CREDENTIAL_CUSTODY_SOURCE
        .split_once("#[cfg(all(test, target_os = \"linux\"))]")
        .map(|(production, _)| production)
        .expect("credential-custody unit tests must remain after production custody");
    let (authority_production, authority_tests) = AUTHORITY_SOURCE
        .split_once("#[cfg(all(test, target_os = \"linux\"))]\nmod tests")
        .expect("authority unit tests must remain after production authority code");
    assert!(!production.contains("pub(crate)"));
    assert!(!production.contains("pub(in "));
    for line in production.lines() {
        let line = line.trim_start();
        assert!(
            !line.starts_with("pub struct")
                && !line.starts_with("pub enum")
                && !line.starts_with("pub fn")
                && !line.starts_with("pub async fn"),
            "credential custody gained an externally public item: {line}",
        );
    }
    for required in [
        "pub(super) struct FreshPlaceCredentialHandoff",
        "pub(super) struct RecoveryOnlyCredentialHandoff",
        "pub(super) fn into_authorities_and_teardown(",
        "pub(super) fn into_authority_and_teardown(",
        "pub(super) struct FreshPlaceCredentialTeardown",
        "pub(super) struct RecoveryOnlyCredentialTeardown",
    ] {
        assert!(
            production.contains(required),
            "credential custody lost parent-only boundary `{required}`",
        );
    }

    let fresh_owner = between(
        authority_production,
        "impl FreshCredentialAuthorityOwner {",
        "/// One-shot fresh custody owner whose only production transition is staged",
    );
    let staged_owner = between(
        authority_production,
        "pub(super) struct FreshStagedObservationCredentialAuthorityOwner {",
        "impl fmt::Debug for FreshStagedObservationCredentialAuthorityOwner",
    );
    let fresh_loader = function_signature(fresh_owner, "fn load_from_protected_files");
    let reviewed_fresh_loader =
        function_signature(staged_owner, "pub(super) fn load_from_reviewed_fresh_token");
    let recovery_owner = authority_production
        .split_once("impl RecoveryCredentialAuthorityOwner {")
        .map(|(_, owner)| owner)
        .expect("recovery credential owner implementation");
    let recovery_loader =
        function_signature(recovery_owner, "pub(super) fn load_from_protected_files");
    assert!(!authority_production.contains("pub(super) struct FreshCredentialAuthorityOwner"));
    assert!(
        authority_production
            .contains("pub(super) struct FreshStagedObservationCredentialAuthorityOwner")
    );
    assert!(fresh_loader.contains("private_key_entry: String"));
    assert!(reviewed_fresh_loader.contains("token: ReviewedFreshCredentialLoadTokenV1"));
    assert!(!reviewed_fresh_loader.contains("directory:"));
    assert!(!reviewed_fresh_loader.contains("_entry:"));
    assert!(!reviewed_fresh_loader.contains("configured_signer:"));
    for forbidden in [
        "pub fn load_from_protected_files(",
        "pub(super) fn load_from_protected_files(",
        "pub(crate) fn load_from_protected_files(",
        "pub(in ",
    ] {
        assert!(
            !fresh_owner.contains(forbidden),
            "raw fresh loader gained sibling visibility `{forbidden}`",
        );
    }
    for required in [
        "token.into_parts()",
        "EoaAddress::parse(&configured_signer_text)",
        "drop(configured_signer_text);",
        "one directory-and-config-signer token minted from one",
        "cannot retry or duplicate that holder",
        "Reloading the protected",
        "neither V2-pinned nor a",
    ] {
        assert!(
            staged_owner.contains(required),
            "reviewed fresh loader docs lost `{required}`",
        );
    }
    for required in [
        "PM_T2_FRESH_PRIVATE_KEY_ENTRY_V1.to_owned()",
        "PM_T2_FRESH_API_KEY_ENTRY_V1.to_owned()",
        "PM_T2_FRESH_L2_SECRET_ENTRY_V1.to_owned()",
        "PM_T2_FRESH_PASSPHRASE_ENTRY_V1.to_owned()",
    ] {
        assert!(
            staged_owner.contains(required),
            "reviewed fresh loader lost `{required}`",
        );
    }
    for forbidden in [
        "pub(super) struct FreshCredentialAuthorityOwner",
        "pub(super) fn spawn_with_mutation_time_finalizers(",
        "pub(super) fn spawn_staged_observation(",
    ] {
        assert!(
            !fresh_owner.contains(forbidden),
            "generic fresh owner regained sibling authority `{forbidden}`",
        );
    }
    for forbidden in [
        "spawn_with_mutation_time_finalizers",
        "fn into_owner(",
        "fn into_parts(",
        "fn owner(",
        "PmPlaceMutationTimeFinalizer",
        "PmCancelMutationTimeFinalizer",
    ] {
        assert!(
            !staged_owner.contains(forbidden),
            "staged-only owner gained mutation or raw projection `{forbidden}`",
        );
    }
    assert!(staged_owner.contains("owner: FreshCredentialAuthorityOwner"));
    assert!(staged_owner.contains("self.owner.spawn_staged_observation(local_set)"));
    assert_eq!(staged_owner.matches("token.into_parts()").count(), 1);
    assert!(!staged_owner.contains("directory.clone()"));
    assert!(!staged_owner.contains("configured_signer_text.clone()"));
    assert!(
        staged_owner
            .contains("#[cfg(test)]\n    pub(super) fn load_from_protected_files_for_test(")
    );
    assert!(!recovery_loader.contains("private_key_entry"));
    for (owner, signature) in [("fresh", fresh_loader), ("recovery", recovery_loader)] {
        assert!(signature.contains(") -> Result<Self, CredentialAuthorityError>"));
        for forbidden in [
            "FixedEoaSigner",
            "L2Credentials",
            "CredentialHandoff",
            "CredentialTeardown",
        ] {
            assert!(
                !signature.contains(forbidden),
                "sealed {owner} loader exposes `{forbidden}`",
            );
        }
    }
    assert!(authority_production.contains("CredentialCustodyLoadFailed,"));
    assert_eq!(
        authority_production
            .matches("const fn from_custody(")
            .count(),
        2
    );
    assert!(!authority_production.contains("pub(super) const fn from_custody("));
    assert!(!authority_production.contains("pub(crate) const fn from_custody("));
    assert_eq!(
        authority_production
            .matches(".into_authorities_and_teardown()")
            .count(),
        2,
        "production may decompose fresh custody only in its full and staged task spawns",
    );
    assert_eq!(
        authority_production
            .matches(".into_authority_and_teardown()")
            .count(),
        1,
        "production may decompose recovery custody only in its credential task spawn",
    );
    assert_eq!(
        authority_tests
            .matches(".into_authority_and_teardown()")
            .count(),
        1,
        "the authority-local timeout test retains its one direct recovery handoff",
    );

    let source_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    assert_no_raw_credential_authority_outside(
        &source_root,
        &source_root,
        &[
            "controlled_trial/authority.rs",
            "controlled_trial/authority/credential_custody.rs",
            "controlled_trial/authority/local_operator_cooperative_custody_v1.rs",
            "controlled_trial/authority/production_order_v1.rs",
        ],
    );
}

#[test]
fn local_operator_cooperative_custody_v1_is_one_private_non_authorizing_boundary() {
    const EXACT_SOURCE_PATH: &str = "crates/reap-pm-controlled-trial-runner/src/controlled_trial/authority/local_operator_cooperative_custody_v1.rs";
    const EXACT_SOURCE_SHA256: &str =
        "42c893b410c3ecf62dab6b61290cfa1d10367fdd8a0e7e247a22acd90cf819f0";

    assert!(
        AUTHORITY_SOURCE
            .contains("#[cfg(target_os = \"linux\")]\nmod local_operator_cooperative_custody_v1;")
    );
    for retained_boundary in [
        "GenerationBoundUnopenedLocalOperatorFreshCredentialCustodyV1",
        "PreparedUnopenedLocalOperatorFreshCredentialCustodyV1",
        "prepare_unopened_local_operator_fresh_credential_custody_v1",
    ] {
        assert!(AUTHORITY_SOURCE.contains(retained_boundary));
    }
    assert!(!AUTHORITY_SOURCE.contains("pub mod local_operator_cooperative_custody_v1;"));
    assert!(!PARENT_SOURCE.contains("local_operator_cooperative_custody_v1"));

    let source = LOCAL_OPERATOR_COOPERATIVE_CUSTODY_V1_SOURCE;
    let production = source
        .split_once("#[cfg(all(test, target_os = \"linux\"))]")
        .map(|(production, _)| production)
        .expect("local cooperative custody tests must follow production code");
    assert_eq!(
        production.matches("pub(super) fn ").count(),
        1,
        "the module must retain exactly one parent-only realization constructor"
    );
    assert_eq!(
        production
            .matches("pub(in crate::controlled_trial) fn ")
            .count(),
        2,
        "the controlled-trial subtree may only prepare and generation-bind unopened custody",
    );
    assert!(!production.contains("pub(crate)"));
    for line in production.lines() {
        let line = line.trim_start();
        assert!(
            !line.starts_with("pub struct")
                && !line.starts_with("pub enum")
                && !line.starts_with("pub fn"),
            "local cooperative custody gained an externally public item: {line}",
        );
    }

    let signature = function_signature(
        production,
        "pub(super) fn realize_local_operator_fresh_credential_custody_v1",
    );
    for required in [
        "context: &ReviewedLocalOperatorCooperativeCustodyProfileContextV1<'_>",
        "reviewed_profile: &CanonicalReviewedLocalOperatorCooperativeCustodyProfileV1",
        "delivery_load_token: FreshCredentialDeliveryLoadTokenV1",
        "retained_delivery_evidence: &CanonicalFreshCredentialDeliveryBindingEvidenceV1",
        "Result<LocalOperatorFreshCredentialCustodyV1, LocalOperatorCooperativeCustodyV1Error>",
    ] {
        assert!(
            signature.contains(required),
            "constructor lost `{required}`"
        );
    }

    let prepare_signature = function_signature(
        production,
        "pub(in crate::controlled_trial) fn prepare_unopened_local_operator_fresh_credential_custody_v1",
    );
    for required in [
        "context: &ReviewedLocalOperatorCooperativeCustodyProfileContextV1<'_>",
        "reviewed_profile: CanonicalReviewedLocalOperatorCooperativeCustodyProfileV1",
        "delivery_load_token: FreshCredentialDeliveryLoadTokenV1",
        "retained_delivery_evidence: CanonicalFreshCredentialDeliveryBindingEvidenceV1",
        "PreparedUnopenedLocalOperatorFreshCredentialCustodyV1",
    ] {
        assert!(prepare_signature.contains(required));
    }
    assert!(production.contains(
        "verify_reviewed_conjunction(context, &reviewed_profile, &retained_delivery_evidence)?;",
    ));
    let bind_signature = function_signature(
        production,
        "pub(in crate::controlled_trial) fn bind_to_actor_generation<G>",
    );
    assert!(bind_signature.contains("self"));
    assert!(bind_signature.contains("selected_actor_generation: Rc<G>"));
    assert!(
        bind_signature.contains("GenerationBoundUnopenedLocalOperatorFreshCredentialCustodyV1<G>")
    );
    for forbidden in [
        "ReviewedFreshCredentialLoadTokenV1",
        "PathBuf",
        "configured_signer",
        "basename",
        "FreshCredentialLinuxObjectSetV1",
        "fingerprint:",
        "VerificationV1",
    ] {
        assert!(
            !signature.contains(forbidden),
            "constructor gained forbidden raw or projected argument `{forbidden}`",
        );
    }
    assert!(!production.contains("ReviewedFreshCredentialLoadTokenV1"));
    assert_eq!(
        production
            .matches("delivery_load_token.into_parts()")
            .count(),
        1
    );
    assert_eq!(
        production
            .matches("reviewed_locator_load_token.into_parts()")
            .count(),
        1
    );
    let token_consumption = production
        .find("delivery_load_token.into_parts()")
        .expect("whole delivery token consumption");
    let mismatch = production
        .find("token_delivery_fingerprint != retained_delivery_evidence.fingerprint()")
        .expect("whole-token/evidence mismatch check");
    let first_open = production
        .find("rustix::fs::open(")
        .expect("exact directory open");
    assert!(token_consumption < mismatch && mismatch < first_open);

    for required in [
        "verify_fresh_credential_delivery_binding_evidence_v1(",
        "verify_reviewed_local_operator_cooperative_custody_profile_v1(context, reviewed_profile)",
        "verification.authorization == OfflineAuthorizationState::DENIED",
        "verification.authorization.place_dispatch_allowance == 0",
        "FlockOperation::NonBlockingLockExclusive",
        "OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC",
        "OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK",
        "rustix::fs::openat(",
        "PM_T2_FRESH_PRIVATE_KEY_ENTRY_V1",
        "PM_T2_FRESH_API_KEY_ENTRY_V1",
        "PM_T2_FRESH_L2_SECRET_ENTRY_V1",
        "PM_T2_FRESH_PASSPHRASE_ENTRY_V1",
        "read_at(&mut bytes[length..], length as u64)",
        "if after != source.snapshot",
        "re_resolve_credential_source(",
        "revalidate_directory_path(",
        "FixedEoaSigner::bind(",
        "L2Credentials::bind(",
        "_credential_sources: [File; 4]",
        "_cooperative_lease_and_directory: File",
        "_signer: FixedEoaSigner",
        "_l2: L2Credentials",
        "On every pre-return failure those values are",
        "every credential basename is left unchanged",
    ] {
        assert!(production.contains(required), "loader lost `{required}`");
    }

    let holder = between(
        production,
        "pub(super) struct LocalOperatorFreshCredentialCustodyV1 {",
        "/// Structurally reviewed but unopened custody inputs",
    );
    assert!(
        holder
            .lines()
            .all(|line| !line.trim_start().starts_with("pub"))
    );
    let signer_field = holder.find("_signer: FixedEoaSigner").unwrap();
    let l2_field = holder.find("_l2: L2Credentials").unwrap();
    let source_fields = holder.find("_credential_sources: [File; 4]").unwrap();
    let lease_field = holder
        .find("_cooperative_lease_and_directory: File")
        .unwrap();
    assert!(signer_field < l2_field && l2_field < source_fields && source_fields < lease_field);
    let realization_initialization = production
        .split_once("Ok(LocalOperatorFreshCredentialCustodyV1 {")
        .map(|(_, initialization)| initialization)
        .unwrap();
    let signer_init = realization_initialization.find("_signer: signer").unwrap();
    let l2_init = realization_initialization.find("_l2: l2").unwrap();
    let source_init = realization_initialization
        .find("_credential_sources: [")
        .unwrap();
    let lease_init = realization_initialization
        .find("_cooperative_lease_and_directory: directory")
        .unwrap();
    assert!(signer_init < l2_init && l2_init < source_init && source_init < lease_init);
    let holder_prefix = declaration_prefix(
        production,
        "pub(super) struct LocalOperatorFreshCredentialCustodyV1",
    );
    assert!(!holder_prefix.contains("derive"));
    let prepared_holder = between(
        production,
        "pub(in crate::controlled_trial) struct PreparedUnopenedLocalOperatorFreshCredentialCustodyV1 {",
        "impl fmt::Debug for PreparedUnopenedLocalOperatorFreshCredentialCustodyV1",
    );
    for field in [
        "reviewed_profile: CanonicalReviewedLocalOperatorCooperativeCustodyProfileV1",
        "retained_delivery_evidence: CanonicalFreshCredentialDeliveryBindingEvidenceV1",
        "whole_delivery_load_token: FreshCredentialDeliveryLoadTokenV1",
    ] {
        assert!(prepared_holder.contains(field));
    }
    assert!(
        prepared_holder
            .lines()
            .all(|line| !line.trim_start().starts_with("pub "))
    );
    let generation_holder = between(
        production,
        "pub(in crate::controlled_trial) struct GenerationBoundUnopenedLocalOperatorFreshCredentialCustodyV1<",
        "impl<G> fmt::Debug for GenerationBoundUnopenedLocalOperatorFreshCredentialCustodyV1<G>",
    );
    for field in [
        "_reviewed_profile: CanonicalReviewedLocalOperatorCooperativeCustodyProfileV1",
        "_retained_delivery_evidence: CanonicalFreshCredentialDeliveryBindingEvidenceV1",
        "_whole_delivery_load_token: FreshCredentialDeliveryLoadTokenV1",
        "_selected_actor_generation: Rc<G>",
    ] {
        assert!(generation_holder.contains(field));
    }
    assert!(
        generation_holder
            .lines()
            .all(|line| !line.trim_start().starts_with("pub "))
    );
    for forbidden in [
        "#[derive(Clone",
        "#[derive(Copy",
        "impl Clone",
        "impl Copy",
        "serde::",
        "Serialize",
        "Deserialize",
        "impl Clone for LocalOperatorFreshCredentialCustodyV1",
        "impl Clone for PreparedUnopenedLocalOperatorFreshCredentialCustodyV1",
        "impl Clone for GenerationBoundUnopenedLocalOperatorFreshCredentialCustodyV1",
        "impl Serialize for LocalOperatorFreshCredentialCustodyV1",
        "Deserialize<'de> for LocalOperatorFreshCredentialCustodyV1",
        "unsafe impl Send",
        "unsafe impl Sync",
        "fn into_parts(",
        "fn split(",
        "fn directory(",
        "fn path(",
        "fn fd(",
        "fn signer(",
        "fn l2(",
        "fn private_key(",
        "fn api_key(",
        "fn secret(",
        "fn passphrase(",
        "remove_file",
        "unlinkat",
        "ftruncate",
        "truncate(",
        "set_len(",
        "write_all",
        "OpenOptions",
        "AuthenticatedL2Headers",
        "AuthenticatedPlaceRequest",
        "PmPlaceMutationTimeProof",
        "PmCancelMutationTimeProof",
        "DispatchOwner",
        "reqwest",
        "hyper::",
        "http::",
        "tokio::net",
        "TcpStream",
        "UdpSocket",
        "Hmac",
    ] {
        assert!(
            !production.contains(forbidden),
            "local custody gained forbidden authority or mutation surface `{forbidden}`",
        );
    }

    use sha2::Digest as _;
    let actual_sha256 = format!(
        "{:x}",
        sha2::Sha256::digest(LOCAL_OPERATOR_COOPERATIVE_CUSTODY_V1_SOURCE.as_bytes())
    );
    assert_eq!(
        actual_sha256, EXACT_SOURCE_SHA256,
        "audited local cooperative custody source changed at {EXACT_SOURCE_PATH}",
    );
}

#[test]
fn staged_observation_is_a_distinct_armed_read_only_fresh_custody() {
    let production = AUTHORITY_SOURCE
        .split_once("#[cfg(all(test, target_os = \"linux\"))]")
        .map(|(production, _)| production)
        .expect("unit tests must remain after the production authority");
    let spawn = between(
        production,
        "// BEGIN STAGED_OBSERVATION_SPAWN",
        "// END STAGED_OBSERVATION_SPAWN",
    );
    let roles = between(
        production,
        "// BEGIN STAGED_OBSERVATION_ROLE_SURFACE",
        "// END STAGED_OBSERVATION_ROLE_SURFACE",
    );
    let custody = between(
        production,
        "// BEGIN STAGED_OBSERVATION_CUSTODY",
        "// END STAGED_OBSERVATION_CUSTODY",
    );
    let task = between(
        production,
        "// BEGIN STAGED_OBSERVATION_TASK",
        "// END STAGED_OBSERVATION_TASK",
    );

    let signature = function_signature(spawn, "fn spawn_staged_observation");
    assert!(signature.contains("self,"));
    assert!(signature.contains("local_set: &tokio::task::LocalSet,"));
    assert!(
        signature.contains(
            ") -> Result<FreshStagedObservationAuthorityRoles, CredentialAuthorityError>"
        )
    );
    for forbidden in ["finalizer", "proof", "timestamp", "sender:", "receiver:"] {
        assert!(
            !signature.contains(forbidden),
            "local-only staged spawn gained `{forbidden}` input authority",
        );
    }
    for required in [
        "same local set must remain driven through",
        "self.custody.into_authorities_and_teardown()",
        "let loaded_signer = signer.address();",
        "TaskSignerCustody::new(signer, Arc::clone(&identity))",
        "mpsc::channel(COMMON_AUTHORITY_CAPACITY)",
        "common_sender.clone()",
        "local_set.spawn_local(run_fresh_staged_observation_authority(",
        "run_fresh_staged_observation_authority(",
        "ArmedTaskSupervisor::new(shutdown, task)",
    ] {
        assert!(spawn.contains(required), "staged spawn lost `{required}`");
    }
    assert_eq!(spawn.matches(".into_authorities_and_teardown()").count(), 1);
    assert_eq!(spawn.matches("TaskSignerCustody::new(").count(), 1);
    assert_eq!(spawn.matches("mpsc::channel(").count(), 1);
    assert!(!spawn.contains("tokio::runtime::Handle"));
    assert!(!spawn.contains(".spawn("));

    let staged_owner = between(
        production,
        "pub(super) struct FreshStagedObservationCredentialAuthorityOwner {",
        "impl fmt::Debug for FreshStagedObservationCredentialAuthorityOwner",
    );
    let staged_spawn = function_signature(staged_owner, "pub(super) fn spawn_staged_observation");
    assert!(staged_spawn.contains("self,"));
    assert!(staged_spawn.contains("local_set: &tokio::task::LocalSet,"));
    assert!(staged_owner.contains("self.owner.spawn_staged_observation(local_set)"));
    for forbidden in ["finalizer", "proof", "timestamp", "place", "cancel"] {
        assert!(
            !staged_spawn.contains(forbidden),
            "staged-only owner gained `{forbidden}` input authority",
        );
    }

    for required in [
        "pub(super) struct FreshStagedObservationAuthorityRoles",
        "http: FixedHttpAuthenticationRole",
        "user_ws: FixedUserWsAuthenticationRole",
        "loaded_signer: EoaAddress",
        "custody: PmObservingFreshCredentialCustody",
        "pub(super) fn into_private_read_runtime_parts(",
    ] {
        assert!(
            roles.contains(required),
            "staged role surface lost `{required}`"
        );
    }
    for forbidden in [
        "FixedEoaSigner",
        "L2Credentials",
        "FreshPlaceCredentialHandoff",
        "FreshPlaceCredentialTeardown",
    ] {
        assert!(
            !roles.contains(forbidden),
            "staged role surface exposed raw custody `{forbidden}`",
        );
    }

    for required in [
        "pub(super) enum PmObservingFreshCredentialShutdownError",
        "pub(super) struct PmObservingFreshCredentialCustody",
        "task: ArmedTaskSupervisor",
        "teardown: FreshPlaceCredentialTeardown",
        "identity: Arc<AuthorityIdentity>",
        "_actor_local: PhantomData<Rc<()>>",
        "pub(super) async fn shutdown_bounded(",
        "let joined = task.join_bounded(bounds).await;",
        "if !identity.signer_dropped()",
        ".remove_private_key()",
        ".remove_l2_files()",
        "Ok(joined.complete_with_teardown())",
    ] {
        assert!(
            custody.contains(required),
            "observing custody lost `{required}`"
        );
    }
    let join = custody
        .find("let joined = task.join_bounded(bounds).await;")
        .unwrap();
    let signer_proof = custody.find("if !identity.signer_dropped()").unwrap();
    let key_remove = custody.find(".remove_private_key()").unwrap();
    let l2_remove = custody.find(".remove_l2_files()").unwrap();
    assert!(join < signer_proof && signer_proof < key_remove && key_remove < l2_remove);
    assert!(CREDENTIAL_CUSTODY_SOURCE.contains("fn unlink_slot("));
    assert!(CREDENTIAL_CUSTODY_SOURCE.contains("self.file.sync_all().map_err(|_|"));
    assert!(CREDENTIAL_CUSTODY_SOURCE.contains("CredentialCustodyError::DirectorySync"));
    assert!(spawn.contains("_actor_local: PhantomData"));
    assert!(!production.contains("unsafe impl Send for PmObservingFreshCredentialCustody"));
    assert!(!production.contains("unsafe impl Sync for PmObservingFreshCredentialCustody"));
    assert!(!roles.contains("fn loaded_signer("));

    for required in [
        "signer: TaskSignerCustody",
        "credentials: L2Credentials",
        "mut common: mpsc::Receiver<CommonAuthorityRequest>",
        "mut shutdown: oneshot::Receiver<()> ",
        "Some(request) => handle_common_request(&credentials, request)",
        "None => common_open = false",
        "drop(credentials);",
        "drop(signer);",
    ] {
        let required = required.trim_end();
        assert!(task.contains(required), "staged task lost `{required}`");
    }
    assert!(!task.contains("signer.take()"));
    assert!(!task.contains("publish_prepared_public_identity"));
    assert!(!task.contains("None => break"));

    for (name, slice) in [
        ("spawn", spawn),
        ("roles", roles),
        ("custody", custody),
        ("task", task),
    ] {
        for forbidden in [
            "PlaceAuthorityRequest",
            "CancelAuthorityRequest",
            "FreshPlaceAuthenticationOnce",
            "ExactOwnedCancelAuthenticationRole",
            "PlaceHmacAdmission",
            "CancelHmacAdmission",
            "PmPlaceMutationTimeFinalizer",
            "PmCancelMutationTimeFinalizer",
            "PmPlaceMutationTimeProof",
            "PmCancelMutationTimeProof",
            "AuthenticatedPlaceRequest",
            "AuthenticatedOwnedCancelRequest",
            "place_sender",
            "place_receiver",
            "cancel_sender",
            "cancel_receiver",
            "authenticate_exact_place",
            "authenticate_exact_owned_cancel",
        ] {
            assert!(
                !slice.contains(forbidden),
                "staged {name} slice gained mutation authority `{forbidden}`",
            );
        }
    }
    assert!(
        AUTHORITY_SOURCE
            .contains("staged_observation_keeps_unused_signer_and_real_common_reads_until_cleanup")
    );
    assert!(AUTHORITY_SOURCE.contains(
        "#[tokio::test(flavor = \"current_thread\")]\n    async fn staged_observation_keeps_unused_signer_and_real_common_reads_until_cleanup"
    ));
    assert!(AUTHORITY_SOURCE.contains(".spawn_staged_observation(&local_set)"));
    assert!(AUTHORITY_SOURCE.contains(".run_until(async {"));
}

#[test]
fn place_is_two_stage_consume_once_cancel_is_bounded_and_supervision_is_fail_stop() {
    assert!(AUTHORITY_SOURCE.contains("pub(super) async fn prepare_place_once(\n        self,"));
    assert!(!AUTHORITY_SOURCE.contains("pub(super) async fn finalize_place_once("));
    assert!(AUTHORITY_SOURCE.contains("async fn finalize_place_once_for_test("));
    assert!(AUTHORITY_SOURCE.contains("// BEGIN TEST_ONLY_PLACE_HMAC_ADMISSION"));
    assert!(!AUTHORITY_SOURCE.contains("authenticate_place_once"));
    let code = AUTHORITY_SOURCE
        .lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n");
    assert_eq!(code.matches("signer: Option<FixedEoaSigner>,").count(), 1);
    assert!(AUTHORITY_SOURCE.contains("const PLACE_AUTHORITY_CAPACITY: usize = 1;"));
    assert!(AUTHORITY_SOURCE.contains("const CANCEL_AUTHORITY_CAPACITY: usize = 1;"));
    assert!(
        AUTHORITY_SOURCE.contains("const MAX_EXACT_CANCEL_AUTHENTICATIONS_PER_AUTHORITY: u8 = 3;")
    );
    assert!(AUTHORITY_SOURCE.contains("PlaceAlreadyConsumed"));
    assert!(AUTHORITY_SOURCE.contains("PlaceDispatchBindingMismatch"));
    assert!(AUTHORITY_SOURCE.contains("CancelAuthenticationPreSendFailureKind::BudgetExhausted"));
    assert!(AUTHORITY_SOURCE.contains("impl Drop for TaskSignerCustody"));
    assert!(AUTHORITY_SOURCE.contains("drop(signer_value);"));
    assert!(AUTHORITY_SOURCE.contains("struct RetainedPreparedPlace"));
    assert!(AUTHORITY_SOURCE.contains("serialized: SerializedPlaceRequest,"));
    assert!(AUTHORITY_SOURCE.contains("place_time_finalizer: PmPlaceMutationTimeFinalizer,"));
    assert!(
        AUTHORITY_SOURCE
            .contains("let FreshAuthorityTaskInputs {\n        mut place_time_finalizer,")
    );
    assert!(AUTHORITY_SOURCE.contains("&mut place_time_finalizer,"));
    assert!(AUTHORITY_SOURCE.contains("struct AdmittedPlaceRequestGuard"));
    assert!(AUTHORITY_SOURCE.contains("impl Drop for AdmittedPlaceRequestGuard"));
    assert!(AUTHORITY_SOURCE.contains("struct AdmittedCancelRequestGuard"));
    assert!(AUTHORITY_SOURCE.contains("impl Drop for AdmittedCancelRequestGuard"));
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
        "admitted_place_and_cancel_future_cancellation_abort_the_process",
        "status.signal(),\n                Some(libc::SIGABRT)",
        "dropping_an_unpolled_prepare_future_preserves_the_sole_task_authority",
        "PlaceRequestTestPause",
        "CancelRequestTestPause",
        "for case in [\"prepare\", \"finalize\", \"cancel\"]",
    ] {
        assert!(
            AUTHORITY_SOURCE.contains(required),
            "behavioral cancellation gate is missing `{required}`",
        );
    }
}

#[test]
fn place_preparation_is_fail_closed_until_a_sealed_online_permit_exists() {
    let production = AUTHORITY_SOURCE
        .split_once("#[cfg(all(test, target_os = \"linux\"))]")
        .map(|(production, _)| production)
        .expect("unit tests must remain after the production authority");
    let test_only = between(
        production,
        "// BEGIN TEST_ONLY_PLACE_HMAC_ADMISSION",
        "// END TEST_ONLY_PLACE_HMAC_ADMISSION",
    );
    let production_without_test_hmac = without_between(
        production,
        "// BEGIN TEST_ONLY_PLACE_HMAC_ADMISSION",
        "// END TEST_ONLY_PLACE_HMAC_ADMISSION",
    );
    for required in [
        "struct SealedPmT2ProxyPlacePreparation",
        "struct SignerDroppedPlacePreparation",
        "pub(super) const fn public_identity(&self) -> PlacePublicRequestIdentity",
        "struct PlaceHmacAdmission",
        "proof: PmPlaceMutationTimeProof",
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
    for forbidden in [
        "PmPhaseAOnlinePreflightDispatchOwnerV2",
        "PmRevalidatedPhaseAPlaceLiveDispatchOwnerV1",
        "pub(super) async fn finalize_place_once(",
        "async fn finalize_with_admission(",
    ] {
        assert!(
            !production_without_test_hmac.contains(forbidden),
            "production place authority gained forbidden admission `{forbidden}`",
        );
    }
    assert_eq!(
        production_without_test_hmac
            .matches("struct PlaceHmacAdmission {")
            .count(),
        1,
        "the task-private admission declaration must remain unique",
    );
    let admission_brace_lines = production_without_test_hmac
        .lines()
        .map(str::trim)
        .filter(|line| line.contains("PlaceHmacAdmission {"))
        .collect::<Vec<_>>();
    assert_eq!(
        admission_brace_lines,
        [
            "struct PlaceHmacAdmission {",
            "impl PlaceHmacAdmission {",
            "impl fmt::Debug for PlaceHmacAdmission {",
        ],
        "production must not contain a PlaceHmacAdmission struct literal",
    );
    for required in [
        "#[cfg(test)]\nimpl SignerDroppedPlacePreparation",
        "async fn finalize_with_admission(",
        "async fn finalize_place_once_for_test(",
        "async fn finalize_place_once_paused_for_test(",
        "expected_l2_timestamp_seconds",
        "proof: PmPlaceMutationTimeProof",
    ] {
        assert!(
            test_only.contains(required),
            "test-only HMAC harness is missing `{required}`",
        );
    }
    assert_eq!(test_only.matches("PlaceHmacAdmission {").count(), 2);
    assert_eq!(test_only.matches("request_place(&self.sender").count(), 1);
    assert!(!production.contains("impl Clone for SignerDroppedPlacePreparation"));
    assert!(!production.contains("impl Clone for PlaceHmacAdmission"));
    assert!(!production.contains("impl Clone for PmPlaceMutationTimeProof"));
    assert!(!production.contains("impl Clone for OpaqueAuthenticatedPlaceRequest"));
    assert!(!production.contains("PmPhaseAOnlinePreflightDispatchOwnerV2"));
    assert!(!production.contains("PmRevalidatedPhaseAPlaceLiveDispatchOwnerV1"));
    assert!(!production.contains("dispatch.profile()"));
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
    assert!(!admission.contains("PmPhaseAOnlinePreflightDispatchOwnerV2"));
    assert!(!admission.contains("dispatch"));
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
        "dispatch: &PmPhaseAOnlinePreflightDispatchOwnerV2",
        "dispatch: &PmRevalidatedPhaseAPlaceLiveDispatchOwnerV1",
        "dispatch.profile()",
        "let profile = dispatch",
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
        "pub(super) fn owner(",
        "pub(super) fn v1(",
        "pub(super) fn profile(",
        "pub(super) fn authenticated_request(",
        "impl OpaqueAuthenticatedPlaceRequest",
        "impl FixedPlaceRequestSink",
    ] {
        assert!(
            !production_without_test_hmac.contains(forbidden),
            "fail-closed place seam gained forbidden escape `{forbidden}`",
        );
    }
}

#[test]
fn cancel_requires_positive_a3_owner_source_time_and_a_dedicated_fail_stop_channel() {
    let production = AUTHORITY_SOURCE
        .split_once("#[cfg(all(test, target_os = \"linux\"))]")
        .map(|(production, _)| production)
        .expect("unit tests must remain after the production authority");
    for required in [
        "PmRevalidatedPhaseALiveCancelDispatchOwnerV1",
        "PmCancelMutationTimeProof",
        "PmCancelMutationTimeFinalizer",
        "const CANCEL_AUTHORITY_CAPACITY: usize = 1;",
        "mpsc::Sender<CancelAuthorityRequest>",
        "mut cancel: mpsc::Receiver<CancelAuthorityRequest>",
        "CancelAuthorityMode::FreshPrimary",
        "CancelAuthorityMode::RecoveryOnly",
        "struct CancelAuthenticationPreSendFailure",
        "owner: Box<PmRevalidatedPhaseALiveCancelDispatchOwnerV1>",
        "pub(super) fn into_owner(self) -> PmRevalidatedPhaseALiveCancelDispatchOwnerV1",
        "struct CancelHmacAdmission",
        "struct OpaqueAuthenticatedExactOwnedCancel",
        "request: AuthenticatedOwnedCancelRequest",
        "owner: PmRevalidatedPhaseALiveCancelDispatchOwnerV1",
        "credentials.serialize_owned_cancel(order_id)",
        "cancel_time_finalizer.authenticate_exact_owned_cancel(",
        "expected_l2_timestamp_seconds",
        "struct AdmittedCancelRequestGuard",
        "if response.send(value).is_err()",
    ] {
        assert!(
            production.contains(required),
            "positive cancel authority is missing `{required}`",
        );
    }
    assert_eq!(
        production
            .matches("credentials.serialize_owned_cancel(")
            .count(),
        1,
        "the sole L2 task must serialize the exact cancel exactly once",
    );
    assert_eq!(
        production
            .matches("cancel_time_finalizer.authenticate_exact_owned_cancel(")
            .count(),
        1,
        "the adapter-owned finalizer must be the sole cancel HMAC edge",
    );
    assert!(!production.contains("credentials.authenticate_owned_cancel("));
    assert!(!PARENT_SOURCE.contains("SealedExactOwnedCancelAuthentication"));
    assert!(!PARENT_SOURCE.contains("AuthenticatedExactOwnedCancel"));

    let role = between(
        production,
        "impl ExactOwnedCancelAuthenticationRole {",
        "impl fmt::Debug for ExactOwnedCancelAuthenticationRole",
    );
    assert!(role.contains("owner: PmRevalidatedPhaseALiveCancelDispatchOwnerV1,"));
    assert!(role.contains("proof: PmCancelMutationTimeProof,"));
    assert!(!role.contains("AuthorizedL2Timestamp"));
    assert!(!role.contains("L2Timestamp"));
    assert!(!role.contains("CommonAuthorityRequest"));

    let handler = between(
        production,
        "fn authenticate_cancel_admission(",
        "fn handle_common_request(",
    );
    for exact_check in [
        "mode.accepts(owner.dispatch_class())",
        "owner.exact_venue_order_id()",
        "owner.semantic_request_commitment()",
        "owner.l2_timestamp_seconds()",
        "preparation.exact_venue_order_id() != order_id",
        "serialized.semantic_request_commitment() != semantic_request_commitment",
        "authenticated.semantic_request_commitment() != semantic_request_commitment",
        "owner.dispatch_class() != preparation.dispatch_class()",
    ] {
        assert!(
            handler.contains(exact_check),
            "cancel handler is missing exact check `{exact_check}`",
        );
    }

    let opaque = between(
        production,
        "pub(super) struct OpaqueAuthenticatedExactOwnedCancel {",
        "impl fmt::Debug for OpaqueAuthenticatedExactOwnedCancel",
    );
    assert!(opaque.contains("request: AuthenticatedOwnedCancelRequest,"));
    assert!(opaque.contains("owner: PmRevalidatedPhaseALiveCancelDispatchOwnerV1,"));
    assert!(
        opaque
            .lines()
            .all(|line| !line.trim_start().starts_with("pub")),
        "opaque cancel fields became visible",
    );
    for forbidden in [
        "impl OpaqueAuthenticatedExactOwnedCancel",
        "pub(super) fn request(",
        "pub(super) fn owner(",
        "pub(super) fn into_parts(",
        "pub(super) fn dispatch(",
        "FixedOwnedCancelRequestSink",
    ] {
        assert!(
            !production.contains(forbidden),
            "opaque cancel gained forbidden escape `{forbidden}`",
        );
    }
}

#[test]
fn sole_task_routes_every_fixed_http_user_ws_binding_and_exact_cancel_operation() {
    assert_eq!(
        AUTHORITY_SOURCE
            .matches("credentials: L2Credentials")
            .count(),
        3
    );
    assert!(AUTHORITY_SOURCE.contains("common_sender.clone()"));
    for required in [
        "authenticate_open_orders",
        "authenticate_trades",
        "authenticate_balance_allowance",
        "authenticate_closed_only",
        "authenticate_order_detail",
        "serialize_owned_cancel",
        "authenticate_exact_owned_cancel",
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
