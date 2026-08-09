use reap_pm_core::{
    EvmAddress, PmAccountHandle, PmChainId, PmEnvironmentId, PmFunderId, PmMarketId, PmSignerId,
    PmTokenId, U256,
};

use super::*;

pub(super) fn test_scope_with_profile_and_identities(
    account_signature_profile: PmAccountSignatureProfile,
    signer: [u8; 20],
    funder: [u8; 20],
    configuration_fingerprint: [u8; 32],
    credential_slot_fingerprint: [u8; 32],
) -> PmAuthenticatedJournalScopeV1 {
    let signer = EvmAddress::from_bytes(signer).expect("test signer");
    let funder = EvmAddress::from_bytes(funder).expect("test funder");
    let account_scope = PmAccountScope::new(
        PmEnvironmentId::new("authenticated-journal-test").expect("environment"),
        PmChainId::new(137).expect("chain"),
        PmSignerId::new(signer),
        PmFunderId::new(funder),
        PmAccountHandle::from_ordinal(9),
    );
    let configured_instrument = PmInstrumentId::new(
        PmMarketId::from_bytes([0x22; 32]).expect("market"),
        PmTokenId::new(U256::from_u64(42)).expect("token"),
    );
    let mut scope = PmAuthenticatedJournalScopeV1 {
        product: "reap-pm".to_owned(),
        schema_family: PM_AUTHENTICATED_JOURNAL_FAMILY.to_owned(),
        schema_version: match account_signature_profile {
            PmAccountSignatureProfile::EoaType0 => PM_AUTHENTICATED_JOURNAL_VERSION,
            PmAccountSignatureProfile::ProxyType1 => PM_T2_PROXY_AUTHENTICATED_JOURNAL_VERSION,
        },
        account_scope,
        configured_instrument,
        configuration_fingerprint: PmAuthenticatedJournalFingerprintV1::from_bytes(
            configuration_fingerprint,
        ),
        credential_slot_fingerprint:
            PmAuthenticatedCredentialSlotFingerprintV1::from_authenticated_journal_scope_bytes(
                credential_slot_fingerprint,
            ),
        production_order_entry_authorized: false,
        account_signature_profile,
        scope_fingerprint: PmAuthenticatedJournalFingerprintV1::from_bytes(ZERO_HASH),
    };
    scope.scope_fingerprint = scope.calculate_fingerprint().expect("fingerprint");
    scope.validate().expect("valid test scope");
    scope
}
