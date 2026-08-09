use reap_pm_core::{
    EvmAddress, PmNumericError, PmOrderSalt, PmOrderSide, PmPrice, PmQuantity, PmTick, PmTokenId,
    U256, exact_order_amounts,
};
use serde::ser::{Serialize, SerializeStruct, Serializer};
use thiserror::Error;

pub const PM_CLOB_V2_EOA_SIGNATURE_TYPE: u8 = 0;
pub const PM_CLOB_V2_PROXY_SIGNATURE_TYPE: u8 = 1;
pub const PM_CLOB_V2_EMPTY_BYTES32: &str =
    "0x0000000000000000000000000000000000000000000000000000000000000000";

/// Closed CLOB V2 order-signature profiles supported by the fixed PM wire
/// contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum PmClobV2SignatureType {
    /// One EOA is both maker/funder and signer.
    Eoa = PM_CLOB_V2_EOA_SIGNATURE_TYPE,
    /// A distinct proxy is maker/funder while its owner EOA signs.
    Proxy = PM_CLOB_V2_PROXY_SIGNATURE_TYPE,
}

impl PmClobV2SignatureType {
    #[must_use]
    pub const fn value(self) -> u8 {
        self as u8
    }
}

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
    signature_profile: PmClobV2SignatureType,
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
        Self::new_checked(
            salt,
            maker,
            signer,
            token_id,
            side,
            price,
            quantity,
            tick,
            minimum_order_size,
            PmClobV2SignatureType::Eoa,
            timestamp_ms,
        )
    }

    /// Construct the fixed PM-T2 proxy profile: the nonzero proxy funder is
    /// the order maker and a distinct nonzero owner EOA is the order signer.
    #[allow(clippy::too_many_arguments)]
    pub fn new_pm_t2_proxy(
        salt: PmOrderSalt,
        proxy_funder: EvmAddress,
        eoa_signer: EvmAddress,
        token_id: PmTokenId,
        side: PmOrderSide,
        price: PmPrice,
        quantity: PmQuantity,
        tick: PmTick,
        minimum_order_size: PmQuantity,
        timestamp_ms: u64,
    ) -> Result<Self, PmUnsignedOrderError> {
        validate_proxy_identities(proxy_funder.bytes(), eoa_signer.bytes())?;
        Self::new_checked(
            salt,
            proxy_funder,
            eoa_signer,
            token_id,
            side,
            price,
            quantity,
            tick,
            minimum_order_size,
            PmClobV2SignatureType::Proxy,
            timestamp_ms,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn new_checked(
        salt: PmOrderSalt,
        maker: EvmAddress,
        signer: EvmAddress,
        token_id: PmTokenId,
        side: PmOrderSide,
        price: PmPrice,
        quantity: PmQuantity,
        tick: PmTick,
        minimum_order_size: PmQuantity,
        signature_profile: PmClobV2SignatureType,
        timestamp_ms: u64,
    ) -> Result<Self, PmUnsignedOrderError> {
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
            signature_profile,
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
        self.signature_profile.value()
    }

    #[must_use]
    pub const fn signature_profile(self) -> PmClobV2SignatureType {
        self.signature_profile
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
        object.serialize_field("signatureType", &self.signature_type())?;
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
    #[error("fixed proxy unsigned order requires a nonzero proxy maker/funder")]
    ZeroProxyFunder,
    #[error("fixed proxy unsigned order requires a nonzero EOA signer")]
    ZeroProxySigner,
    #[error("fixed proxy unsigned order requires distinct maker/funder and EOA signer")]
    ProxyIdentityMismatch,
    #[error("unsigned order timestamp must be a positive Unix millisecond value")]
    ZeroTimestamp,
    #[error(transparent)]
    Numeric(#[from] PmNumericError),
}

fn validate_proxy_identities(
    proxy_funder: [u8; 20],
    eoa_signer: [u8; 20],
) -> Result<(), PmUnsignedOrderError> {
    if proxy_funder == [0; 20] {
        return Err(PmUnsignedOrderError::ZeroProxyFunder);
    }
    if eoa_signer == [0; 20] {
        return Err(PmUnsignedOrderError::ZeroProxySigner);
    }
    if proxy_funder == eoa_signer {
        return Err(PmUnsignedOrderError::ProxyIdentityMismatch);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{PmUnsignedOrderError, validate_proxy_identities};

    #[test]
    fn proxy_identity_gate_explicitly_rejects_each_zero_role_and_equal_identities() {
        assert_eq!(
            validate_proxy_identities([0; 20], [0x11; 20]),
            Err(PmUnsignedOrderError::ZeroProxyFunder)
        );
        assert_eq!(
            validate_proxy_identities([0x11; 20], [0; 20]),
            Err(PmUnsignedOrderError::ZeroProxySigner)
        );
        assert_eq!(
            validate_proxy_identities([0x11; 20], [0x11; 20]),
            Err(PmUnsignedOrderError::ProxyIdentityMismatch)
        );
        assert_eq!(validate_proxy_identities([0x11; 20], [0x22; 20]), Ok(()));
    }
}
