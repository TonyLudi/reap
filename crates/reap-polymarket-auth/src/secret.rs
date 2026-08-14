use std::fmt;

use base64::{Engine as _, engine::general_purpose::URL_SAFE};
use k256::ecdsa::SigningKey;
use reap_polymarket_wire::PmCredentialOwner;
use sha3::{Digest as _, Keccak256};
use zeroize::Zeroizing;

use crate::{EoaAddress, PmAuthError};

const MAX_L2_SECRET_DECODED_BYTES: usize = 128;
const MAX_L2_SECRET_ENCODED_BYTES: usize = 172;

/// Move-only, zeroizing injection value for one EOA private-key string.
pub struct EoaPrivateKeyInput(Zeroizing<String>);

impl EoaPrivateKeyInput {
    #[must_use]
    pub fn new(value: String) -> Self {
        Self(Zeroizing::new(value))
    }
}

impl fmt::Debug for EoaPrivateKeyInput {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("EoaPrivateKeyInput([REDACTED])")
    }
}

/// Move-only, zeroizing injection values for a pre-provisioned L2 bundle.
pub struct L2CredentialInput {
    api_key: Zeroizing<String>,
    secret: Zeroizing<String>,
    passphrase: Zeroizing<String>,
}

impl L2CredentialInput {
    #[must_use]
    pub fn new(api_key: String, secret: String, passphrase: String) -> Self {
        Self {
            api_key: Zeroizing::new(api_key),
            secret: Zeroizing::new(secret),
            passphrase: Zeroizing::new(passphrase),
        }
    }

    pub(crate) fn from_zeroizing(
        api_key: Zeroizing<String>,
        secret: Zeroizing<String>,
        passphrase: Zeroizing<String>,
    ) -> Self {
        Self {
            api_key,
            secret,
            passphrase,
        }
    }
}

impl fmt::Debug for L2CredentialInput {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("L2CredentialInput([REDACTED])")
    }
}

/// Move-only fixed-profile EOA signer.
pub struct FixedEoaSigner {
    key: Zeroizing<[u8; 32]>,
    address: EoaAddress,
}

impl FixedEoaSigner {
    /// Consume one private key, validate its exact encoding, and derive the
    /// only EOA identity controlled by it.
    ///
    /// This constructor is intended for credential sources whose signer
    /// address is not stored separately. It returns no key bytes or generic
    /// signing surface; the resulting authority remains limited to the fixed
    /// Polymarket operations implemented by this crate.
    pub fn derive(private_key: EoaPrivateKeyInput) -> Result<Self, PmAuthError> {
        let key = decode_private_key(private_key)?;
        let signing_key = SigningKey::from_slice(key.as_ref())
            .map_err(|_| PmAuthError::InvalidPrivateKeyScalar)?;
        let address = derive_address(&signing_key);
        Ok(Self { key, address })
    }

    pub fn bind(
        private_key: EoaPrivateKeyInput,
        configured_address: &str,
    ) -> Result<Self, PmAuthError> {
        let address = EoaAddress::parse(configured_address)?;
        let key = decode_private_key(private_key)?;
        let signing_key = SigningKey::from_slice(key.as_ref())
            .map_err(|_| PmAuthError::InvalidPrivateKeyScalar)?;
        let derived = derive_address(&signing_key);
        if derived != address {
            return Err(PmAuthError::PrivateKeyAddressMismatch);
        }

        Ok(Self { key, address })
    }

    #[must_use]
    pub const fn address(&self) -> EoaAddress {
        self.address
    }

    pub(crate) fn signing_key(&self) -> Result<SigningKey, PmAuthError> {
        SigningKey::from_slice(self.key.as_ref()).map_err(|_| PmAuthError::CryptographicFailure)
    }
}

fn decode_private_key(private_key: EoaPrivateKeyInput) -> Result<Zeroizing<[u8; 32]>, PmAuthError> {
    let input = private_key.0.as_bytes();
    if input.len() != 66
        || !input.starts_with(b"0x")
        || input[2..]
            .iter()
            .any(|byte| !byte.is_ascii_hexdigit() || byte.is_ascii_uppercase())
    {
        return Err(PmAuthError::InvalidPrivateKeyEncoding);
    }

    let mut key = Zeroizing::new([0_u8; 32]);
    for (index, output) in key.iter_mut().enumerate() {
        let high =
            lower_hex_nibble(input[index * 2 + 2]).ok_or(PmAuthError::InvalidPrivateKeyEncoding)?;
        let low =
            lower_hex_nibble(input[index * 2 + 3]).ok_or(PmAuthError::InvalidPrivateKeyEncoding)?;
        *output = (high << 4) | low;
    }
    Ok(key)
}

impl fmt::Debug for FixedEoaSigner {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("FixedEoaSigner([REDACTED])")
    }
}

/// Move-only, zeroizing, account-bound L2 credentials.
pub struct L2Credentials {
    address: EoaAddress,
    api_key: SecretText<36>,
    secret: Zeroizing<Vec<u8>>,
    passphrase: SecretText<128>,
}

impl L2Credentials {
    pub fn bind(configured_address: &str, input: L2CredentialInput) -> Result<Self, PmAuthError> {
        let address = EoaAddress::parse(configured_address)?;
        Self::bind_to_address(address, input)
    }

    pub(crate) fn bind_to_address(
        address: EoaAddress,
        input: L2CredentialInput,
    ) -> Result<Self, PmAuthError> {
        if !valid_api_key(input.api_key.as_bytes()) {
            return Err(PmAuthError::InvalidApiKey);
        }
        if !valid_passphrase(input.passphrase.as_bytes()) {
            return Err(PmAuthError::InvalidPassphrase);
        }
        if input.secret.is_empty() {
            return Err(PmAuthError::InvalidL2Secret);
        }
        if input.secret.len() > MAX_L2_SECRET_ENCODED_BYTES {
            return Err(PmAuthError::L2SecretTooLong);
        }

        let secret = Zeroizing::new(
            URL_SAFE
                .decode(input.secret.as_bytes())
                .map_err(|_| PmAuthError::InvalidL2Secret)?,
        );
        if secret.is_empty() || secret.len() > MAX_L2_SECRET_DECODED_BYTES {
            return Err(if secret.len() > MAX_L2_SECRET_DECODED_BYTES {
                PmAuthError::L2SecretTooLong
            } else {
                PmAuthError::InvalidL2Secret
            });
        }
        let canonical_secret = Zeroizing::new(URL_SAFE.encode(secret.as_slice()));
        if canonical_secret.as_bytes() != input.secret.as_bytes() {
            return Err(PmAuthError::InvalidL2Secret);
        }

        Ok(Self {
            address,
            api_key: SecretText::copy_from(input.api_key.as_bytes())
                .ok_or(PmAuthError::InvalidApiKey)?,
            secret,
            passphrase: SecretText::copy_from(input.passphrase.as_bytes())
                .ok_or(PmAuthError::InvalidPassphrase)?,
        })
    }

    #[must_use]
    pub const fn address(&self) -> EoaAddress {
        self.address
    }

    /// Compare opaque authenticated-response owner evidence with this exact
    /// L2 bundle without exposing the API key or a secret-derived hash.
    #[must_use]
    pub fn matches_credential_owner(&self, observed: &PmCredentialOwner) -> bool {
        observed.matches_exact(self.api_key())
    }

    pub(crate) fn api_key(&self) -> &str {
        self.api_key.as_str()
    }

    pub(crate) fn hmac_key(&self) -> &[u8] {
        self.secret.as_slice()
    }

    pub(crate) fn passphrase(&self) -> &str {
        self.passphrase.as_str()
    }

    /// Compare every credential component across a fixed local bound without
    /// exposing a component, its length, or a secret-derived projection.
    ///
    /// This is deliberately only constant-time-ish: every component always
    /// executes its full fixed-bound loop, but ordinary Rust indexing and the
    /// optimizer are not a formally verified constant-time implementation.
    pub(crate) fn matches_exact_l2_bundle(&self, candidate: &Self) -> bool {
        let api_key_matches = fixed_bound_eq(
            self.api_key.as_str().as_bytes(),
            candidate.api_key.as_str().as_bytes(),
            36,
        );
        let secret_matches = fixed_bound_eq(
            self.secret.as_slice(),
            candidate.secret.as_slice(),
            MAX_L2_SECRET_DECODED_BYTES,
        );
        let passphrase_matches = fixed_bound_eq(
            self.passphrase.as_str().as_bytes(),
            candidate.passphrase.as_str().as_bytes(),
            128,
        );
        api_key_matches & secret_matches & passphrase_matches
    }
}

impl fmt::Debug for L2Credentials {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("L2Credentials([REDACTED])")
    }
}

pub(crate) struct SecretText<const N: usize> {
    length: u8,
    bytes: Zeroizing<[u8; N]>,
}

impl<const N: usize> SecretText<N> {
    pub(crate) fn copy_from(value: &[u8]) -> Option<Self> {
        if value.is_empty() || value.len() > N || value.len() > usize::from(u8::MAX) {
            return None;
        }
        let mut bytes = Zeroizing::new([0_u8; N]);
        bytes[..value.len()].copy_from_slice(value);
        Some(Self {
            length: value.len() as u8,
            bytes,
        })
    }

    pub(crate) fn as_str(&self) -> &str {
        std::str::from_utf8(&self.bytes[..usize::from(self.length)])
            .expect("validated ASCII secret text")
    }
}

fn derive_address(signing_key: &SigningKey) -> EoaAddress {
    let encoded = signing_key.verifying_key().to_encoded_point(false);
    let digest = Keccak256::digest(&encoded.as_bytes()[1..]);
    let mut address = [0_u8; 20];
    address.copy_from_slice(&digest[12..]);
    EoaAddress::from_bytes(address)
}

fn valid_api_key(value: &[u8]) -> bool {
    value.len() == 36
        && value.iter().enumerate().all(|(index, byte)| {
            if matches!(index, 8 | 13 | 18 | 23) {
                *byte == b'-'
            } else {
                byte.is_ascii_digit() || (b'a'..=b'f').contains(byte)
            }
        })
}

fn valid_passphrase(value: &[u8]) -> bool {
    !value.is_empty() && value.len() <= 128 && value.iter().all(|byte| (0x21..=0x7e).contains(byte))
}

fn fixed_bound_eq(left: &[u8], right: &[u8], bound: usize) -> bool {
    let mut difference = left.len() ^ right.len();
    for index in 0..bound {
        let left_byte = left.get(index).copied().unwrap_or(0);
        let right_byte = right.get(index).copied().unwrap_or(0);
        difference |= usize::from(left_byte ^ right_byte);
    }
    difference == 0
}

fn lower_hex_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{EoaPrivateKeyInput, FixedEoaSigner, L2CredentialInput, L2Credentials};

    const KEY: &str = "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";
    const ADDRESS: &str = "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266";

    #[test]
    fn signer_requires_exact_key_address_binding() {
        assert!(FixedEoaSigner::bind(EoaPrivateKeyInput::new(KEY.into()), ADDRESS).is_ok());
        assert!(
            FixedEoaSigner::bind(EoaPrivateKeyInput::new(KEY.to_ascii_uppercase()), ADDRESS)
                .is_err()
        );
        assert!(
            FixedEoaSigner::bind(
                EoaPrivateKeyInput::new(KEY.into()),
                "0x1111111111111111111111111111111111111111"
            )
            .is_err()
        );
    }

    #[test]
    fn credential_grammars_are_closed_and_canonical() {
        let valid = || {
            L2CredentialInput::new(
                "00000000-0000-4000-8000-000000000001".into(),
                "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=".into(),
                "synthetic-passphrase".into(),
            )
        };
        assert!(L2Credentials::bind(ADDRESS, valid()).is_ok());
        assert!(
            L2Credentials::bind(
                ADDRESS,
                L2CredentialInput::new(
                    "00000000-0000-4000-8000-000000000001".into(),
                    "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=".into(),
                    "opaque+/=passphrase".into(),
                )
            )
            .is_ok()
        );
        assert!(
            L2Credentials::bind(
                ADDRESS,
                L2CredentialInput::new(
                    "00000000-0000-4000-8000-00000000000A".into(),
                    "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=".into(),
                    "synthetic-passphrase".into(),
                )
            )
            .is_err()
        );
        assert!(
            L2Credentials::bind(
                ADDRESS,
                L2CredentialInput::new(
                    "00000000-0000-4000-8000-000000000001".into(),
                    "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA".into(),
                    "synthetic-passphrase".into(),
                )
            )
            .is_err()
        );
        for invalid_passphrase in [
            "",
            "contains space",
            "line\nbreak",
            "tab\tvalue",
            "nonascii-é",
        ] {
            assert!(
                L2Credentials::bind(
                    ADDRESS,
                    L2CredentialInput::new(
                        "00000000-0000-4000-8000-000000000001".into(),
                        "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=".into(),
                        invalid_passphrase.into(),
                    )
                )
                .is_err(),
                "accepted invalid passphrase"
            );
        }
        assert!(
            L2Credentials::bind(
                ADDRESS,
                L2CredentialInput::new(
                    "00000000-0000-4000-8000-000000000001".into(),
                    "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=".into(),
                    "x".repeat(129),
                )
            )
            .is_err()
        );
    }
}
