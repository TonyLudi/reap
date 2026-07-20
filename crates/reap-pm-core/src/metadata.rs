use std::fmt;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use thiserror::Error;

use crate::identity::{
    EvmAddress, PmAssetId, PmChainId, PmConditionId, PmMarketId, PmSpenderDomain,
    PmSpenderRequirement, PmTokenId,
};
use crate::numeric::{CLOB_V2_LOT_UNITS, PmQuantity, PmTick};

pub const MAX_REQUIRED_SPENDERS: usize = 8;
const OUTCOME_LABEL_CAPACITY: usize = 96;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum PmMetadataError {
    #[error("outcome label is empty")]
    EmptyOutcomeLabel,
    #[error("outcome label exceeds its fixed capacity")]
    OutcomeLabelTooLong,
    #[error("outcome label must contain visible ASCII text")]
    InvalidOutcomeLabel,
    #[error("market minimum is not aligned to the fixed lot")]
    MinimumOffLot,
    #[error("required spender count is outside its fixed bound")]
    InvalidSpenderCount,
    #[error("required spender array is not canonical for its count")]
    NonCanonicalSpenderArray,
    #[error("required spender identities contain a duplicate")]
    DuplicateSpender,
    #[error("required spender domain does not match the market")]
    SpenderDomainMismatch,
    #[error("required spender chain does not match the market")]
    SpenderChainMismatch,
    #[error("required spender address does not match the market exchange")]
    SpenderAddressMismatch,
    #[error("required spender outcome token does not match the market outcome")]
    SpenderOutcomeTokenMismatch,
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PmOutcomeLabel {
    length: u8,
    bytes: [u8; OUTCOME_LABEL_CAPACITY],
}

impl PmOutcomeLabel {
    pub fn new(value: &str) -> Result<Self, PmMetadataError> {
        if value.is_empty() {
            return Err(PmMetadataError::EmptyOutcomeLabel);
        }
        if value.len() > OUTCOME_LABEL_CAPACITY {
            return Err(PmMetadataError::OutcomeLabelTooLong);
        }
        if !value.is_ascii()
            || value
                .bytes()
                .any(|byte| byte.is_ascii_control() || byte == 0x7f)
            || value.trim_ascii() != value
        {
            return Err(PmMetadataError::InvalidOutcomeLabel);
        }
        let mut bytes = [0_u8; OUTCOME_LABEL_CAPACITY];
        bytes[..value.len()].copy_from_slice(value.as_bytes());
        Ok(Self {
            length: value.len() as u8,
            bytes,
        })
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        std::str::from_utf8(&self.bytes[..usize::from(self.length)]).expect("bounded outcome label")
    }
}

impl fmt::Debug for PmOutcomeLabel {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("PmOutcomeLabel")
            .field(&self.as_str())
            .finish()
    }
}

impl fmt::Display for PmOutcomeLabel {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl Serialize for PmOutcomeLabel {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for PmOutcomeLabel {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let input = String::deserialize(deserializer)?;
        Self::new(&input).map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PmOutcomeMetadata {
    token: PmTokenId,
    label: PmOutcomeLabel,
}

impl PmOutcomeMetadata {
    #[must_use]
    pub const fn new(token: PmTokenId, label: PmOutcomeLabel) -> Self {
        Self { token, label }
    }

    #[must_use]
    pub const fn token(self) -> PmTokenId {
        self.token
    }

    #[must_use]
    pub const fn label(self) -> PmOutcomeLabel {
        self.label
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PmMarketLifecycle {
    active: bool,
    closed: bool,
    archived: bool,
    accepting_orders: bool,
    order_book_enabled: bool,
}

impl PmMarketLifecycle {
    #[must_use]
    pub const fn new(
        active: bool,
        closed: bool,
        archived: bool,
        accepting_orders: bool,
        order_book_enabled: bool,
    ) -> Self {
        Self {
            active,
            closed,
            archived,
            accepting_orders,
            order_book_enabled,
        }
    }

    #[must_use]
    pub const fn active(self) -> bool {
        self.active
    }

    #[must_use]
    pub const fn closed(self) -> bool {
        self.closed
    }

    #[must_use]
    pub const fn archived(self) -> bool {
        self.archived
    }

    #[must_use]
    pub const fn accepting_orders(self) -> bool {
        self.accepting_orders
    }

    #[must_use]
    pub const fn order_book_enabled(self) -> bool {
        self.order_book_enabled
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
pub struct PmMarketMetadata {
    condition: PmConditionId,
    market: PmMarketId,
    outcome: PmOutcomeMetadata,
    lifecycle: PmMarketLifecycle,
    tick: PmTick,
    minimum_order_size: PmQuantity,
    negative_risk: bool,
    chain: PmChainId,
    exchange: EvmAddress,
    required_spenders: [Option<PmSpenderRequirement>; MAX_REQUIRED_SPENDERS],
    required_spender_count: u8,
}

impl PmMarketMetadata {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        condition: PmConditionId,
        market: PmMarketId,
        outcome: PmOutcomeMetadata,
        lifecycle: PmMarketLifecycle,
        tick: PmTick,
        minimum_order_size: PmQuantity,
        negative_risk: bool,
        chain: PmChainId,
        exchange: EvmAddress,
        mut required_spenders: [Option<PmSpenderRequirement>; MAX_REQUIRED_SPENDERS],
        required_spender_count: u8,
    ) -> Result<Self, PmMetadataError> {
        let (_, remainder) = minimum_order_size
            .protocol_units()
            .checked_div_rem_u32(CLOB_V2_LOT_UNITS)
            .map_err(|_| PmMetadataError::MinimumOffLot)?;
        if remainder != 0 {
            return Err(PmMetadataError::MinimumOffLot);
        }

        let count = usize::from(required_spender_count);
        if count == 0 || count > MAX_REQUIRED_SPENDERS {
            return Err(PmMetadataError::InvalidSpenderCount);
        }
        if required_spenders[..count].iter().any(Option::is_none)
            || required_spenders[count..].iter().any(Option::is_some)
        {
            return Err(PmMetadataError::NonCanonicalSpenderArray);
        }
        required_spenders[..count].sort_unstable();
        if required_spenders[..count]
            .windows(2)
            .any(|pair| pair[0] == pair[1])
        {
            return Err(PmMetadataError::DuplicateSpender);
        }

        let expected_domain = if negative_risk {
            PmSpenderDomain::NegativeRisk
        } else {
            PmSpenderDomain::Standard
        };
        for requirement in required_spenders[..count].iter().copied() {
            let requirement = requirement.expect("canonical required spender prefix");
            if requirement.domain() != expected_domain {
                return Err(PmMetadataError::SpenderDomainMismatch);
            }
            if requirement.chain() != chain {
                return Err(PmMetadataError::SpenderChainMismatch);
            }
            if requirement.spender() != exchange {
                return Err(PmMetadataError::SpenderAddressMismatch);
            }
            if matches!(
                requirement.asset(),
                PmAssetId::Outcome { token, .. } if token != outcome.token()
            ) {
                return Err(PmMetadataError::SpenderOutcomeTokenMismatch);
            }
        }

        Ok(Self {
            condition,
            market,
            outcome,
            lifecycle,
            tick,
            minimum_order_size,
            negative_risk,
            chain,
            exchange,
            required_spenders,
            required_spender_count,
        })
    }

    #[must_use]
    pub const fn condition(self) -> PmConditionId {
        self.condition
    }

    #[must_use]
    pub const fn market(self) -> PmMarketId {
        self.market
    }

    #[must_use]
    pub const fn outcome(self) -> PmOutcomeMetadata {
        self.outcome
    }

    #[must_use]
    pub const fn lifecycle(self) -> PmMarketLifecycle {
        self.lifecycle
    }

    #[must_use]
    pub const fn tick(self) -> PmTick {
        self.tick
    }

    #[must_use]
    pub const fn minimum_order_size(self) -> PmQuantity {
        self.minimum_order_size
    }

    #[must_use]
    pub const fn lot_units(self) -> u32 {
        CLOB_V2_LOT_UNITS
    }

    #[must_use]
    pub const fn negative_risk(self) -> bool {
        self.negative_risk
    }

    #[must_use]
    pub const fn chain(self) -> PmChainId {
        self.chain
    }

    #[must_use]
    pub const fn exchange(self) -> EvmAddress {
        self.exchange
    }

    pub fn required_spenders(&self) -> impl Iterator<Item = PmSpenderRequirement> + '_ {
        self.required_spenders[..usize::from(self.required_spender_count)]
            .iter()
            .flatten()
            .copied()
    }

    #[must_use]
    pub const fn required_spender_count(self) -> u8 {
        self.required_spender_count
    }
}

impl<'de> Deserialize<'de> for PmMarketMetadata {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Wire {
            condition: PmConditionId,
            market: PmMarketId,
            outcome: PmOutcomeMetadata,
            lifecycle: PmMarketLifecycle,
            tick: PmTick,
            minimum_order_size: PmQuantity,
            negative_risk: bool,
            chain: PmChainId,
            exchange: EvmAddress,
            required_spenders: [Option<PmSpenderRequirement>; MAX_REQUIRED_SPENDERS],
            required_spender_count: u8,
        }

        let wire = Wire::deserialize(deserializer)?;
        Self::new(
            wire.condition,
            wire.market,
            wire.outcome,
            wire.lifecycle,
            wire.tick,
            wire.minimum_order_size,
            wire.negative_risk,
            wire.chain,
            wire.exchange,
            wire.required_spenders,
            wire.required_spender_count,
        )
        .map_err(serde::de::Error::custom)
    }
}
