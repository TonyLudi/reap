use std::fmt;

use reap_pm_core::{
    PmAccountHandle, PmAccountScope, PmClientOrderKey, PmInstrumentId, PmVenueOrderKey,
};
use reap_pm_live_contracts::{PmAccountSignatureProfile, PmConnectivityConfig};
use serde::{Deserialize, Deserializer, Serialize, Serializer, de};
use sha2::{Digest as _, Sha256};
use thiserror::Error;

pub(super) const PM_AUTHENTICATED_JOURNAL_FAMILY: &str = "reap-pm-authenticated-mutation-journal";
// Re-frozen before coordinator integration: Prepared records persist only a
// secret-free fixed-profile semantic identity, never an authenticated-body
// digest. Ambiguous/out-of-profile results may carry no observed identity or
// the exact contradictory identity returned by the venue. Conclusive
// place/cancel outcomes bind the observed identity to the signed/fixed
// expected identity. V1 remains the frozen EOA domain and is accepted without
// migration; PM-T2 proxy scopes use the separately fingerprinted V2 domain.
pub(super) const PM_AUTHENTICATED_JOURNAL_VERSION: u16 = 1;
// Offline/loopback PM-T2 evidence only. V2 intentionally has no reviewed
// live-authorization or take-once preflight digest. A future live trial must
// use a new version/domain (or a dedicated controlled-trial family) rather
// than reinterpreting these records as production authority.
pub(super) const PM_T2_PROXY_AUTHENTICATED_JOURNAL_VERSION: u16 = 2;
pub(super) const MAX_PM_AUTHENTICATED_JOURNAL_LINE_BYTES: usize = 4 * 1_024;
pub(super) const MAX_PM_AUTHENTICATED_JOURNAL_BYTES: u64 = 64 * 1_024 * 1_024;
pub(super) const MAX_PM_AUTHENTICATED_JOURNAL_RECORDS: usize = 65_536;

const SCOPE_HASH_PREFIX_V1: &[u8] = b"reap.pm.authenticated-journal.scope.v1\0";
const SCOPE_HASH_PREFIX_V2: &[u8] = b"reap.pm.authenticated-journal.scope.v2.proxy-type-1\0";
const REQUEST_HASH_PREFIX_V1: &[u8] = b"reap.pm.authenticated-request.v1\0";
const REQUEST_HASH_PREFIX_V2: &[u8] = b"reap.pm.authenticated-request.v2.proxy-type-1\0";
const ZERO_HASH: [u8; 32] = [0; 32];
const MIN_L2_TIMESTAMP: u64 = 1_000_000_000;
const MAX_L2_TIMESTAMP: u64 = 9_999_999_999;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct PmAuthenticatedJournalFingerprintV1([u8; 32]);

impl PmAuthenticatedJournalFingerprintV1 {
    pub(super) const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    pub(super) const fn bytes(self) -> [u8; 32] {
        self.0
    }
}

impl fmt::Debug for PmAuthenticatedJournalFingerprintV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("PmAuthenticatedJournalFingerprintV1")
            .field(&LowerHex(&self.0))
            .finish()
    }
}

impl Serialize for PmAuthenticatedJournalFingerprintV1 {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.collect_str(&LowerHex(&self.0))
    }
}

impl<'de> Deserialize<'de> for PmAuthenticatedJournalFingerprintV1 {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        decode_hex32(String::deserialize(deserializer)?.as_str())
            .map(Self)
            .ok_or_else(|| de::Error::custom("invalid authenticated-journal fingerprint"))
    }
}

/// Opaque, non-secret identity of the explicitly configured L2 credential slot.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct PmAuthenticatedCredentialSlotFingerprintV1([u8; 32]);

impl PmAuthenticatedCredentialSlotFingerprintV1 {
    const fn from_authenticated_journal_scope_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }
}

impl fmt::Debug for PmAuthenticatedCredentialSlotFingerprintV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PmAuthenticatedCredentialSlotFingerprintV1([REDACTED])")
    }
}

impl Serialize for PmAuthenticatedCredentialSlotFingerprintV1 {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.collect_str(&LowerHex(&self.0))
    }
}

impl<'de> Deserialize<'de> for PmAuthenticatedCredentialSlotFingerprintV1 {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        decode_hex32(String::deserialize(deserializer)?.as_str())
            .map(Self)
            .ok_or_else(|| de::Error::custom("invalid authenticated credential-slot fingerprint"))
    }
}

/// A non-secret SHA-256 commitment over public scope, identity, or semantic
/// request evidence. Credential material, authenticated bytes, and their
/// derived hashes are not valid inputs to this journal-owned representation.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct PmAuthenticatedCommitmentV1([u8; 32]);

impl PmAuthenticatedCommitmentV1 {
    pub(super) const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    pub(super) const fn bytes(self) -> [u8; 32] {
        self.0
    }
}

impl fmt::Debug for PmAuthenticatedCommitmentV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PmAuthenticatedCommitmentV1(<redacted-commitment>)")
    }
}

impl Serialize for PmAuthenticatedCommitmentV1 {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.collect_str(&LowerHex(&self.0))
    }
}

impl<'de> Deserialize<'de> for PmAuthenticatedCommitmentV1 {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        decode_hex32(String::deserialize(deserializer)?.as_str())
            .map(Self)
            .ok_or_else(|| de::Error::custom("invalid authenticated request commitment"))
    }
}

struct LowerHex<'a>(&'a [u8]);

impl fmt::Display for LowerHex<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in self.0 {
            write!(formatter, "{byte:02x}")?;
        }
        Ok(())
    }
}

impl fmt::Debug for LowerHex<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, formatter)
    }
}

fn decode_hex32(input: &str) -> Option<[u8; 32]> {
    if input.len() != 64
        || input
            .bytes()
            .any(|byte| !byte.is_ascii_digit() && !(b'a'..=b'f').contains(&byte))
    {
        return None;
    }
    let mut output = [0_u8; 32];
    for (index, byte) in output.iter_mut().enumerate() {
        *byte = (decode_nibble(input.as_bytes()[index * 2])? << 4)
            | decode_nibble(input.as_bytes()[index * 2 + 1])?;
    }
    Some(output)
}

const fn decode_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        _ => None,
    }
}

/// Exact, secret-free lease scope for one authenticated-dispatch journal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PmAuthenticatedJournalScopeV1 {
    product: String,
    schema_family: String,
    schema_version: u16,
    account_scope: PmAccountScope,
    configured_instrument: PmInstrumentId,
    configuration_fingerprint: PmAuthenticatedJournalFingerprintV1,
    credential_slot_fingerprint: PmAuthenticatedCredentialSlotFingerprintV1,
    production_order_entry_authorized: bool,
    account_signature_profile: PmAccountSignatureProfile,
    scope_fingerprint: PmAuthenticatedJournalFingerprintV1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum PmAuthenticatedProxySignatureProfileV2 {
    #[serde(rename = "proxy_type_1")]
    ProxyType1,
}

#[derive(Serialize)]
struct ScopeWireV1Ref<'a> {
    product: &'a str,
    schema_family: &'a str,
    schema_version: u16,
    account_scope: PmAccountScope,
    configured_instrument: PmInstrumentId,
    configuration_fingerprint: PmAuthenticatedJournalFingerprintV1,
    credential_slot_fingerprint: PmAuthenticatedCredentialSlotFingerprintV1,
    production_order_entry_authorized: bool,
    scope_fingerprint: PmAuthenticatedJournalFingerprintV1,
}

#[derive(Serialize)]
struct ScopeWireV2Ref<'a> {
    product: &'a str,
    schema_family: &'a str,
    schema_version: u16,
    account_scope: PmAccountScope,
    configured_instrument: PmInstrumentId,
    configuration_fingerprint: PmAuthenticatedJournalFingerprintV1,
    credential_slot_fingerprint: PmAuthenticatedCredentialSlotFingerprintV1,
    production_order_entry_authorized: bool,
    account_signature_profile: PmAuthenticatedProxySignatureProfileV2,
    scope_fingerprint: PmAuthenticatedJournalFingerprintV1,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ScopeWireV1 {
    product: String,
    schema_family: String,
    schema_version: u16,
    account_scope: PmAccountScope,
    configured_instrument: PmInstrumentId,
    configuration_fingerprint: PmAuthenticatedJournalFingerprintV1,
    credential_slot_fingerprint: PmAuthenticatedCredentialSlotFingerprintV1,
    production_order_entry_authorized: bool,
    scope_fingerprint: PmAuthenticatedJournalFingerprintV1,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ScopeWireV2 {
    product: String,
    schema_family: String,
    schema_version: u16,
    account_scope: PmAccountScope,
    configured_instrument: PmInstrumentId,
    configuration_fingerprint: PmAuthenticatedJournalFingerprintV1,
    credential_slot_fingerprint: PmAuthenticatedCredentialSlotFingerprintV1,
    production_order_entry_authorized: bool,
    account_signature_profile: PmAuthenticatedProxySignatureProfileV2,
    scope_fingerprint: PmAuthenticatedJournalFingerprintV1,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum ScopeWire {
    V1(ScopeWireV1),
    V2(ScopeWireV2),
}

#[derive(Serialize)]
struct ScopeFingerprintBasisV1<'a> {
    product: &'a str,
    schema_family: &'a str,
    schema_version: u16,
    account_scope: PmAccountScope,
    configured_instrument: PmInstrumentId,
    configuration_fingerprint: PmAuthenticatedJournalFingerprintV1,
    credential_slot_fingerprint: PmAuthenticatedCredentialSlotFingerprintV1,
    production_order_entry_authorized: bool,
}

#[derive(Serialize)]
struct ScopeFingerprintBasisV2<'a> {
    product: &'a str,
    schema_family: &'a str,
    schema_version: u16,
    account_scope: PmAccountScope,
    configured_instrument: PmInstrumentId,
    configuration_fingerprint: PmAuthenticatedJournalFingerprintV1,
    credential_slot_fingerprint: PmAuthenticatedCredentialSlotFingerprintV1,
    production_order_entry_authorized: bool,
    account_signature_profile: PmAuthenticatedProxySignatureProfileV2,
}

impl<'a> From<&'a PmAuthenticatedJournalScopeV1> for ScopeWireV1Ref<'a> {
    fn from(scope: &'a PmAuthenticatedJournalScopeV1) -> Self {
        Self {
            product: &scope.product,
            schema_family: &scope.schema_family,
            schema_version: scope.schema_version,
            account_scope: scope.account_scope,
            configured_instrument: scope.configured_instrument,
            configuration_fingerprint: scope.configuration_fingerprint,
            credential_slot_fingerprint: scope.credential_slot_fingerprint,
            production_order_entry_authorized: scope.production_order_entry_authorized,
            scope_fingerprint: scope.scope_fingerprint,
        }
    }
}

impl<'a> From<&'a PmAuthenticatedJournalScopeV1> for ScopeWireV2Ref<'a> {
    fn from(scope: &'a PmAuthenticatedJournalScopeV1) -> Self {
        Self {
            product: &scope.product,
            schema_family: &scope.schema_family,
            schema_version: scope.schema_version,
            account_scope: scope.account_scope,
            configured_instrument: scope.configured_instrument,
            configuration_fingerprint: scope.configuration_fingerprint,
            credential_slot_fingerprint: scope.credential_slot_fingerprint,
            production_order_entry_authorized: scope.production_order_entry_authorized,
            account_signature_profile: PmAuthenticatedProxySignatureProfileV2::ProxyType1,
            scope_fingerprint: scope.scope_fingerprint,
        }
    }
}

impl<'a> From<&'a PmAuthenticatedJournalScopeV1> for ScopeFingerprintBasisV1<'a> {
    fn from(scope: &'a PmAuthenticatedJournalScopeV1) -> Self {
        Self {
            product: &scope.product,
            schema_family: &scope.schema_family,
            schema_version: scope.schema_version,
            account_scope: scope.account_scope,
            configured_instrument: scope.configured_instrument,
            configuration_fingerprint: scope.configuration_fingerprint,
            credential_slot_fingerprint: scope.credential_slot_fingerprint,
            production_order_entry_authorized: scope.production_order_entry_authorized,
        }
    }
}

impl<'a> From<&'a PmAuthenticatedJournalScopeV1> for ScopeFingerprintBasisV2<'a> {
    fn from(scope: &'a PmAuthenticatedJournalScopeV1) -> Self {
        Self {
            product: &scope.product,
            schema_family: &scope.schema_family,
            schema_version: scope.schema_version,
            account_scope: scope.account_scope,
            configured_instrument: scope.configured_instrument,
            configuration_fingerprint: scope.configuration_fingerprint,
            credential_slot_fingerprint: scope.credential_slot_fingerprint,
            production_order_entry_authorized: scope.production_order_entry_authorized,
            account_signature_profile: PmAuthenticatedProxySignatureProfileV2::ProxyType1,
        }
    }
}

impl Serialize for PmAuthenticatedJournalScopeV1 {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self.account_signature_profile {
            PmAccountSignatureProfile::EoaType0 => ScopeWireV1Ref::from(self).serialize(serializer),
            PmAccountSignatureProfile::ProxyType1 => {
                ScopeWireV2Ref::from(self).serialize(serializer)
            }
        }
    }
}

impl<'de> Deserialize<'de> for PmAuthenticatedJournalScopeV1 {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let scope = match ScopeWire::deserialize(deserializer)? {
            ScopeWire::V1(scope) => Self {
                product: scope.product,
                schema_family: scope.schema_family,
                schema_version: scope.schema_version,
                account_scope: scope.account_scope,
                configured_instrument: scope.configured_instrument,
                configuration_fingerprint: scope.configuration_fingerprint,
                credential_slot_fingerprint: scope.credential_slot_fingerprint,
                production_order_entry_authorized: scope.production_order_entry_authorized,
                account_signature_profile: PmAccountSignatureProfile::EoaType0,
                scope_fingerprint: scope.scope_fingerprint,
            },
            ScopeWire::V2(scope) => {
                let PmAuthenticatedProxySignatureProfileV2::ProxyType1 =
                    scope.account_signature_profile;
                Self {
                    product: scope.product,
                    schema_family: scope.schema_family,
                    schema_version: scope.schema_version,
                    account_scope: scope.account_scope,
                    configured_instrument: scope.configured_instrument,
                    configuration_fingerprint: scope.configuration_fingerprint,
                    credential_slot_fingerprint: scope.credential_slot_fingerprint,
                    production_order_entry_authorized: scope.production_order_entry_authorized,
                    account_signature_profile: PmAccountSignatureProfile::ProxyType1,
                    scope_fingerprint: scope.scope_fingerprint,
                }
            }
        };
        Ok(scope)
    }
}

impl PmAuthenticatedJournalScopeV1 {
    pub(crate) fn from_config(
        config: &PmConnectivityConfig,
        credential_slot_fingerprint: [u8; 32],
    ) -> Result<Self, PmAuthenticatedJournalSchemaError> {
        let mut scope = Self {
            product: "reap-pm".to_owned(),
            schema_family: PM_AUTHENTICATED_JOURNAL_FAMILY.to_owned(),
            schema_version: match config.account().signature_profile() {
                PmAccountSignatureProfile::EoaType0 => PM_AUTHENTICATED_JOURNAL_VERSION,
                PmAccountSignatureProfile::ProxyType1 => PM_T2_PROXY_AUTHENTICATED_JOURNAL_VERSION,
            },
            account_scope: config.account().account_scope(),
            configured_instrument: config.account().instrument_id(),
            configuration_fingerprint: PmAuthenticatedJournalFingerprintV1::from_bytes(
                config.public().configuration_fingerprint().bytes(),
            ),
            credential_slot_fingerprint:
                PmAuthenticatedCredentialSlotFingerprintV1::from_authenticated_journal_scope_bytes(
                    credential_slot_fingerprint,
                ),
            production_order_entry_authorized: false,
            account_signature_profile: config.account().signature_profile(),
            scope_fingerprint: PmAuthenticatedJournalFingerprintV1::from_bytes(ZERO_HASH),
        };
        scope.scope_fingerprint = scope.calculate_fingerprint()?;
        scope.validate()?;
        Ok(scope)
    }

    pub(crate) const fn fingerprint(&self) -> PmAuthenticatedJournalFingerprintV1 {
        self.scope_fingerprint
    }

    pub(crate) const fn account_scope(&self) -> PmAccountScope {
        self.account_scope
    }

    pub(crate) const fn account_signature_profile(&self) -> PmAccountSignatureProfile {
        self.account_signature_profile
    }

    pub(super) const fn schema_version(&self) -> u16 {
        self.schema_version
    }

    pub(crate) const fn account(&self) -> PmAccountHandle {
        self.account_scope.handle()
    }

    pub(crate) const fn instrument(&self) -> PmInstrumentId {
        self.configured_instrument
    }

    /// Exact non-secret configuration identity shared with the Goal-F scope.
    /// Cross-journal recovery compares this value before pairing any attempt.
    pub(crate) const fn configuration_fingerprint(&self) -> [u8; 32] {
        self.configuration_fingerprint.bytes()
    }

    pub(super) fn validate(&self) -> Result<(), PmAuthenticatedJournalSchemaError> {
        let expected_version = match self.account_signature_profile {
            PmAccountSignatureProfile::EoaType0 => PM_AUTHENTICATED_JOURNAL_VERSION,
            PmAccountSignatureProfile::ProxyType1 => PM_T2_PROXY_AUTHENTICATED_JOURNAL_VERSION,
        };
        if self.product != "reap-pm"
            || self.schema_family != PM_AUTHENTICATED_JOURNAL_FAMILY
            || self.schema_version != expected_version
        {
            return Err(PmAuthenticatedJournalSchemaError::WrongScopeDomain);
        }
        if self.production_order_entry_authorized {
            return Err(PmAuthenticatedJournalSchemaError::ProductionAuthorityForbidden);
        }
        if self.credential_slot_fingerprint.0 == ZERO_HASH {
            return Err(PmAuthenticatedJournalSchemaError::MissingCredentialSlotFingerprint);
        }
        if !self
            .account_signature_profile
            .matches_account_scope(self.account_scope)
        {
            return Err(PmAuthenticatedJournalSchemaError::AccountIdentityMismatch);
        }
        if self.account_signature_profile == PmAccountSignatureProfile::ProxyType1
            && self.account_scope.chain().value() != 137
        {
            return Err(PmAuthenticatedJournalSchemaError::AccountIdentityMismatch);
        }
        if self.calculate_fingerprint()? != self.scope_fingerprint {
            return Err(PmAuthenticatedJournalSchemaError::ScopeFingerprintMismatch);
        }
        Ok(())
    }

    fn calculate_fingerprint(
        &self,
    ) -> Result<PmAuthenticatedJournalFingerprintV1, PmAuthenticatedJournalSchemaError> {
        let mut hasher = Sha256::new();
        match self.account_signature_profile {
            PmAccountSignatureProfile::EoaType0 => {
                hasher.update(SCOPE_HASH_PREFIX_V1);
                serde_json::to_writer(
                    HashWriter(&mut hasher),
                    &ScopeFingerprintBasisV1::from(self),
                )?;
            }
            PmAccountSignatureProfile::ProxyType1 => {
                hasher.update(SCOPE_HASH_PREFIX_V2);
                serde_json::to_writer(
                    HashWriter(&mut hasher),
                    &ScopeFingerprintBasisV2::from(self),
                )?;
            }
        }
        Ok(PmAuthenticatedJournalFingerprintV1::from_bytes(
            hasher.finalize().into(),
        ))
    }
}

struct HashWriter<'a>(&'a mut Sha256);

impl std::io::Write for HashWriter<'_> {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.update(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Coordinator-owned identity copied into every authenticated transition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PmAuthenticatedCoordinatorIdentityV1 {
    pub(super) client_order: PmClientOrderKey,
    pub(super) instrument: PmInstrumentId,
}

impl PmAuthenticatedCoordinatorIdentityV1 {
    pub(crate) const fn new(client_order: PmClientOrderKey, instrument: PmInstrumentId) -> Self {
        Self {
            client_order,
            instrument,
        }
    }

    fn validate(
        self,
        scope: &PmAuthenticatedJournalScopeV1,
    ) -> Result<(), PmAuthenticatedJournalSchemaError> {
        if self.client_order.account() != scope.account() || self.instrument != scope.instrument() {
            return Err(PmAuthenticatedJournalSchemaError::RecordOutsideScope);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case")]
pub(crate) enum PmAuthenticatedOperationKeyV1 {
    Place {
        coordinator: PmAuthenticatedCoordinatorIdentityV1,
    },
    Cancel {
        coordinator: PmAuthenticatedCoordinatorIdentityV1,
        venue_order: PmVenueOrderKey,
    },
}

impl PmAuthenticatedOperationKeyV1 {
    pub(crate) const fn place(coordinator: PmAuthenticatedCoordinatorIdentityV1) -> Self {
        Self::Place { coordinator }
    }

    pub(crate) const fn cancel(
        coordinator: PmAuthenticatedCoordinatorIdentityV1,
        venue_order: PmVenueOrderKey,
    ) -> Self {
        Self::Cancel {
            coordinator,
            venue_order,
        }
    }

    pub(super) const fn coordinator(self) -> PmAuthenticatedCoordinatorIdentityV1 {
        match self {
            Self::Place { coordinator } | Self::Cancel { coordinator, .. } => coordinator,
        }
    }

    pub(super) fn validate(
        self,
        scope: &PmAuthenticatedJournalScopeV1,
    ) -> Result<(), PmAuthenticatedJournalSchemaError> {
        self.coordinator().validate(scope)?;
        if let Self::Cancel { venue_order, .. } = self
            && venue_order.account() != scope.account()
        {
            return Err(PmAuthenticatedJournalSchemaError::RecordOutsideScope);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PmAuthenticatedJournalHeaderV1 {
    pub(super) scope: PmAuthenticatedJournalScopeV1,
}

impl PmAuthenticatedJournalHeaderV1 {
    pub(crate) const fn new(scope: PmAuthenticatedJournalScopeV1) -> Self {
        Self { scope }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PmAuthenticatedPlacePreparedV1 {
    pub(super) operation: PmAuthenticatedOperationKeyV1,
    pub(super) prior_intent_sequence: u64,
    /// Secret-free fixed-profile identity supplied by the auth edge. This is
    /// deliberately not a hash of the serialized/authenticated body.
    pub(super) semantic_request_commitment: PmAuthenticatedCommitmentV1,
    pub(super) request_commitment: PmAuthenticatedCommitmentV1,
    pub(super) expected_order_id: PmAuthenticatedCommitmentV1,
    pub(super) l2_timestamp_seconds: u64,
}

impl PmAuthenticatedPlacePreparedV1 {
    #[allow(
        clippy::too_many_arguments,
        reason = "the durable request identity is intentionally explicit"
    )]
    pub(crate) fn new(
        scope: &PmAuthenticatedJournalScopeV1,
        coordinator: PmAuthenticatedCoordinatorIdentityV1,
        prior_intent_sequence: u64,
        semantic_request_commitment: [u8; 32],
        expected_order_id: [u8; 32],
        l2_timestamp_seconds: u64,
    ) -> Result<Self, PmAuthenticatedJournalSchemaError> {
        let operation = PmAuthenticatedOperationKeyV1::place(coordinator);
        let mut prepared = Self {
            operation,
            prior_intent_sequence,
            semantic_request_commitment: PmAuthenticatedCommitmentV1::from_bytes(
                semantic_request_commitment,
            ),
            request_commitment: PmAuthenticatedCommitmentV1::from_bytes(ZERO_HASH),
            expected_order_id: PmAuthenticatedCommitmentV1::from_bytes(expected_order_id),
            l2_timestamp_seconds,
        };
        prepared.request_commitment = prepared.calculate_request_commitment(scope)?;
        prepared.validate(scope)?;
        Ok(prepared)
    }

    fn calculate_request_commitment(
        self,
        scope: &PmAuthenticatedJournalScopeV1,
    ) -> Result<PmAuthenticatedCommitmentV1, PmAuthenticatedJournalSchemaError> {
        #[derive(Serialize)]
        struct Basis {
            scope: PmAuthenticatedJournalFingerprintV1,
            method: &'static str,
            route: &'static str,
            operation: PmAuthenticatedOperationKeyV1,
            prior_intent_sequence: u64,
            semantic_request_commitment: PmAuthenticatedCommitmentV1,
            expected_order_id: PmAuthenticatedCommitmentV1,
            l2_timestamp_seconds: u64,
        }
        request_hash(
            scope,
            &Basis {
                scope: scope.fingerprint(),
                method: "POST",
                route: "/order",
                operation: self.operation,
                prior_intent_sequence: self.prior_intent_sequence,
                semantic_request_commitment: self.semantic_request_commitment,
                expected_order_id: self.expected_order_id,
                l2_timestamp_seconds: self.l2_timestamp_seconds,
            },
        )
    }

    fn validate(
        self,
        scope: &PmAuthenticatedJournalScopeV1,
    ) -> Result<(), PmAuthenticatedJournalSchemaError> {
        self.operation.validate(scope)?;
        if !matches!(self.operation, PmAuthenticatedOperationKeyV1::Place { .. }) {
            return Err(PmAuthenticatedJournalSchemaError::OperationMismatch);
        }
        validate_prior_and_timestamp(self.prior_intent_sequence, self.l2_timestamp_seconds)?;
        if self.calculate_request_commitment(scope)? != self.request_commitment {
            return Err(PmAuthenticatedJournalSchemaError::RequestCommitmentMismatch);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PmAuthenticatedCancelPreparedV1 {
    pub(super) operation: PmAuthenticatedOperationKeyV1,
    pub(super) prior_cancel_sequence: u64,
    /// Secret-free exact-owned cancel identity; never an authenticated-body
    /// digest or other credential-derived value.
    pub(super) semantic_request_commitment: PmAuthenticatedCommitmentV1,
    pub(super) request_commitment: PmAuthenticatedCommitmentV1,
    pub(super) fixed_order_id: PmAuthenticatedCommitmentV1,
    pub(super) l2_timestamp_seconds: u64,
}

impl PmAuthenticatedCancelPreparedV1 {
    #[allow(
        clippy::too_many_arguments,
        reason = "the durable request identity is intentionally explicit"
    )]
    pub(crate) fn new(
        scope: &PmAuthenticatedJournalScopeV1,
        coordinator: PmAuthenticatedCoordinatorIdentityV1,
        venue_order: PmVenueOrderKey,
        prior_cancel_sequence: u64,
        semantic_request_commitment: [u8; 32],
        fixed_order_id: [u8; 32],
        l2_timestamp_seconds: u64,
    ) -> Result<Self, PmAuthenticatedJournalSchemaError> {
        let operation = PmAuthenticatedOperationKeyV1::cancel(coordinator, venue_order);
        let mut prepared = Self {
            operation,
            prior_cancel_sequence,
            semantic_request_commitment: PmAuthenticatedCommitmentV1::from_bytes(
                semantic_request_commitment,
            ),
            request_commitment: PmAuthenticatedCommitmentV1::from_bytes(ZERO_HASH),
            fixed_order_id: PmAuthenticatedCommitmentV1::from_bytes(fixed_order_id),
            l2_timestamp_seconds,
        };
        prepared.request_commitment = prepared.calculate_request_commitment(scope)?;
        prepared.validate(scope)?;
        Ok(prepared)
    }

    fn calculate_request_commitment(
        self,
        scope: &PmAuthenticatedJournalScopeV1,
    ) -> Result<PmAuthenticatedCommitmentV1, PmAuthenticatedJournalSchemaError> {
        #[derive(Serialize)]
        struct Basis {
            scope: PmAuthenticatedJournalFingerprintV1,
            method: &'static str,
            route: &'static str,
            operation: PmAuthenticatedOperationKeyV1,
            prior_cancel_sequence: u64,
            semantic_request_commitment: PmAuthenticatedCommitmentV1,
            fixed_order_id: PmAuthenticatedCommitmentV1,
            l2_timestamp_seconds: u64,
        }
        request_hash(
            scope,
            &Basis {
                scope: scope.fingerprint(),
                method: "DELETE",
                route: "/order",
                operation: self.operation,
                prior_cancel_sequence: self.prior_cancel_sequence,
                semantic_request_commitment: self.semantic_request_commitment,
                fixed_order_id: self.fixed_order_id,
                l2_timestamp_seconds: self.l2_timestamp_seconds,
            },
        )
    }

    fn validate(
        self,
        scope: &PmAuthenticatedJournalScopeV1,
    ) -> Result<(), PmAuthenticatedJournalSchemaError> {
        self.operation.validate(scope)?;
        let PmAuthenticatedOperationKeyV1::Cancel { venue_order, .. } = self.operation else {
            return Err(PmAuthenticatedJournalSchemaError::OperationMismatch);
        };
        validate_prior_and_timestamp(self.prior_cancel_sequence, self.l2_timestamp_seconds)?;
        let parsed = decode_prefixed_hex32(venue_order.id().as_str())
            .ok_or(PmAuthenticatedJournalSchemaError::FixedOrderIdentityMismatch)?;
        if PmAuthenticatedCommitmentV1::from_bytes(parsed) != self.fixed_order_id {
            return Err(PmAuthenticatedJournalSchemaError::FixedOrderIdentityMismatch);
        }
        if self.calculate_request_commitment(scope)? != self.request_commitment {
            return Err(PmAuthenticatedJournalSchemaError::RequestCommitmentMismatch);
        }
        Ok(())
    }
}

fn validate_prior_and_timestamp(
    prior_sequence: u64,
    timestamp_seconds: u64,
) -> Result<(), PmAuthenticatedJournalSchemaError> {
    if prior_sequence == 0 {
        return Err(PmAuthenticatedJournalSchemaError::ZeroPriorSequence);
    }
    if !(MIN_L2_TIMESTAMP..=MAX_L2_TIMESTAMP).contains(&timestamp_seconds) {
        return Err(PmAuthenticatedJournalSchemaError::InvalidL2Timestamp);
    }
    Ok(())
}

fn decode_prefixed_hex32(input: &str) -> Option<[u8; 32]> {
    input.strip_prefix("0x").and_then(decode_hex32)
}

fn request_hash(
    scope: &PmAuthenticatedJournalScopeV1,
    basis: &impl Serialize,
) -> Result<PmAuthenticatedCommitmentV1, PmAuthenticatedJournalSchemaError> {
    let mut hasher = Sha256::new();
    hasher.update(match scope.account_signature_profile() {
        PmAccountSignatureProfile::EoaType0 => REQUEST_HASH_PREFIX_V1,
        PmAccountSignatureProfile::ProxyType1 => REQUEST_HASH_PREFIX_V2,
    });
    serde_json::to_writer(HashWriter(&mut hasher), basis)?;
    Ok(PmAuthenticatedCommitmentV1::from_bytes(
        hasher.finalize().into(),
    ))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PmAuthenticatedDispatchAuthorizedV1 {
    pub(super) operation: PmAuthenticatedOperationKeyV1,
    pub(super) prepared_sequence: u64,
}

impl PmAuthenticatedDispatchAuthorizedV1 {
    pub(super) const fn from_durable_prepared(
        operation: PmAuthenticatedOperationKeyV1,
        prepared_sequence: u64,
    ) -> Self {
        Self {
            operation,
            prepared_sequence,
        }
    }

    fn validate(
        self,
        scope: &PmAuthenticatedJournalScopeV1,
        sequence: u64,
    ) -> Result<(), PmAuthenticatedJournalSchemaError> {
        self.operation.validate(scope)?;
        validate_backward_link(self.prepared_sequence, sequence)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum PmAuthenticatedPlaceResultKindV1 {
    Accepted,
    Rejected,
    DefinitelyNotDispatched,
    OutOfProfile,
    AcknowledgementUnknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PmAuthenticatedPlaceResultV1 {
    pub(super) operation: PmAuthenticatedOperationKeyV1,
    pub(super) grant_sequence: u64,
    pub(super) outcome: PmAuthenticatedPlaceResultKindV1,
    pub(super) observed_order_id: Option<PmAuthenticatedCommitmentV1>,
}

impl PmAuthenticatedPlaceResultV1 {
    pub(crate) fn accepted(
        coordinator: PmAuthenticatedCoordinatorIdentityV1,
        grant_sequence: u64,
        observed_order_id: [u8; 32],
    ) -> Self {
        Self {
            operation: PmAuthenticatedOperationKeyV1::place(coordinator),
            grant_sequence,
            outcome: PmAuthenticatedPlaceResultKindV1::Accepted,
            observed_order_id: Some(PmAuthenticatedCommitmentV1::from_bytes(observed_order_id)),
        }
    }

    pub(crate) const fn rejected(
        coordinator: PmAuthenticatedCoordinatorIdentityV1,
        grant_sequence: u64,
    ) -> Self {
        Self {
            operation: PmAuthenticatedOperationKeyV1::place(coordinator),
            grant_sequence,
            outcome: PmAuthenticatedPlaceResultKindV1::Rejected,
            observed_order_id: None,
        }
    }

    pub(crate) const fn definitely_not_dispatched(
        coordinator: PmAuthenticatedCoordinatorIdentityV1,
        grant_sequence: u64,
    ) -> Self {
        Self {
            operation: PmAuthenticatedOperationKeyV1::place(coordinator),
            grant_sequence,
            outcome: PmAuthenticatedPlaceResultKindV1::DefinitelyNotDispatched,
            observed_order_id: None,
        }
    }

    pub(crate) fn out_of_profile(
        coordinator: PmAuthenticatedCoordinatorIdentityV1,
        grant_sequence: u64,
        observed_order_id: Option<[u8; 32]>,
    ) -> Self {
        Self {
            operation: PmAuthenticatedOperationKeyV1::place(coordinator),
            grant_sequence,
            outcome: PmAuthenticatedPlaceResultKindV1::OutOfProfile,
            observed_order_id: observed_order_id.map(PmAuthenticatedCommitmentV1::from_bytes),
        }
    }

    pub(crate) fn acknowledgement_unknown(
        coordinator: PmAuthenticatedCoordinatorIdentityV1,
        grant_sequence: u64,
        observed_order_id: Option<[u8; 32]>,
    ) -> Self {
        Self {
            operation: PmAuthenticatedOperationKeyV1::place(coordinator),
            grant_sequence,
            outcome: PmAuthenticatedPlaceResultKindV1::AcknowledgementUnknown,
            observed_order_id: observed_order_id.map(PmAuthenticatedCommitmentV1::from_bytes),
        }
    }

    pub(crate) const fn client_order(self) -> PmClientOrderKey {
        self.operation.coordinator().client_order
    }

    pub(crate) const fn instrument(self) -> PmInstrumentId {
        self.operation.coordinator().instrument
    }

    pub(crate) const fn grant_sequence(self) -> u64 {
        self.grant_sequence
    }

    pub(crate) const fn outcome(self) -> PmAuthenticatedPlaceResultKindV1 {
        self.outcome
    }

    pub(crate) const fn observed_order_id(self) -> Option<[u8; 32]> {
        match self.observed_order_id {
            Some(identity) => Some(identity.bytes()),
            None => None,
        }
    }

    fn validate(
        self,
        scope: &PmAuthenticatedJournalScopeV1,
        sequence: u64,
    ) -> Result<(), PmAuthenticatedJournalSchemaError> {
        self.operation.validate(scope)?;
        if !matches!(self.operation, PmAuthenticatedOperationKeyV1::Place { .. }) {
            return Err(PmAuthenticatedJournalSchemaError::OperationMismatch);
        }
        validate_backward_link(self.grant_sequence, sequence)?;
        let valid = match self.outcome {
            PmAuthenticatedPlaceResultKindV1::Accepted => self.observed_order_id.is_some(),
            PmAuthenticatedPlaceResultKindV1::Rejected
            | PmAuthenticatedPlaceResultKindV1::DefinitelyNotDispatched => {
                self.observed_order_id.is_none()
            }
            PmAuthenticatedPlaceResultKindV1::OutOfProfile
            | PmAuthenticatedPlaceResultKindV1::AcknowledgementUnknown => true,
        };
        if !valid {
            return Err(PmAuthenticatedJournalSchemaError::InvalidResultShape);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum PmAuthenticatedCancelResultKindV1 {
    Accepted,
    Rejected,
    DefinitelyNotDispatched,
    OutOfProfile,
    AcknowledgementUnknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PmAuthenticatedCancelResultV1 {
    pub(super) operation: PmAuthenticatedOperationKeyV1,
    pub(super) grant_sequence: u64,
    pub(super) outcome: PmAuthenticatedCancelResultKindV1,
    pub(super) observed_order_id: Option<PmAuthenticatedCommitmentV1>,
}

impl PmAuthenticatedCancelResultV1 {
    pub(crate) fn accepted(
        coordinator: PmAuthenticatedCoordinatorIdentityV1,
        venue_order: PmVenueOrderKey,
        grant_sequence: u64,
        observed_order_id: [u8; 32],
    ) -> Self {
        Self {
            operation: PmAuthenticatedOperationKeyV1::cancel(coordinator, venue_order),
            grant_sequence,
            outcome: PmAuthenticatedCancelResultKindV1::Accepted,
            observed_order_id: Some(PmAuthenticatedCommitmentV1::from_bytes(observed_order_id)),
        }
    }

    pub(crate) fn rejected(
        coordinator: PmAuthenticatedCoordinatorIdentityV1,
        venue_order: PmVenueOrderKey,
        grant_sequence: u64,
        observed_order_id: [u8; 32],
    ) -> Self {
        Self {
            operation: PmAuthenticatedOperationKeyV1::cancel(coordinator, venue_order),
            grant_sequence,
            outcome: PmAuthenticatedCancelResultKindV1::Rejected,
            observed_order_id: Some(PmAuthenticatedCommitmentV1::from_bytes(observed_order_id)),
        }
    }

    pub(crate) const fn definitely_not_dispatched(
        coordinator: PmAuthenticatedCoordinatorIdentityV1,
        venue_order: PmVenueOrderKey,
        grant_sequence: u64,
    ) -> Self {
        Self {
            operation: PmAuthenticatedOperationKeyV1::cancel(coordinator, venue_order),
            grant_sequence,
            outcome: PmAuthenticatedCancelResultKindV1::DefinitelyNotDispatched,
            observed_order_id: None,
        }
    }

    pub(crate) fn out_of_profile(
        coordinator: PmAuthenticatedCoordinatorIdentityV1,
        venue_order: PmVenueOrderKey,
        grant_sequence: u64,
        observed_order_id: Option<[u8; 32]>,
    ) -> Self {
        Self {
            operation: PmAuthenticatedOperationKeyV1::cancel(coordinator, venue_order),
            grant_sequence,
            outcome: PmAuthenticatedCancelResultKindV1::OutOfProfile,
            observed_order_id: observed_order_id.map(PmAuthenticatedCommitmentV1::from_bytes),
        }
    }

    pub(crate) fn acknowledgement_unknown(
        coordinator: PmAuthenticatedCoordinatorIdentityV1,
        venue_order: PmVenueOrderKey,
        grant_sequence: u64,
        observed_order_id: Option<[u8; 32]>,
    ) -> Self {
        Self {
            operation: PmAuthenticatedOperationKeyV1::cancel(coordinator, venue_order),
            grant_sequence,
            outcome: PmAuthenticatedCancelResultKindV1::AcknowledgementUnknown,
            observed_order_id: observed_order_id.map(PmAuthenticatedCommitmentV1::from_bytes),
        }
    }

    pub(crate) const fn client_order(self) -> PmClientOrderKey {
        self.operation.coordinator().client_order
    }

    pub(crate) const fn instrument(self) -> PmInstrumentId {
        self.operation.coordinator().instrument
    }

    pub(crate) fn venue_order(self) -> PmVenueOrderKey {
        match self.operation {
            PmAuthenticatedOperationKeyV1::Cancel { venue_order, .. } => venue_order,
            PmAuthenticatedOperationKeyV1::Place { .. } => {
                unreachable!("cancel result constructor validates operation kind")
            }
        }
    }

    pub(crate) const fn grant_sequence(self) -> u64 {
        self.grant_sequence
    }

    pub(crate) const fn outcome(self) -> PmAuthenticatedCancelResultKindV1 {
        self.outcome
    }

    pub(crate) const fn observed_order_id(self) -> Option<[u8; 32]> {
        match self.observed_order_id {
            Some(identity) => Some(identity.bytes()),
            None => None,
        }
    }

    fn validate(
        self,
        scope: &PmAuthenticatedJournalScopeV1,
        sequence: u64,
    ) -> Result<(), PmAuthenticatedJournalSchemaError> {
        self.operation.validate(scope)?;
        let PmAuthenticatedOperationKeyV1::Cancel { venue_order, .. } = self.operation else {
            return Err(PmAuthenticatedJournalSchemaError::OperationMismatch);
        };
        validate_backward_link(self.grant_sequence, sequence)?;
        let exact_order_id = decode_prefixed_hex32(venue_order.id().as_str())
            .map(PmAuthenticatedCommitmentV1::from_bytes);
        let valid = match self.outcome {
            PmAuthenticatedCancelResultKindV1::Accepted
            | PmAuthenticatedCancelResultKindV1::Rejected => {
                exact_order_id.is_some() && self.observed_order_id == exact_order_id
            }
            PmAuthenticatedCancelResultKindV1::DefinitelyNotDispatched => {
                self.observed_order_id.is_none()
            }
            PmAuthenticatedCancelResultKindV1::OutOfProfile
            | PmAuthenticatedCancelResultKindV1::AcknowledgementUnknown => true,
        };
        if !valid {
            return Err(PmAuthenticatedJournalSchemaError::InvalidResultShape);
        }
        Ok(())
    }
}

fn validate_backward_link(
    prior: u64,
    sequence: u64,
) -> Result<(), PmAuthenticatedJournalSchemaError> {
    if prior == 0 || prior >= sequence {
        Err(PmAuthenticatedJournalSchemaError::InvalidSequenceLink)
    } else {
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum PmAuthenticatedJournalRecordV1 {
    Header(PmAuthenticatedJournalHeaderV1),
    PlacePrepared(PmAuthenticatedPlacePreparedV1),
    CancelPrepared(PmAuthenticatedCancelPreparedV1),
    DispatchAuthorized(PmAuthenticatedDispatchAuthorizedV1),
    PlaceResult(PmAuthenticatedPlaceResultV1),
    CancelResult(PmAuthenticatedCancelResultV1),
}

impl PmAuthenticatedJournalRecordV1 {
    pub(super) fn validate(
        &self,
        scope: &PmAuthenticatedJournalScopeV1,
        sequence: u64,
    ) -> Result<(), PmAuthenticatedJournalSchemaError> {
        match self {
            Self::Header(header) => {
                if sequence != 0 {
                    return Err(PmAuthenticatedJournalSchemaError::HeaderAfterStart);
                }
                header.scope.validate()?;
                if &header.scope != scope {
                    return Err(PmAuthenticatedJournalSchemaError::ScopeMismatch);
                }
            }
            Self::PlacePrepared(prepared) => {
                require_non_header_sequence(sequence)?;
                prepared.validate(scope)?;
            }
            Self::CancelPrepared(prepared) => {
                require_non_header_sequence(sequence)?;
                prepared.validate(scope)?;
            }
            Self::DispatchAuthorized(grant) => grant.validate(scope, sequence)?,
            Self::PlaceResult(result) => result.validate(scope, sequence)?,
            Self::CancelResult(result) => result.validate(scope, sequence)?,
        }
        Ok(())
    }
}

fn require_non_header_sequence(sequence: u64) -> Result<(), PmAuthenticatedJournalSchemaError> {
    if sequence == 0 {
        Err(PmAuthenticatedJournalSchemaError::MissingHeader)
    } else {
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct PmAuthenticatedJournalLineV1(
    PmAuthenticatedJournalFamily,
    PmAuthenticatedJournalVersion,
    PmAuthenticatedJournalFingerprintV1,
    u64,
    PmAuthenticatedJournalRecordV1,
);

impl PmAuthenticatedJournalLineV1 {
    /// Constructs the frozen version-1 EOA envelope for golden/negative tests.
    /// Runtime writers must derive the version from a validated scope.
    #[cfg(test)]
    pub(super) const fn new(
        scope: PmAuthenticatedJournalFingerprintV1,
        sequence: u64,
        record: PmAuthenticatedJournalRecordV1,
    ) -> Self {
        Self(
            PmAuthenticatedJournalFamily,
            PmAuthenticatedJournalVersion(PM_AUTHENTICATED_JOURNAL_VERSION),
            scope,
            sequence,
            record,
        )
    }

    pub(super) const fn new_for_scope(
        scope: &PmAuthenticatedJournalScopeV1,
        sequence: u64,
        record: PmAuthenticatedJournalRecordV1,
    ) -> Self {
        Self(
            PmAuthenticatedJournalFamily,
            PmAuthenticatedJournalVersion(scope.schema_version()),
            scope.fingerprint(),
            sequence,
            record,
        )
    }

    pub(super) const fn schema_version(&self) -> u16 {
        self.1.0
    }

    pub(super) const fn scope(&self) -> PmAuthenticatedJournalFingerprintV1 {
        self.2
    }

    pub(super) const fn sequence(&self) -> u64 {
        self.3
    }

    pub(super) const fn record(&self) -> &PmAuthenticatedJournalRecordV1 {
        &self.4
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PmAuthenticatedJournalFamily;

impl Serialize for PmAuthenticatedJournalFamily {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(PM_AUTHENTICATED_JOURNAL_FAMILY)
    }
}

impl<'de> Deserialize<'de> for PmAuthenticatedJournalFamily {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        if String::deserialize(deserializer)? == PM_AUTHENTICATED_JOURNAL_FAMILY {
            Ok(Self)
        } else {
            Err(de::Error::custom("wrong PM authenticated journal family"))
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PmAuthenticatedJournalVersion(u16);

impl Serialize for PmAuthenticatedJournalVersion {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_u16(self.0)
    }
}

impl<'de> Deserialize<'de> for PmAuthenticatedJournalVersion {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let version = u16::deserialize(deserializer)?;
        if matches!(
            version,
            PM_AUTHENTICATED_JOURNAL_VERSION | PM_T2_PROXY_AUTHENTICATED_JOURNAL_VERSION
        ) {
            Ok(Self(version))
        } else {
            Err(de::Error::custom(
                "unsupported PM authenticated journal version",
            ))
        }
    }
}

pub(super) fn next_sequence(sequence: u64) -> Result<u64, PmAuthenticatedJournalSchemaError> {
    sequence
        .checked_add(1)
        .ok_or(PmAuthenticatedJournalSchemaError::SequenceExhausted)
}

#[derive(Debug, Error)]
pub(crate) enum PmAuthenticatedJournalSchemaError {
    #[error("PM authenticated journal JSON encoding failed: {0}")]
    Json(#[from] serde_json::Error),
    #[error("PM authenticated journal scope names the wrong product, family, or version")]
    WrongScopeDomain,
    #[error("PM authenticated journal cannot authorize production order entry")]
    ProductionAuthorityForbidden,
    #[error("PM authenticated journal requires a bound credential-slot fingerprint")]
    MissingCredentialSlotFingerprint,
    #[error("PM authenticated journal account identities do not match its signature profile")]
    AccountIdentityMismatch,
    #[error("PM authenticated journal scope fingerprint does not match its descriptor")]
    ScopeFingerprintMismatch,
    #[error("PM authenticated journal line scope differs from the leased scope")]
    ScopeMismatch,
    #[error("PM authenticated journal record lies outside its coordinator scope")]
    RecordOutsideScope,
    #[error("PM authenticated journal record uses the wrong fixed operation")]
    OperationMismatch,
    #[error("PM authenticated journal prior Goal F sequence must be nonzero")]
    ZeroPriorSequence,
    #[error("PM authenticated journal has an invalid L2 timestamp")]
    InvalidL2Timestamp,
    #[error("PM authenticated journal exact-owned cancel identity is inconsistent")]
    FixedOrderIdentityMismatch,
    #[error("PM authenticated journal request commitment does not match its exact fields")]
    RequestCommitmentMismatch,
    #[error("PM authenticated journal sequence link is zero, forward, or self-referential")]
    InvalidSequenceLink,
    #[error("PM authenticated journal result fields contradict their classification")]
    InvalidResultShape,
    #[error("PM authenticated journal is missing its sequence-zero header")]
    MissingHeader,
    #[error("PM authenticated journal header cannot be appended after startup")]
    HeaderAfterStart,
    #[error("PM authenticated journal sequence is exhausted")]
    SequenceExhausted,
}

#[cfg(test)]
pub(super) fn test_scope() -> PmAuthenticatedJournalScopeV1 {
    test_scope_with_credential_slot([0x44; 32])
}

#[cfg(test)]
#[path = "schema_tests.rs"]
mod test_support;

#[cfg(test)]
pub(super) fn test_scope_with_credential_slot(
    credential_slot_fingerprint: [u8; 32],
) -> PmAuthenticatedJournalScopeV1 {
    test_support::test_scope_with_profile_and_identities(
        PmAccountSignatureProfile::EoaType0,
        [0x11; 20],
        [0x11; 20],
        [0x33; 32],
        credential_slot_fingerprint,
    )
}

#[cfg(test)]
pub(super) fn test_proxy_scope() -> PmAuthenticatedJournalScopeV1 {
    test_proxy_scope_with_identities([0x11; 20], [0x12; 20], [0x33; 32])
}

#[cfg(test)]
pub(super) fn test_proxy_scope_with_identities(
    signer: [u8; 20],
    funder: [u8; 20],
    configuration_fingerprint: [u8; 32],
) -> PmAuthenticatedJournalScopeV1 {
    test_support::test_scope_with_profile_and_identities(
        PmAccountSignatureProfile::ProxyType1,
        signer,
        funder,
        configuration_fingerprint,
        [0x44; 32],
    )
}

#[cfg(test)]
impl PmAuthenticatedPlacePreparedV1 {
    pub(super) fn test_new(
        scope: &PmAuthenticatedJournalScopeV1,
        coordinator: PmAuthenticatedCoordinatorIdentityV1,
        prior_intent_sequence: u64,
        semantic_request_commitment: [u8; 32],
        expected_order_id: [u8; 32],
        l2_timestamp_seconds: u64,
    ) -> Self {
        let mut prepared = Self {
            operation: PmAuthenticatedOperationKeyV1::place(coordinator),
            prior_intent_sequence,
            semantic_request_commitment: PmAuthenticatedCommitmentV1::from_bytes(
                semantic_request_commitment,
            ),
            request_commitment: PmAuthenticatedCommitmentV1::from_bytes(ZERO_HASH),
            expected_order_id: PmAuthenticatedCommitmentV1::from_bytes(expected_order_id),
            l2_timestamp_seconds,
        };
        prepared.request_commitment = prepared
            .calculate_request_commitment(scope)
            .expect("test request commitment");
        prepared.validate(scope).expect("valid test place request");
        prepared
    }
}

#[cfg(test)]
impl PmAuthenticatedCancelPreparedV1 {
    pub(super) fn test_new(
        scope: &PmAuthenticatedJournalScopeV1,
        coordinator: PmAuthenticatedCoordinatorIdentityV1,
        venue_order: PmVenueOrderKey,
        prior_cancel_sequence: u64,
        semantic_request_commitment: [u8; 32],
        fixed_order_id: [u8; 32],
        l2_timestamp_seconds: u64,
    ) -> Self {
        let mut prepared = Self {
            operation: PmAuthenticatedOperationKeyV1::cancel(coordinator, venue_order),
            prior_cancel_sequence,
            semantic_request_commitment: PmAuthenticatedCommitmentV1::from_bytes(
                semantic_request_commitment,
            ),
            request_commitment: PmAuthenticatedCommitmentV1::from_bytes(ZERO_HASH),
            fixed_order_id: PmAuthenticatedCommitmentV1::from_bytes(fixed_order_id),
            l2_timestamp_seconds,
        };
        prepared.request_commitment = prepared
            .calculate_request_commitment(scope)
            .expect("test request commitment");
        prepared.validate(scope).expect("valid test cancel request");
        prepared
    }
}
