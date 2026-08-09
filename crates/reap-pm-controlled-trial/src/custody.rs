use std::{path::PathBuf, str};

use reap_polymarket_auth::{EoaPrivateKeyInput, FixedEoaSigner, L2CredentialInput, L2Credentials};
use serde::Serialize;
use thiserror::Error;
use zeroize::Zeroizing;

use crate::{
    CanonicalTrialConfig, OfflineAuthorizationState,
    protected_file::{ProtectedFileKind, read_four},
};

const PRIVATE_KEY_BYTES: usize = 66;
const API_KEY_BYTES: usize = 36;
const MAX_L2_SECRET_BYTES: usize = 172;
const MAX_PASSPHRASE_BYTES: usize = 128;

/// Exactly four operator-staged secret paths under one protected runtime dir.
pub struct CustodyPaths {
    pub private_key: PathBuf,
    pub api_key: PathBuf,
    pub l2_secret: PathBuf,
    pub passphrase: PathBuf,
}

/// Move-only custody result. Its secret authorities have no getters and are
/// zeroized by their owning authentication types on drop.
pub struct CustodyInspection {
    _signer: FixedEoaSigner,
    _l2: L2Credentials,
    summary: CustodySummary,
}

impl CustodyInspection {
    #[must_use]
    pub const fn summary(&self) -> &CustodySummary {
        &self.summary
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CustodySummary {
    pub schema_version: u32,
    pub signer: String,
    pub credential_slot_id: String,
    pub credential_slot_nonsecret_fingerprint_sha256: String,
    pub directory_mode_0700: bool,
    pub four_regular_single_link_mode_0600_files: bool,
    pub no_follow_descriptor_pinning_and_stable_reresolution: bool,
    pub private_key_derived_signer_matches_config: bool,
    pub l2_bundle_structurally_bound_to_signer: bool,
    pub remote_api_key_owner_attested: bool,
    pub secret_values_exposed: bool,
    #[serde(flatten)]
    pub authorization: OfflineAuthorizationState,
}

pub fn inspect_custody(
    config: &CanonicalTrialConfig,
    paths: CustodyPaths,
) -> Result<CustodyInspection, PmTrialCustodyError> {
    let mut values = read_four([
        (
            paths.private_key.as_path(),
            ProtectedFileKind::PrivateKey,
            PRIVATE_KEY_BYTES,
        ),
        (
            paths.api_key.as_path(),
            ProtectedFileKind::ApiKey,
            API_KEY_BYTES,
        ),
        (
            paths.l2_secret.as_path(),
            ProtectedFileKind::L2Secret,
            MAX_L2_SECRET_BYTES,
        ),
        (
            paths.passphrase.as_path(),
            ProtectedFileKind::Passphrase,
            MAX_PASSPHRASE_BYTES,
        ),
    ])
    .map_err(|_| PmTrialCustodyError::Rejected("protected file custody check failed"))?;

    let mut private_key = take_utf8(&mut values[0], "private key is not canonical UTF-8")?;
    let mut api_key = take_utf8(&mut values[1], "API key is not canonical UTF-8")?;
    let mut l2_secret = take_utf8(&mut values[2], "L2 secret is not canonical UTF-8")?;
    let mut passphrase = take_utf8(&mut values[3], "passphrase is not canonical UTF-8")?;

    let signer_address = &config.value().account.signer;
    let signer = FixedEoaSigner::bind(
        EoaPrivateKeyInput::new(std::mem::take(&mut *private_key)),
        signer_address,
    )
    .map_err(|_| {
        PmTrialCustodyError::Rejected("private key does not bind the configured signer")
    })?;
    let l2 = L2Credentials::bind(
        signer_address,
        L2CredentialInput::new(
            std::mem::take(&mut *api_key),
            std::mem::take(&mut *l2_secret),
            std::mem::take(&mut *passphrase),
        ),
    )
    .map_err(|_| PmTrialCustodyError::Rejected("L2 bundle grammar or signer binding is invalid"))?;
    if signer.address() != l2.address() {
        return Err(PmTrialCustodyError::Rejected(
            "private-key and L2 signer identities disagree",
        ));
    }

    let summary = CustodySummary {
        schema_version: config.value().schema_version,
        signer: signer.address().to_string(),
        credential_slot_id: config.value().credential_slot.slot_id.clone(),
        credential_slot_nonsecret_fingerprint_sha256: config
            .value()
            .credential_slot
            .nonsecret_fingerprint_sha256
            .clone(),
        directory_mode_0700: true,
        four_regular_single_link_mode_0600_files: true,
        no_follow_descriptor_pinning_and_stable_reresolution: true,
        private_key_derived_signer_matches_config: true,
        l2_bundle_structurally_bound_to_signer: true,
        remote_api_key_owner_attested: false,
        secret_values_exposed: false,
        authorization: OfflineAuthorizationState::DENIED,
    };
    Ok(CustodyInspection {
        _signer: signer,
        _l2: l2,
        summary,
    })
}

fn take_utf8(
    bytes: &mut Zeroizing<Vec<u8>>,
    message: &'static str,
) -> Result<Zeroizing<String>, PmTrialCustodyError> {
    if str::from_utf8(bytes.as_slice()).is_err() {
        return Err(PmTrialCustodyError::Rejected(message));
    }
    match String::from_utf8(std::mem::take(&mut **bytes)) {
        Ok(value) => Ok(Zeroizing::new(value)),
        Err(error) => {
            let _secret = Zeroizing::new(error.into_bytes());
            Err(PmTrialCustodyError::Rejected(message))
        }
    }
}

#[derive(Debug, Error)]
pub enum PmTrialCustodyError {
    #[error("custody inspection rejected: {0}; all four staged files remain unchanged")]
    Rejected(&'static str),
}
