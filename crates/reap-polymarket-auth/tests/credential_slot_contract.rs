use reap_polymarket_auth::{CredentialSlotId, L2CredentialInput, L2Credentials};

const ADDRESS: &str = "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266";
const API_KEY: &str = "00000000-0000-4000-8000-000000000001";
const API_SECRET: &str = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";
const PASSPHRASE: &str = "synthetic-passphrase";

#[test]
fn journal_slot_boundary_is_redacted_and_exposes_only_its_purpose() {
    let credentials = L2Credentials::bind(
        ADDRESS,
        L2CredentialInput::new(API_KEY.into(), API_SECRET.into(), PASSPHRASE.into()),
    )
    .unwrap();
    let slot = CredentialSlotId::new("pm-primary-credentials-v1".into()).unwrap();
    let fingerprint = credentials.authenticated_journal_credential_slot(slot);
    let rendered = format!("{fingerprint:?}");
    assert_eq!(
        rendered,
        "AuthenticatedJournalCredentialSlotFingerprint([REDACTED])"
    );
    assert!(!rendered.contains(API_KEY));
    assert!(!rendered.contains(API_SECRET));
    assert!(!rendered.contains(PASSPHRASE));
    assert_ne!(
        fingerprint.into_authenticated_journal_scope_bytes(),
        [0; 32]
    );
}

#[test]
fn source_never_hashes_l2_material_or_adds_a_generic_fingerprint_escape() {
    let source = include_str!("../src/credential_slot.rs");
    assert!(source.contains("reap.pm.authenticated-journal.credential-slot.v1\\0"));
    for forbidden in [
        "self.api_key(",
        "self.hmac_key(",
        "self.passphrase(",
        "impl std::fmt::Display for AuthenticatedJournalCredentialSlotFingerprint",
        "pub const fn bytes(",
        "pub fn as_bytes(",
        "impl Serialize for AuthenticatedJournalCredentialSlotFingerprint",
    ] {
        assert!(
            !source.contains(forbidden),
            "credential-slot source crossed forbidden boundary: {forbidden}"
        );
    }
    assert!(source.contains("into_authenticated_journal_scope_bytes"));
}
