use std::fs::File;
use std::io::Read;

use sha2::{Digest, Sha256};

use crate::config::TradingEnvironment;

pub(crate) fn sha256_bytes(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

pub(crate) fn current_executable_sha256() -> Result<String, String> {
    #[cfg(target_os = "linux")]
    let path = std::path::PathBuf::from("/proc/self/exe");
    #[cfg(not(target_os = "linux"))]
    let path = std::env::current_exe()
        .map_err(|error| format!("failed to resolve current executable: {error}"))?;

    let mut file = File::open(&path)
        .map_err(|error| format!("failed to open executable {}: {error}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|error| format!("failed to hash executable {}: {error}", path.display()))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

pub(crate) fn host_identity_sha256() -> Result<String, String> {
    let machine_id = std::fs::read("/etc/machine-id")
        .map_err(|error| format!("failed to read /etc/machine-id: {error}"))?;
    let machine_id = machine_id
        .strip_suffix(b"\n")
        .unwrap_or(machine_id.as_slice());
    if machine_id.is_empty() {
        return Err("/etc/machine-id is empty".to_string());
    }
    Ok(identity_sha256(b"reap-host-v1", &[machine_id]))
}

pub(crate) fn identity_sha256(domain: &[u8], fields: &[&[u8]]) -> String {
    let mut hasher = Sha256::new();
    hasher.update((domain.len() as u64).to_le_bytes());
    hasher.update(domain);
    for field in fields {
        hasher.update((field.len() as u64).to_le_bytes());
        hasher.update(field);
    }
    format!("{:x}", hasher.finalize())
}

pub(crate) fn okx_account_identity_sha256(
    environment: TradingEnvironment,
    account_id: &str,
    user_id: &str,
    main_user_id: &str,
) -> String {
    let environment = match environment {
        TradingEnvironment::Demo => b"demo".as_slice(),
        TradingEnvironment::Production => b"production".as_slice(),
    };
    identity_sha256(
        b"reap-okx-account-v1",
        &[
            environment,
            account_id.as_bytes(),
            user_id.trim().as_bytes(),
            main_user_id.trim().as_bytes(),
        ],
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn byte_and_identity_hashes_are_stable_and_field_delimited() {
        assert_eq!(
            sha256_bytes(b"reap"),
            "3995ddb81dee48b29fa3feea1a8331ac75c044bf155c8ba04054dbf0b424a617"
        );
        assert_eq!(
            identity_sha256(b"account", &[b"ab", b"c"]),
            identity_sha256(b"account", &[b"ab", b"c"])
        );
        assert_ne!(
            identity_sha256(b"account", &[b"ab", b"c"]),
            identity_sha256(b"account", &[b"a", b"bc"])
        );
        assert_ne!(
            okx_account_identity_sha256(TradingEnvironment::Demo, "main", "7", "6"),
            okx_account_identity_sha256(TradingEnvironment::Production, "main", "7", "6")
        );
    }
}
