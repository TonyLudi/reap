use std::{fmt, fmt::Write as _, time::Duration};

use reap_pm_core::{EvmAddress, PmTokenId};
use reap_polymarket_auth::{L2CredentialInput, L2Credentials};
use reap_polymarket_wire::PmWireScope;
use sha3::{Digest as _, Keccak256};
use zeroize::Zeroizing;

use crate::{
    PM_CLOB_PRODUCTION_ORIGIN, PmAuthenticatedHttpOwner, PmAuthenticatedUserWsRole,
    PmCredentialAuthoritySupervisor, PmGeoblockHttpConfig, PmGeoblockHttpRole, PmLiveAdapterError,
    PmPrivateConnectivityOwner, PmPrivateHttpConfig, PmPublicHttpConfig, PmPublicMetadataHttpRole,
    PmReadOnlyAccountHttpOwner, PmReadOnlySignatureType, PmReadServerTimeHttpRole, PmUserWsBounds,
    PmUserWsConfig, config::OriginMode, private_credentials::account_http_credential_role,
    private_http::PmPrivateHttpTransport,
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

/// Sole owner of the minimal credentialed account-read composition.
///
/// It can release only public server time, the two fixed account GETs, and
/// the credential supervisor. No user WebSocket, reconciliation, exact-order,
/// signing, or mutation capability is constructed.
///
/// Type-1 responses prove that signer-bound L2 credentials reached the proxy
/// balance profile, but the API does not echo the configured funder. The
/// funder is therefore an operator-reviewed structural input, not a remotely
/// attested response field.
pub struct PmReadOnlyAccountConnectivityOwner {
    server_time: PmReadServerTimeHttpRole,
    account_transport: PmPrivateHttpTransport,
    credentials: L2Credentials,
    conditional_token: PmTokenId,
    signature_type: PmReadOnlySignatureType,
}

impl PmReadOnlyAccountConnectivityOwner {
    #[allow(clippy::too_many_arguments)]
    pub fn production(
        expected_signer: EvmAddress,
        expected_funder: EvmAddress,
        signature_type: PmReadOnlySignatureType,
        conditional_token: PmTokenId,
        connect_timeout: Duration,
        request_timeout: Duration,
        credentials: PmReadOnlyCredentialInput,
    ) -> Result<Self, PmLiveAdapterError> {
        let config = PmPublicHttpConfig::production(
            PM_CLOB_PRODUCTION_ORIGIN,
            connect_timeout,
            request_timeout,
        )?;
        Self::bind(
            expected_signer,
            expected_funder,
            signature_type,
            conditional_token,
            config,
            credentials,
        )
    }

    /// Construct the minimal account-only facade over a literal-loopback
    /// endpoint for end-to-end evidence tests.
    #[cfg(any(test, feature = "read-only-evidence"))]
    #[allow(clippy::too_many_arguments)]
    pub fn read_only_evidence(
        expected_signer: EvmAddress,
        expected_funder: EvmAddress,
        signature_type: PmReadOnlySignatureType,
        conditional_token: PmTokenId,
        origin: &str,
        connect_timeout: Duration,
        request_timeout: Duration,
        credentials: PmReadOnlyCredentialInput,
    ) -> Result<Self, PmLiveAdapterError> {
        let config =
            PmPublicHttpConfig::read_only_evidence(origin, connect_timeout, request_timeout)?;
        Self::bind(
            expected_signer,
            expected_funder,
            signature_type,
            conditional_token,
            config,
            credentials,
        )
    }

    fn bind(
        expected_signer: EvmAddress,
        expected_funder: EvmAddress,
        signature_type: PmReadOnlySignatureType,
        conditional_token: PmTokenId,
        config: PmPublicHttpConfig,
        credentials: PmReadOnlyCredentialInput,
    ) -> Result<Self, PmLiveAdapterError> {
        validate_account_profile(expected_signer, expected_funder, signature_type)?;
        let credentials =
            L2Credentials::bind(&eip55(expected_signer), credentials.into_auth_input())?;
        if credentials.address().as_core() != expected_signer {
            return Err(PmLiveAdapterError::CredentialOwnerMismatch);
        }
        let account_transport =
            PmPrivateHttpTransport::for_account(&config, credentials.address())?;
        let server_time = PmReadServerTimeHttpRole::new(config)?;
        Ok(Self {
            server_time,
            account_transport,
            credentials,
            conditional_token,
            signature_type,
        })
    }

    #[must_use]
    pub const fn signature_type(&self) -> PmReadOnlySignatureType {
        self.signature_type
    }

    #[must_use]
    pub const fn production_order_entry_authorized(&self) -> bool {
        false
    }

    pub fn split(self) -> Result<PmReadOnlyAccountConnectivityRoles, PmLiveAdapterError> {
        let (authority, credential_supervisor) = account_http_credential_role(self.credentials)?;
        let authenticated_account = PmReadOnlyAccountHttpOwner::from_authority(
            authority,
            self.account_transport,
            self.conditional_token,
            self.signature_type,
        );
        Ok(PmReadOnlyAccountConnectivityRoles {
            server_time: self.server_time,
            authenticated_account,
            credential_supervisor,
        })
    }
}

impl fmt::Debug for PmReadOnlyAccountConnectivityOwner {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PmReadOnlyAccountConnectivityOwner")
            .field("conditional_token", &self.conditional_token)
            .field("signature_type", &self.signature_type)
            .field("credentials", &"[REDACTED]")
            .finish()
    }
}

/// Minimal account-read roles. All fields are move-only and purpose-specific.
pub struct PmReadOnlyAccountConnectivityRoles {
    pub server_time: PmReadServerTimeHttpRole,
    pub authenticated_account: PmReadOnlyAccountHttpOwner,
    pub credential_supervisor: PmCredentialAuthoritySupervisor,
}

impl PmReadOnlyAccountConnectivityRoles {
    #[must_use]
    pub const fn production_order_entry_authorized(&self) -> bool {
        false
    }
}

impl fmt::Debug for PmReadOnlyAccountConnectivityRoles {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PmReadOnlyAccountConnectivityRoles([REDACTED])")
    }
}

/// Sole production-safe constructor for one account's authenticated read
/// connectivity.
///
/// The owner binds credentials to the exact configured signer/funder before
/// any authority task is started. It exposes no general credential, signing,
/// mutation-time or validator capability. The only public time capability is
/// the read-only `/time` role needed to authenticate the fixed private GETs.
pub struct PmReadOnlyPrivateConnectivityOwner {
    inner: PmPrivateConnectivityOwner,
    exact_scope: PmWireScope,
    server_time: PmReadServerTimeHttpRole,
    geoblock: PmGeoblockHttpRole,
    market_details: PmPublicMetadataHttpRole,
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
            expected_funder,
            PmReadOnlySignatureType::Eoa,
            exact_scope,
            http_config,
            user_ws_config,
            credentials,
        )
    }

    /// Construct the full authenticated read-only facade for a signature
    /// type-1 account. L2 headers are signed by `expected_signer`, while exact
    /// order identity is checked against the distinct proxy funder.
    #[allow(clippy::too_many_arguments)]
    pub fn production_proxy(
        expected_signer: EvmAddress,
        expected_funder: EvmAddress,
        exact_scope: PmWireScope,
        http_connect_timeout: Duration,
        http_request_timeout: Duration,
        user_ws_bounds: PmUserWsBounds,
        credentials: PmReadOnlyCredentialInput,
    ) -> Result<Self, PmLiveAdapterError> {
        validate_account_profile(
            expected_signer,
            expected_funder,
            PmReadOnlySignatureType::Proxy,
        )?;
        let http_config = PmPrivateHttpConfig::production(
            http_connect_timeout,
            http_request_timeout,
            exact_scope,
        )?;
        let user_ws_config = PmUserWsConfig::production(exact_scope.condition(), user_ws_bounds)?;
        Self::bind(
            expected_signer,
            expected_funder,
            PmReadOnlySignatureType::Proxy,
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
        validate_one_eoa(expected_signer, expected_funder)?;
        if http_config.mode() != OriginMode::LocalEvidence || user_ws_config.is_production() {
            return Err(PmLiveAdapterError::InvalidConfiguration(
                "read-only evidence facade requires literal-loopback transports",
            ));
        }
        Self::bind(
            expected_signer,
            expected_funder,
            PmReadOnlySignatureType::Eoa,
            exact_scope,
            http_config,
            user_ws_config,
            credentials,
        )
    }

    /// Construct the signature type-1 full read facade over literal-loopback
    /// transports for end-to-end contract evidence.
    #[cfg(any(test, feature = "read-only-evidence"))]
    #[allow(clippy::too_many_arguments)]
    pub fn proxy_read_only_evidence(
        expected_signer: EvmAddress,
        expected_funder: EvmAddress,
        exact_scope: PmWireScope,
        http_config: PmPrivateHttpConfig,
        user_ws_config: PmUserWsConfig,
        credentials: PmReadOnlyCredentialInput,
    ) -> Result<Self, PmLiveAdapterError> {
        validate_account_profile(
            expected_signer,
            expected_funder,
            PmReadOnlySignatureType::Proxy,
        )?;
        if http_config.mode() != OriginMode::LocalEvidence || user_ws_config.is_production() {
            return Err(PmLiveAdapterError::InvalidConfiguration(
                "read-only evidence facade requires literal-loopback transports",
            ));
        }
        Self::bind(
            expected_signer,
            expected_funder,
            PmReadOnlySignatureType::Proxy,
            exact_scope,
            http_config,
            user_ws_config,
            credentials,
        )
    }

    fn bind(
        expected_signer: EvmAddress,
        expected_funder: EvmAddress,
        signature_type: PmReadOnlySignatureType,
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

        let public_config = match http_config.mode() {
            OriginMode::Production => PmPublicHttpConfig::production(
                PM_CLOB_PRODUCTION_ORIGIN,
                http_config.connect_timeout(),
                http_config.request_timeout(),
            )?,
            #[cfg(any(test, feature = "loopback-evidence", feature = "read-only-evidence"))]
            OriginMode::LocalEvidence => PmPublicHttpConfig::local_evidence(
                http_config.origin().as_str(),
                http_config.connect_timeout(),
                http_config.request_timeout(),
            )?,
        };
        let geoblock_config = match http_config.mode() {
            OriginMode::Production => PmGeoblockHttpConfig::production(
                http_config.connect_timeout(),
                http_config.request_timeout(),
            )?,
            #[cfg(any(test, feature = "loopback-evidence", feature = "read-only-evidence"))]
            OriginMode::LocalEvidence => PmGeoblockHttpConfig::local_evidence(
                http_config.origin().as_str(),
                http_config.connect_timeout(),
                http_config.request_timeout(),
            )?,
        };
        let server_time = PmReadServerTimeHttpRole::new(public_config.clone())?;
        let market_details = PmPublicMetadataHttpRole::new(public_config, exact_scope)?;
        let geoblock = PmGeoblockHttpRole::new(geoblock_config)?;

        // Retain the original one-EOA binding name and check for the existing
        // type-0 path; the proxy path uses the same signer-bound credential
        // proof while carrying a distinct expected funder separately.
        let expected_eoa = expected_signer;
        let expected_eip55 = eip55(expected_eoa);
        let credentials = L2Credentials::bind(&expected_eip55, credentials.into_auth_input())?;
        if credentials.address().as_core() != expected_eoa {
            return Err(PmLiveAdapterError::CredentialOwnerMismatch);
        }
        let inner = match signature_type {
            PmReadOnlySignatureType::Eoa => {
                debug_assert_eq!(expected_signer, expected_funder);
                PmPrivateConnectivityOwner::new(http_config, user_ws_config, credentials)?
            }
            PmReadOnlySignatureType::Proxy => PmPrivateConnectivityOwner::new_proxy_read_only(
                http_config,
                user_ws_config,
                expected_funder,
                credentials,
            )?,
        };
        Ok(Self {
            inner,
            exact_scope,
            server_time,
            geoblock,
            market_details,
        })
    }

    #[must_use]
    pub const fn configured_scope(&self) -> PmWireScope {
        self.exact_scope
    }

    #[must_use]
    pub const fn production_order_entry_authorized(&self) -> bool {
        false
    }

    /// Consume the sole owner into its fixed public-safety and authenticated
    /// read capabilities. None carries mutation authentication or transport.
    pub fn split(self) -> Result<PmReadOnlyPrivateConnectivityRoles, PmLiveAdapterError> {
        let (authenticated_http, authenticated_user_ws, credential_supervisor) =
            self.inner.split()?.into_read_roles();
        Ok(PmReadOnlyPrivateConnectivityRoles {
            server_time: self.server_time,
            geoblock: self.geoblock,
            market_details: self.market_details,
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

/// Named, move-only result of splitting the read-only credential owner.
///
/// In particular, this bundle contains no mutation authentication role,
/// signer, general L2 credential, mutation-time proof, or mutation validator.
pub struct PmReadOnlyPrivateConnectivityRoles {
    pub server_time: PmReadServerTimeHttpRole,
    pub geoblock: PmGeoblockHttpRole,
    pub market_details: PmPublicMetadataHttpRole,
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

fn validate_account_profile(
    expected_signer: EvmAddress,
    expected_funder: EvmAddress,
    signature_type: PmReadOnlySignatureType,
) -> Result<(), PmLiveAdapterError> {
    if expected_signer.bytes() == [0; 20] || expected_funder.bytes() == [0; 20] {
        return Err(PmLiveAdapterError::InvalidConfiguration(
            "read-only signer and funder addresses must be nonzero",
        ));
    }
    match signature_type {
        PmReadOnlySignatureType::Eoa if expected_signer != expected_funder => {
            return Err(PmLiveAdapterError::InvalidConfiguration(
                "signature_type 0 requires signer and funder to be the same EOA",
            ));
        }
        PmReadOnlySignatureType::Proxy if expected_signer == expected_funder => {
            return Err(PmLiveAdapterError::InvalidConfiguration(
                "signature_type 1 requires a proxy funder distinct from the signer EOA",
            ));
        }
        PmReadOnlySignatureType::Eoa | PmReadOnlySignatureType::Proxy => {}
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
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
        sync::mpsc,
        task::JoinHandle,
    };

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

    async fn read_preflight_server(
        bodies: Vec<Vec<u8>>,
    ) -> (String, mpsc::UnboundedReceiver<String>, JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (request_tx, request_rx) = mpsc::unbounded_channel();
        let task = tokio::spawn(async move {
            for body in bodies {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut raw = Vec::new();
                let mut chunk = [0_u8; 1024];
                loop {
                    let read = stream.read(&mut chunk).await.unwrap();
                    if read == 0 {
                        break;
                    }
                    raw.extend_from_slice(&chunk[..read]);
                    if raw.windows(4).any(|window| window == b"\r\n\r\n") {
                        break;
                    }
                }
                let _ = request_tx.send(String::from_utf8(raw).unwrap());
                let headers = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                stream.write_all(headers.as_bytes()).await.unwrap();
                stream.write_all(&body).await.unwrap();
            }
        });
        (format!("http://{address}"), request_rx, task)
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
    fn proxy_full_read_facade_requires_and_retains_split_identity() {
        let owner = PmReadOnlyPrivateConnectivityOwner::production_proxy(
            address(ADDRESS),
            address(OTHER_ADDRESS),
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

        assert!(
            PmReadOnlyPrivateConnectivityOwner::production_proxy(
                address(ADDRESS),
                address(ADDRESS),
                scope(),
                Duration::from_secs(1),
                Duration::from_secs(2),
                user_bounds(),
                credential_input(),
            )
            .is_err()
        );
    }

    #[test]
    fn balance_signature_type_rejects_every_unreviewed_value() {
        assert_eq!(
            PmReadOnlySignatureType::try_from(0).unwrap(),
            PmReadOnlySignatureType::Eoa
        );
        assert_eq!(
            PmReadOnlySignatureType::try_from(1).unwrap(),
            PmReadOnlySignatureType::Proxy
        );
        for value in 2..=u8::MAX {
            assert!(PmReadOnlySignatureType::try_from(value).is_err());
        }
    }

    #[test]
    fn dedicated_account_owner_accepts_only_reviewed_identity_profiles() {
        let owner = PmReadOnlyAccountConnectivityOwner::production(
            address(ADDRESS),
            address(OTHER_ADDRESS),
            PmReadOnlySignatureType::Proxy,
            scope().token(),
            Duration::from_secs(1),
            Duration::from_secs(2),
            credential_input(),
        )
        .unwrap();
        assert_eq!(owner.signature_type(), PmReadOnlySignatureType::Proxy);
        assert!(!owner.production_order_entry_authorized());
        let debug = format!("{owner:?}");
        assert!(debug.contains("[REDACTED]"));
        assert!(!debug.contains(API_KEY));

        assert!(
            PmReadOnlyAccountConnectivityOwner::production(
                address(ADDRESS),
                address(OTHER_ADDRESS),
                PmReadOnlySignatureType::Eoa,
                scope().token(),
                Duration::from_secs(1),
                Duration::from_secs(2),
                credential_input(),
            )
            .is_err()
        );
    }

    #[tokio::test]
    async fn dedicated_account_split_releases_only_account_time_and_supervisor() {
        let owner = PmReadOnlyAccountConnectivityOwner::read_only_evidence(
            address(ADDRESS),
            address(OTHER_ADDRESS),
            PmReadOnlySignatureType::Proxy,
            scope().token(),
            "http://127.0.0.1:18080",
            Duration::from_secs(1),
            Duration::from_secs(2),
            credential_input(),
        )
        .unwrap();
        let roles = owner.split().unwrap();
        assert!(!roles.production_order_entry_authorized());
        assert!(
            !roles
                .authenticated_account
                .production_order_entry_authorized()
        );
        let debug = format!("{roles:?}");
        assert!(debug.contains("[REDACTED]"));
        assert!(!debug.contains(API_KEY));
        roles.credential_supervisor.shutdown().await.unwrap();
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

    #[tokio::test]
    async fn proxy_evidence_facade_splits_only_full_read_roles() {
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
        let owner = PmReadOnlyPrivateConnectivityOwner::proxy_read_only_evidence(
            address(ADDRESS),
            address(OTHER_ADDRESS),
            exact_scope,
            http,
            user_ws,
            credential_input(),
        )
        .unwrap();
        let roles = owner.split().unwrap();
        assert!(!roles.production_order_entry_authorized());
        assert!(!roles.authenticated_http.production_order_entry_authorized());
        roles.credential_supervisor.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn proxy_facade_runs_the_complete_literal_loopback_read_preflight() {
        let exact_scope = scope();
        let condition = exact_scope.condition().to_string();
        let market = exact_scope.market().to_string();
        let long_market = format!(
            r#"{{"condition_id":"{condition}","question_id":"{market}","active":true,"closed":false,"archived":false,"accepting_orders":true,"enable_order_book":true}}"#
        );
        let short_market = format!(
            r#"{{"c":"{condition}","t":[{{"t":"7","o":"Yes"}},{{"t":"8","o":"No"}}],"mts":0.01,"mos":5,"nr":false,"fd":{{"r":0.020,"e":2.0,"to":true}},"mbf":0,"tbf":0,"itode":false,"oas":0}}"#
        );
        let responses = vec![
            br#"{"blocked":false,"ip":"203.0.113.9","country":"US","region":"NY"}"#.to_vec(),
            long_market.into_bytes(),
            short_market.into_bytes(),
            b"1780449126".to_vec(),
            br#"{"closed_only":false}"#.to_vec(),
        ];
        let (origin, mut requests, server) = read_preflight_server(responses).await;
        let http = PmPrivateHttpConfig::read_only_evidence(
            &origin,
            Duration::from_secs(1),
            Duration::from_secs(2),
            exact_scope,
        )
        .unwrap();
        let ws_origin = format!("ws://{}/ws/user", origin.trim_start_matches("http://"));
        let user_ws = PmUserWsConfig::read_only_evidence(
            &ws_origin,
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
        let owner = PmReadOnlyPrivateConnectivityOwner::proxy_read_only_evidence(
            address(ADDRESS),
            address(OTHER_ADDRESS),
            exact_scope,
            http,
            user_ws,
            credential_input(),
        )
        .unwrap();
        let mut roles = owner.split().unwrap();

        let geoblock = roles.geoblock.status().await.unwrap();
        assert!(!geoblock.blocked());
        assert_eq!(geoblock.country(), "US");
        let details = roles.market_details.refresh_typed().await.unwrap();
        assert_eq!(details.lifecycle().condition(), exact_scope.condition());
        assert_eq!(details.lifecycle().market(), exact_scope.market());
        assert_eq!(
            details.clob().configured_outcome().token(),
            exact_scope.token()
        );
        assert_eq!(details.clob().tokens().len(), 2);
        assert_eq!(details.clob().maker_base_fee_bps(), 0);
        assert_eq!(details.clob().taker_base_fee_bps(), 0);
        assert_eq!(
            details.clob().fee_details().rate().unwrap().as_str(),
            "0.020"
        );
        assert_eq!(
            details.clob().fee_details().exponent().unwrap().as_str(),
            "2.0"
        );
        assert!(!details.clob().take_only_delay_enabled());
        assert_eq!(details.clob().minimum_order_age_seconds(), 0);
        let server_time = roles.server_time.fresh_read_server_time().await.unwrap();
        let closed_only = roles
            .authenticated_http
            .preflight()
            .closed_only(server_time)
            .await
            .unwrap();
        assert!(!closed_only.closed_only());

        let mut captured = Vec::new();
        for _ in 0..5 {
            captured.push(requests.recv().await.unwrap());
        }
        assert_eq!(
            captured
                .iter()
                .map(|request| request.lines().next().unwrap())
                .collect::<Vec<_>>(),
            [
                "GET /api/geoblock HTTP/1.1".to_owned(),
                format!("GET /markets/{condition} HTTP/1.1"),
                format!("GET /clob-markets/{condition} HTTP/1.1"),
                "GET /time HTTP/1.1".to_owned(),
                "GET /auth/ban-status/closed-only HTTP/1.1".to_owned(),
            ]
        );
        let closed_request = captured.last().unwrap().to_ascii_lowercase();
        assert!(
            closed_request.contains(&format!("poly_address: {}", ADDRESS.to_ascii_lowercase()))
        );
        assert!(!closed_request.contains(&format!(
            "poly_address: {}",
            OTHER_ADDRESS.to_ascii_lowercase()
        )));
        assert!(
            closed_request.contains("poly_signature: n1obdnq7auhb1m63pmycfo6tkgegkwjgrzkv86-ztfe=")
        );

        server.await.unwrap();
        assert!(requests.try_recv().is_err());
        assert!(!roles.production_order_entry_authorized());
        assert!(!roles.geoblock.production_order_entry_authorized());
        assert!(!roles.authenticated_http.production_order_entry_authorized());
        roles.credential_supervisor.shutdown().await.unwrap();
    }
}
