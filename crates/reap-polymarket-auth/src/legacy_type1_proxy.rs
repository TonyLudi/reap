//! Pure derivation of the legacy Polymarket type-1 proxy address.
//!
//! The calculation pins Polygon chain context and the factory's historical
//! CREATE2 inputs. It proves only a deterministic structural address relation.
//! It performs no I/O and makes no claim about deployed code,
//! current chain state, signer-key possession or exclusivity, proxy control,
//! provider acceptance, authentication, or authorization.

use sha3::{Digest as _, Keccak256};

use crate::{EoaAddress, LegacyType1ProxyAddress};

/// Polygon chain context for the pinned legacy type-1 relation.
///
/// CREATE2 does not include a chain ID in its preimage. This constant names
/// the only chain context in which this helper's fixed inputs are reviewed.
pub const POLYMARKET_LEGACY_TYPE1_PROXY_CHAIN_ID: u64 = 137;

const FACTORY: [u8; 20] = [
    0xab, 0x45, 0xc5, 0xa4, 0xb0, 0xc9, 0x41, 0xa2, 0xf2, 0x31, 0xc0, 0x4c, 0x3f, 0x49, 0x18, 0x2e,
    0x1a, 0x25, 0x40, 0x52,
];
#[cfg(test)]
const IMPLEMENTATION: [u8; 20] = [
    0x44, 0xe9, 0x99, 0xd5, 0xc2, 0xf6, 0x6e, 0xf0, 0x86, 0x13, 0x17, 0xf9, 0xa4, 0x80, 0x5a, 0xc2,
    0xe9, 0x0a, 0xeb, 0x4f,
];
const INIT_CODE_KECCAK256: [u8; 32] = [
    0xd2, 0x1d, 0xf8, 0xdc, 0x65, 0x88, 0x0a, 0x86, 0x06, 0xf0, 0x9f, 0xe0, 0xce, 0x3d, 0xf9, 0xb8,
    0x86, 0x92, 0x87, 0xab, 0x0b, 0x05, 0x8b, 0xe0, 0x5a, 0xa9, 0xe8, 0xaf, 0x63, 0x30, 0xa0, 0x0b,
];
#[cfg(test)]
const RUNTIME_KECCAK256: [u8; 32] = [
    0x2f, 0xba, 0x6f, 0xc1, 0x87, 0xf7, 0x78, 0x26, 0xfa, 0xf1, 0x97, 0xb5, 0x50, 0x8d, 0x25, 0xc9,
    0x8b, 0x54, 0x58, 0x1c, 0x51, 0x23, 0x33, 0x3f, 0x10, 0x60, 0xf2, 0xbd, 0x87, 0xf3, 0x8b, 0x9b,
];

// Exact 167-byte factory init-code input, including FACTORY, the 45-byte
// EIP-1167 runtime for IMPLEMENTATION, selector 0x52e831dd, and the fixed
// abi.encode(bytes("")) empty payload.
#[cfg(test)]
const INIT_CODE: [u8; 167] = [
    0x3d, 0x3d, 0x60, 0x63, 0x80, 0x38, 0x03, 0x80, 0x91, 0x3d, 0x39, 0x3d, 0x73, 0xab, 0x45, 0xc5,
    0xa4, 0xb0, 0xc9, 0x41, 0xa2, 0xf2, 0x31, 0xc0, 0x4c, 0x3f, 0x49, 0x18, 0x2e, 0x1a, 0x25, 0x40,
    0x52, 0x5a, 0xf4, 0x60, 0x2a, 0x57, 0x60, 0x00, 0x80, 0xfd, 0x5b, 0x60, 0x2d, 0x80, 0x60, 0x36,
    0x60, 0x00, 0x39, 0x60, 0x00, 0xf3, 0x36, 0x3d, 0x3d, 0x37, 0x3d, 0x3d, 0x3d, 0x36, 0x3d, 0x73,
    0x44, 0xe9, 0x99, 0xd5, 0xc2, 0xf6, 0x6e, 0xf0, 0x86, 0x13, 0x17, 0xf9, 0xa4, 0x80, 0x5a, 0xc2,
    0xe9, 0x0a, 0xeb, 0x4f, 0x5a, 0xf4, 0x3d, 0x82, 0x80, 0x3e, 0x90, 0x3d, 0x91, 0x60, 0x2b, 0x57,
    0xfd, 0x5b, 0xf3, 0x52, 0xe8, 0x31, 0xdd, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x20, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
];
#[cfg(test)]
const RUNTIME: [u8; 45] = [
    0x36, 0x3d, 0x3d, 0x37, 0x3d, 0x3d, 0x3d, 0x36, 0x3d, 0x73, 0x44, 0xe9, 0x99, 0xd5, 0xc2, 0xf6,
    0x6e, 0xf0, 0x86, 0x13, 0x17, 0xf9, 0xa4, 0x80, 0x5a, 0xc2, 0xe9, 0x0a, 0xeb, 0x4f, 0x5a, 0xf4,
    0x3d, 0x82, 0x80, 0x3e, 0x90, 0x3d, 0x91, 0x60, 0x2b, 0x57, 0xfd, 0x5b, 0xf3,
];

/// Derive the fixed Polygon legacy type-1 proxy address for `signer`.
///
/// The salt is `keccak256(abi.encodePacked(signer))`, where the packed signer
/// is exactly 20 bytes. The result is the low 20 bytes of
/// `keccak256(0xff || factory || salt || init_code_keccak256)`.
///
/// This is a structural calculation only. A matching address does not prove
/// deployed bytecode, current state, signer possession, proxy control,
/// provider acceptance, authentication, or authorization.
#[must_use]
pub fn derive_legacy_type1_proxy_address(signer: EoaAddress) -> LegacyType1ProxyAddress {
    let salt = <[u8; 32]>::from(Keccak256::digest(signer.bytes()));
    derive_from_salt(salt)
}

/// Compare `candidate` with the deterministic legacy type-1 address for
/// `signer`, without performing I/O or establishing any authority claim.
#[must_use]
pub fn legacy_type1_proxy_address_matches(
    signer: EoaAddress,
    candidate: LegacyType1ProxyAddress,
) -> bool {
    derive_legacy_type1_proxy_address(signer) == candidate
}

fn derive_from_salt(salt: [u8; 32]) -> LegacyType1ProxyAddress {
    let mut hasher = Keccak256::new();
    hasher.update([0xff]);
    hasher.update(FACTORY);
    hasher.update(salt);
    hasher.update(INIT_CODE_KECCAK256);
    let digest = hasher.finalize();
    let mut address = [0_u8; 20];
    address.copy_from_slice(&digest[12..]);
    LegacyType1ProxyAddress::from_bytes(address)
}

#[cfg(test)]
mod tests {
    use super::{
        FACTORY, IMPLEMENTATION, INIT_CODE, INIT_CODE_KECCAK256, RUNTIME, RUNTIME_KECCAK256,
        derive_from_salt, derive_legacy_type1_proxy_address, legacy_type1_proxy_address_matches,
    };
    use crate::{EoaAddress, LegacyType1ProxyAddress};
    use sha3::{Digest as _, Keccak256};

    const DUMMY_SIGNER: &str = "0x0000000000000000000000000000000000000001";
    const DUMMY_SALT: [u8; 32] = [
        0x14, 0x68, 0x28, 0x80, 0x56, 0x31, 0x0c, 0x82, 0xaa, 0x4c, 0x01, 0xa7, 0xe1, 0x2a, 0x10,
        0xf8, 0x11, 0x1a, 0x05, 0x60, 0xe7, 0x2b, 0x70, 0x05, 0x55, 0x47, 0x90, 0x31, 0xb8, 0x6c,
        0x35, 0x7d,
    ];
    const DUMMY_PROXY: &str = "0x7754536ecd85c00b2E0CF9c1aA679340D8550756";

    #[test]
    fn exact_factory_init_and_runtime_templates_are_frozen() {
        assert_eq!(
            FACTORY,
            [
                0xab, 0x45, 0xc5, 0xa4, 0xb0, 0xc9, 0x41, 0xa2, 0xf2, 0x31, 0xc0, 0x4c, 0x3f, 0x49,
                0x18, 0x2e, 0x1a, 0x25, 0x40, 0x52,
            ]
        );
        assert_eq!(&INIT_CODE[13..33], FACTORY.as_slice());
        assert_eq!(&INIT_CODE[64..84], IMPLEMENTATION.as_slice());
        assert_eq!(&RUNTIME[10..30], IMPLEMENTATION.as_slice());
        assert_eq!(INIT_CODE.len(), 167);
        assert_eq!(RUNTIME.len(), 45);
        assert_eq!(
            <[u8; 32]>::from(Keccak256::digest(INIT_CODE)),
            INIT_CODE_KECCAK256
        );
        assert_eq!(
            <[u8; 32]>::from(Keccak256::digest(RUNTIME)),
            RUNTIME_KECCAK256
        );
    }

    #[test]
    fn dummy_signer_matches_the_independent_create2_vector() {
        let signer = EoaAddress::parse(DUMMY_SIGNER).unwrap();
        assert_eq!(
            <[u8; 32]>::from(Keccak256::digest(signer.bytes())),
            DUMMY_SALT
        );
        let proxy = derive_legacy_type1_proxy_address(signer);
        assert_eq!(proxy.to_string(), DUMMY_PROXY);
        assert!(legacy_type1_proxy_address_matches(
            signer,
            LegacyType1ProxyAddress::parse(DUMMY_PROXY).unwrap()
        ));
    }

    #[test]
    fn packed_twenty_byte_signer_is_not_a_padded_abi_address_word() {
        let signer = EoaAddress::parse(DUMMY_SIGNER).unwrap();
        let mut padded_address_word = [0_u8; 32];
        padded_address_word[12..].copy_from_slice(&signer.bytes());
        let padded_salt = <[u8; 32]>::from(Keccak256::digest(padded_address_word));
        assert_ne!(padded_salt, DUMMY_SALT);
        assert_ne!(derive_from_salt(padded_salt).to_string(), DUMMY_PROXY);
    }

    #[test]
    fn generic_hardhat_fixture_is_verified_as_not_the_official_relation() {
        let fixture_signer =
            EoaAddress::parse("0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266").unwrap();
        let fixture_maker =
            LegacyType1ProxyAddress::parse("0x70997970C51812dc3A010C7d01b50e0d17dc79C8").unwrap();
        let derived = derive_legacy_type1_proxy_address(fixture_signer);
        assert_eq!(
            derived.bytes(),
            [
                0x36, 0x5f, 0x0c, 0xa3, 0x6a, 0xe1, 0xf6, 0x41, 0xe0, 0x2f, 0xe3, 0xb7, 0x74, 0x36,
                0x73, 0xda, 0x42, 0xa1, 0x3a, 0x70,
            ]
        );
        assert!(!legacy_type1_proxy_address_matches(
            fixture_signer,
            fixture_maker
        ));
    }
}
