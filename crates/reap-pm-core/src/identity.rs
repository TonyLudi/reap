use std::fmt;
use std::str::FromStr;

use reap_core::Venue;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use thiserror::Error;

use crate::numeric::U256;

const OPAQUE_ID_CAPACITY: usize = 96;
const CONFIG_ID_CAPACITY: usize = 32;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum PmIdentityError {
    #[error("identity is empty")]
    Empty,
    #[error("identity exceeds its fixed capacity")]
    TooLong,
    #[error("identity must contain ASCII bytes only")]
    NonAscii,
    #[error("identity contains whitespace or a control byte")]
    WhitespaceOrControl,
    #[error("identity has an invalid hexadecimal encoding")]
    InvalidHex,
    #[error("identity is not in its canonical encoding")]
    NonCanonical,
    #[error("identity must not be zero")]
    Zero,
    #[error("chain identity must be positive")]
    InvalidChain,
    #[error("OKX index instrument identity is invalid")]
    InvalidOkxInstrument,
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct BoundedAscii<const N: usize> {
    length: u8,
    bytes: [u8; N],
}

impl<const N: usize> BoundedAscii<N> {
    pub(crate) fn new(value: &str) -> Result<Self, PmIdentityError> {
        if value.is_empty() {
            return Err(PmIdentityError::Empty);
        }
        if value.len() > N || value.len() > usize::from(u8::MAX) {
            return Err(PmIdentityError::TooLong);
        }
        if !value.is_ascii() {
            return Err(PmIdentityError::NonAscii);
        }
        if value
            .bytes()
            .any(|byte| byte.is_ascii_whitespace() || byte.is_ascii_control())
        {
            return Err(PmIdentityError::WhitespaceOrControl);
        }

        let mut bytes = [0_u8; N];
        bytes[..value.len()].copy_from_slice(value.as_bytes());
        Ok(Self {
            length: value.len() as u8,
            bytes,
        })
    }

    #[must_use]
    pub(crate) fn as_str(&self) -> &str {
        std::str::from_utf8(&self.bytes[..usize::from(self.length)])
            .expect("bounded ASCII identity")
    }
}

impl<const N: usize> fmt::Debug for BoundedAscii<N> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("BoundedAscii")
            .field(&self.as_str())
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct HexBytes<const N: usize>([u8; N]);

impl<const N: usize> HexBytes<N> {
    fn parse_prefixed(input: &str) -> Result<Self, PmIdentityError> {
        let digits = input
            .strip_prefix("0x")
            .ok_or(PmIdentityError::InvalidHex)?;
        if digits.len() != N * 2 {
            return Err(PmIdentityError::InvalidHex);
        }
        let mut bytes = [0_u8; N];
        for (index, output) in bytes.iter_mut().enumerate() {
            let high = hex_value(digits.as_bytes()[index * 2])?;
            let low = hex_value(digits.as_bytes()[index * 2 + 1])?;
            *output = (high << 4) | low;
        }
        Self::from_bytes(bytes)
    }

    fn parse_unprefixed_lower(input: &str) -> Result<Self, PmIdentityError> {
        if input.len() != N * 2 {
            return Err(PmIdentityError::InvalidHex);
        }
        if input
            .bytes()
            .any(|byte| byte.is_ascii_uppercase() || !byte.is_ascii_hexdigit())
        {
            return Err(PmIdentityError::NonCanonical);
        }
        let mut bytes = [0_u8; N];
        for (index, output) in bytes.iter_mut().enumerate() {
            let high = hex_value(input.as_bytes()[index * 2])?;
            let low = hex_value(input.as_bytes()[index * 2 + 1])?;
            *output = (high << 4) | low;
        }
        Self::from_bytes(bytes)
    }

    fn from_bytes(bytes: [u8; N]) -> Result<Self, PmIdentityError> {
        if bytes.iter().all(|byte| *byte == 0) {
            Err(PmIdentityError::Zero)
        } else {
            Ok(Self(bytes))
        }
    }

    fn write_hex(&self, formatter: &mut fmt::Formatter<'_>, prefixed: bool) -> fmt::Result {
        if prefixed {
            formatter.write_str("0x")?;
        }
        for byte in self.0 {
            write!(formatter, "{byte:02x}")?;
        }
        Ok(())
    }
}

fn hex_value(byte: u8) -> Result<u8, PmIdentityError> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        b'A'..=b'F' => Ok(byte - b'A' + 10),
        _ => Err(PmIdentityError::InvalidHex),
    }
}

macro_rules! prefixed_hex_identity {
    ($name:ident, $size:expr) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(HexBytes<$size>);

        impl $name {
            pub fn parse(input: &str) -> Result<Self, PmIdentityError> {
                HexBytes::parse_prefixed(input).map(Self)
            }

            pub fn from_bytes(bytes: [u8; $size]) -> Result<Self, PmIdentityError> {
                HexBytes::from_bytes(bytes).map(Self)
            }

            #[must_use]
            pub const fn bytes(self) -> [u8; $size] {
                self.0.0
            }
        }

        impl FromStr for $name {
            type Err = PmIdentityError;

            fn from_str(input: &str) -> Result<Self, Self::Err> {
                Self::parse(input)
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.write_hex(formatter, true)
            }
        }

        impl Serialize for $name {
            fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
            where
                S: Serializer,
            {
                serializer.collect_str(self)
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: Deserializer<'de>,
            {
                let input = String::deserialize(deserializer)?;
                Self::parse(&input).map_err(serde::de::Error::custom)
            }
        }
    };
}

prefixed_hex_identity!(EvmAddress, 20);
prefixed_hex_identity!(PmConditionId, 32);
prefixed_hex_identity!(PmMarketId, 32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PmTokenId(U256);

impl PmTokenId {
    pub fn new(units: U256) -> Result<Self, PmIdentityError> {
        if units.is_zero() {
            Err(PmIdentityError::Zero)
        } else {
            Ok(Self(units))
        }
    }

    #[must_use]
    pub const fn units(self) -> U256 {
        self.0
    }
}

impl Serialize for PmTokenId {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        self.0.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for PmTokenId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::new(U256::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct PmInstrumentId {
    market: PmMarketId,
    token: PmTokenId,
}

impl PmInstrumentId {
    #[must_use]
    pub const fn new(market: PmMarketId, token: PmTokenId) -> Self {
        Self { market, token }
    }

    #[must_use]
    pub const fn market(self) -> PmMarketId {
        self.market
    }

    #[must_use]
    pub const fn token(self) -> PmTokenId {
        self.token
    }
}

/// The fixed-width client-order identity component returned on the wire.
///
/// Use [`PmClientOrderKey`] when an identity must be unique across configured
/// accounts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PmClientOrderId(HexBytes<16>);

impl PmClientOrderId {
    pub fn parse(input: &str) -> Result<Self, PmIdentityError> {
        HexBytes::parse_unprefixed_lower(input).map(Self)
    }

    pub fn from_bytes(bytes: [u8; 16]) -> Result<Self, PmIdentityError> {
        HexBytes::from_bytes(bytes).map(Self)
    }

    #[must_use]
    pub const fn bytes(self) -> [u8; 16] {
        self.0.0
    }
}

impl FromStr for PmClientOrderId {
    type Err = PmIdentityError;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        Self::parse(input)
    }
}

impl fmt::Display for PmClientOrderId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.write_hex(formatter, false)
    }
}

impl Serialize for PmClientOrderId {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for PmClientOrderId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let input = String::deserialize(deserializer)?;
        Self::parse(&input).map_err(serde::de::Error::custom)
    }
}

macro_rules! bounded_ascii_identity {
    ($(#[$attribute:meta])* $name:ident, $capacity:expr) => {
        $(#[$attribute])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(BoundedAscii<$capacity>);

        impl $name {
            pub fn new(value: &str) -> Result<Self, PmIdentityError> {
                BoundedAscii::new(value).map(Self)
            }

            #[must_use]
            pub fn as_str(&self) -> &str {
                self.0.as_str()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(self.as_str())
            }
        }

        impl Serialize for $name {
            fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
            where
                S: Serializer,
            {
                serializer.serialize_str(self.as_str())
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: Deserializer<'de>,
            {
                let input = String::deserialize(deserializer)?;
                Self::new(&input).map_err(serde::de::Error::custom)
            }
        }
    };
}

bounded_ascii_identity!(
    /// An opaque venue-order identity component.
    ///
    /// Use [`PmVenueOrderKey`] when an identity must be unique across
    /// configured accounts.
    PmVenueOrderId,
    OPAQUE_ID_CAPACITY
);
bounded_ascii_identity!(
    /// An opaque fill identity component.
    ///
    /// Use [`PmFillKey`] when an identity must be unique across configured
    /// accounts.
    PmFillId,
    OPAQUE_ID_CAPACITY
);
bounded_ascii_identity!(PmConnectionId, OPAQUE_ID_CAPACITY);
bounded_ascii_identity!(PmEnvironmentId, CONFIG_ID_CAPACITY);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct OkxInstrumentId(BoundedAscii<CONFIG_ID_CAPACITY>);

impl OkxInstrumentId {
    pub fn new(value: &str) -> Result<Self, PmIdentityError> {
        let inner = BoundedAscii::new(value)?;
        let bytes = value.as_bytes();
        let valid = bytes.first().is_some_and(u8::is_ascii_alphanumeric)
            && bytes.last().is_some_and(u8::is_ascii_alphanumeric)
            && bytes.contains(&b'-')
            && !bytes.windows(2).any(|pair| pair == b"--")
            && bytes
                .iter()
                .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || *byte == b'-');
        if valid {
            Ok(Self(inner))
        } else {
            Err(PmIdentityError::InvalidOkxInstrument)
        }
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

impl fmt::Display for OkxInstrumentId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl Serialize for OkxInstrumentId {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for OkxInstrumentId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let input = String::deserialize(deserializer)?;
        Self::new(&input).map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OkxReferenceKind {
    Index,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct OkxReferenceInstrument {
    kind: OkxReferenceKind,
    instrument_id: OkxInstrumentId,
}

impl OkxReferenceInstrument {
    #[must_use]
    pub const fn index(instrument_id: OkxInstrumentId) -> Self {
        Self {
            kind: OkxReferenceKind::Index,
            instrument_id,
        }
    }

    #[must_use]
    pub const fn venue(self) -> Venue {
        Venue::Okx
    }

    #[must_use]
    pub const fn kind(self) -> OkxReferenceKind {
        self.kind
    }

    #[must_use]
    pub const fn instrument_id(self) -> OkxInstrumentId {
        self.instrument_id
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct PmChainId(u64);

impl PmChainId {
    pub fn new(value: u64) -> Result<Self, PmIdentityError> {
        if value == 0 {
            Err(PmIdentityError::InvalidChain)
        } else {
            Ok(Self(value))
        }
    }

    #[must_use]
    pub const fn value(self) -> u64 {
        self.0
    }
}

impl<'de> Deserialize<'de> for PmChainId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::new(u64::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct PmSignerId(EvmAddress);

impl PmSignerId {
    #[must_use]
    pub const fn new(address: EvmAddress) -> Self {
        Self(address)
    }

    #[must_use]
    pub const fn address(self) -> EvmAddress {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct PmFunderId(EvmAddress);

impl PmFunderId {
    #[must_use]
    pub const fn new(address: EvmAddress) -> Self {
        Self(address)
    }

    #[must_use]
    pub const fn address(self) -> EvmAddress {
        self.0
    }
}

macro_rules! compact_handle {
    ($name:ident) => {
        #[derive(
            Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
        )]
        #[serde(transparent)]
        pub struct $name(u16);

        impl $name {
            #[must_use]
            pub const fn from_ordinal(ordinal: u16) -> Self {
                Self(ordinal)
            }

            #[must_use]
            pub const fn ordinal(self) -> u16 {
                self.0
            }
        }
    };
}

compact_handle!(OkxReferenceHandle);
compact_handle!(PmMarketHandle);
compact_handle!(PmTokenHandle);
compact_handle!(PmAccountHandle);
compact_handle!(PmSpenderHandle);
compact_handle!(PmSourceHandle);

macro_rules! account_scoped_key {
    ($(#[$attribute:meta])* $name:ident, $id:ty) => {
        $(#[$attribute])*
        #[derive(
            Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
        )]
        #[serde(deny_unknown_fields)]
        pub struct $name {
            account: PmAccountHandle,
            id: $id,
        }

        impl $name {
            #[must_use]
            pub const fn new(account: PmAccountHandle, id: $id) -> Self {
                Self { account, id }
            }

            #[must_use]
            pub const fn account(self) -> PmAccountHandle {
                self.account
            }

            #[must_use]
            pub const fn id(self) -> $id {
                self.id
            }
        }
    };
}

account_scoped_key!(
    /// The canonical account-scoped key for a client-order identity.
    PmClientOrderKey,
    PmClientOrderId
);
account_scoped_key!(
    /// The canonical account-scoped key for a venue-order identity.
    PmVenueOrderKey,
    PmVenueOrderId
);
account_scoped_key!(
    /// The canonical account-scoped key for a fill identity.
    PmFillKey,
    PmFillId
);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct PmInstrumentHandle {
    market: PmMarketHandle,
    token: PmTokenHandle,
}

impl PmInstrumentHandle {
    #[must_use]
    pub const fn new(market: PmMarketHandle, token: PmTokenHandle) -> Self {
        Self { market, token }
    }

    #[must_use]
    pub const fn market(self) -> PmMarketHandle {
        self.market
    }

    #[must_use]
    pub const fn token(self) -> PmTokenHandle {
        self.token
    }
}

macro_rules! revision_identity {
    ($name:ident) => {
        #[derive(
            Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
        )]
        #[serde(transparent)]
        pub struct $name(u64);

        impl $name {
            #[must_use]
            pub const fn new(value: u64) -> Self {
                Self(value)
            }

            #[must_use]
            pub const fn value(self) -> u64 {
                self.0
            }
        }
    };
}

revision_identity!(ConnectionEpoch);
revision_identity!(SnapshotRevision);
revision_identity!(IngressSequence);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PmAccountScope {
    environment: PmEnvironmentId,
    chain: PmChainId,
    signer: PmSignerId,
    funder: PmFunderId,
    handle: PmAccountHandle,
}

impl PmAccountScope {
    #[must_use]
    pub const fn new(
        environment: PmEnvironmentId,
        chain: PmChainId,
        signer: PmSignerId,
        funder: PmFunderId,
        handle: PmAccountHandle,
    ) -> Self {
        Self {
            environment,
            chain,
            signer,
            funder,
            handle,
        }
    }

    #[must_use]
    pub const fn environment(self) -> PmEnvironmentId {
        self.environment
    }

    #[must_use]
    pub const fn chain(self) -> PmChainId {
        self.chain
    }

    #[must_use]
    pub const fn signer(self) -> PmSignerId {
        self.signer
    }

    #[must_use]
    pub const fn funder(self) -> PmFunderId {
        self.funder
    }

    #[must_use]
    pub const fn handle(self) -> PmAccountHandle {
        self.handle
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PmSpenderDomain {
    Standard,
    NegativeRisk,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PmAssetId {
    Collateral {
        contract: EvmAddress,
    },
    Outcome {
        contract: EvmAddress,
        token: PmTokenId,
    },
}

impl PmAssetId {
    #[must_use]
    pub const fn collateral(contract: EvmAddress) -> Self {
        Self::Collateral { contract }
    }

    #[must_use]
    pub const fn outcome(contract: EvmAddress, token: PmTokenId) -> Self {
        Self::Outcome { contract, token }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct PmSpenderRequirement {
    chain: PmChainId,
    spender: EvmAddress,
    domain: PmSpenderDomain,
    asset: PmAssetId,
}

impl PmSpenderRequirement {
    #[must_use]
    pub const fn new(
        chain: PmChainId,
        spender: EvmAddress,
        domain: PmSpenderDomain,
        asset: PmAssetId,
    ) -> Self {
        Self {
            chain,
            spender,
            domain,
            asset,
        }
    }

    #[must_use]
    pub const fn chain(self) -> PmChainId {
        self.chain
    }

    #[must_use]
    pub const fn spender(self) -> EvmAddress {
        self.spender
    }

    #[must_use]
    pub const fn domain(self) -> PmSpenderDomain {
        self.domain
    }

    #[must_use]
    pub const fn asset(self) -> PmAssetId {
        self.asset
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PmSpenderId {
    account: PmAccountHandle,
    requirement: PmSpenderRequirement,
}

impl PmSpenderId {
    #[must_use]
    pub const fn new(account: PmAccountHandle, requirement: PmSpenderRequirement) -> Self {
        Self {
            account,
            requirement,
        }
    }

    #[must_use]
    pub const fn account(self) -> PmAccountHandle {
        self.account
    }

    #[must_use]
    pub const fn requirement(self) -> PmSpenderRequirement {
        self.requirement
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PmProductSource {
    OkxReference {
        source: PmSourceHandle,
        reference: OkxReferenceHandle,
    },
    PolymarketMarket {
        source: PmSourceHandle,
        token: PmTokenHandle,
    },
    PolymarketAccount {
        source: PmSourceHandle,
        account: PmAccountHandle,
    },
}

impl PmProductSource {
    #[must_use]
    pub const fn okx_reference(source: PmSourceHandle, reference: OkxReferenceHandle) -> Self {
        Self::OkxReference { source, reference }
    }

    #[must_use]
    pub const fn polymarket_market(source: PmSourceHandle, token: PmTokenHandle) -> Self {
        Self::PolymarketMarket { source, token }
    }

    #[must_use]
    pub const fn polymarket_account(source: PmSourceHandle, account: PmAccountHandle) -> Self {
        Self::PolymarketAccount { source, account }
    }

    #[must_use]
    pub const fn venue(self) -> Venue {
        match self {
            Self::OkxReference { .. } => Venue::Okx,
            Self::PolymarketMarket { .. } | Self::PolymarketAccount { .. } => Venue::Polymarket,
        }
    }

    #[must_use]
    pub const fn source(self) -> PmSourceHandle {
        match self {
            Self::OkxReference { source, .. }
            | Self::PolymarketMarket { source, .. }
            | Self::PolymarketAccount { source, .. } => source,
        }
    }
}

/// Static source binding for a concrete normalized payload.
///
/// This is deliberately a generic bound rather than a runtime interface. It
/// lets the envelope prove that its source is the payload's exact configured
/// source, including the compact token or account handle.
pub trait PmSourceBound: Sized {
    fn source(&self) -> PmProductSource;
}
