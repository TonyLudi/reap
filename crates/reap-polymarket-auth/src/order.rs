use std::fmt;

use reap_pm_core::{PmOrderSalt, PmOrderSide, PmTokenId, U256};
use reap_polymarket_wire::{
    PM_CLOB_V2_EMPTY_BYTES32, PM_CLOB_V2_EOA_SIGNATURE_TYPE, PM_CLOB_V2_PROXY_SIGNATURE_TYPE,
    PmClobV2SignatureType, PmUnsignedClobV2Order,
};
use serde::Serialize;
use sha2::{Digest as _, Sha256};
use sha3::Keccak256;
use zeroize::Zeroizing;

use crate::identity::lower_hex;
use crate::{
    EoaAddress, ExpectedOrderId, FixedEoaSigner, FixedOrderId, L2Credentials,
    OwnedCancelSemanticRequestCommitment, PlaceSemanticRequestCommitment, PmAuthError,
    PmClobDomain, RuntimeExactBodyCommitment,
};

const DOMAIN_NAME: &str = "Polymarket CTF Exchange";
const DOMAIN_VERSION: &str = "2";
const CHAIN_ID: u64 = 137;
const DOMAIN_TYPE: &str =
    "EIP712Domain(string name,string version,uint256 chainId,address verifyingContract)";
const ORDER_TYPE: &str = concat!(
    "Order(uint256 salt,address maker,address signer,uint256 tokenId,",
    "uint256 makerAmount,uint256 takerAmount,uint8 side,uint8 signatureType,",
    "uint256 timestamp,bytes32 metadata,bytes32 builder)"
);
const MAX_PLACE_BODY_BYTES: usize = 1_024;
const MAX_CANCEL_BODY_BYTES: usize = 128;
const PLACE_SEMANTIC_COMMITMENT_DOMAIN: &[u8] = b"reap.polymarket.auth.place-semantic-request.v1\0";
const CANCEL_SEMANTIC_COMMITMENT_DOMAIN: &[u8] =
    b"reap.polymarket.auth.owned-cancel-semantic-request.v1\0";

#[derive(Clone, Copy)]
struct FixedPlaceSemanticProfile {
    method: &'static [u8],
    route: &'static [u8],
    domain_name: &'static [u8],
    domain_version: &'static [u8],
    chain_id: u64,
    signature_type: u8,
    order_type: &'static [u8],
    post_only: bool,
    defer_exec: bool,
    expiration: &'static [u8],
    metadata: &'static [u8],
    builder: &'static [u8],
    maker_equals_signer: bool,
}

const FIXED_PLACE_SEMANTIC_PROFILE: FixedPlaceSemanticProfile = FixedPlaceSemanticProfile {
    method: b"POST",
    route: b"/order",
    domain_name: DOMAIN_NAME.as_bytes(),
    domain_version: DOMAIN_VERSION.as_bytes(),
    chain_id: CHAIN_ID,
    signature_type: PM_CLOB_V2_EOA_SIGNATURE_TYPE,
    order_type: b"GTC",
    post_only: true,
    defer_exec: false,
    expiration: b"0",
    metadata: PM_CLOB_V2_EMPTY_BYTES32.as_bytes(),
    builder: PM_CLOB_V2_EMPTY_BYTES32.as_bytes(),
    maker_equals_signer: true,
};

const FIXED_PROXY_PLACE_SEMANTIC_PROFILE: FixedPlaceSemanticProfile = FixedPlaceSemanticProfile {
    signature_type: PM_CLOB_V2_PROXY_SIGNATURE_TYPE,
    maker_equals_signer: false,
    ..FIXED_PLACE_SEMANTIC_PROFILE
};

#[derive(Clone, Copy)]
struct PlaceSemanticCommitmentBasis {
    profile: FixedPlaceSemanticProfile,
    expected_order_id: ExpectedOrderId,
    domain: PmClobDomain,
    salt: PmOrderSalt,
    maker: EoaAddress,
    signer: EoaAddress,
    token_id: PmTokenId,
    maker_amount: U256,
    taker_amount: U256,
    side: PmOrderSide,
    order_timestamp_ms: u64,
}

#[derive(Clone, Copy)]
struct FixedCancelSemanticProfile {
    method: &'static [u8],
    route: &'static [u8],
    operation: &'static [u8],
}

const FIXED_CANCEL_SEMANTIC_PROFILE: FixedCancelSemanticProfile = FixedCancelSemanticProfile {
    method: b"DELETE",
    route: b"/order",
    operation: b"exact-journal-proven-owned-order",
};

#[derive(Clone, Copy)]
struct CancelSemanticCommitmentBasis {
    profile: FixedCancelSemanticProfile,
    order_id: FixedOrderId,
}

/// One fixed-profile signed order. It can only be consumed into the fixed
/// GTC/post-only place body.
pub struct SignedClobV2Order {
    domain: PmClobDomain,
    salt: PmOrderSalt,
    maker: EoaAddress,
    signer: EoaAddress,
    token_id: PmTokenId,
    maker_amount: U256,
    taker_amount: U256,
    side: PmOrderSide,
    signature_profile: PmClobV2SignatureType,
    timestamp_ms: u64,
    signature: Zeroizing<[u8; 65]>,
    expected_order_id: ExpectedOrderId,
}

impl SignedClobV2Order {
    #[must_use]
    pub const fn expected_order_id(&self) -> ExpectedOrderId {
        self.expected_order_id
    }
}

impl fmt::Debug for SignedClobV2Order {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SignedClobV2Order([REDACTED])")
    }
}

/// Final place body serialized exactly once. The only way to reveal its bytes
/// is to consume it into a fixed authenticated place request and dispatch it
/// to a fixed-place sink.
pub struct SerializedPlaceRequest {
    pub(crate) body: Zeroizing<Vec<u8>>,
    expected_order_id: ExpectedOrderId,
    expected_making_amount: U256,
    expected_taking_amount: U256,
    runtime_exact_body_commitment: RuntimeExactBodyCommitment,
    semantic_request_commitment: PlaceSemanticRequestCommitment,
}

impl SerializedPlaceRequest {
    #[must_use]
    pub const fn expected_order_id(&self) -> ExpectedOrderId {
        self.expected_order_id
    }

    #[must_use]
    pub const fn expected_making_amount(&self) -> U256 {
        self.expected_making_amount
    }

    #[must_use]
    pub const fn expected_taking_amount(&self) -> U256 {
        self.expected_taking_amount
    }

    #[must_use]
    pub const fn runtime_exact_body_commitment(&self) -> RuntimeExactBodyCommitment {
        self.runtime_exact_body_commitment
    }

    /// Secret-free fixed-place identity suitable for an upper durable request
    /// commitment. It never incorporates body bytes or credential material.
    #[must_use]
    pub const fn semantic_request_commitment(&self) -> PlaceSemanticRequestCommitment {
        self.semantic_request_commitment
    }
}

impl fmt::Debug for SerializedPlaceRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SerializedPlaceRequest([REDACTED])")
    }
}

/// Final exact-owned cancel body serialized exactly once.
pub struct SerializedOwnedCancelRequest {
    pub(crate) body: Zeroizing<Vec<u8>>,
    order_id: FixedOrderId,
    runtime_exact_body_commitment: RuntimeExactBodyCommitment,
    semantic_request_commitment: OwnedCancelSemanticRequestCommitment,
}

impl SerializedOwnedCancelRequest {
    #[must_use]
    pub const fn order_id(&self) -> FixedOrderId {
        self.order_id
    }

    #[must_use]
    pub const fn runtime_exact_body_commitment(&self) -> RuntimeExactBodyCommitment {
        self.runtime_exact_body_commitment
    }

    /// Secret-free fixed-cancel identity suitable for an upper durable
    /// request commitment.
    #[must_use]
    pub const fn semantic_request_commitment(&self) -> OwnedCancelSemanticRequestCommitment {
        self.semantic_request_commitment
    }
}

impl fmt::Debug for SerializedOwnedCancelRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SerializedOwnedCancelRequest([REDACTED])")
    }
}

impl FixedEoaSigner {
    /// Sign one already validated Goal F unsigned order under one of the two
    /// fixed Polygon CLOB V2 domains.
    pub fn sign_clob_v2_order(
        &self,
        domain: PmClobDomain,
        order: PmUnsignedClobV2Order,
    ) -> Result<SignedClobV2Order, PmAuthError> {
        let account = self.address().as_core();
        let identity_matches = match order.signature_profile() {
            PmClobV2SignatureType::Eoa => order.maker() == account && order.signer() == account,
            PmClobV2SignatureType::Proxy => order.maker() != account && order.signer() == account,
        };
        if !identity_matches {
            return Err(PmAuthError::OrderIdentityMismatch);
        }

        let domain_separator = domain_separator(domain);
        let struct_hash = order_struct_hash(order);
        let digest = order_digest(domain_separator, struct_hash);
        let signing_key = self.signing_key()?;
        let (signature, recovery_id) = signing_key
            .sign_prehash_recoverable(&digest)
            .map_err(|_| PmAuthError::CryptographicFailure)?;
        if signature.normalize_s().is_some() || recovery_id.to_byte() > 1 {
            return Err(PmAuthError::CryptographicFailure);
        }

        let mut encoded_signature = Zeroizing::new([0_u8; 65]);
        encoded_signature[..64].copy_from_slice(signature.to_bytes().as_slice());
        encoded_signature[64] = 27 + recovery_id.to_byte();

        Ok(SignedClobV2Order {
            domain,
            salt: order.salt(),
            maker: EoaAddress::from_bytes(order.maker().bytes()),
            signer: self.address(),
            token_id: order.token_id(),
            maker_amount: order.maker_amount(),
            taker_amount: order.taker_amount(),
            side: order.side(),
            signature_profile: order.signature_profile(),
            timestamp_ms: order.timestamp_ms(),
            signature: encoded_signature,
            expected_order_id: ExpectedOrderId::from_bytes(digest),
        })
    }
}

impl L2Credentials {
    /// Consume one signed EOA or proxy order into the only supported place
    /// body: GTC, post-only, non-deferred, with owner equal to this bundle's
    /// API key. L2 credentials remain bound to the EOA signer for both modes.
    pub fn serialize_gtc_post_only(
        &self,
        order: SignedClobV2Order,
    ) -> Result<SerializedPlaceRequest, PmAuthError> {
        if order.signer != self.address() {
            return Err(PmAuthError::CredentialIdentityMismatch);
        }

        let semantic_request_commitment = place_semantic_request_commitment(&order);
        let maker = order.maker.to_string();
        let signer = order.signer.to_string();
        let mut signature_text = Zeroizing::new(String::with_capacity(132));
        signature_text.push_str("0x");
        signature_text.push_str(&lower_hex(order.signature.as_slice()));
        let body = PlaceBody {
            defer_exec: false,
            order: WireOrder {
                builder: PM_CLOB_V2_EMPTY_BYTES32,
                expiration: "0",
                maker: &maker,
                maker_amount: order.maker_amount,
                metadata: PM_CLOB_V2_EMPTY_BYTES32,
                salt: order.salt,
                side: match order.side {
                    PmOrderSide::Buy => "BUY",
                    PmOrderSide::Sell => "SELL",
                },
                signature: signature_text.as_str(),
                signature_type: order.signature_profile.value(),
                signer: &signer,
                taker_amount: order.taker_amount,
                timestamp: QuotedU64(order.timestamp_ms),
                token_id: order.token_id,
            },
            order_type: "GTC",
            owner: self.api_key(),
            post_only: true,
        };
        let body = Zeroizing::new(
            serde_json::to_vec(&body).map_err(|_| PmAuthError::SerializationFailure)?,
        );
        if body.len() > MAX_PLACE_BODY_BYTES {
            return Err(PmAuthError::RequestBodyTooLong);
        }
        let runtime_exact_body_commitment = runtime_exact_body_commitment(body.as_slice());
        Ok(SerializedPlaceRequest {
            body,
            expected_order_id: order.expected_order_id,
            expected_making_amount: order.maker_amount,
            expected_taking_amount: order.taker_amount,
            runtime_exact_body_commitment,
            semantic_request_commitment,
        })
    }

    /// Serialize one exact canonical order ID into the only supported cancel
    /// body. Ownership proof remains an upper lifecycle capability.
    pub fn serialize_owned_cancel(
        &self,
        order_id: FixedOrderId,
    ) -> Result<SerializedOwnedCancelRequest, PmAuthError> {
        let semantic_request_commitment = owned_cancel_semantic_request_commitment(order_id);
        let order_id_text = order_id.to_string();
        let body = Zeroizing::new(
            serde_json::to_vec(&CancelBody {
                order_id: &order_id_text,
            })
            .map_err(|_| PmAuthError::SerializationFailure)?,
        );
        if body.len() > MAX_CANCEL_BODY_BYTES {
            return Err(PmAuthError::RequestBodyTooLong);
        }
        let runtime_exact_body_commitment = runtime_exact_body_commitment(body.as_slice());
        Ok(SerializedOwnedCancelRequest {
            body,
            order_id,
            runtime_exact_body_commitment,
            semantic_request_commitment,
        })
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PlaceBody<'a> {
    defer_exec: bool,
    order: WireOrder<'a>,
    order_type: &'static str,
    owner: &'a str,
    post_only: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WireOrder<'a> {
    builder: &'static str,
    expiration: &'static str,
    maker: &'a str,
    maker_amount: U256,
    metadata: &'static str,
    salt: PmOrderSalt,
    side: &'static str,
    signature: &'a str,
    signature_type: u8,
    signer: &'a str,
    taker_amount: U256,
    timestamp: QuotedU64,
    token_id: PmTokenId,
}

#[derive(Serialize)]
struct CancelBody<'a> {
    #[serde(rename = "orderID")]
    order_id: &'a str,
}

struct QuotedU64(u64);

impl Serialize for QuotedU64 {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.collect_str(&self.0)
    }
}

fn domain_separator(domain: PmClobDomain) -> [u8; 32] {
    let type_hash = Keccak256::digest(DOMAIN_TYPE.as_bytes());
    let name_hash = Keccak256::digest(DOMAIN_NAME.as_bytes());
    let version_hash = Keccak256::digest(DOMAIN_VERSION.as_bytes());
    let mut hasher = Keccak256::new();
    hasher.update(type_hash);
    hasher.update(name_hash);
    hasher.update(version_hash);
    hasher.update(u64_word(CHAIN_ID));
    hasher.update(address_word(domain.verifying_contract()));
    hasher.finalize().into()
}

fn order_struct_hash(order: PmUnsignedClobV2Order) -> [u8; 32] {
    let mut hasher = Keccak256::new();
    hasher.update(Keccak256::digest(ORDER_TYPE.as_bytes()));
    hasher.update(u64_word(order.salt().value()));
    hasher.update(address_word(order.maker().bytes()));
    hasher.update(address_word(order.signer().bytes()));
    hasher.update(order.token_id().units().to_be_bytes());
    hasher.update(order.maker_amount().to_be_bytes());
    hasher.update(order.taker_amount().to_be_bytes());
    hasher.update(u8_word(match order.side() {
        PmOrderSide::Buy => 0,
        PmOrderSide::Sell => 1,
    }));
    hasher.update(u8_word(order.signature_type()));
    hasher.update(u64_word(order.timestamp_ms()));
    hasher.update([0_u8; 32]);
    hasher.update([0_u8; 32]);
    hasher.finalize().into()
}

fn order_digest(domain: [u8; 32], order: [u8; 32]) -> [u8; 32] {
    let mut hasher = Keccak256::new();
    hasher.update([0x19, 0x01]);
    hasher.update(domain);
    hasher.update(order);
    hasher.finalize().into()
}

fn address_word(address: [u8; 20]) -> [u8; 32] {
    let mut word = [0_u8; 32];
    word[12..].copy_from_slice(&address);
    word
}

fn u64_word(value: u64) -> [u8; 32] {
    let mut word = [0_u8; 32];
    word[24..].copy_from_slice(&value.to_be_bytes());
    word
}

fn u8_word(value: u8) -> [u8; 32] {
    let mut word = [0_u8; 32];
    word[31] = value;
    word
}

fn runtime_exact_body_commitment(body: &[u8]) -> RuntimeExactBodyCommitment {
    RuntimeExactBodyCommitment::from_bytes(Sha256::digest(body).into())
}

fn place_semantic_request_commitment(order: &SignedClobV2Order) -> PlaceSemanticRequestCommitment {
    hash_place_semantic_basis(PlaceSemanticCommitmentBasis {
        profile: match order.signature_profile {
            PmClobV2SignatureType::Eoa => FIXED_PLACE_SEMANTIC_PROFILE,
            PmClobV2SignatureType::Proxy => FIXED_PROXY_PLACE_SEMANTIC_PROFILE,
        },
        expected_order_id: order.expected_order_id,
        domain: order.domain,
        salt: order.salt,
        maker: order.maker,
        signer: order.signer,
        token_id: order.token_id,
        maker_amount: order.maker_amount,
        taker_amount: order.taker_amount,
        side: order.side,
        order_timestamp_ms: order.timestamp_ms,
    })
}

fn hash_place_semantic_basis(
    basis: PlaceSemanticCommitmentBasis,
) -> PlaceSemanticRequestCommitment {
    let profile = basis.profile;
    let mut hasher = Sha256::new();
    hasher.update(PLACE_SEMANTIC_COMMITMENT_DOMAIN);
    update_semantic_bytes(&mut hasher, profile.method);
    update_semantic_bytes(&mut hasher, profile.route);
    update_semantic_bytes(&mut hasher, profile.domain_name);
    update_semantic_bytes(&mut hasher, profile.domain_version);
    hasher.update(profile.chain_id.to_be_bytes());
    hasher.update([profile.signature_type]);
    update_semantic_bytes(&mut hasher, profile.order_type);
    hasher.update([u8::from(profile.post_only)]);
    hasher.update([u8::from(profile.defer_exec)]);
    update_semantic_bytes(&mut hasher, profile.expiration);
    update_semantic_bytes(&mut hasher, profile.metadata);
    update_semantic_bytes(&mut hasher, profile.builder);
    hasher.update([u8::from(profile.maker_equals_signer)]);
    hasher.update([match basis.domain {
        PmClobDomain::Standard => 0,
        PmClobDomain::NegativeRisk => 1,
    }]);
    hasher.update(basis.domain.verifying_contract());
    hasher.update(basis.expected_order_id.bytes());
    hasher.update(basis.salt.value().to_be_bytes());
    hasher.update(basis.maker.bytes());
    hasher.update(basis.signer.bytes());
    hasher.update(basis.token_id.units().to_be_bytes());
    hasher.update(basis.maker_amount.to_be_bytes());
    hasher.update(basis.taker_amount.to_be_bytes());
    hasher.update([match basis.side {
        PmOrderSide::Buy => 0,
        PmOrderSide::Sell => 1,
    }]);
    hasher.update(basis.order_timestamp_ms.to_be_bytes());
    PlaceSemanticRequestCommitment::from_bytes(hasher.finalize().into())
}

fn owned_cancel_semantic_request_commitment(
    order_id: FixedOrderId,
) -> OwnedCancelSemanticRequestCommitment {
    hash_cancel_semantic_basis(CancelSemanticCommitmentBasis {
        profile: FIXED_CANCEL_SEMANTIC_PROFILE,
        order_id,
    })
}

fn hash_cancel_semantic_basis(
    basis: CancelSemanticCommitmentBasis,
) -> OwnedCancelSemanticRequestCommitment {
    let mut hasher = Sha256::new();
    hasher.update(CANCEL_SEMANTIC_COMMITMENT_DOMAIN);
    update_semantic_bytes(&mut hasher, basis.profile.method);
    update_semantic_bytes(&mut hasher, basis.profile.route);
    update_semantic_bytes(&mut hasher, basis.profile.operation);
    hasher.update(basis.order_id.bytes());
    OwnedCancelSemanticRequestCommitment::from_bytes(hasher.finalize().into())
}

fn update_semantic_bytes(hasher: &mut Sha256, value: &[u8]) {
    let length = u64::try_from(value.len()).expect("fixed semantic field length fits u64");
    hasher.update(length.to_be_bytes());
    hasher.update(value);
}

#[cfg(test)]
mod tests {
    use super::{
        CancelSemanticCommitmentBasis, FIXED_CANCEL_SEMANTIC_PROFILE, FIXED_PLACE_SEMANTIC_PROFILE,
        FIXED_PROXY_PLACE_SEMANTIC_PROFILE, FixedCancelSemanticProfile, FixedPlaceSemanticProfile,
        ORDER_TYPE, PlaceSemanticCommitmentBasis, domain_separator, hash_cancel_semantic_basis,
        hash_place_semantic_basis, order_struct_hash,
    };
    use crate::{EoaAddress, ExpectedOrderId, FixedOrderId, PmClobDomain};
    use reap_pm_core::{
        EvmAddress, PmOrderSalt, PmOrderSide, PmPrice, PmQuantity, PmTick, PmTokenId, U256,
    };
    use reap_polymarket_wire::PM_CLOB_V2_PROXY_SIGNATURE_TYPE;
    use reap_polymarket_wire::PmUnsignedClobV2Order;
    use sha3::{Digest as _, Keccak256};

    fn vector_order(side: PmOrderSide) -> PmUnsignedClobV2Order {
        let address = EvmAddress::parse("0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266").unwrap();
        PmUnsignedClobV2Order::new_goal_f(
            PmOrderSalt::from_u64(479_249_096_354).unwrap(),
            address,
            address,
            PmTokenId::new(U256::from_u64(1_234)).unwrap(),
            side,
            PmPrice::parse_decimal("0.52").unwrap(),
            PmQuantity::parse_decimal("10").unwrap(),
            PmTick::parse_decimal("0.01").unwrap(),
            PmQuantity::parse_decimal("5").unwrap(),
            1_780_449_126_930,
        )
        .unwrap()
    }

    #[test]
    fn contract_type_and_hash_vectors_are_exact() {
        assert_eq!(
            format!(
                "0x{}",
                crate::identity::lower_hex(&Keccak256::digest(ORDER_TYPE.as_bytes()))
            ),
            "0xbb86318a2138f5fa8ae32fbe8e659f8fcf13cc6ae4014a707893055433818589"
        );
        assert_eq!(
            format!(
                "0x{}",
                crate::identity::lower_hex(&domain_separator(PmClobDomain::Standard))
            ),
            "0x3264e159346253e26a64e00b69032db0e7d32f94628de3e6eecb50304d7af3d2"
        );
        assert_eq!(
            format!(
                "0x{}",
                crate::identity::lower_hex(&order_struct_hash(vector_order(PmOrderSide::Buy)))
            ),
            "0x600e0697b4d487190e10b8f3a79b4489c8d172ac41fefac6efc4a00b459a3b2e"
        );
        assert_eq!(
            format!(
                "0x{}",
                crate::identity::lower_hex(&order_struct_hash(vector_order(PmOrderSide::Sell)))
            ),
            "0x8633966131c65c5cabe59dee955d024d6406ff2fee8fc4e6cb1c74c00a1f6866"
        );
    }

    fn place_semantic_basis() -> PlaceSemanticCommitmentBasis {
        PlaceSemanticCommitmentBasis {
            profile: FIXED_PLACE_SEMANTIC_PROFILE,
            expected_order_id: ExpectedOrderId::from_bytes([0x11; 32]),
            domain: PmClobDomain::Standard,
            salt: PmOrderSalt::from_u64(7).unwrap(),
            maker: EoaAddress::from_bytes([0x22; 20]),
            signer: EoaAddress::from_bytes([0x22; 20]),
            token_id: PmTokenId::new(U256::from_u64(1_234)).unwrap(),
            maker_amount: U256::from_u64(5_200_000),
            taker_amount: U256::from_u64(10_000_000),
            side: PmOrderSide::Buy,
            order_timestamp_ms: 1_780_449_126_930,
        }
    }

    fn changed_profile(
        change: impl FnOnce(&mut FixedPlaceSemanticProfile),
    ) -> PlaceSemanticCommitmentBasis {
        let mut basis = place_semantic_basis();
        change(&mut basis.profile);
        basis
    }

    #[test]
    fn place_semantic_commitment_binds_every_public_order_and_fixed_profile_category() {
        let basis = place_semantic_basis();
        let exact = hash_place_semantic_basis(basis);
        let mut changed = vec![
            hash_place_semantic_basis(PlaceSemanticCommitmentBasis {
                expected_order_id: ExpectedOrderId::from_bytes([0x12; 32]),
                ..basis
            }),
            hash_place_semantic_basis(PlaceSemanticCommitmentBasis {
                domain: PmClobDomain::NegativeRisk,
                ..basis
            }),
            hash_place_semantic_basis(PlaceSemanticCommitmentBasis {
                salt: PmOrderSalt::from_u64(8).unwrap(),
                ..basis
            }),
            hash_place_semantic_basis(PlaceSemanticCommitmentBasis {
                maker: EoaAddress::from_bytes([0x23; 20]),
                ..basis
            }),
            hash_place_semantic_basis(PlaceSemanticCommitmentBasis {
                signer: EoaAddress::from_bytes([0x23; 20]),
                ..basis
            }),
            hash_place_semantic_basis(PlaceSemanticCommitmentBasis {
                token_id: PmTokenId::new(U256::from_u64(1_235)).unwrap(),
                ..basis
            }),
            hash_place_semantic_basis(PlaceSemanticCommitmentBasis {
                maker_amount: U256::from_u64(5_200_001),
                ..basis
            }),
            hash_place_semantic_basis(PlaceSemanticCommitmentBasis {
                taker_amount: U256::from_u64(10_000_001),
                ..basis
            }),
            hash_place_semantic_basis(PlaceSemanticCommitmentBasis {
                side: PmOrderSide::Sell,
                ..basis
            }),
            hash_place_semantic_basis(PlaceSemanticCommitmentBasis {
                order_timestamp_ms: basis.order_timestamp_ms + 1,
                ..basis
            }),
        ];

        let profile_mutations: [fn(&mut FixedPlaceSemanticProfile); 13] = [
            |profile: &mut FixedPlaceSemanticProfile| profile.method = b"PUT",
            |profile: &mut FixedPlaceSemanticProfile| profile.route = b"/orders",
            |profile: &mut FixedPlaceSemanticProfile| profile.domain_name = b"other-domain",
            |profile: &mut FixedPlaceSemanticProfile| profile.domain_version = b"3",
            |profile: &mut FixedPlaceSemanticProfile| profile.chain_id += 1,
            |profile: &mut FixedPlaceSemanticProfile| profile.signature_type += 1,
            |profile: &mut FixedPlaceSemanticProfile| profile.order_type = b"GTD",
            |profile: &mut FixedPlaceSemanticProfile| profile.post_only = false,
            |profile: &mut FixedPlaceSemanticProfile| profile.defer_exec = true,
            |profile: &mut FixedPlaceSemanticProfile| profile.expiration = b"1",
            |profile: &mut FixedPlaceSemanticProfile| profile.metadata = b"other-metadata",
            |profile: &mut FixedPlaceSemanticProfile| profile.builder = b"other-builder",
            |profile: &mut FixedPlaceSemanticProfile| profile.maker_equals_signer = false,
        ];
        for mutation in profile_mutations {
            changed.push(hash_place_semantic_basis(changed_profile(mutation)));
        }

        assert!(changed.into_iter().all(|candidate| candidate != exact));
    }

    #[test]
    fn proxy_semantic_profile_binds_distinct_signer_and_type_one() {
        let eoa = place_semantic_basis();
        let proxy = PlaceSemanticCommitmentBasis {
            profile: FIXED_PROXY_PLACE_SEMANTIC_PROFILE,
            maker: EoaAddress::from_bytes([0x23; 20]),
            ..eoa
        };
        assert_eq!(
            proxy.profile.signature_type,
            PM_CLOB_V2_PROXY_SIGNATURE_TYPE
        );
        assert!(!proxy.profile.maker_equals_signer);
        assert_ne!(
            hash_place_semantic_basis(proxy),
            hash_place_semantic_basis(eoa)
        );
        assert_ne!(
            hash_place_semantic_basis(PlaceSemanticCommitmentBasis {
                signer: EoaAddress::from_bytes([0x24; 20]),
                ..proxy
            }),
            hash_place_semantic_basis(proxy)
        );
    }

    #[test]
    fn cancel_semantic_commitment_binds_fixed_profile_and_exact_order_identity() {
        let order_id = FixedOrderId::parse(
            "0x1111111111111111111111111111111111111111111111111111111111111111",
        )
        .unwrap();
        let basis = CancelSemanticCommitmentBasis {
            profile: FIXED_CANCEL_SEMANTIC_PROFILE,
            order_id,
        };
        let exact = hash_cancel_semantic_basis(basis);
        let changed_order = FixedOrderId::parse(
            "0x1211111111111111111111111111111111111111111111111111111111111111",
        )
        .unwrap();
        assert_ne!(
            hash_cancel_semantic_basis(CancelSemanticCommitmentBasis {
                order_id: changed_order,
                ..basis
            }),
            exact
        );

        for profile in [
            FixedCancelSemanticProfile {
                method: b"POST",
                ..FIXED_CANCEL_SEMANTIC_PROFILE
            },
            FixedCancelSemanticProfile {
                route: b"/orders",
                ..FIXED_CANCEL_SEMANTIC_PROFILE
            },
            FixedCancelSemanticProfile {
                operation: b"unowned-order",
                ..FIXED_CANCEL_SEMANTIC_PROFILE
            },
        ] {
            assert_ne!(
                hash_cancel_semantic_basis(CancelSemanticCommitmentBasis { profile, order_id }),
                exact
            );
        }

        assert_ne!(
            exact.bytes(),
            hash_place_semantic_basis(place_semantic_basis()).bytes()
        );
    }
}
