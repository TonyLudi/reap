pub(crate) use reap_live_contracts::okx_account_identity_sha256;
#[cfg(test)]
pub(crate) use reap_telemetry::identity_sha256;
pub(crate) use reap_telemetry::sha256_bytes;
pub use reap_telemetry::{current_executable_sha256, host_identity_sha256};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TradingEnvironment;

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
