//! One-purpose L1 ClobAuth request construction for a separate credential-
//! provisioning plane.
//!
//! The only operation represented here is exact `GET /auth/derive-api-key`
//! with no query, body, or content type and exactly the four L1 headers
//! `POLY_ADDRESS`, `POLY_SIGNATURE`, `POLY_TIMESTAMP`, and `POLY_NONCE`.
//! This helper only computes an EIP-712 signature for the supplied typed
//! values. The deterministic signature does not itself prove when it was
//! constructed and may be retained or replayed. It does not prove that the
//! timestamp came from the CLOB server or is fresh, that any server call or
//! response occurred, that a returned credential tuple matches, that L2
//! authentication will be accepted or is current or unrevoked, that a
//! provider delivered credentials, that a signer maps to or controls a proxy,
//! or that any mutation is authorized.
//!
//! This module has no network, response parser, credential holder, API-key
//! create/list/delete operation, L2 authentication, generic signing, reusable
//! header projection, request body, or mutation capability.
//! [`L1CredentialDerivationRequestSink`] is a trusted external transport
//! boundary: an implementation can retain or replay the signature, ignore the
//! semantic operation, route elsewhere, or perform network I/O. Consumption
//! is local only to this signer/request holder. It does not enforce sink
//! compliance, a host or destination, TLS, confidentiality, non-retention,
//! currentness, or global no-replay.

use std::fmt;

use sha3::{Digest as _, Keccak256};
use zeroize::Zeroizing;

use crate::{EoaAddress, FixedEoaSigner, PmAuthError};

const MIN_CANONICAL_TIMESTAMP: u64 = 1_000_000_000;
const MAX_CANONICAL_TIMESTAMP: u64 = 9_999_999_999;
const CLOB_AUTH_CHAIN_ID: u64 = 137;
const CLOB_AUTH_DOMAIN_NAME: &str = "ClobAuthDomain";
const CLOB_AUTH_DOMAIN_VERSION: &str = "1";
const CLOB_AUTH_DOMAIN_TYPE: &str = "EIP712Domain(string name,string version,uint256 chainId)";
const CLOB_AUTH_STRUCT_TYPE: &str =
    "ClobAuth(address address,string timestamp,uint256 nonce,string message)";
const CLOB_AUTH_MESSAGE: &str = "This message attests that I control the given wallet";

/// A canonical ten-digit Unix-seconds value supplied to the separate L1
/// credential-provisioning plane.
///
/// Construction validates syntax and range only. It does not establish that
/// the value came from the CLOB server, is current, or is fresh. A separate
/// provider plane must prove those properties before using this type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct L1CredentialDerivationTimestamp(u64);

impl L1CredentialDerivationTimestamp {
    pub fn from_unix_seconds(value: u64) -> Result<Self, PmAuthError> {
        if !(MIN_CANONICAL_TIMESTAMP..=MAX_CANONICAL_TIMESTAMP).contains(&value) {
            return Err(PmAuthError::InvalidL1CredentialDerivationTimestamp);
        }
        Ok(Self(value))
    }

    #[must_use]
    pub const fn unix_seconds(self) -> u64 {
        self.0
    }
}

impl fmt::Display for L1CredentialDerivationTimestamp {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.0)
    }
}

/// A deliberately narrow canonical subset of the official EIP-712 `uint256`
/// nonce field.
///
/// The SDK contract admits a number and defaults it to zero. This type admits
/// only `u64` values, ABI-encodes them as `uint256`, and emits their minimal
/// unsigned decimal spelling in `POLY_NONCE`. It does not claim that a nonce
/// is unused, current, server-accepted, or globally replay-proof.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct L1CredentialDerivationNonce(u64);

impl L1CredentialDerivationNonce {
    #[must_use]
    pub const fn from_u64(value: u64) -> Self {
        Self(value)
    }

    #[must_use]
    pub const fn value(self) -> u64 {
        self.0
    }
}

impl fmt::Display for L1CredentialDerivationNonce {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.0)
    }
}

/// Consume-only authenticated headers for exact `GET /auth/derive-api-key`.
///
/// The value has no decomposition or reusable header API. Its sole public
/// operation consumes it into [`L1CredentialDerivationRequestSink`].
pub struct AuthenticatedL1CredentialDerivationRequest {
    address: EoaAddress,
    timestamp: L1CredentialDerivationTimestamp,
    nonce: L1CredentialDerivationNonce,
    signature: Zeroizing<[u8; 65]>,
}

impl AuthenticatedL1CredentialDerivationRequest {
    /// Consume this local request holder at the trusted provisioning edge.
    ///
    /// The caller supplies no method, path, body, query, content type, or
    /// generic headers. The sink remains trusted and can retain or replay the
    /// values or disregard the semantic operation.
    pub fn dispatch<S: L1CredentialDerivationRequestSink>(
        self,
        sink: &mut S,
    ) -> Result<S::Output, S::Error> {
        let address = self.address.to_string();
        let timestamp = self.timestamp.to_string();
        let nonce = self.nonce.to_string();
        let mut signature = Zeroizing::new(String::with_capacity(132));
        signature.push_str("0x");
        push_lower_hex(&mut signature, self.signature.as_slice());
        sink.send_exact_get_auth_derive_api_key(&address, signature.as_str(), &timestamp, &nonce)
    }
}

impl fmt::Debug for AuthenticatedL1CredentialDerivationRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(
            "AuthenticatedL1CredentialDerivationRequest([REDACTED; SIGNATURE_CONSTRUCTION_ONLY; NO_SERVER_PROOF_OR_AUTHORITY])",
        )
    }
}

/// Trusted transport boundary for the semantic operation exact
/// `GET /auth/derive-api-key` with no caller-supplied query, body, or content
/// type.
///
/// These four arguments are, in order, `POLY_ADDRESS`, `POLY_SIGNATURE`,
/// `POLY_TIMESTAMP`, and `POLY_NONCE`. There is deliberately no generic
/// caller method, path, header, body, or query parameter. Implementations can
/// nevertheless retain or replay these values, use a different route, perform
/// network I/O, and return any `Output`; this trait does not attest transport
/// compliance or response semantics.
pub trait L1CredentialDerivationRequestSink {
    type Output;
    type Error;

    fn send_exact_get_auth_derive_api_key(
        &mut self,
        poly_address: &str,
        poly_signature: &str,
        poly_timestamp: &str,
        poly_nonce: &str,
    ) -> Result<Self::Output, Self::Error>;
}

impl FixedEoaSigner {
    /// Consume this signer into one sealed L1 credential-derivation request.
    ///
    /// Consuming the signer enforces one construction through this particular
    /// holder. It does not prove global no-replay because the same key could
    /// exist in another holder or process. The result proves neither a server
    /// call nor credential validity and carries no mutation authority.
    pub fn consume_into_l1_credential_derivation_request(
        self,
        timestamp: L1CredentialDerivationTimestamp,
        nonce: L1CredentialDerivationNonce,
    ) -> Result<AuthenticatedL1CredentialDerivationRequest, PmAuthError> {
        let address = self.address();
        let digest = clob_auth_digest(address, timestamp, nonce);
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
        Ok(AuthenticatedL1CredentialDerivationRequest {
            address,
            timestamp,
            nonce,
            signature: encoded_signature,
        })
    }
}

fn clob_auth_domain_separator() -> [u8; 32] {
    let mut hasher = Keccak256::new();
    hasher.update(Keccak256::digest(CLOB_AUTH_DOMAIN_TYPE.as_bytes()));
    hasher.update(Keccak256::digest(CLOB_AUTH_DOMAIN_NAME.as_bytes()));
    hasher.update(Keccak256::digest(CLOB_AUTH_DOMAIN_VERSION.as_bytes()));
    hasher.update(uint256_word(CLOB_AUTH_CHAIN_ID));
    hasher.finalize().into()
}

fn clob_auth_struct_hash(
    address: EoaAddress,
    timestamp: L1CredentialDerivationTimestamp,
    nonce: L1CredentialDerivationNonce,
) -> [u8; 32] {
    let timestamp = timestamp.to_string();
    let mut hasher = Keccak256::new();
    hasher.update(Keccak256::digest(CLOB_AUTH_STRUCT_TYPE.as_bytes()));
    hasher.update(address_word(address.bytes()));
    hasher.update(Keccak256::digest(timestamp.as_bytes()));
    hasher.update(uint256_word(nonce.value()));
    hasher.update(Keccak256::digest(CLOB_AUTH_MESSAGE.as_bytes()));
    hasher.finalize().into()
}

fn clob_auth_digest(
    address: EoaAddress,
    timestamp: L1CredentialDerivationTimestamp,
    nonce: L1CredentialDerivationNonce,
) -> [u8; 32] {
    let mut hasher = Keccak256::new();
    hasher.update([0x19, 0x01]);
    hasher.update(clob_auth_domain_separator());
    hasher.update(clob_auth_struct_hash(address, timestamp, nonce));
    hasher.finalize().into()
}

fn address_word(address: [u8; 20]) -> [u8; 32] {
    let mut word = [0_u8; 32];
    word[12..].copy_from_slice(&address);
    word
}

fn uint256_word(value: u64) -> [u8; 32] {
    let mut word = [0_u8; 32];
    word[24..].copy_from_slice(&value.to_be_bytes());
    word
}

fn push_lower_hex(output: &mut String, bytes: &[u8]) {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    for &byte in bytes {
        output.push(HEX[usize::from(byte >> 4)] as char);
        output.push(HEX[usize::from(byte & 0x0f)] as char);
    }
}

#[cfg(test)]
mod tests {
    use super::{
        CLOB_AUTH_CHAIN_ID, CLOB_AUTH_DOMAIN_NAME, CLOB_AUTH_DOMAIN_TYPE, CLOB_AUTH_DOMAIN_VERSION,
        CLOB_AUTH_MESSAGE, CLOB_AUTH_STRUCT_TYPE, L1CredentialDerivationNonce,
        L1CredentialDerivationTimestamp, clob_auth_digest, clob_auth_domain_separator,
        clob_auth_struct_hash,
    };
    use crate::{EoaAddress, EoaPrivateKeyInput, FixedEoaSigner};
    use k256::ecdsa::{RecoveryId, Signature, VerifyingKey};
    use sha3::{Digest as _, Keccak256};

    const KEY: &str = "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";
    const ADDRESS: &str = "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266";
    const TIMESTAMP: u64 = 1_780_449_126;
    const NONCE: u64 = 7;

    fn independent_address_word(address: EoaAddress) -> [u8; 32] {
        let mut word = [0_u8; 32];
        word[12..].copy_from_slice(&address.bytes());
        word
    }

    fn independent_uint256_word(value: u64) -> [u8; 32] {
        let mut word = [0_u8; 32];
        word[24..].copy_from_slice(&value.to_be_bytes());
        word
    }

    fn independent_digest(
        domain_name: &str,
        domain_version: &str,
        chain_id: u64,
        address: EoaAddress,
        timestamp: &str,
        nonce: u64,
        message: &str,
    ) -> [u8; 32] {
        let mut domain = Keccak256::new();
        domain.update(Keccak256::digest(
            b"EIP712Domain(string name,string version,uint256 chainId)",
        ));
        domain.update(Keccak256::digest(domain_name.as_bytes()));
        domain.update(Keccak256::digest(domain_version.as_bytes()));
        domain.update(independent_uint256_word(chain_id));

        let mut structure = Keccak256::new();
        structure.update(Keccak256::digest(
            b"ClobAuth(address address,string timestamp,uint256 nonce,string message)",
        ));
        structure.update(independent_address_word(address));
        structure.update(Keccak256::digest(timestamp.as_bytes()));
        structure.update(independent_uint256_word(nonce));
        structure.update(Keccak256::digest(message.as_bytes()));

        let mut digest = Keccak256::new();
        digest.update([0x19, 0x01]);
        digest.update(domain.finalize());
        digest.update(structure.finalize());
        digest.finalize().into()
    }

    #[test]
    fn exact_eip712_profile_and_digest_are_independently_recomputed() {
        assert_eq!(CLOB_AUTH_CHAIN_ID, 137);
        assert_eq!(CLOB_AUTH_DOMAIN_NAME, "ClobAuthDomain");
        assert_eq!(CLOB_AUTH_DOMAIN_VERSION, "1");
        assert_eq!(
            CLOB_AUTH_DOMAIN_TYPE,
            "EIP712Domain(string name,string version,uint256 chainId)"
        );
        assert_eq!(
            CLOB_AUTH_STRUCT_TYPE,
            "ClobAuth(address address,string timestamp,uint256 nonce,string message)"
        );
        assert_eq!(
            CLOB_AUTH_MESSAGE,
            "This message attests that I control the given wallet"
        );

        let address = EoaAddress::parse(ADDRESS).unwrap();
        let timestamp = L1CredentialDerivationTimestamp::from_unix_seconds(TIMESTAMP).unwrap();
        let nonce = L1CredentialDerivationNonce::from_u64(NONCE);
        let independent = independent_digest(
            "ClobAuthDomain",
            "1",
            137,
            address,
            "1780449126",
            7,
            "This message attests that I control the given wallet",
        );
        assert_eq!(clob_auth_digest(address, timestamp, nonce), independent);
        assert_eq!(
            crate::identity::lower_hex(&Keccak256::digest(CLOB_AUTH_STRUCT_TYPE.as_bytes())),
            "52578c5c725a28a84fedc8c22aa47947822942f35b4dc350db028e45320e035c"
        );
        assert_eq!(
            crate::identity::lower_hex(&clob_auth_domain_separator()),
            "cfc66be2a3b30464cb3b588324101f660c9a205fa76e8e5f83ee16a528e1c4cb"
        );
        assert_eq!(
            crate::identity::lower_hex(&clob_auth_struct_hash(address, timestamp, nonce)),
            "b52d9dd83bee73f739543fa9fdead782d7bc6a633de263d178d77c402b1ed56e"
        );
        assert_eq!(
            crate::identity::lower_hex(&independent),
            "7a7a27b49f9dac2545e073b55236e1d70cdfacc99dc09739cde106ab41310c50"
        );
    }

    #[test]
    fn wrong_timestamp_nonce_domain_and_message_change_the_digest() {
        let address = EoaAddress::parse(ADDRESS).unwrap();
        let exact = independent_digest(
            "ClobAuthDomain",
            "1",
            137,
            address,
            "1780449126",
            7,
            "This message attests that I control the given wallet",
        );
        for changed in [
            independent_digest(
                "ClobAuthDomain",
                "1",
                137,
                address,
                "1780449127",
                7,
                "This message attests that I control the given wallet",
            ),
            independent_digest(
                "ClobAuthDomain",
                "1",
                137,
                address,
                "1780449126",
                8,
                "This message attests that I control the given wallet",
            ),
            independent_digest(
                "OtherDomain",
                "1",
                137,
                address,
                "1780449126",
                7,
                "This message attests that I control the given wallet",
            ),
            independent_digest(
                "ClobAuthDomain",
                "2",
                137,
                address,
                "1780449126",
                7,
                "This message attests that I control the given wallet",
            ),
            independent_digest(
                "ClobAuthDomain",
                "1",
                138,
                address,
                "1780449126",
                7,
                "This message attests that I control the given wallet",
            ),
            independent_digest(
                "ClobAuthDomain",
                "1",
                137,
                address,
                "1780449126",
                7,
                "This message attests to something else",
            ),
        ] {
            assert_ne!(changed, exact);
        }
    }

    #[test]
    fn signature_is_low_s_recoverable_and_matches_the_fixed_vector() {
        let address = EoaAddress::parse(ADDRESS).unwrap();
        let timestamp = L1CredentialDerivationTimestamp::from_unix_seconds(TIMESTAMP).unwrap();
        let nonce = L1CredentialDerivationNonce::from_u64(NONCE);
        let digest = independent_digest(
            "ClobAuthDomain",
            "1",
            137,
            address,
            "1780449126",
            7,
            "This message attests that I control the given wallet",
        );
        let request = FixedEoaSigner::bind(EoaPrivateKeyInput::new(KEY.into()), ADDRESS)
            .unwrap()
            .consume_into_l1_credential_derivation_request(timestamp, nonce)
            .unwrap();
        assert_eq!(
            crate::identity::lower_hex(request.signature.as_slice()),
            "9670627f2da09dc111b4044b1259b7c510188a87655ec2857b135ed5d7c6517c1030e5d4af93c70eaa24836d185cdd7f8befb2054d875878067921248010593b1b"
        );

        let signature = Signature::try_from(&request.signature[..64]).unwrap();
        assert!(signature.normalize_s().is_none());
        let recovery = RecoveryId::from_byte(request.signature[64] - 27).unwrap();
        let recovered = VerifyingKey::recover_from_prehash(&digest, &signature, recovery).unwrap();
        let expected = FixedEoaSigner::bind(EoaPrivateKeyInput::new(KEY.into()), ADDRESS)
            .unwrap()
            .signing_key()
            .unwrap();
        assert_eq!(recovered, *expected.verifying_key());
    }
}
