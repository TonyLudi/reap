use thiserror::Error;

/// Redacted failures for the fixed PM-T1 authentication profile.
///
/// Variants intentionally carry no injected value, signature, body, header,
/// or secret-derived hash.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum PmAuthError {
    #[error("configured EOA address is not canonical EIP-55")]
    InvalidEoaAddress,
    #[error("legacy type-1 proxy address is not canonical EIP-55")]
    InvalidLegacyType1ProxyAddress,
    #[error("injected EOA private key is not canonical lowercase prefixed hex")]
    InvalidPrivateKeyEncoding,
    #[error("injected EOA private key is not a valid secp256k1 scalar")]
    InvalidPrivateKeyScalar,
    #[error("injected EOA private key does not match the configured EOA")]
    PrivateKeyAddressMismatch,
    #[error("L2 API key does not match the fixed lowercase UUID grammar")]
    InvalidApiKey,
    #[error("L2 HMAC secret is not nonempty canonical padded base64url")]
    InvalidL2Secret,
    #[error("decoded L2 HMAC secret exceeds the fixed local bound")]
    L2SecretTooLong,
    #[error("L2 passphrase is not nonempty bounded visible ASCII")]
    InvalidPassphrase,
    #[error("credential slot ID does not match the fixed non-secret local grammar")]
    InvalidCredentialSlotId,
    #[error("L2 timestamp must be a canonical ten-digit Unix-seconds value")]
    InvalidL2Timestamp,
    #[error("L1 credential-derivation timestamp must be a canonical ten-digit Unix-seconds value")]
    InvalidL1CredentialDerivationTimestamp,
    #[error("L1 credential-derivation response exceeds the fixed local bound")]
    L1CredentialDerivationResponseTooLong,
    #[error("L1 credential-derivation response is not the exact canonical credential object")]
    InvalidL1CredentialDerivationResponse,
    #[error("L1 credential-derivation response does not match the staged L2 credentials")]
    L1CredentialDerivationCredentialMismatch,
    #[error("signed order identity does not match the fixed EOA signer")]
    OrderIdentityMismatch,
    #[error("signed order identity does not match the L2 credential binding")]
    CredentialIdentityMismatch,
    #[error("venue order ID is not canonical lowercase prefixed bytes32 hex")]
    InvalidOrderId,
    #[error("fixed-profile cryptographic operation failed")]
    CryptographicFailure,
    #[error("fixed-profile JSON serialization failed")]
    SerializationFailure,
    #[error("fixed-profile request body exceeds its local bound")]
    RequestBodyTooLong,
    #[error("authenticated user order `owner` does not match the L2 credential owner")]
    UserOrderOwnerMismatch,
    #[error("authenticated user order `order_owner` does not match the L2 credential owner")]
    UserOrderOrderOwnerMismatch,
    #[error("authenticated user trade `owner` does not match the L2 credential owner")]
    UserTradeOwnerMismatch,
    #[error("authenticated user trade `trade_owner` does not match the L2 credential owner")]
    UserTradeTradeOwnerMismatch,
}
