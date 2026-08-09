use std::{fmt, fmt::Write as _, time::Duration};

use reap_pm_core::EvmAddress;
use reap_polymarket_auth::{L2CredentialInput, L2Credentials};
use reap_polymarket_wire::PmWireScope;
use sha3::{Digest as _, Keccak256};
use zeroize::Zeroizing;

use crate::{
    PmAuthenticatedHttpOwner, PmAuthenticatedUserWsRole, PmCredentialAuthoritySupervisor,
    PmLiveAdapterError, PmPrivateConnectivityOwner, PmPrivateHttpConfig, PmUserWsBounds,
    PmUserWsConfig,
};

/// Move-only, zeroizing injection value for production read-only credentials.
///
/// This adapter-owned type prevents the general authentication crate's
/// credential and signing types from entering an operator composition API.
pub struct PmReadOnlyCredentialInput {
    api_key: Zeroizing<String>,
    secret: Zeroizing<String>,
    passphrase: Zeroizing<String>,
}

impl PmReadOnlyCredentialInput {
    #[must_use]
    pub fn new(api_key: String, secret: String, passphrase: String) -> Self {
        Self {
            api_key: Zeroizing::new(api_key),
            secret: Zeroizing::new(secret),
            passphrase: Zeroizing::new(passphrase),
        }
    }

    fn into_auth_input(mut self) -> L2CredentialInput {
        L2CredentialInput::new(
            std::mem::take(&mut *self.api_key),
            std::mem::take(&mut *self.secret),
            std::mem::take(&mut *self.passphrase),
        )
    }
}

impl fmt::Debug for PmReadOnlyCredentialInput {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PmReadOnlyCredentialInput([REDACTED])")
    }
}

/// Sole production-safe constructor for one account's authenticated read
/// connectivity.
///
/// The owner binds credentials to the exact configured signer/funder before
/// any authority task is started. It exposes no general credential, signing,
/// mutation, server-time, or validator capability.
pub struct PmReadOnlyPrivateConnectivityOwner {
    inner: PmPrivateConnectivityOwner,
    exact_scope: PmWireScope,
}

impl PmReadOnlyPrivateConnectivityOwner {
    #[allow(clippy::too_many_arguments)]
    pub fn production(
        expected_signer: EvmAddress,
        expected_funder: EvmAddress,
        exact_scope: PmWireScope,
        http_connect_timeout: Duration,
        http_request_timeout: Duration,
        user_ws_bounds: PmUserWsBounds,
        credentials: PmReadOnlyCredentialInput,
    ) -> Result<Self, PmLiveAdapterError> {
        validate_one_eoa(expected_signer, expected_funder)?;
        let http_config = PmPrivateHttpConfig::production(
            http_connect_timeout,
            http_request_timeout,
            exact_scope,
        )?;
        let user_ws_config = PmUserWsConfig::production(exact_scope.condition(), user_ws_bounds)?;
        Self::bind(
            expected_signer,
            exact_scope,
            http_config,
            user_ws_config,
            credentials,
        )
    }

    /// Construct the same read-only facade over literal-loopback transports.
    ///
    /// Callers must build both configurations with their corresponding
    /// `read_only_evidence` constructors. This API is deliberately unrelated
    /// to the separate mutation-loopback feature.
    #[cfg(any(test, feature = "read-only-evidence"))]
    pub fn read_only_evidence(
        expected_signer: EvmAddress,
        expected_funder: EvmAddress,
        exact_scope: PmWireScope,
        http_config: PmPrivateHttpConfig,
        user_ws_config: PmUserWsConfig,
        credentials: PmReadOnlyCredentialInput,
    ) -> Result<Self, PmLiveAdapterError> {
        use crate::config::OriginMode;

        validate_one_eoa(expected_signer, expected_funder)?;
        if http_config.mode() != OriginMode::LocalEvidence || user_ws_config.is_production() {
            return Err(PmLiveAdapterError::InvalidConfiguration(
                "read-only evidence facade requires literal-loopback transports",
            ));
        }
        Self::bind(
            expected_signer,
            exact_scope,
            http_config,
            user_ws_config,
            credentials,
        )
    }

    fn bind(
        expected_eoa: EvmAddress,
        exact_scope: PmWireScope,
        http_config: PmPrivateHttpConfig,
        user_ws_config: PmUserWsConfig,
        credentials: PmReadOnlyCredentialInput,
    ) -> Result<Self, PmLiveAdapterError> {
        if http_config.exact_order_scope() != exact_scope
            || user_ws_config.condition() != exact_scope.condition()
        {
            return Err(PmLiveAdapterError::InvalidConfiguration(
                "read-only private transports must bind the exact wire scope",
            ));
        }

        let expected_eip55 = eip55(expected_eoa);
        let credentials = L2Credentials::bind(&expected_eip55, credentials.into_auth_input())?;
        if credentials.address().as_core() != expected_eoa {
            return Err(PmLiveAdapterError::CredentialOwnerMismatch);
        }
        let inner = PmPrivateConnectivityOwner::new(http_config, user_ws_config, credentials)?;
        Ok(Self { inner, exact_scope })
    }

    #[must_use]
    pub const fn configured_scope(&self) -> PmWireScope {
        self.exact_scope
    }

    #[must_use]
    pub const fn production_order_entry_authorized(&self) -> bool {
        false
    }

    /// Consume the sole owner into exactly the three authenticated read
    /// capabilities required by an operator.
    pub fn split(self) -> Result<PmReadOnlyPrivateConnectivityRoles, PmLiveAdapterError> {
        let (authenticated_http, authenticated_user_ws, credential_supervisor) =
            self.inner.split()?.into_read_roles();
        Ok(PmReadOnlyPrivateConnectivityRoles {
            authenticated_http,
            authenticated_user_ws,
            credential_supervisor,
        })
    }
}

impl fmt::Debug for PmReadOnlyPrivateConnectivityOwner {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PmReadOnlyPrivateConnectivityOwner")
            .field("exact_scope", &self.exact_scope)
            .field("credentials", &"[REDACTED]")
            .finish()
    }
}

/// Named, move-only result of splitting the read-only credential owner.
///
/// These are the only three capabilities released by the facade. In
/// particular, this bundle contains no mutation authentication role, signer,
/// general L2 credential, mutation-time proof, or mutation validator.
pub struct PmReadOnlyPrivateConnectivityRoles {
    pub authenticated_http: PmAuthenticatedHttpOwner,
    pub authenticated_user_ws: PmAuthenticatedUserWsRole,
    pub credential_supervisor: PmCredentialAuthoritySupervisor,
}

impl PmReadOnlyPrivateConnectivityRoles {
    #[must_use]
    pub const fn production_order_entry_authorized(&self) -> bool {
        false
    }
}

impl fmt::Debug for PmReadOnlyPrivateConnectivityRoles {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PmReadOnlyPrivateConnectivityRoles([REDACTED])")
    }
}

fn validate_one_eoa(
    expected_signer: EvmAddress,
    expected_funder: EvmAddress,
) -> Result<(), PmLiveAdapterError> {
    if expected_signer != expected_funder {
        return Err(PmLiveAdapterError::InvalidConfiguration(
            "read-only credentials require signer and funder to be the same EOA",
        ));
    }
    Ok(())
}

fn eip55(address: EvmAddress) -> String {
    let mut lower = String::with_capacity(40);
    for byte in address.bytes() {
        write!(&mut lower, "{byte:02x}").expect("writing an address to String cannot fail");
    }
    let digest = Keccak256::digest(lower.as_bytes());
    let mut checksum = String::with_capacity(42);
    checksum.push_str("0x");
    for (index, byte) in lower.bytes().enumerate() {
        let hash_nibble = if index % 2 == 0 {
            digest[index / 2] >> 4
        } else {
            digest[index / 2] & 0x0f
        };
        if byte.is_ascii_alphabetic() && hash_nibble >= 8 {
            checksum.push(char::from(byte.to_ascii_uppercase()));
        } else {
            checksum.push(char::from(byte));
        }
    }
    checksum
}

#[cfg(test)]
mod tests {
    use reap_pm_core::{ConnectionEpoch, PmConditionId, PmMarketId, PmTokenId, U256};

    use super::*;

    const ADDRESS: &str = "0xf39fd6e51aad88f6f4ce6ab8827279cfffb92266";
    const CHECKSUM_ADDRESS: &str = "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266";
    const OTHER_ADDRESS: &str = "0x1000000000000000000000000000000000000001";
    const API_KEY: &str = "00000000-0000-4000-8000-000000000001";
    const API_SECRET: &str = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";
    const PASSPHRASE: &str = "synthetic-passphrase";

    fn address(value: &str) -> EvmAddress {
        EvmAddress::parse(value).unwrap()
    }

    fn scope() -> PmWireScope {
        PmWireScope::new(
            PmConditionId::from_bytes([0x11; 32]).unwrap(),
            PmMarketId::from_bytes([0x22; 32]).unwrap(),
            PmTokenId::new(U256::from_u64(7)).unwrap(),
        )
    }

    fn user_bounds() -> PmUserWsBounds {
        PmUserWsBounds::new(
            Duration::from_secs(2),
            Duration::from_secs(30),
            Duration::from_secs(5),
            64 * 1_024,
            3,
            Duration::from_millis(100),
            32,
            ConnectionEpoch::new(1),
        )
        .unwrap()
    }

    fn credential_input() -> PmReadOnlyCredentialInput {
        PmReadOnlyCredentialInput::new(API_KEY.into(), API_SECRET.into(), PASSPHRASE.into())
    }

    #[test]
    fn typed_evm_address_is_bound_as_the_exact_eip55_eoa() {
        assert_eq!(eip55(address(ADDRESS)), CHECKSUM_ADDRESS);
        let owner = PmReadOnlyPrivateConnectivityOwner::production(
            address(ADDRESS),
            address(ADDRESS),
            scope(),
            Duration::from_secs(1),
            Duration::from_secs(2),
            user_bounds(),
            credential_input(),
        )
        .unwrap();
        assert_eq!(owner.configured_scope(), scope());
        assert!(!owner.production_order_entry_authorized());
        assert!(!format!("{owner:?}").contains(API_KEY));
    }

    #[test]
    fn facade_rejects_distinct_signer_and_funder_before_binding() {
        assert!(matches!(
            PmReadOnlyPrivateConnectivityOwner::production(
                address(ADDRESS),
                address(OTHER_ADDRESS),
                scope(),
                Duration::from_secs(1),
                Duration::from_secs(2),
                user_bounds(),
                credential_input(),
            ),
            Err(PmLiveAdapterError::InvalidConfiguration(
                "read-only credentials require signer and funder to be the same EOA"
            ))
        ));
    }

    #[test]
    fn evidence_facade_accepts_only_exact_local_read_configs() {
        let exact_scope = scope();
        let http = PmPrivateHttpConfig::read_only_evidence(
            "http://127.0.0.1:18080",
            Duration::from_secs(1),
            Duration::from_secs(2),
            exact_scope,
        )
        .unwrap();
        let user_ws = PmUserWsConfig::read_only_evidence(
            "ws://127.0.0.1:18081/ws/user",
            exact_scope.condition(),
            Duration::from_secs(1),
            Duration::from_secs(5),
            Duration::from_secs(1),
            Duration::from_millis(500),
            64 * 1_024,
            3,
            Duration::from_millis(100),
            32,
            ConnectionEpoch::new(1),
        )
        .unwrap();
        let owner = PmReadOnlyPrivateConnectivityOwner::read_only_evidence(
            address(ADDRESS),
            address(ADDRESS),
            exact_scope,
            http,
            user_ws,
            credential_input(),
        )
        .unwrap();
        assert_eq!(owner.configured_scope(), exact_scope);
        assert!(!owner.production_order_entry_authorized());
    }
}
