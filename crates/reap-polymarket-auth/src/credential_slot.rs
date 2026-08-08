use std::fmt;

use sha2::{Digest as _, Sha256};
use zeroize::Zeroizing;

use crate::{L2Credentials, PmAuthError};

const MAX_CREDENTIAL_SLOT_ID_BYTES: usize = 128;
const AUTHENTICATED_JOURNAL_SLOT_PREFIX: &[u8] =
    b"reap.pm.authenticated-journal.credential-slot.v1\0";

/// Move-only, non-secret operator identity for one provisioned L2 credential slot.
///
/// The caller must allocate a new slot ID when rotating the L2 bundle. The ID
/// is deliberately independent of the API key, HMAC secret, and passphrase so
/// no secret-derived value is persisted by the authenticated journal.
pub struct CredentialSlotId(Zeroizing<String>);

impl CredentialSlotId {
    pub fn new(value: String) -> Result<Self, PmAuthError> {
        if value.is_empty()
            || value.len() > MAX_CREDENTIAL_SLOT_ID_BYTES
            || !value.bytes().all(|byte| {
                byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':' | b'/')
            })
        {
            return Err(PmAuthError::InvalidCredentialSlotId);
        }
        Ok(Self(Zeroizing::new(value)))
    }
}

impl fmt::Debug for CredentialSlotId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("CredentialSlotId([REDACTED])")
    }
}

/// Non-secret identity used only to bind an authenticated-journal lease scope.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AuthenticatedJournalCredentialSlotFingerprint([u8; 32]);

impl AuthenticatedJournalCredentialSlotFingerprint {
    /// The only raw-value boundary: consume this typed value while building
    /// the authenticated-journal scope descriptor.
    #[must_use]
    pub const fn into_authenticated_journal_scope_bytes(self) -> [u8; 32] {
        self.0
    }
}

impl fmt::Debug for AuthenticatedJournalCredentialSlotFingerprint {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("AuthenticatedJournalCredentialSlotFingerprint([REDACTED])")
    }
}

impl L2Credentials {
    /// Bind an explicitly versioned operator slot to this credential bundle's
    /// configured EOA without hashing or exposing any L2 credential material.
    #[must_use]
    pub fn authenticated_journal_credential_slot(
        &self,
        slot: CredentialSlotId,
    ) -> AuthenticatedJournalCredentialSlotFingerprint {
        let slot_bytes = slot.0.as_bytes();
        let mut hasher = Sha256::new();
        hasher.update(AUTHENTICATED_JOURNAL_SLOT_PREFIX);
        hasher.update(self.address().bytes());
        hasher.update(
            u16::try_from(slot_bytes.len())
                .expect("credential slot bound fits u16")
                .to_be_bytes(),
        );
        hasher.update(slot_bytes);
        AuthenticatedJournalCredentialSlotFingerprint(hasher.finalize().into())
    }
}

#[cfg(test)]
mod tests {
    use super::CredentialSlotId;
    use crate::{L2CredentialInput, L2Credentials, PmAuthError};

    const ADDRESS: &str = "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266";
    const OTHER_ADDRESS: &str = "0x1111111111111111111111111111111111111111";
    const API_KEY: &str = "00000000-0000-4000-8000-000000000001";
    const OTHER_API_KEY: &str = "00000000-0000-4000-8000-000000000002";
    const SECRET: &str = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";

    fn credentials(address: &str, api_key: &str, passphrase: &str) -> L2Credentials {
        L2Credentials::bind(
            address,
            L2CredentialInput::new(api_key.into(), SECRET.into(), passphrase.into()),
        )
        .expect("synthetic credentials")
    }

    #[test]
    fn slot_fingerprint_is_deterministic_and_domain_bound_without_l2_material() {
        let first = credentials(ADDRESS, API_KEY, "first-passphrase")
            .authenticated_journal_credential_slot(
                CredentialSlotId::new("primary-v1".into()).unwrap(),
            );
        let rotated_material_same_operator_slot =
            credentials(ADDRESS, OTHER_API_KEY, "second-passphrase")
                .authenticated_journal_credential_slot(
                    CredentialSlotId::new("primary-v1".into()).unwrap(),
                );
        let changed_slot = credentials(ADDRESS, API_KEY, "first-passphrase")
            .authenticated_journal_credential_slot(
                CredentialSlotId::new("primary-v2".into()).unwrap(),
            );
        let changed_address = credentials(OTHER_ADDRESS, API_KEY, "first-passphrase")
            .authenticated_journal_credential_slot(
                CredentialSlotId::new("primary-v1".into()).unwrap(),
            );

        assert_eq!(first, rotated_material_same_operator_slot);
        assert_ne!(first, changed_slot);
        assert_ne!(first, changed_address);
        assert_ne!(first.into_authenticated_journal_scope_bytes(), [0; 32]);
    }

    #[test]
    fn slot_input_is_bounded_canonical_and_debug_is_redacted() {
        for invalid in ["", "contains space", "bad@slot"] {
            assert_eq!(
                CredentialSlotId::new(invalid.into()).unwrap_err(),
                PmAuthError::InvalidCredentialSlotId
            );
        }
        assert_eq!(
            CredentialSlotId::new("x".repeat(129)).unwrap_err(),
            PmAuthError::InvalidCredentialSlotId
        );

        let slot = CredentialSlotId::new("synthetic-slot-v1".into()).unwrap();
        assert_eq!(format!("{slot:?}"), "CredentialSlotId([REDACTED])");
        let fingerprint = credentials(ADDRESS, API_KEY, "first-passphrase")
            .authenticated_journal_credential_slot(slot);
        let rendered = format!("{fingerprint:?}");
        assert_eq!(
            rendered,
            "AuthenticatedJournalCredentialSlotFingerprint([REDACTED])"
        );
        assert!(!rendered.contains("synthetic-slot-v1"));
        assert!(!rendered.contains(API_KEY));
    }
}
