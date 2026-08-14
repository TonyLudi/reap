//! Protected loader for the Predarb-style Polymarket credential file.
//!
//! The loader accepts only an owner-held regular `0600` file with one link,
//! opens it with `O_NOFOLLOW`, verifies the opened inode, bounds all input,
//! and admits only the exact legacy-proxy credential profile used by this
//! repository. It returns typed signer/L2 authorities, never raw strings.

#![forbid(unsafe_code)]

use std::{
    fs::{self, OpenOptions},
    io::Read as _,
    os::unix::fs::{MetadataExt as _, OpenOptionsExt as _},
    path::Path,
};

use reap_polymarket_auth::{
    EoaPrivateKeyInput, FixedEoaSigner, L2CredentialInput, L2Credentials, LegacyType1ProxyAddress,
    legacy_type1_proxy_address_matches,
};
use thiserror::Error;
use zeroize::Zeroizing;

const MAX_CREDENTIAL_FILE_BYTES: u64 = 64 * 1_024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum PmCredentialFileError {
    #[error("Polymarket credential file protection check failed")]
    Protection,
    #[error("Polymarket credential file could not be read")]
    Read,
    #[error("Polymarket credential environment is incomplete or malformed")]
    Environment,
    #[error("Polymarket credential profile is not legacy proxy signature type 1")]
    Profile,
    #[error("Polymarket signer, proxy, or L2 credential binding failed")]
    Binding,
}

/// Opaque, move-only typed authorities loaded from one protected file.
pub struct PmPredarbCredentialAuthorities {
    signer: FixedEoaSigner,
    l2: L2Credentials,
    funder: LegacyType1ProxyAddress,
}

impl PmPredarbCredentialAuthorities {
    #[must_use]
    pub fn into_parts(self) -> (FixedEoaSigner, L2Credentials, LegacyType1ProxyAddress) {
        (self.signer, self.l2, self.funder)
    }
}

impl std::fmt::Debug for PmPredarbCredentialAuthorities {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("PmPredarbCredentialAuthorities([REDACTED])")
    }
}

pub fn load_predarb_credential_file(
    path: &Path,
) -> Result<PmPredarbCredentialAuthorities, PmCredentialFileError> {
    let before = fs::symlink_metadata(path).map_err(|_| PmCredentialFileError::Protection)?;
    validate_metadata(&before)?;
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW)
        .open(path)
        .map_err(|_| PmCredentialFileError::Read)?;
    let opened = file.metadata().map_err(|_| PmCredentialFileError::Read)?;
    validate_metadata(&opened)?;
    if before.dev() != opened.dev() || before.ino() != opened.ino() {
        return Err(PmCredentialFileError::Protection);
    }
    let mut contents = Zeroizing::new(String::new());
    file.take(MAX_CREDENTIAL_FILE_BYTES + 1)
        .read_to_string(&mut contents)
        .map_err(|_| PmCredentialFileError::Read)?;
    if contents.len() as u64 > MAX_CREDENTIAL_FILE_BYTES {
        return Err(PmCredentialFileError::Read);
    }
    parse_environment(contents.as_str())
}

fn validate_metadata(metadata: &fs::Metadata) -> Result<(), PmCredentialFileError> {
    if !metadata.file_type().is_file()
        || metadata.uid() != rustix::process::geteuid().as_raw()
        || metadata.mode() & 0o777 != 0o600
        || metadata.nlink() != 1
        || metadata.len() > MAX_CREDENTIAL_FILE_BYTES
    {
        return Err(PmCredentialFileError::Protection);
    }
    Ok(())
}

fn parse_environment(
    contents: &str,
) -> Result<PmPredarbCredentialAuthorities, PmCredentialFileError> {
    let mut values: [Option<Zeroizing<String>>; 6] = std::array::from_fn(|_| None);
    for raw_line in contents.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let line = line.strip_prefix("export ").unwrap_or(line);
        let Some((name, value)) = line.split_once('=') else {
            continue;
        };
        let index = match name.trim() {
            "POLYMARKET_PRIVATE_KEY" => 0,
            "POLYMARKET_FUNDER" => 1,
            "POLYMARKET_SIGNATURE_TYPE" => 2,
            "POLYMARKET_API_KEY" => 3,
            "POLYMARKET_API_SECRET" => 4,
            "POLYMARKET_API_PASSPHRASE" => 5,
            _ => continue,
        };
        if values[index].is_some() {
            return Err(PmCredentialFileError::Environment);
        }
        values[index] = Some(Zeroizing::new(parse_value(value)?));
    }
    let [
        private_key,
        funder,
        signature_type,
        api_key,
        secret,
        passphrase,
    ] = values;
    let mut private_key = private_key.ok_or(PmCredentialFileError::Environment)?;
    let funder = funder.ok_or(PmCredentialFileError::Environment)?;
    let signature_type = signature_type.ok_or(PmCredentialFileError::Environment)?;
    let mut api_key = api_key.ok_or(PmCredentialFileError::Environment)?;
    let mut secret = secret.ok_or(PmCredentialFileError::Environment)?;
    let mut passphrase = passphrase.ok_or(PmCredentialFileError::Environment)?;
    if signature_type.as_str() != "1" {
        return Err(PmCredentialFileError::Profile);
    }

    let signer = FixedEoaSigner::derive(EoaPrivateKeyInput::new(std::mem::take(&mut *private_key)))
        .map_err(|_| PmCredentialFileError::Binding)?;
    let signer_address = signer.address();
    let funder = LegacyType1ProxyAddress::parse(funder.as_str())
        .map_err(|_| PmCredentialFileError::Binding)?;
    if !legacy_type1_proxy_address_matches(signer_address, funder) {
        return Err(PmCredentialFileError::Binding);
    }
    let l2 = L2Credentials::bind(
        &signer_address.to_string(),
        L2CredentialInput::new(
            std::mem::take(&mut *api_key),
            std::mem::take(&mut *secret),
            std::mem::take(&mut *passphrase),
        ),
    )
    .map_err(|_| PmCredentialFileError::Binding)?;
    Ok(PmPredarbCredentialAuthorities { signer, l2, funder })
}

fn parse_value(value: &str) -> Result<String, PmCredentialFileError> {
    let value = value.trim();
    if value.is_empty() || value.as_bytes().contains(&0) {
        return Err(PmCredentialFileError::Environment);
    }
    if let Some(inner) = value
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
    {
        if inner.is_empty() || inner.contains(['"', '\\', '\n', '\r']) {
            return Err(PmCredentialFileError::Environment);
        }
        return Ok(inner.to_owned());
    }
    if let Some(inner) = value
        .strip_prefix('\'')
        .and_then(|value| value.strip_suffix('\''))
    {
        if inner.is_empty() || inner.contains('\'') {
            return Err(PmCredentialFileError::Environment);
        }
        return Ok(inner.to_owned());
    }
    if value.bytes().any(|byte| byte.is_ascii_whitespace()) {
        return Err(PmCredentialFileError::Environment);
    }
    Ok(value.to_owned())
}

#[cfg(test)]
mod tests {
    use std::{io::Write as _, os::unix::fs::PermissionsExt as _};

    use super::*;

    const KEY: &str = "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";

    fn valid_text() -> String {
        let signer = FixedEoaSigner::derive(EoaPrivateKeyInput::new(KEY.to_owned())).unwrap();
        let funder = reap_polymarket_auth::derive_legacy_type1_proxy_address(signer.address());
        format!(
            "POLYMARKET_PRIVATE_KEY={KEY}\nPOLYMARKET_FUNDER={funder}\nPOLYMARKET_SIGNATURE_TYPE=1\nPOLYMARKET_API_KEY=00000000-0000-4000-8000-000000000001\nPOLYMARKET_API_SECRET=AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=\nPOLYMARKET_API_PASSPHRASE=synthetic-passphrase\n"
        )
    }

    fn target_tempdir() -> tempfile::TempDir {
        let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/tmp");
        fs::create_dir_all(&base).unwrap();
        tempfile::Builder::new()
            .prefix("pm-credential-file-")
            .tempdir_in(base)
            .unwrap()
    }

    #[test]
    fn loads_only_owner_held_exact_profile_and_redacts_debug() {
        let directory = target_tempdir();
        let path = directory.path().join("credentials.env");
        let mut file = fs::File::create(&path).unwrap();
        file.write_all(valid_text().as_bytes()).unwrap();
        file.sync_all().unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        let loaded = load_predarb_credential_file(&path).unwrap();
        assert_eq!(
            format!("{loaded:?}"),
            "PmPredarbCredentialAuthorities([REDACTED])"
        );
    }

    #[test]
    fn rejects_group_readable_and_duplicate_credentials() {
        let directory = target_tempdir();
        let path = directory.path().join("credentials.env");
        fs::write(&path, valid_text()).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
        assert_eq!(
            load_predarb_credential_file(&path).unwrap_err(),
            PmCredentialFileError::Protection
        );
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        let duplicate = format!("{}POLYMARKET_API_KEY=duplicate\n", valid_text());
        fs::write(&path, duplicate).unwrap();
        assert_eq!(
            load_predarb_credential_file(&path).unwrap_err(),
            PmCredentialFileError::Environment
        );
    }
}
