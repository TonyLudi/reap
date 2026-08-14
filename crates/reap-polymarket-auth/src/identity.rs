use std::fmt;
use std::str::FromStr;

use reap_pm_core::EvmAddress;
use sha3::{Digest as _, Keccak256};

use crate::PmAuthError;

const STANDARD_EXCHANGE: [u8; 20] = [
    0xe1, 0x11, 0x18, 0x00, 0x00, 0xd2, 0x66, 0x3c, 0x00, 0x91, 0xe4, 0xf4, 0x00, 0x23, 0x75, 0x45,
    0xb8, 0x7b, 0x99, 0x6b,
];
const NEGATIVE_RISK_EXCHANGE: [u8; 20] = [
    0xe2, 0x22, 0x2d, 0x27, 0x9d, 0x74, 0x40, 0x50, 0xd2, 0x8e, 0x00, 0x52, 0x00, 0x10, 0x52, 0x00,
    0x00, 0x31, 0x0f, 0x59,
];

/// The only two CLOB V2 EIP-712 domains reachable in PM-T1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PmClobDomain {
    Standard,
    NegativeRisk,
}

impl PmClobDomain {
    pub(crate) const fn verifying_contract(self) -> [u8; 20] {
        match self {
            Self::Standard => STANDARD_EXCHANGE,
            Self::NegativeRisk => NEGATIVE_RISK_EXCHANGE,
        }
    }
}

/// A nonzero EOA in its exact canonical EIP-55 spelling.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EoaAddress([u8; 20]);

impl EoaAddress {
    pub fn parse(input: &str) -> Result<Self, PmAuthError> {
        let bytes = parse_prefixed_hex::<20>(input, false).ok_or(PmAuthError::InvalidEoaAddress)?;
        if bytes.iter().all(|byte| *byte == 0) {
            return Err(PmAuthError::InvalidEoaAddress);
        }
        let address = Self(bytes);
        if input != address.to_string() {
            return Err(PmAuthError::InvalidEoaAddress);
        }
        Ok(address)
    }

    pub(crate) const fn from_bytes(bytes: [u8; 20]) -> Self {
        Self(bytes)
    }

    #[must_use]
    pub const fn bytes(self) -> [u8; 20] {
        self.0
    }

    #[must_use]
    pub fn as_core(self) -> EvmAddress {
        EvmAddress::from_bytes(self.0).expect("validated nonzero EOA")
    }
}

impl FromStr for EoaAddress {
    type Err = PmAuthError;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        Self::parse(input)
    }
}

impl fmt::Display for EoaAddress {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write_eip55(formatter, self.0)
    }
}

impl fmt::Debug for EoaAddress {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("EoaAddress")
            .field(&self.to_string())
            .finish()
    }
}

/// An address in exact canonical EIP-55 spelling that is used only for the
/// deterministic legacy Polymarket type-1 proxy-address relation.
///
/// This value does not attest deployed code, current chain state, signer-key
/// possession or exclusivity, proxy control, provider acceptance,
/// authentication, or authorization.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LegacyType1ProxyAddress([u8; 20]);

impl LegacyType1ProxyAddress {
    pub fn parse(input: &str) -> Result<Self, PmAuthError> {
        let bytes = parse_prefixed_hex::<20>(input, false)
            .ok_or(PmAuthError::InvalidLegacyType1ProxyAddress)?;
        let address = Self(bytes);
        if input != address.to_string() {
            return Err(PmAuthError::InvalidLegacyType1ProxyAddress);
        }
        Ok(address)
    }

    pub(crate) const fn from_bytes(bytes: [u8; 20]) -> Self {
        Self(bytes)
    }

    #[must_use]
    pub const fn bytes(self) -> [u8; 20] {
        self.0
    }
}

impl FromStr for LegacyType1ProxyAddress {
    type Err = PmAuthError;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        Self::parse(input)
    }
}

impl fmt::Display for LegacyType1ProxyAddress {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write_eip55(formatter, self.0)
    }
}

impl fmt::Debug for LegacyType1ProxyAddress {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("LegacyType1ProxyAddress")
            .field(&self.to_string())
            .finish()
    }
}

/// Exact CLOB V2 EIP-712 order identity expected from the venue.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ExpectedOrderId([u8; 32]);

impl ExpectedOrderId {
    pub(crate) const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    #[must_use]
    pub const fn bytes(self) -> [u8; 32] {
        self.0
    }
}

impl fmt::Display for ExpectedOrderId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write_prefixed_lower_hex(formatter, &self.0)
    }
}

impl fmt::Debug for ExpectedOrderId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("ExpectedOrderId")
            .field(&self.to_string())
            .finish()
    }
}

/// A canonical exact order identity accepted by the fixed owned-cancel body.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FixedOrderId([u8; 32]);

impl FixedOrderId {
    pub fn parse(input: &str) -> Result<Self, PmAuthError> {
        parse_prefixed_hex::<32>(input, true)
            .map(Self)
            .ok_or(PmAuthError::InvalidOrderId)
    }

    #[must_use]
    pub const fn bytes(self) -> [u8; 32] {
        self.0
    }
}

impl From<ExpectedOrderId> for FixedOrderId {
    fn from(value: ExpectedOrderId) -> Self {
        Self(value.0)
    }
}

impl FromStr for FixedOrderId {
    type Err = PmAuthError;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        Self::parse(input)
    }
}

impl fmt::Display for FixedOrderId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write_prefixed_lower_hex(formatter, &self.0)
    }
}

impl fmt::Debug for FixedOrderId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("FixedOrderId")
            .field(&self.to_string())
            .finish()
    }
}

/// Runtime-only SHA-256 correlation for one final, once-serialized body.
///
/// A place body contains an API-key owner and a private-key-derived signature,
/// so this value is secret-derived even though SHA-256 is one-way. It must
/// never enter a journal, capture, metric, log, or other durable artifact. The
/// type intentionally implements neither `Serialize` nor `Display`, and its
/// debug representation never reveals the digest.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct RuntimeExactBodyCommitment([u8; 32]);

impl RuntimeExactBodyCommitment {
    pub(crate) const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Expose the digest only for in-memory exact-body correlation at the
    /// fixed transport edge. These bytes are forbidden from durable storage.
    #[must_use]
    pub const fn runtime_only_bytes(self) -> [u8; 32] {
        self.0
    }
}

impl fmt::Debug for RuntimeExactBodyCommitment {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("RuntimeExactBodyCommitment([REDACTED; NON_DURABLE])")
    }
}

/// Secret-free semantic identity of one fixed GTC post-only place request.
///
/// This commitment binds only public order and fixed-profile semantics. It is
/// safe to use as an input to an upper-layer durable request identity.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PlaceSemanticRequestCommitment([u8; 32]);

impl PlaceSemanticRequestCommitment {
    pub(crate) const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    #[must_use]
    pub const fn bytes(self) -> [u8; 32] {
        self.0
    }
}

impl fmt::Display for PlaceSemanticRequestCommitment {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write_prefixed_lower_hex(formatter, &self.0)
    }
}

impl fmt::Debug for PlaceSemanticRequestCommitment {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("PlaceSemanticRequestCommitment")
            .field(&self.to_string())
            .finish()
    }
}

/// Secret-free semantic identity of one fixed exact-owned cancel request.
///
/// The separate type prevents a place commitment from being correlated with
/// a cancel grant even though both durable representations are 32 bytes.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct OwnedCancelSemanticRequestCommitment([u8; 32]);

impl OwnedCancelSemanticRequestCommitment {
    pub(crate) const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    #[must_use]
    pub const fn bytes(self) -> [u8; 32] {
        self.0
    }
}

impl fmt::Display for OwnedCancelSemanticRequestCommitment {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write_prefixed_lower_hex(formatter, &self.0)
    }
}

impl fmt::Debug for OwnedCancelSemanticRequestCommitment {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("OwnedCancelSemanticRequestCommitment")
            .field(&self.to_string())
            .finish()
    }
}

pub(crate) fn lower_hex(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(HEX[usize::from(byte >> 4)] as char);
        output.push(HEX[usize::from(byte & 0x0f)] as char);
    }
    output
}

fn write_prefixed_lower_hex(formatter: &mut fmt::Formatter<'_>, bytes: &[u8]) -> fmt::Result {
    formatter.write_str("0x")?;
    for byte in bytes {
        write!(formatter, "{byte:02x}")?;
    }
    Ok(())
}

fn write_eip55(formatter: &mut fmt::Formatter<'_>, bytes: [u8; 20]) -> fmt::Result {
    let lowercase = lower_hex(&bytes);
    let checksum = Keccak256::digest(lowercase.as_bytes());
    formatter.write_str("0x")?;
    for (index, byte) in lowercase.bytes().enumerate() {
        let hash_nibble = if index % 2 == 0 {
            checksum[index / 2] >> 4
        } else {
            checksum[index / 2] & 0x0f
        };
        let output = if byte.is_ascii_alphabetic() && hash_nibble >= 8 {
            byte.to_ascii_uppercase()
        } else {
            byte
        };
        formatter.write_str(std::str::from_utf8(&[output]).expect("ASCII hex"))?;
    }
    Ok(())
}

fn parse_prefixed_hex<const N: usize>(input: &str, lowercase_only: bool) -> Option<[u8; N]> {
    let digits = input.strip_prefix("0x")?;
    if digits.len() != N * 2 {
        return None;
    }
    if lowercase_only && digits.bytes().any(|byte| byte.is_ascii_uppercase()) {
        return None;
    }
    let mut output = [0_u8; N];
    for (index, byte) in output.iter_mut().enumerate() {
        let high = hex_nibble(digits.as_bytes()[index * 2])?;
        let low = hex_nibble(digits.as_bytes()[index * 2 + 1])?;
        *byte = (high << 4) | low;
    }
    Some(output)
}

fn hex_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

const HEX: &[u8; 16] = b"0123456789abcdef";

#[cfg(test)]
mod tests {
    use super::EoaAddress;

    #[test]
    fn eip55_is_strict_and_round_trips() {
        let canonical = "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266";
        assert_eq!(EoaAddress::parse(canonical).unwrap().to_string(), canonical);
        assert!(EoaAddress::parse(&canonical.to_ascii_lowercase()).is_err());
        assert!(EoaAddress::parse("0x0000000000000000000000000000000000000000").is_err());
    }
}
