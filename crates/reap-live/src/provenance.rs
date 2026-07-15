pub use reap_telemetry::{current_executable_sha256, host_identity_sha256};
pub(crate) use reap_telemetry::{identity_sha256, sha256_bytes};

use crate::config::TradingEnvironment;

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
