use reap_pm_core::{
    EvmAddress, PmNumericError, PmOrderSalt, PmOrderSide, PmPrice, PmQuantity, PmTick, PmTokenId,
    U256, exact_order_amounts,
};
use serde::ser::{Serialize, SerializeStruct, Serializer};
use thiserror::Error;

pub const PM_CLOB_V2_EOA_SIGNATURE_TYPE: u8 = 0;
pub const PM_CLOB_V2_EMPTY_BYTES32: &str =
    "0x0000000000000000000000000000000000000000000000000000000000000000";

/// Canonical unsigned CLOB V2 fields for the fixed Goal F EOA profile.
///
/// This value is structural wire data. It contains neither mutation authority
/// nor key material, and deliberately has no deserialization implementation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PmUnsignedClobV2Order {
    salt: PmOrderSalt,
    maker: EvmAddress,
    signer: EvmAddress,
    token_id: PmTokenId,
    maker_amount: U256,
    taker_amount: U256,
    side: PmOrderSide,
    timestamp_ms: u64,
}

impl PmUnsignedClobV2Order {
    #[allow(clippy::too_many_arguments)]
    pub fn new_goal_f(
        salt: PmOrderSalt,
        maker: EvmAddress,
        signer: EvmAddress,
        token_id: PmTokenId,
        side: PmOrderSide,
        price: PmPrice,
        quantity: PmQuantity,
        tick: PmTick,
        minimum_order_size: PmQuantity,
        timestamp_ms: u64,
    ) -> Result<Self, PmUnsignedOrderError> {
        if maker != signer {
            return Err(PmUnsignedOrderError::MakerIdentityMismatch);
        }
        if timestamp_ms == 0 {
            return Err(PmUnsignedOrderError::ZeroTimestamp);
        }
        price.validate_tick(tick)?;
        quantity.validate_order(minimum_order_size)?;
        let amounts = exact_order_amounts(side, price, quantity)?;

        Ok(Self {
            salt,
            maker,
            signer,
            token_id,
            maker_amount: amounts.maker(),
            taker_amount: amounts.taker(),
            side,
            timestamp_ms,
        })
    }

    #[must_use]
    pub const fn salt(self) -> PmOrderSalt {
        self.salt
    }

    #[must_use]
    pub const fn maker(self) -> EvmAddress {
        self.maker
    }

    #[must_use]
    pub const fn signer(self) -> EvmAddress {
        self.signer
    }

    #[must_use]
    pub const fn token_id(self) -> PmTokenId {
        self.token_id
    }

    #[must_use]
    pub const fn maker_amount(self) -> U256 {
        self.maker_amount
    }

    #[must_use]
    pub const fn taker_amount(self) -> U256 {
        self.taker_amount
    }

    #[must_use]
    pub const fn side(self) -> PmOrderSide {
        self.side
    }

    #[must_use]
    pub const fn signature_type(self) -> u8 {
        PM_CLOB_V2_EOA_SIGNATURE_TYPE
    }

    #[must_use]
    pub const fn timestamp_ms(self) -> u64 {
        self.timestamp_ms
    }

    #[must_use]
    pub const fn metadata(self) -> &'static str {
        PM_CLOB_V2_EMPTY_BYTES32
    }

    #[must_use]
    pub const fn builder(self) -> &'static str {
        PM_CLOB_V2_EMPTY_BYTES32
    }
}

impl Serialize for PmUnsignedClobV2Order {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        // Lexicographic field order makes the canonical bytes independent of
        // map implementations and matches the frozen Goal F golden vector.
        let mut object = serializer.serialize_struct("PmUnsignedClobV2Order", 11)?;
        object.serialize_field("builder", PM_CLOB_V2_EMPTY_BYTES32)?;
        object.serialize_field("maker", &self.maker)?;
        object.serialize_field("makerAmount", &self.maker_amount)?;
        object.serialize_field("metadata", PM_CLOB_V2_EMPTY_BYTES32)?;
        object.serialize_field("salt", &self.salt)?;
        object.serialize_field(
            "side",
            match self.side {
                PmOrderSide::Buy => "BUY",
                PmOrderSide::Sell => "SELL",
            },
        )?;
        object.serialize_field("signatureType", &PM_CLOB_V2_EOA_SIGNATURE_TYPE)?;
        object.serialize_field("signer", &self.signer)?;
        object.serialize_field("takerAmount", &self.taker_amount)?;
        object.serialize_field("timestamp", &QuotedU64(self.timestamp_ms))?;
        object.serialize_field("tokenId", &self.token_id)?;
        object.end()
    }
}

struct QuotedU64(u64);

impl Serialize for QuotedU64 {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.collect_str(&self.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum PmUnsignedOrderError {
    #[error("fixed EOA unsigned order requires maker and signer to match")]
    MakerIdentityMismatch,
    #[error("unsigned order timestamp must be a positive Unix millisecond value")]
    ZeroTimestamp,
    #[error(transparent)]
    Numeric(#[from] PmNumericError),
}
