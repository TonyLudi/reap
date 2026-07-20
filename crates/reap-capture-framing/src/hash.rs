use sha2::{Digest, Sha256};

/// Returns the lowercase SHA-256 digest of an immutable byte slice.
pub fn sha256_hex(bytes: &[u8]) -> String {
    digest_hex(Sha256::digest(bytes))
}

/// Encodes already-computed digest bytes as lowercase hexadecimal.
pub fn digest_hex(bytes: impl AsRef<[u8]>) -> String {
    bytes
        .as_ref()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
