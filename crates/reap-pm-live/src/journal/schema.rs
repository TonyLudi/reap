use std::fmt;

use reap_pm_core::{
    ConnectionEpoch, EvmAddress, IngressSequence, PmAccountHandle, PmAccountScope, PmAssetId,
    PmClientOrderId, PmClientOrderKey, PmConnectionId, PmFillFee, PmFillId, PmFillRole,
    PmFillSettlementStatus, PmInstrumentId, PmNumericError, PmOrderSalt, PmOrderSide, PmPrice,
    PmQuantity, PmSign, PmSignedUnits, PmVenueOrderKey, SnapshotRevision, U256,
    exact_order_amounts,
};
use reap_pm_live_contracts::PmConnectivityConfig;
use serde::{Deserialize, Deserializer, Serialize, Serializer, de};
use sha2::{Digest, Sha256};
use thiserror::Error;

pub const PM_MUTATION_JOURNAL_FAMILY: &str = "reap-pm-mutation-journal";
pub const PM_MUTATION_JOURNAL_VERSION: u16 = 1;
pub const MAX_PM_JOURNAL_LINE_BYTES: usize = 64 * 1_024;
pub const MAX_PM_JOURNAL_BYTES: u64 = 512 * 1_024 * 1_024;
pub const MAX_PM_JOURNAL_RECORDS: usize = 262_144;
pub const MAX_PM_JOURNAL_OWNED_ORDERS: usize = 1_024;
pub const MAX_PM_JOURNAL_FILL_KEYS: usize = 8_192;
// Kept as an independent schema literal to preserve journal -> core-only
// dependency direction. A live contract test pins it to the adapter result
// bound.
pub const MAX_PM_ACKNOWLEDGEMENT_FILL_LEGS: usize = 64;
const PM_ACKNOWLEDGEMENT_FILL_CHUNK: usize = 32;
const PM_ACKNOWLEDGEMENT_FILL_CHUNKS: usize =
    MAX_PM_ACKNOWLEDGEMENT_FILL_LEGS / PM_ACKNOWLEDGEMENT_FILL_CHUNK;

const SCOPE_HASH_PREFIX: &[u8] = b"reap.pm.mutation-journal.scope.v1\0";
const CLIENT_ORDER_HASH_PREFIX: &[u8] = b"reap.pm.mutation-journal.client-order.v1\0";
const ZERO_HASH: [u8; 32] = [0; 32];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PmJournalFingerprintV1([u8; 32]);

impl PmJournalFingerprintV1 {
    #[must_use]
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    #[must_use]
    pub const fn bytes(self) -> [u8; 32] {
        self.0
    }
}

impl fmt::Display for PmJournalFingerprintV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in self.0 {
            write!(formatter, "{byte:02x}")?;
        }
        Ok(())
    }
}

impl Serialize for PmJournalFingerprintV1 {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for PmJournalFingerprintV1 {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct FingerprintVisitor;

        impl de::Visitor<'_> for FingerprintVisitor {
            type Value = PmJournalFingerprintV1;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("exactly 64 lowercase hexadecimal characters")
            }

            fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                if value.len() != 64
                    || value
                        .as_bytes()
                        .iter()
                        .any(|byte| !byte.is_ascii_digit() && !(b'a'..=b'f').contains(byte))
                {
                    return Err(E::custom("invalid PM journal fingerprint"));
                }
                let mut bytes = [0_u8; 32];
                for (index, output) in bytes.iter_mut().enumerate() {
                    let high = decode_hex(value.as_bytes()[index * 2])
                        .ok_or_else(|| E::custom("invalid PM journal fingerprint"))?;
                    let low = decode_hex(value.as_bytes()[index * 2 + 1])
                        .ok_or_else(|| E::custom("invalid PM journal fingerprint"))?;
                    *output = (high << 4) | low;
                }
                Ok(PmJournalFingerprintV1(bytes))
            }
        }

        deserializer.deserialize_str(FingerprintVisitor)
    }
}

const fn decode_hex(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        _ => None,
    }
}

/// Exact non-secret lease scope for one Goal-F PM product journal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PmJournalScopeV1 {
    product: String,
    schema_family: String,
    schema_version: u16,
    account_scope: PmAccountScope,
    configured_instruments: [PmInstrumentId; 1],
    configuration_fingerprint: PmJournalFingerprintV1,
    authentication_enabled: bool,
    production_authorized: bool,
    scope_fingerprint: PmJournalFingerprintV1,
}

impl PmJournalScopeV1 {
    pub fn from_config(config: &PmConnectivityConfig) -> Result<Self, PmJournalSchemaError> {
        let account = config.account().account_scope();
        let mut scope = Self {
            product: "reap-pm".to_owned(),
            schema_family: PM_MUTATION_JOURNAL_FAMILY.to_owned(),
            schema_version: PM_MUTATION_JOURNAL_VERSION,
            account_scope: account,
            configured_instruments: [config.account().instrument_id()],
            configuration_fingerprint: PmJournalFingerprintV1::from_bytes(
                config.public().configuration_fingerprint().bytes(),
            ),
            authentication_enabled: false,
            production_authorized: false,
            scope_fingerprint: PmJournalFingerprintV1::from_bytes(ZERO_HASH),
        };
        scope.scope_fingerprint = scope.calculate_fingerprint()?;
        scope.validate()?;
        Ok(scope)
    }

    #[must_use]
    pub const fn fingerprint(&self) -> PmJournalFingerprintV1 {
        self.scope_fingerprint
    }

    #[must_use]
    pub const fn account(&self) -> PmAccountHandle {
        self.account_scope.handle()
    }

    #[must_use]
    pub const fn account_scope(&self) -> PmAccountScope {
        self.account_scope
    }

    #[must_use]
    pub const fn instrument(&self) -> PmInstrumentId {
        self.configured_instruments[0]
    }

    #[must_use]
    pub const fn configuration_fingerprint(&self) -> PmJournalFingerprintV1 {
        self.configuration_fingerprint
    }

    pub fn client_order_for_intent(
        &self,
        intent_id: u64,
    ) -> Result<PmClientOrderKey, PmJournalSchemaError> {
        derive_pm_journal_client_order(self, intent_id)
    }

    #[must_use]
    pub const fn authentication_enabled(&self) -> bool {
        self.authentication_enabled
    }

    #[must_use]
    pub const fn production_authorized(&self) -> bool {
        self.production_authorized
    }

    pub(crate) fn validate(&self) -> Result<(), PmJournalSchemaError> {
        if self.product != "reap-pm"
            || self.schema_family != PM_MUTATION_JOURNAL_FAMILY
            || self.schema_version != PM_MUTATION_JOURNAL_VERSION
        {
            return Err(PmJournalSchemaError::WrongScopeDomain);
        }
        if self.authentication_enabled || self.production_authorized {
            return Err(PmJournalSchemaError::ForbiddenLiveAuthority);
        }
        if self.calculate_fingerprint()? != self.scope_fingerprint {
            return Err(PmJournalSchemaError::ScopeFingerprintMismatch);
        }
        Ok(())
    }

    fn calculate_fingerprint(&self) -> Result<PmJournalFingerprintV1, PmJournalSchemaError> {
        #[derive(Serialize)]
        struct FingerprintBasis<'a> {
            product: &'a str,
            schema_family: &'a str,
            schema_version: u16,
            account_scope: PmAccountScope,
            configured_instruments: [PmInstrumentId; 1],
            configuration_fingerprint: PmJournalFingerprintV1,
            authentication_enabled: bool,
            production_authorized: bool,
        }

        let basis = FingerprintBasis {
            product: &self.product,
            schema_family: &self.schema_family,
            schema_version: self.schema_version,
            account_scope: self.account_scope,
            configured_instruments: self.configured_instruments,
            configuration_fingerprint: self.configuration_fingerprint,
            authentication_enabled: self.authentication_enabled,
            production_authorized: self.production_authorized,
        };
        let mut hasher = Sha256::new();
        hasher.update(SCOPE_HASH_PREFIX);
        serde_json::to_writer(HashWriter(&mut hasher), &basis)?;
        Ok(PmJournalFingerprintV1::from_bytes(hasher.finalize().into()))
    }
}

/// Derives the one canonical account-scoped client identity for an intent.
///
/// The intent high-water is checked independently during recovery. Including
/// the checked scope fingerprint prevents the same local intent ordinal from
/// naming an order in another product/account/market scope.
pub fn derive_pm_journal_client_order(
    scope: &PmJournalScopeV1,
    intent_id: u64,
) -> Result<PmClientOrderKey, PmJournalSchemaError> {
    if intent_id == 0 {
        return Err(PmJournalSchemaError::ZeroIntentId);
    }
    let mut hasher = Sha256::new();
    hasher.update(CLIENT_ORDER_HASH_PREFIX);
    hasher.update(scope.fingerprint().bytes());
    hasher.update(intent_id.to_be_bytes());
    let digest: [u8; 32] = hasher.finalize().into();
    let mut bytes = [0_u8; 16];
    bytes.copy_from_slice(&digest[..16]);
    // A core identity cannot be all-zero. Keep this derivation total without
    // introducing a retry loop or a second source of identity state.
    if bytes.iter().all(|byte| *byte == 0) {
        bytes[15] = 1;
    }
    let id = PmClientOrderId::from_bytes(bytes)
        .expect("the deterministic all-zero digest case is canonicalized");
    Ok(PmClientOrderKey::new(scope.account(), id))
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PmJournalHeaderV1 {
    scope: PmJournalScopeV1,
}

impl PmJournalHeaderV1 {
    #[must_use]
    pub const fn new(scope: PmJournalScopeV1) -> Self {
        Self { scope }
    }

    #[must_use]
    pub const fn scope(&self) -> &PmJournalScopeV1 {
        &self.scope
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PmJournalSideV1 {
    Buy,
    Sell,
}

impl From<PmOrderSide> for PmJournalSideV1 {
    fn from(side: PmOrderSide) -> Self {
        match side {
            PmOrderSide::Buy => Self::Buy,
            PmOrderSide::Sell => Self::Sell,
        }
    }
}

impl From<PmJournalSideV1> for PmOrderSide {
    fn from(side: PmJournalSideV1) -> Self {
        match side {
            PmJournalSideV1::Buy => Self::Buy,
            PmJournalSideV1::Sell => Self::Sell,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PmJournalQuoteProfileV1 {
    PassiveGtcPostOnlyEoa,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PmJournalQuoteIntentV1 {
    pub intent_id: u64,
    pub client_order: PmClientOrderKey,
    pub instrument: PmInstrumentId,
    pub side: PmJournalSideV1,
    pub price_units: u32,
    pub quantity: PmQuantity,
    pub reserved_collateral: U256,
    pub reserved_outcome: U256,
    pub profile: PmJournalQuoteProfileV1,
    pub metadata_revision: u64,
    pub book_revision: u64,
    pub model_revision: u64,
    pub book_readiness_revision: u64,
    pub private_readiness_revision: u64,
    pub expires_at_monotonic_ns: u64,
    pub salt: PmOrderSalt,
    pub timestamp_ms: u64,
    pub maker: EvmAddress,
    pub signer: EvmAddress,
    pub maker_amount: U256,
    pub taker_amount: U256,
}

impl PmJournalQuoteIntentV1 {
    pub(crate) fn validate(&self, scope: &PmJournalScopeV1) -> Result<(), PmJournalSchemaError> {
        if self.client_order.account() != scope.account() || self.instrument != scope.instrument() {
            return Err(PmJournalSchemaError::RecordOutsideScope);
        }
        if self.intent_id == 0 {
            return Err(PmJournalSchemaError::ZeroIntentId);
        }
        if derive_pm_journal_client_order(scope, self.intent_id)? != self.client_order {
            return Err(PmJournalSchemaError::ClientOrderDerivationMismatch);
        }
        let price = PmPrice::from_units(self.price_units)?;
        if self.reserved_collateral.is_zero() && self.reserved_outcome.is_zero() {
            return Err(PmJournalSchemaError::ZeroReservation);
        }
        let side: PmOrderSide = self.side.into();
        let amounts = exact_order_amounts(side, price, self.quantity)?;
        if self.maker_amount != amounts.maker() || self.taker_amount != amounts.taker() {
            return Err(PmJournalSchemaError::UnsignedAmountMismatch);
        }
        let reservation_covers_base = match side {
            PmOrderSide::Buy => self.reserved_collateral >= amounts.maker(),
            PmOrderSide::Sell => self.reserved_outcome >= self.quantity.protocol_units(),
        };
        if !reservation_covers_base {
            return Err(PmJournalSchemaError::WrongReservationKind);
        }
        let account_scope = scope.account_scope();
        if self.maker != account_scope.funder().address()
            || self.signer != account_scope.signer().address()
            || self.maker != self.signer
        {
            return Err(PmJournalSchemaError::UnsignedIdentityMismatch);
        }
        if self.timestamp_ms == 0
            || self.metadata_revision == 0
            || self.book_revision == 0
            || self.model_revision == 0
            || self.book_readiness_revision == 0
            || self.private_readiness_revision == 0
            || self.expires_at_monotonic_ns == 0
        {
            return Err(PmJournalSchemaError::InvalidRevisionOrExpiry);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PmJournalFillKeyV1 {
    pub venue_order: PmVenueOrderKey,
    pub fill_id: PmFillId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PmJournalFillRoleV1 {
    Maker,
    Taker,
}

impl From<PmFillRole> for PmJournalFillRoleV1 {
    fn from(role: PmFillRole) -> Self {
        match role {
            PmFillRole::Maker => Self::Maker,
            PmFillRole::Taker => Self::Taker,
        }
    }
}

impl From<PmJournalFillRoleV1> for PmFillRole {
    fn from(role: PmJournalFillRoleV1) -> Self {
        match role {
            PmJournalFillRoleV1::Maker => Self::Maker,
            PmJournalFillRoleV1::Taker => Self::Taker,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PmJournalFillSettlementV1 {
    Matched,
    Mined,
    Confirmed,
    Retrying,
    Failed,
}

impl From<PmFillSettlementStatus> for PmJournalFillSettlementV1 {
    fn from(status: PmFillSettlementStatus) -> Self {
        match status {
            PmFillSettlementStatus::Matched => Self::Matched,
            PmFillSettlementStatus::Mined => Self::Mined,
            PmFillSettlementStatus::Confirmed => Self::Confirmed,
            PmFillSettlementStatus::Retrying => Self::Retrying,
            PmFillSettlementStatus::Failed => Self::Failed,
        }
    }
}

impl From<PmJournalFillSettlementV1> for PmFillSettlementStatus {
    fn from(status: PmJournalFillSettlementV1) -> Self {
        match status {
            PmJournalFillSettlementV1::Matched => Self::Matched,
            PmJournalFillSettlementV1::Mined => Self::Mined,
            PmJournalFillSettlementV1::Confirmed => Self::Confirmed,
            PmJournalFillSettlementV1::Retrying => Self::Retrying,
            PmJournalFillSettlementV1::Failed => Self::Failed,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PmJournalSignV1 {
    Positive,
    Negative,
}

impl From<PmSign> for PmJournalSignV1 {
    fn from(sign: PmSign) -> Self {
        match sign {
            PmSign::Positive => Self::Positive,
            PmSign::Negative => Self::Negative,
        }
    }
}

impl From<PmJournalSignV1> for PmSign {
    fn from(sign: PmJournalSignV1) -> Self {
        match sign {
            PmJournalSignV1::Positive => Self::Positive,
            PmJournalSignV1::Negative => Self::Negative,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "knowledge", rename_all = "snake_case")]
pub enum PmJournalFillFeeV1 {
    Known {
        asset: PmAssetId,
        sign: PmJournalSignV1,
        magnitude: U256,
    },
    Unknown,
    Incomplete,
}

impl From<PmFillFee> for PmJournalFillFeeV1 {
    fn from(fee: PmFillFee) -> Self {
        match fee {
            PmFillFee::Known { asset, delta } => Self::Known {
                asset,
                sign: delta.sign().into(),
                magnitude: delta.magnitude(),
            },
            PmFillFee::Unknown => Self::Unknown,
            PmFillFee::Incomplete => Self::Incomplete,
        }
    }
}

impl TryFrom<PmJournalFillFeeV1> for PmFillFee {
    type Error = PmNumericError;

    fn try_from(fee: PmJournalFillFeeV1) -> Result<Self, Self::Error> {
        match fee {
            PmJournalFillFeeV1::Known {
                asset,
                sign,
                magnitude,
            } => Ok(Self::Known {
                asset,
                delta: PmSignedUnits::from_parts(sign.into(), magnitude)?,
            }),
            PmJournalFillFeeV1::Unknown => Ok(Self::Unknown),
            PmJournalFillFeeV1::Incomplete => Ok(Self::Incomplete),
        }
    }
}

impl PmJournalFillFeeV1 {
    fn validate(self) -> Result<(), PmJournalSchemaError> {
        if let Self::Known {
            sign, magnitude, ..
        } = self
        {
            let _ = PmSignedUnits::from_parts(sign.into(), magnitude)?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PmJournalFillV1 {
    pub key: PmJournalFillKeyV1,
    pub client_order: PmClientOrderKey,
    pub instrument: PmInstrumentId,
    pub side: PmJournalSideV1,
    pub price_units: u32,
    pub role: PmJournalFillRoleV1,
    pub settlement: PmJournalFillSettlementV1,
    pub fee: PmJournalFillFeeV1,
    pub delta: PmQuantity,
    pub authoritative_cumulative: Option<U256>,
    pub cumulative: U256,
    pub remaining: U256,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PmJournalImmediateFillsV1 {
    count: u8,
    entries: [[Option<PmJournalFillKeyV1>; PM_ACKNOWLEDGEMENT_FILL_CHUNK];
        PM_ACKNOWLEDGEMENT_FILL_CHUNKS],
}

impl PmJournalImmediateFillsV1 {
    #[must_use]
    pub const fn empty() -> Self {
        Self {
            count: 0,
            entries: [[None; PM_ACKNOWLEDGEMENT_FILL_CHUNK]; PM_ACKNOWLEDGEMENT_FILL_CHUNKS],
        }
    }

    pub fn from_slice(entries: &[PmJournalFillKeyV1]) -> Result<Self, PmJournalSchemaError> {
        if entries.len() > MAX_PM_ACKNOWLEDGEMENT_FILL_LEGS {
            return Err(PmJournalSchemaError::TooManyAcknowledgementFills);
        }
        let mut result = Self::empty();
        for (index, entry) in entries.iter().copied().enumerate() {
            result.entries[index / PM_ACKNOWLEDGEMENT_FILL_CHUNK]
                [index % PM_ACKNOWLEDGEMENT_FILL_CHUNK] = Some(entry);
        }
        result.count = entries.len() as u8;
        result.validate()?;
        Ok(result)
    }

    pub fn iter(&self) -> impl Iterator<Item = PmJournalFillKeyV1> + '_ {
        self.entries
            .iter()
            .flatten()
            .take(usize::from(self.count).min(MAX_PM_ACKNOWLEDGEMENT_FILL_LEGS))
            .copied()
            .flatten()
    }

    fn entry(&self, index: usize) -> Option<PmJournalFillKeyV1> {
        self.entries[index / PM_ACKNOWLEDGEMENT_FILL_CHUNK][index % PM_ACKNOWLEDGEMENT_FILL_CHUNK]
    }

    pub(crate) fn validate(&self) -> Result<(), PmJournalSchemaError> {
        let count = usize::from(self.count);
        if count > MAX_PM_ACKNOWLEDGEMENT_FILL_LEGS
            || (0..count).any(|index| self.entry(index).is_none())
            || (count..MAX_PM_ACKNOWLEDGEMENT_FILL_LEGS).any(|index| self.entry(index).is_some())
        {
            return Err(PmJournalSchemaError::InvalidAcknowledgementFills);
        }
        for index in 0..count {
            let key = self.entry(index).expect("validated immediate-fill prefix");
            if (0..index).any(|prior| self.entry(prior) == Some(key)) {
                return Err(PmJournalSchemaError::DuplicateAcknowledgementFill);
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PmJournalPlaceOutcomeV1 {
    AcceptedResting,
    AcceptedWithImmediateFill,
    Rejected,
    AmbiguousTimeout,
    LateAcknowledgement,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PmJournalPlaceRejectReasonV1 {
    FixtureRejected,
    PostOnlyWouldTake,
    AuthorityInvalidatedBeforeDispatch,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PmJournalPlaceResultV1 {
    pub client_order: PmClientOrderKey,
    pub outcome: PmJournalPlaceOutcomeV1,
    pub reject_reason: Option<PmJournalPlaceRejectReasonV1>,
    pub venue_order: Option<PmVenueOrderKey>,
    pub immediate_fills: PmJournalImmediateFillsV1,
}

impl PmJournalPlaceResultV1 {
    pub(crate) fn validate(&self, scope: &PmJournalScopeV1) -> Result<(), PmJournalSchemaError> {
        self.immediate_fills.validate()?;
        if self.client_order.account() != scope.account()
            || self
                .venue_order
                .is_some_and(|order| order.account() != scope.account())
        {
            return Err(PmJournalSchemaError::RecordOutsideScope);
        }
        if self.immediate_fills.iter().any(|key| {
            key.venue_order.account() != scope.account()
                || self.venue_order != Some(key.venue_order)
        }) {
            return Err(PmJournalSchemaError::InvalidAcknowledgementFillOwnership);
        }
        let fill_count = self.immediate_fills.iter().count();
        let valid = match self.outcome {
            PmJournalPlaceOutcomeV1::AcceptedResting => {
                self.reject_reason.is_none() && self.venue_order.is_some() && fill_count == 0
            }
            PmJournalPlaceOutcomeV1::AcceptedWithImmediateFill => {
                self.reject_reason.is_none() && self.venue_order.is_some() && fill_count > 0
            }
            PmJournalPlaceOutcomeV1::LateAcknowledgement => {
                self.reject_reason.is_none() && self.venue_order.is_some()
            }
            PmJournalPlaceOutcomeV1::Rejected => {
                self.reject_reason.is_some() && self.venue_order.is_none() && fill_count == 0
            }
            PmJournalPlaceOutcomeV1::AmbiguousTimeout => {
                self.reject_reason.is_none() && self.venue_order.is_none() && fill_count == 0
            }
        };
        if valid {
            Ok(())
        } else {
            Err(PmJournalSchemaError::InvalidPlaceResult)
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PmJournalCancelOutcomeV1 {
    Accepted,
    Rejected,
    AlreadyFilled,
    AmbiguousTimeout,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PmJournalCancelRejectReasonV1 {
    FixtureRejected,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PmJournalCancelReasonV1 {
    Replacement,
    StaleReference,
    StaleBook,
    SafetyHalt,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PmJournalCancelIntentV1 {
    pub client_order: PmClientOrderKey,
    pub venue_order: PmVenueOrderKey,
    pub reason: PmJournalCancelReasonV1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PmJournalCancelResultV1 {
    pub client_order: PmClientOrderKey,
    pub venue_order: PmVenueOrderKey,
    pub outcome: PmJournalCancelOutcomeV1,
    pub reject_reason: Option<PmJournalCancelRejectReasonV1>,
}

impl PmJournalCancelResultV1 {
    fn validate(self, scope: &PmJournalScopeV1) -> Result<(), PmJournalSchemaError> {
        validate_order_keys(scope, self.client_order, self.venue_order)?;
        let valid = match self.outcome {
            PmJournalCancelOutcomeV1::Rejected => self.reject_reason.is_some(),
            PmJournalCancelOutcomeV1::Accepted
            | PmJournalCancelOutcomeV1::AlreadyFilled
            | PmJournalCancelOutcomeV1::AmbiguousTimeout => self.reject_reason.is_none(),
        };
        if valid {
            Ok(())
        } else {
            Err(PmJournalSchemaError::InvalidCancelResult)
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PmJournalFillSourceV1 {
    PlaceAcknowledgement,
    PrivateWebsocket,
    RestReconciliation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PmJournalFillDeliveryV1 {
    Live,
    Replay,
}

/// Original source and deterministic occurrence of one applied fill.
///
/// Replay is represented independently by [`PmJournalFillDeliveryV1`] and
/// never overwrites the source facts carried by the original observation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PmJournalFillOccurrenceV1 {
    /// Monotonic coordinator-owner occurrence. This is local ordering and
    /// never substitutes for a PM connection epoch/ingress sequence.
    pub owner_sequence: IngressSequence,
    pub connection: Option<PmConnectionId>,
    pub connection_epoch: Option<ConnectionEpoch>,
    pub ingress_sequence: Option<IngressSequence>,
    pub snapshot_revision: Option<SnapshotRevision>,
    pub monotonic_service_ns: u64,
}

impl PmJournalFillOccurrenceV1 {
    fn validate(self, source: PmJournalFillSourceV1) -> Result<(), PmJournalSchemaError> {
        if self.owner_sequence.value() == 0
            || self.monotonic_service_ns == 0
            || self
                .connection_epoch
                .is_some_and(|epoch| epoch.value() == 0)
            || self
                .ingress_sequence
                .is_some_and(|sequence| sequence.value() == 0)
            || self
                .snapshot_revision
                .is_some_and(|revision| revision.value() == 0)
        {
            return Err(PmJournalSchemaError::InvalidFillOccurrence);
        }
        let connected = self.connection.is_some()
            && self.connection_epoch.is_some()
            && self.ingress_sequence.is_some();
        let valid = match source {
            PmJournalFillSourceV1::PlaceAcknowledgement => {
                self.connection.is_none()
                    && self.connection_epoch.is_none()
                    && self.ingress_sequence.is_none()
                    && self.snapshot_revision.is_none()
            }
            PmJournalFillSourceV1::PrivateWebsocket => connected,
            PmJournalFillSourceV1::RestReconciliation => {
                connected && self.snapshot_revision.is_some()
            }
        };
        if valid {
            Ok(())
        } else {
            Err(PmJournalSchemaError::InvalidFillOccurrence)
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PmJournalFillAppliedV1 {
    pub fill: PmJournalFillV1,
    pub source: PmJournalFillSourceV1,
    pub occurrence: PmJournalFillOccurrenceV1,
    pub delivery: PmJournalFillDeliveryV1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PmJournalTerminalStatusV1 {
    Filled,
    Cancelled,
    Rejected,
    Expired,
}

/// Original private observation domain for a durable terminal transition.
///
/// Immediate fake acknowledgements and cancel outcomes have their own
/// journal records. A terminal progress record therefore always proves a
/// reached private or reconciliation observation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PmJournalOrderProgressSourceV1 {
    PrivateWebsocket,
    RestReconciliation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PmJournalOrderTerminalV1 {
    pub client_order: PmClientOrderKey,
    pub venue_order: PmVenueOrderKey,
    pub status: PmJournalTerminalStatusV1,
    pub cumulative: U256,
    pub remaining: U256,
    pub source: PmJournalOrderProgressSourceV1,
    pub occurrence: PmJournalFillOccurrenceV1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PmJournalSafetyReasonV1 {
    ContractViolation,
    UnresolvedOwnership,
    DurableWriteFailure,
    QueueSaturation,
    StaleDependency,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PmJournalSafetyHaltV1 {
    pub account: PmAccountHandle,
    pub reason: PmJournalSafetyReasonV1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PmJournalFillCursorV1 {
    pub account_scope: PmAccountScope,
    pub opaque: PmJournalFingerprintV1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PmJournalFillWatermarkV1 {
    pub cursor: PmJournalFillCursorV1,
}

// The place-result arm intentionally owns its fixed 64-leg acknowledgement
// buffer. Keeping this schema allocation-free gives the bounded journal queue
// deterministic storage; boxing would only move the same bound to an
// allocator and would add a fallible hot-path dependency.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "body", rename_all = "snake_case")]
pub enum PmJournalRecordV1 {
    Header(PmJournalHeaderV1),
    QuoteIntent(PmJournalQuoteIntentV1),
    PlaceResult(PmJournalPlaceResultV1),
    CancelIntent(PmJournalCancelIntentV1),
    CancelResult(PmJournalCancelResultV1),
    FillApplied(PmJournalFillAppliedV1),
    OrderTerminal(PmJournalOrderTerminalV1),
    SafetyHalt(PmJournalSafetyHaltV1),
    FillWatermarkAdvanced(PmJournalFillWatermarkV1),
}

impl PmJournalRecordV1 {
    pub(crate) fn validate(&self, scope: &PmJournalScopeV1) -> Result<(), PmJournalSchemaError> {
        match self {
            Self::Header(header) => {
                header.scope.validate()?;
                if &header.scope != scope {
                    return Err(PmJournalSchemaError::ScopeMismatch);
                }
            }
            Self::QuoteIntent(intent) => intent.validate(scope)?,
            Self::PlaceResult(result) => result.validate(scope)?,
            Self::CancelIntent(intent) => {
                validate_order_keys(scope, intent.client_order, intent.venue_order)?;
            }
            Self::CancelResult(result) => {
                result.validate(scope)?;
            }
            Self::FillApplied(applied) => {
                validate_fill(scope, &applied.fill)?;
                applied.occurrence.validate(applied.source)?;
            }
            Self::OrderTerminal(terminal) => {
                if terminal.client_order.account() != scope.account()
                    || terminal.venue_order.account() != scope.account()
                {
                    return Err(PmJournalSchemaError::RecordOutsideScope);
                }
                terminal.occurrence.validate(match terminal.source {
                    PmJournalOrderProgressSourceV1::PrivateWebsocket => {
                        PmJournalFillSourceV1::PrivateWebsocket
                    }
                    PmJournalOrderProgressSourceV1::RestReconciliation => {
                        PmJournalFillSourceV1::RestReconciliation
                    }
                })?;
                let status_matches = match terminal.status {
                    PmJournalTerminalStatusV1::Filled => terminal.remaining.is_zero(),
                    PmJournalTerminalStatusV1::Rejected => terminal.cumulative.is_zero(),
                    PmJournalTerminalStatusV1::Cancelled | PmJournalTerminalStatusV1::Expired => {
                        true
                    }
                };
                if !status_matches || terminal.cumulative.checked_add(terminal.remaining).is_err() {
                    return Err(PmJournalSchemaError::InvalidTerminalProgress);
                }
            }
            Self::SafetyHalt(halt) if halt.account != scope.account() => {
                return Err(PmJournalSchemaError::RecordOutsideScope);
            }
            Self::FillWatermarkAdvanced(watermark)
                if watermark.cursor.account_scope != scope.account_scope() =>
            {
                return Err(PmJournalSchemaError::RecordOutsideScope);
            }
            Self::SafetyHalt(_) | Self::FillWatermarkAdvanced(_) => {}
        }
        Ok(())
    }
}

fn validate_order_keys(
    scope: &PmJournalScopeV1,
    client_order: PmClientOrderKey,
    venue_order: PmVenueOrderKey,
) -> Result<(), PmJournalSchemaError> {
    if client_order.account() == scope.account() && venue_order.account() == scope.account() {
        Ok(())
    } else {
        Err(PmJournalSchemaError::RecordOutsideScope)
    }
}

fn validate_fill(
    scope: &PmJournalScopeV1,
    fill: &PmJournalFillV1,
) -> Result<(), PmJournalSchemaError> {
    if fill.client_order.account() != scope.account()
        || fill.key.venue_order.account() != scope.account()
        || fill.instrument != scope.instrument()
    {
        return Err(PmJournalSchemaError::RecordOutsideScope);
    }
    let _ = PmPrice::from_units(fill.price_units)?;
    fill.fee.validate()?;
    if fill.cumulative < fill.delta.protocol_units()
        || fill.authoritative_cumulative.is_some_and(|reported| {
            reported < fill.delta.protocol_units() || reported > fill.cumulative
        })
        || fill.cumulative.checked_add(fill.remaining).is_err()
    {
        return Err(PmJournalSchemaError::InvalidFillProgress);
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct PmJournalLineV1(
    PmJournalFamily,
    PmJournalVersion,
    PmJournalFingerprintV1,
    u64,
    PmJournalRecordV1,
);

impl PmJournalLineV1 {
    pub(crate) const fn new(
        scope: PmJournalFingerprintV1,
        sequence: u64,
        record: PmJournalRecordV1,
    ) -> Self {
        Self(PmJournalFamily, PmJournalVersion, scope, sequence, record)
    }

    pub(crate) const fn scope(&self) -> PmJournalFingerprintV1 {
        self.2
    }

    pub(crate) const fn sequence(&self) -> u64 {
        self.3
    }

    pub(crate) const fn record(&self) -> &PmJournalRecordV1 {
        &self.4
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PmJournalFamily;

impl Serialize for PmJournalFamily {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(PM_MUTATION_JOURNAL_FAMILY)
    }
}

impl<'de> Deserialize<'de> for PmJournalFamily {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let family = String::deserialize(deserializer)?;
        if family == PM_MUTATION_JOURNAL_FAMILY {
            Ok(Self)
        } else {
            Err(de::Error::custom("wrong PM mutation journal family"))
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PmJournalVersion;

impl Serialize for PmJournalVersion {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_u16(PM_MUTATION_JOURNAL_VERSION)
    }
}

impl<'de> Deserialize<'de> for PmJournalVersion {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        if u16::deserialize(deserializer)? == PM_MUTATION_JOURNAL_VERSION {
            Ok(Self)
        } else {
            Err(de::Error::custom("unsupported PM mutation journal version"))
        }
    }
}

pub(crate) fn next_sequence(sequence: u64) -> Result<u64, PmJournalSchemaError> {
    sequence
        .checked_add(1)
        .ok_or(PmJournalSchemaError::SequenceExhausted)
}

#[derive(Debug, Error)]
pub enum PmJournalSchemaError {
    #[error("PM journal JSON encoding failed: {0}")]
    Json(#[from] serde_json::Error),
    #[error("PM journal scope names the wrong product, family, or version")]
    WrongScopeDomain,
    #[error("PM journal scope cannot carry authentication or production authority")]
    ForbiddenLiveAuthority,
    #[error("PM journal scope fingerprint does not match its exact descriptor")]
    ScopeFingerprintMismatch,
    #[error("PM journal line scope differs from the expected lease scope")]
    ScopeMismatch,
    #[error("PM journal record lies outside the configured account or instrument scope")]
    RecordOutsideScope,
    #[error("PM journal quote reservation is zero")]
    ZeroReservation,
    #[error("PM journal quote reservation does not cover its side")]
    WrongReservationKind,
    #[error("PM journal quote intent identity must be nonzero")]
    ZeroIntentId,
    #[error("PM journal client-order identity does not match its scope and intent")]
    ClientOrderDerivationMismatch,
    #[error("PM journal unsigned maker/signer identity differs from its account scope")]
    UnsignedIdentityMismatch,
    #[error("PM journal unsigned maker/taker amounts differ from exact quote units")]
    UnsignedAmountMismatch,
    #[error("PM journal quote has an invalid revision or expiry")]
    InvalidRevisionOrExpiry,
    #[error("PM journal place acknowledgement carries too many fill legs")]
    TooManyAcknowledgementFills,
    #[error("PM journal place acknowledgement has a malformed compact fill set")]
    InvalidAcknowledgementFills,
    #[error("PM journal place acknowledgement repeats an immediate fill key")]
    DuplicateAcknowledgementFill,
    #[error("PM journal place acknowledgement fill key differs from its venue-order binding")]
    InvalidAcknowledgementFillOwnership,
    #[error("PM journal place-result fields contradict its outcome")]
    InvalidPlaceResult,
    #[error("PM journal cancel-result rejection reason contradicts its outcome")]
    InvalidCancelResult,
    #[error("PM journal fill source occurrence is incomplete or contradictory")]
    InvalidFillOccurrence,
    #[error("PM journal fill progress is invalid")]
    InvalidFillProgress,
    #[error("PM journal terminal order progress is invalid")]
    InvalidTerminalProgress,
    #[error("PM journal header cannot be appended after startup")]
    HeaderAfterStart,
    #[error("PM journal dispatch intent must use its typed durable path")]
    WrongRecordPath,
    #[error("PM journal sequence is exhausted")]
    SequenceExhausted,
    #[error(transparent)]
    Numeric(#[from] reap_pm_core::PmNumericError),
}

#[cfg(test)]
pub(super) fn test_scope() -> PmJournalScopeV1 {
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
        scope_fingerprint: PmJournalFingerprintV1::from_bytes(ZERO_HASH),
    };
    scope.scope_fingerprint = scope.calculate_fingerprint().expect("test fingerprint");
    scope.validate().expect("valid test scope");
    scope
}

#[cfg(test)]
mod tests {
    use reap_pm_core::{PmFillId, PmVenueOrderId, PmVenueOrderKey};

    use super::*;

    fn fill_key(scope: &PmJournalScopeV1, ordinal: usize) -> PmJournalFillKeyV1 {
        let venue_order = PmVenueOrderKey::new(
            scope.account(),
            PmVenueOrderId::new("venue-order").expect("venue order"),
        );
        PmJournalFillKeyV1 {
            venue_order,
            fill_id: PmFillId::new(&format!("fill-{ordinal}")).expect("fill identity"),
        }
    }

    fn quote(scope: &PmJournalScopeV1, intent_id: u64) -> PmJournalQuoteIntentV1 {
        let side = PmOrderSide::Buy;
        let price = PmPrice::from_units(500_000).expect("price");
        let quantity = PmQuantity::parse_decimal("2").expect("quantity");
        let amounts = exact_order_amounts(side, price, quantity).expect("exact amounts");
        let account = scope.account_scope();
        PmJournalQuoteIntentV1 {
            intent_id,
            client_order: derive_pm_journal_client_order(scope, intent_id).expect("client order"),
            instrument: scope.instrument(),
            side: side.into(),
            price_units: price.units(),
            quantity,
            reserved_collateral: amounts.maker(),
            reserved_outcome: U256::ZERO,
            profile: PmJournalQuoteProfileV1::PassiveGtcPostOnlyEoa,
            metadata_revision: 1,
            book_revision: 2,
            model_revision: 3,
            book_readiness_revision: 4,
            private_readiness_revision: 5,
            expires_at_monotonic_ns: 5,
            salt: PmOrderSalt::from_u64(intent_id).expect("salt"),
            timestamp_ms: 6,
            maker: account.funder().address(),
            signer: account.signer().address(),
            maker_amount: amounts.maker(),
            taker_amount: amounts.taker(),
        }
    }

    #[test]
    fn client_identity_is_deterministic_and_scope_bound() {
        let scope = test_scope();
        let first = derive_pm_journal_client_order(&scope, 1).expect("first derivation");
        assert_eq!(
            first,
            derive_pm_journal_client_order(&scope, 1).expect("repeat derivation")
        );
        assert_ne!(
            first,
            derive_pm_journal_client_order(&scope, 2).expect("next derivation")
        );
        assert!(matches!(
            derive_pm_journal_client_order(&scope, 0),
            Err(PmJournalSchemaError::ZeroIntentId)
        ));

        let mut other_scope = scope.clone();
        other_scope.account_scope = PmAccountScope::new(
            other_scope.account_scope.environment(),
            other_scope.account_scope.chain(),
            other_scope.account_scope.signer(),
            other_scope.account_scope.funder(),
            PmAccountHandle::from_ordinal(8),
        );
        other_scope.scope_fingerprint = other_scope
            .calculate_fingerprint()
            .expect("other fingerprint");
        assert_ne!(scope.fingerprint(), other_scope.fingerprint());
        assert_ne!(
            first,
            derive_pm_journal_client_order(&other_scope, 1).expect("other derivation")
        );
    }

    #[test]
    fn quote_validation_binds_unsigned_order_facts() {
        let scope = test_scope();
        quote(&scope, 1).validate(&scope).expect("valid quote");

        let mut wrong_client = quote(&scope, 1);
        wrong_client.client_order =
            derive_pm_journal_client_order(&scope, 2).expect("different client");
        assert!(matches!(
            wrong_client.validate(&scope),
            Err(PmJournalSchemaError::ClientOrderDerivationMismatch)
        ));

        let mut wrong_amount = quote(&scope, 1);
        wrong_amount.maker_amount = wrong_amount
            .maker_amount
            .checked_add(U256::from_u64(1))
            .expect("bounded amount");
        assert!(matches!(
            wrong_amount.validate(&scope),
            Err(PmJournalSchemaError::UnsignedAmountMismatch)
        ));
    }

    #[test]
    fn immediate_fill_set_is_fixed_bounded_and_fully_validated() {
        let scope = test_scope();
        let keys = (0..MAX_PM_ACKNOWLEDGEMENT_FILL_LEGS)
            .map(|ordinal| fill_key(&scope, ordinal))
            .collect::<Vec<_>>();
        let fills = PmJournalImmediateFillsV1::from_slice(&keys).expect("bounded fill set");
        assert_eq!(fills.iter().collect::<Vec<_>>(), keys);
        fills.validate().expect("valid compact prefix");

        let mut too_many = keys.clone();
        too_many.push(fill_key(&scope, MAX_PM_ACKNOWLEDGEMENT_FILL_LEGS));
        assert!(matches!(
            PmJournalImmediateFillsV1::from_slice(&too_many),
            Err(PmJournalSchemaError::TooManyAcknowledgementFills)
        ));
        assert!(matches!(
            PmJournalImmediateFillsV1::from_slice(&[keys[0], keys[0]]),
            Err(PmJournalSchemaError::DuplicateAcknowledgementFill)
        ));

        let malformed = PmJournalImmediateFillsV1 {
            count: u8::MAX,
            entries: [[None; PM_ACKNOWLEDGEMENT_FILL_CHUNK]; PM_ACKNOWLEDGEMENT_FILL_CHUNKS],
        };
        assert_eq!(malformed.iter().count(), 0);
        assert!(matches!(
            malformed.validate(),
            Err(PmJournalSchemaError::InvalidAcknowledgementFills)
        ));
    }

    #[test]
    fn fill_occurrence_schema_keeps_owner_order_separate_from_venue_ingress() {
        let occurrence = PmJournalFillOccurrenceV1 {
            owner_sequence: IngressSequence::new(7),
            connection: None,
            connection_epoch: None,
            ingress_sequence: None,
            snapshot_revision: None,
            monotonic_service_ns: 11,
        };
        occurrence
            .validate(PmJournalFillSourceV1::PlaceAcknowledgement)
            .expect("local acknowledgement occurrence");
        let encoded = serde_json::to_value(occurrence).expect("encode occurrence");
        assert_eq!(
            encoded,
            serde_json::json!({
                "owner_sequence": 7,
                "connection": null,
                "connection_epoch": null,
                "ingress_sequence": null,
                "snapshot_revision": null,
                "monotonic_service_ns": 11
            })
        );

        let mut missing_owner = encoded;
        missing_owner
            .as_object_mut()
            .expect("object")
            .remove("owner_sequence");
        assert!(
            serde_json::from_value::<PmJournalFillOccurrenceV1>(missing_owner).is_err(),
            "owner ordering is a required durable field"
        );

        let mut zero_owner = occurrence;
        zero_owner.owner_sequence = IngressSequence::new(0);
        assert!(matches!(
            zero_owner.validate(PmJournalFillSourceV1::PlaceAcknowledgement),
            Err(PmJournalSchemaError::InvalidFillOccurrence)
        ));
    }

    #[test]
    fn rejected_results_require_and_preserve_typed_reasons() {
        let scope = test_scope();
        let client_order = derive_pm_journal_client_order(&scope, 1).expect("client identity");
        let venue_order = PmVenueOrderKey::new(
            scope.account(),
            PmVenueOrderId::new("rejected-order").expect("venue order"),
        );
        let mut place = PmJournalPlaceResultV1 {
            client_order,
            outcome: PmJournalPlaceOutcomeV1::Rejected,
            reject_reason: Some(PmJournalPlaceRejectReasonV1::PostOnlyWouldTake),
            venue_order: None,
            immediate_fills: PmJournalImmediateFillsV1::empty(),
        };
        place.validate(&scope).expect("typed place rejection");
        assert_eq!(
            serde_json::to_value(place)
                .expect("place JSON")
                .get("reject_reason"),
            Some(&serde_json::json!("post_only_would_take"))
        );
        place.reject_reason = None;
        assert!(matches!(
            place.validate(&scope),
            Err(PmJournalSchemaError::InvalidPlaceResult)
        ));

        let mut cancel = PmJournalCancelResultV1 {
            client_order,
            venue_order,
            outcome: PmJournalCancelOutcomeV1::Rejected,
            reject_reason: Some(PmJournalCancelRejectReasonV1::FixtureRejected),
        };
        cancel.validate(&scope).expect("typed cancel rejection");
        assert_eq!(
            serde_json::to_value(cancel)
                .expect("cancel JSON")
                .get("reject_reason"),
            Some(&serde_json::json!("fixture_rejected"))
        );
        cancel.outcome = PmJournalCancelOutcomeV1::Accepted;
        assert!(matches!(
            cancel.validate(&scope),
            Err(PmJournalSchemaError::InvalidCancelResult)
        ));
    }
}
