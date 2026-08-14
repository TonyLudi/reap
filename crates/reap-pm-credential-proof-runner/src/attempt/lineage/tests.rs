use std::{
    fs::OpenOptions,
    io::Write as _,
    os::unix::fs::{MetadataExt as _, OpenOptionsExt as _, PermissionsExt as _},
};

use super::*;

fn protected_directory() -> tempfile::TempDir {
    let directory = tempfile::tempdir().unwrap();
    std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    directory
}

fn binding() -> AttemptPublicBinding {
    AttemptPublicBinding {
        policy_commitment: "11".repeat(32),
        source_commitment: "22".repeat(32),
        destination_selection_commitment: "33".repeat(32),
        local_egress_selection_commitment: "44".repeat(32),
        signer_identity_commitment: "55".repeat(32),
        selected_actor_generation_commitment: "66".repeat(32),
        attempt_commitment: "77".repeat(32),
    }
}

#[test]
fn prepared_without_claim_is_nonresumable_and_never_reopened() {
    let directory = protected_directory();
    let prepared = PreparedAttemptLineage::create_new(directory.path(), binding()).unwrap();
    drop(prepared);

    assert_eq!(
        PreparedAttemptLineage::create_new(directory.path(), binding()).unwrap_err(),
        LineageError::InvalidInspection
    );
    assert!(!directory.path().join(ATTEMPT_BURN_CLAIM_FILE).exists());
}

fn write_claim_bytes(directory: &std::path::Path, bytes: &[u8]) {
    let mut claim = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(directory.join(ATTEMPT_BURN_CLAIM_FILE))
        .unwrap();
    claim.write_all(bytes).unwrap();
    claim.sync_all().unwrap();
    std::fs::File::open(directory).unwrap().sync_all().unwrap();
}

fn expected_claim_bytes(directory: &std::path::Path) -> Vec<u8> {
    let ledger = std::fs::read(directory.join(ATTEMPT_LEDGER_FILE)).unwrap();
    let lines = exact_lines(&ledger).unwrap();
    let prepared = parse_exact_line::<PreparedRecord>(lines[0]).unwrap();
    expected_claim(&prepared).unwrap().1
}

fn assert_denied_nonresumable(state: AttemptLineageInspection) {
    assert_eq!(state.authorization(), "DENIED");
    assert!(!state.resume_allowed());
}

#[test]
fn inspector_classifies_all_seven_exact_nonresumable_crash_shapes() {
    let pristine_directory = protected_directory();
    let pristine = inspect_attempt_lineage(pristine_directory.path()).unwrap();
    assert_eq!(pristine, AttemptLineageInspection::Pristine);
    assert_denied_nonresumable(pristine);

    let prepared_directory = protected_directory();
    drop(PreparedAttemptLineage::create_new(prepared_directory.path(), binding()).unwrap());
    let prepared = inspect_attempt_lineage(prepared_directory.path()).unwrap();
    assert_eq!(prepared, AttemptLineageInspection::PreparedUnclaimed);
    assert_denied_nonresumable(prepared);

    let empty_directory = protected_directory();
    let empty_prepared =
        PreparedAttemptLineage::create_new(empty_directory.path(), binding()).unwrap();
    write_claim_bytes(empty_directory.path(), b"");
    drop(empty_prepared);
    let empty = inspect_attempt_lineage(empty_directory.path()).unwrap();
    assert_eq!(empty, AttemptLineageInspection::BurnClaimEmpty);
    assert_denied_nonresumable(empty);

    let partial_directory = protected_directory();
    let partial_prepared =
        PreparedAttemptLineage::create_new(partial_directory.path(), binding()).unwrap();
    let claim = expected_claim_bytes(partial_directory.path());
    write_claim_bytes(partial_directory.path(), &claim[..claim.len() / 2]);
    drop(partial_prepared);
    let partial = inspect_attempt_lineage(partial_directory.path()).unwrap();
    assert_eq!(partial, AttemptLineageInspection::BurnClaimPartial);
    assert_denied_nonresumable(partial);

    let durable_directory = protected_directory();
    let durable_prepared =
        PreparedAttemptLineage::create_new(durable_directory.path(), binding()).unwrap();
    let claim = expected_claim_bytes(durable_directory.path());
    write_claim_bytes(durable_directory.path(), &claim);
    drop(durable_prepared);
    let durable = inspect_attempt_lineage(durable_directory.path()).unwrap();
    assert_eq!(durable, AttemptLineageInspection::BurnClaimDurable);
    assert_denied_nonresumable(durable);

    let consumed_directory = protected_directory();
    let consumed = PreparedAttemptLineage::create_new(consumed_directory.path(), binding())
        .unwrap()
        .burn()
        .unwrap();
    drop(consumed);
    let consumed = inspect_attempt_lineage(consumed_directory.path()).unwrap();
    assert_eq!(consumed, AttemptLineageInspection::ConsumedDenied);
    assert_denied_nonresumable(consumed);

    let final_directory = protected_directory();
    let final_lineage = PreparedAttemptLineage::create_new(final_directory.path(), binding())
        .unwrap()
        .burn()
        .unwrap()
        .finish()
        .unwrap();
    drop(final_lineage);
    let final_state = inspect_attempt_lineage(final_directory.path()).unwrap();
    assert_eq!(final_state, AttemptLineageInspection::CompleteDenied);
    assert_denied_nonresumable(final_state);
}

#[test]
fn inspector_rejects_nonprefix_claim_and_ledger_suffix_as_hard_burned_errors() {
    let claim_directory = protected_directory();
    let prepared = PreparedAttemptLineage::create_new(claim_directory.path(), binding()).unwrap();
    write_claim_bytes(claim_directory.path(), b"not-an-exact-claim-prefix");
    drop(prepared);
    assert_eq!(
        inspect_attempt_lineage(claim_directory.path()).unwrap_err(),
        LineageError::InvalidInspection
    );

    let ledger_directory = protected_directory();
    let burned = PreparedAttemptLineage::create_new(ledger_directory.path(), binding())
        .unwrap()
        .burn()
        .unwrap();
    drop(burned);
    let mut ledger = OpenOptions::new()
        .append(true)
        .open(ledger_directory.path().join(ATTEMPT_LEDGER_FILE))
        .unwrap();
    ledger.write_all(b"torn-suffix").unwrap();
    ledger.sync_all().unwrap();
    assert_eq!(
        inspect_attempt_lineage(ledger_directory.path()).unwrap_err(),
        LineageError::InvalidInspection
    );
}

#[test]
fn claim_presence_burns_even_when_the_ledger_is_absent() {
    let directory = protected_directory();
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(directory.path().join(ATTEMPT_BURN_CLAIM_FILE))
        .unwrap()
        .sync_all()
        .unwrap();

    assert_eq!(
        PreparedAttemptLineage::create_new(directory.path(), binding()).unwrap_err(),
        LineageError::AlreadyBurned
    );
    assert!(!directory.path().join(ATTEMPT_LEDGER_FILE).exists());
}

#[test]
fn complete_lineage_is_exact_protected_denied_and_secret_free() {
    let directory = protected_directory();
    let prepared = PreparedAttemptLineage::create_new(directory.path(), binding()).unwrap();
    let mut burned = prepared.burn().unwrap();
    burned.validate_exact().unwrap();
    let final_lineage = burned.finish().unwrap();

    let ledger_path = directory.path().join(ATTEMPT_LEDGER_FILE);
    let claim_path = directory.path().join(ATTEMPT_BURN_CLAIM_FILE);
    let ledger = std::fs::read_to_string(&ledger_path).unwrap();
    let claim = std::fs::read_to_string(&claim_path).unwrap();
    assert_eq!(ledger.lines().count(), 3);
    assert!(ledger.contains("\"record\":\"Prepared\""));
    assert!(ledger.contains("\"record\":\"Consumed\""));
    assert!(ledger.contains("\"record\":\"Final\""));
    assert_eq!(ledger.matches("\"authorization\":\"DENIED\"").count(), 3);
    assert_eq!(ledger.matches("\"production_permit\":false").count(), 3);
    assert_eq!(ledger.matches("\"resume_allowed\":false").count(), 3);
    assert!(claim.contains("\"authorization\":\"DENIED\""));
    assert!(claim.contains("\"production_permit\":false"));
    assert!(claim.contains("\"resume_allowed\":false"));
    for forbidden in [
        "1780449126",
        "poly_signature",
        "poly_timestamp",
        "apiKey",
        "passphrase",
        "synthetic-passphrase",
        "response_digest",
    ] {
        assert!(!format!("{ledger}\n{claim}").contains(forbidden));
    }

    for path in [ledger_path, claim_path] {
        let metadata = path.metadata().unwrap();
        assert_eq!(metadata.mode() & 0o7777, 0o600);
        assert_eq!(metadata.nlink(), 1);
    }
    assert_eq!(directory.path().metadata().unwrap().mode() & 0o7777, 0o700);
    assert!(format!("{final_lineage:?}").contains("DENIED"));
}

#[test]
fn exact_byte_revalidation_detects_a_same_inode_append() {
    let directory = protected_directory();
    let prepared = PreparedAttemptLineage::create_new(directory.path(), binding()).unwrap();
    let mut burned = prepared.burn().unwrap();
    let mut attacker = OpenOptions::new()
        .append(true)
        .open(directory.path().join(ATTEMPT_LEDGER_FILE))
        .unwrap();
    attacker.write_all(b"x").unwrap();
    attacker.sync_all().unwrap();

    assert!(matches!(
        burned.validate_exact(),
        Err(LineageError::Protection | LineageError::AmbiguousTail)
    ));
}
