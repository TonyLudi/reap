use super::*;

pub(in crate::journal) fn test_scope() -> PmJournalScopeV1 {
    let address = EvmAddress::from_bytes([0x11; 20]).expect("test address");
    let account_scope = PmAccountScope::new(
        reap_pm_core::PmEnvironmentId::new("journal-test").expect("test environment"),
        reap_pm_core::PmChainId::new(137).expect("test chain"),
        reap_pm_core::PmSignerId::new(address),
        reap_pm_core::PmFunderId::new(address),
        PmAccountHandle::from_ordinal(7),
    );
    let instrument = PmInstrumentId::new(
        reap_pm_core::PmMarketId::from_bytes([0x22; 32]).expect("test market"),
        reap_pm_core::PmTokenId::new(U256::from_u64(42)).expect("test token"),
    );
    let mut scope = PmJournalScopeV1 {
        product: "reap-pm".to_owned(),
        schema_family: PM_MUTATION_JOURNAL_FAMILY.to_owned(),
        schema_version: PM_MUTATION_JOURNAL_VERSION,
        account_scope,
        configured_instruments: [instrument],
        configuration_fingerprint: PmJournalFingerprintV1::from_bytes([0x33; 32]),
        authentication_enabled: false,
        production_authorized: false,
        account_signature_profile: PmAccountSignatureProfile::EoaType0,
        scope_fingerprint: PmJournalFingerprintV1::from_bytes(ZERO_HASH),
    };
    scope.scope_fingerprint = scope.calculate_fingerprint().expect("test fingerprint");
    scope.validate().expect("valid test scope");
    scope
}

pub(in crate::journal) fn test_proxy_scope() -> PmJournalScopeV1 {
    let signer = EvmAddress::from_bytes([0x11; 20]).expect("test signer");
    let funder = EvmAddress::from_bytes([0x44; 20]).expect("test proxy funder");
    let mut scope = test_scope();
    scope.schema_version = PM_T2_PROXY_MUTATION_JOURNAL_VERSION;
    scope.account_scope = PmAccountScope::new(
        scope.account_scope.environment(),
        scope.account_scope.chain(),
        reap_pm_core::PmSignerId::new(signer),
        reap_pm_core::PmFunderId::new(funder),
        scope.account_scope.handle(),
    );
    scope.account_signature_profile = PmAccountSignatureProfile::ProxyType1;
    scope.scope_fingerprint = scope.calculate_fingerprint().expect("proxy fingerprint");
    scope.validate().expect("valid proxy test scope");
    scope
}
