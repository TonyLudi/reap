use serde::Serialize;
use sha2::{Digest as _, Sha256};

use crate::PmTrialLiveJournalError;

pub(crate) const ZERO_FINGERPRINT: &str =
    "0000000000000000000000000000000000000000000000000000000000000000";

pub(crate) fn canonical_json(value: &impl Serialize) -> Result<Vec<u8>, PmTrialLiveJournalError> {
    serde_json::to_vec(value).map_err(|_| PmTrialLiveJournalError::InvalidRecord)
}

pub(crate) fn hash_domain(
    domain: &[u8],
    value: &impl Serialize,
) -> Result<String, PmTrialLiveJournalError> {
    let bytes = canonical_json(value)?;
    Ok(hash_bytes(domain, &bytes))
}

pub(crate) fn hash_bytes(domain: &[u8], bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(domain);
    hasher.update(bytes);
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

pub(crate) fn validate_fingerprint(value: &str) -> Result<(), PmTrialLiveJournalError> {
    if value.len() != 64
        || value
            .bytes()
            .any(|byte| !byte.is_ascii_digit() && !(b'a'..=b'f').contains(&byte))
    {
        return Err(PmTrialLiveJournalError::InvalidBinding);
    }
    Ok(())
}
