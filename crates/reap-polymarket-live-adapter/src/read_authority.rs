use std::{fmt, time::Duration};

use async_trait::async_trait;
use reap_pm_core::{EvmAddress, PmConditionId};
use reap_polymarket_auth::{
    AuthenticatedL2Headers, AuthenticatedUserSubscription, CredentialOwnedUserFrame, EoaAddress,
    FixedOrderId, L2Timestamp,
};
use reap_polymarket_egress_binding::{PmFixedTlsPeerSelection, PmLocalEgressSelection};
use reap_polymarket_wire::{
    PmLiveOpenOrderPage, PmLiveOrder, PmLiveTradePage, PmLiveUserFrame, PmWireScope,
};

#[cfg(any(test, feature = "loopback-evidence"))]
use crate::config::OriginMode;

use crate::{
    PmAuthenticatedHttpOwner, PmAuthenticatedUserWsRole, PmLiveAdapterError, PmPrivateHttpConfig,
    PmReadOnlySignatureType, PmUserWsBounds, PmUserWsConfig, private_http::PmPrivateHttpTransport,
};

/// Purpose-closed provider for the complete set of authenticated HTTP reads.
///
/// Implementations retain credential custody. This interface accepts only
/// typed timestamps and exact order identities, returns only consume-once
/// header carriers or complete parsed owner-bound values, and has no generic
/// route, raw-header, signing, mutation, or body surface.
#[async_trait]
pub trait PmHttpReadAuthorityProvider: Send {
    async fn authenticate_open_orders(
        &mut self,
        timestamp: L2Timestamp,
    ) -> Result<AuthenticatedL2Headers, PmLiveAdapterError>;

    async fn authenticate_trades(
        &mut self,
        timestamp: L2Timestamp,
    ) -> Result<AuthenticatedL2Headers, PmLiveAdapterError>;

    async fn authenticate_balance_allowance(
        &mut self,
        timestamp: L2Timestamp,
    ) -> Result<AuthenticatedL2Headers, PmLiveAdapterError>;

    async fn authenticate_closed_only(
        &mut self,
        timestamp: L2Timestamp,
    ) -> Result<AuthenticatedL2Headers, PmLiveAdapterError>;

    async fn authenticate_exact_order(
        &mut self,
        timestamp: L2Timestamp,
        order_id: FixedOrderId,
    ) -> Result<AuthenticatedL2Headers, PmLiveAdapterError>;

    async fn bind_open_orders(
        &mut self,
        page: PmLiveOpenOrderPage,
    ) -> Result<PmLiveOpenOrderPage, PmLiveAdapterError>;

    async fn bind_trades(
        &mut self,
        page: PmLiveTradePage,
    ) -> Result<PmLiveTradePage, PmLiveAdapterError>;

    async fn bind_exact_order(
        &mut self,
        order: PmLiveOrder,
    ) -> Result<PmLiveOrder, PmLiveAdapterError>;
}

/// Purpose-closed provider for one authenticated user subscription and full
/// parsed-frame credential-owner binding.
///
/// The authenticated subscription is an opaque consume-once carrier. Neither
/// this interface nor the returned bound frame exposes credential material or
/// a generic WebSocket/request surface.
#[async_trait]
pub trait PmUserWsReadAuthorityProvider: Send {
    async fn authenticate_user_subscription(
        &mut self,
        condition: PmConditionId,
    ) -> Result<AuthenticatedUserSubscription, PmLiveAdapterError>;

    async fn bind_user_frame(
        &mut self,
        frame: PmLiveUserFrame,
    ) -> Result<CredentialOwnedUserFrame, PmLiveAdapterError>;
}

/// Read-only production connectivity whose credentials remain owned by an
/// external authority, such as the bin-private controlled-trial runner.
///
/// Construction fixes both venue endpoints, the proxy balance profile, the
/// authenticated signer, and the distinct expected proxy maker. The external
/// authority lifecycle remains with the caller and is deliberately not
/// represented as mutation or dispatch authority here.
pub struct PmExternalProxyReadConnectivityOwner {
    http: PmAuthenticatedHttpOwner,
    user_ws: PmAuthenticatedUserWsRole,
}

impl PmExternalProxyReadConnectivityOwner {
    #[allow(clippy::too_many_arguments)]
    pub fn production<H, U>(
        connect_timeout: Duration,
        request_timeout: Duration,
        exact_order_scope: PmWireScope,
        user_ws_bounds: PmUserWsBounds,
        l2_signer_address: EoaAddress,
        proxy_funder: EvmAddress,
        http_authority: H,
        user_ws_authority: U,
    ) -> Result<Self, PmLiveAdapterError>
    where
        H: PmHttpReadAuthorityProvider + 'static,
        U: PmUserWsReadAuthorityProvider + 'static,
    {
        let http_config =
            PmPrivateHttpConfig::production(connect_timeout, request_timeout, exact_order_scope)?;
        let user_ws_config =
            PmUserWsConfig::production(exact_order_scope.condition(), user_ws_bounds)?;
        Self::from_configs(
            http_config,
            user_ws_config,
            l2_signer_address,
            proxy_funder,
            http_authority,
            user_ws_authority,
        )
    }

    /// Construct the same external-authority read owner while fixing only its
    /// authenticated HTTP client to one exact TLS peer and selected local
    /// egress. The user WebSocket remains the ordinary fixed production
    /// endpoint; selected WebSocket egress is a separate, later capability.
    #[allow(clippy::too_many_arguments)]
    pub fn production_on_fixed_tls_peer_and_selected_local_egress<H, U>(
        connect_timeout: Duration,
        request_timeout: Duration,
        exact_order_scope: PmWireScope,
        user_ws_bounds: PmUserWsBounds,
        fixed_tls_peer: PmFixedTlsPeerSelection,
        selected_local_egress: PmLocalEgressSelection,
        l2_signer_address: EoaAddress,
        proxy_funder: EvmAddress,
        http_authority: H,
        user_ws_authority: U,
    ) -> Result<Self, PmLiveAdapterError>
    where
        H: PmHttpReadAuthorityProvider + 'static,
        U: PmUserWsReadAuthorityProvider + 'static,
    {
        let http_config =
            PmPrivateHttpConfig::production_on_fixed_tls_peer_and_selected_local_egress(
                connect_timeout,
                request_timeout,
                exact_order_scope,
                fixed_tls_peer,
                selected_local_egress,
            )?;
        let user_ws_config =
            PmUserWsConfig::production(exact_order_scope.condition(), user_ws_bounds)?;
        Self::from_configs(
            http_config,
            user_ws_config,
            l2_signer_address,
            proxy_funder,
            http_authority,
            user_ws_authority,
        )
    }

    /// Literal-loopback construction for synthetic protocol evidence only.
    #[cfg(any(test, feature = "loopback-evidence"))]
    pub fn loopback_evidence<H, U>(
        http_config: PmPrivateHttpConfig,
        user_ws_config: PmUserWsConfig,
        l2_signer_address: EoaAddress,
        proxy_funder: EvmAddress,
        http_authority: H,
        user_ws_authority: U,
    ) -> Result<Self, PmLiveAdapterError>
    where
        H: PmHttpReadAuthorityProvider + 'static,
        U: PmUserWsReadAuthorityProvider + 'static,
    {
        if http_config.mode() != OriginMode::LocalEvidence || user_ws_config.is_production() {
            return Err(PmLiveAdapterError::InvalidConfiguration(
                "loopback external read owner requires loopback private HTTP and user WebSocket configurations",
            ));
        }
        Self::from_configs(
            http_config,
            user_ws_config,
            l2_signer_address,
            proxy_funder,
            http_authority,
            user_ws_authority,
        )
    }

    fn from_configs<H, U>(
        http_config: PmPrivateHttpConfig,
        user_ws_config: PmUserWsConfig,
        l2_signer_address: EoaAddress,
        proxy_funder: EvmAddress,
        http_authority: H,
        user_ws_authority: U,
    ) -> Result<Self, PmLiveAdapterError>
    where
        H: PmHttpReadAuthorityProvider + 'static,
        U: PmUserWsReadAuthorityProvider + 'static,
    {
        if http_config.exact_order_scope().condition() != user_ws_config.condition() {
            return Err(PmLiveAdapterError::InvalidConfiguration(
                "external private HTTP and user WebSocket must bind the same condition",
            ));
        }
        if l2_signer_address.as_core() == proxy_funder {
            return Err(PmLiveAdapterError::InvalidConfiguration(
                "external proxy read profile requires distinct signer and funder",
            ));
        }

        let transport = PmPrivateHttpTransport::new(&http_config, l2_signer_address)?;
        let http = PmAuthenticatedHttpOwner::from_external_authority_with_account_profile(
            transport,
            http_config.exact_order_scope(),
            l2_signer_address,
            proxy_funder,
            PmReadOnlySignatureType::Proxy,
            Box::new(http_authority),
        );
        let user_ws = PmAuthenticatedUserWsRole::from_external_authority(
            user_ws_config,
            l2_signer_address.as_core(),
            proxy_funder,
            Box::new(user_ws_authority),
        );
        Ok(Self { http, user_ws })
    }

    #[must_use]
    pub fn into_read_roles(self) -> (PmAuthenticatedHttpOwner, PmAuthenticatedUserWsRole) {
        (self.http, self.user_ws)
    }

    #[must_use]
    pub const fn production_order_entry_authorized(&self) -> bool {
        false
    }
}

impl fmt::Debug for PmExternalProxyReadConnectivityOwner {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PmExternalProxyReadConnectivityOwner([REDACTED])")
    }
}

#[cfg(test)]
mod tests {
    use std::{convert::Infallible, time::Duration};

    use futures_util::{SinkExt as _, StreamExt as _};
    use reap_pm_core::{ConnectionEpoch, PmMarketId, PmTokenId, U256};
    use reap_polymarket_auth::{L2CredentialInput, L2Credentials};
    use tokio::{
        io::{AsyncReadExt as _, AsyncWriteExt as _},
        net::TcpListener,
    };
    use tokio_tungstenite::{accept_async, tungstenite::Message};

    use super::*;
    use crate::{
        PmUserWsDisconnectReason, PmUserWsEvent, PmUserWsEventSink, PmUserWsRunError,
        PmUserWsTransportError, pm_user_ws_shutdown_channel,
        private_credentials::test_read_credential_roles,
        product_clock::test_support_read_server_time,
    };

    const SIGNER: &str = "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266";
    const PROXY: &str = "0x70997970C51812dc3A010C7d01b50e0d17dc79C8";
    const API_KEY: &str = "00000000-0000-4000-8000-000000000001";
    const FOREIGN_API_KEY: &str = "00000000-0000-4000-8000-000000000002";
    const API_SECRET: &str = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";
    const PASSPHRASE: &str = "synthetic-passphrase";
    const CONDITION: &str = "0x1111111111111111111111111111111111111111111111111111111111111111";
    const QUESTION: &str = "0x2222222222222222222222222222222222222222222222222222222222222222";
    const ORDER: &str = "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    fn scope() -> PmWireScope {
        PmWireScope::new(
            PmConditionId::parse(CONDITION).unwrap(),
            PmMarketId::parse(QUESTION).unwrap(),
            PmTokenId::new(U256::from_u64(123)).unwrap(),
        )
    }

    fn credentials() -> L2Credentials {
        L2Credentials::bind(
            SIGNER,
            L2CredentialInput::new(API_KEY.into(), API_SECRET.into(), PASSPHRASE.into()),
        )
        .unwrap()
    }

    fn user_ws_bounds() -> PmUserWsBounds {
        PmUserWsBounds::new(
            Duration::from_secs(1),
            Duration::from_secs(30),
            Duration::from_secs(5),
            4 * 1_024,
            0,
            Duration::from_millis(1),
            8,
            ConnectionEpoch::new(1),
        )
        .unwrap()
    }

    fn foreign_frame() -> String {
        format!(
            r#"{{"event_type":"order","id":"{ORDER}","owner":"{FOREIGN_API_KEY}","market":"{CONDITION}","asset_id":"123","side":"BUY","original_size":"10","size_matched":"0","price":"0.42","type":"PLACEMENT","status":"LIVE","timestamp":"1782753357257"}}"#,
        )
    }

    #[derive(Default)]
    struct EvidenceSink {
        saw_bound: bool,
        retirement: Option<PmUserWsDisconnectReason>,
        generations: Vec<u64>,
    }

    #[async_trait]
    impl PmUserWsEventSink for EvidenceSink {
        type Error = Infallible;

        async fn deliver_user_ws_event(&mut self, event: PmUserWsEvent) -> Result<(), Self::Error> {
            self.generations.push(event.activity_generation());
            match event {
                PmUserWsEvent::BoundFrame(_) => self.saw_bound = true,
                PmUserWsEvent::ConnectionRetired(retirement) => {
                    self.retirement = Some(retirement.reason());
                }
                _ => {}
            }
            Ok(())
        }
    }

    #[tokio::test]
    async fn loopback_read_owner_rejects_production_private_http_before_roles_exist() {
        let http_config = PmPrivateHttpConfig::production(
            Duration::from_secs(1),
            Duration::from_secs(2),
            scope(),
        )
        .unwrap();
        let user_ws_config = PmUserWsConfig::loopback_evidence(
            "ws://127.0.0.1:18081/ws/user",
            scope().condition(),
            Duration::from_secs(1),
            Duration::from_secs(2),
            Duration::from_millis(250),
            Duration::from_millis(100),
            4 * 1_024,
            0,
            Duration::from_millis(1),
            8,
            ConnectionEpoch::new(1),
        )
        .unwrap();
        let (http_authority, user_ws_authority, supervisor) =
            test_read_credential_roles(credentials()).unwrap();
        let result = PmExternalProxyReadConnectivityOwner::loopback_evidence(
            http_config,
            user_ws_config,
            EoaAddress::parse(SIGNER).unwrap(),
            EvmAddress::parse(PROXY).unwrap(),
            http_authority,
            user_ws_authority,
        );
        assert!(matches!(
            result,
            Err(PmLiveAdapterError::InvalidConfiguration(
                "loopback external read owner requires loopback private HTTP and user WebSocket configurations"
            ))
        ));
        supervisor.shutdown().await.unwrap();
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn fixed_peer_selected_http_preserves_external_read_authorities_without_selecting_ws() {
        let fixed_tls_peer =
            PmFixedTlsPeerSelection::production("clob.polymarket.com", "8.8.8.8").unwrap();
        let selected_local_egress =
            PmLocalEgressSelection::production("pm-tunnel0", "192.0.2.10".parse().unwrap())
                .unwrap();
        let (http_authority, user_ws_authority, supervisor) =
            test_read_credential_roles(credentials()).unwrap();
        let owner = PmExternalProxyReadConnectivityOwner::
            production_on_fixed_tls_peer_and_selected_local_egress(
                Duration::from_secs(1),
                Duration::from_secs(2),
                scope(),
                user_ws_bounds(),
                fixed_tls_peer,
                selected_local_egress,
                EoaAddress::parse(SIGNER).unwrap(),
                EvmAddress::parse(PROXY).unwrap(),
                http_authority,
                user_ws_authority,
            )
            .unwrap();
        assert!(!owner.production_order_entry_authorized());
        drop(owner);
        supervisor.shutdown().await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn one_external_authority_drives_http_and_user_ws_and_rejects_foreign_owner() {
        let http_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let http_address = http_listener.local_addr().unwrap();
        let http_server = tokio::spawn(async move {
            let (mut stream, _) = http_listener.accept().await.unwrap();
            let mut request = vec![0_u8; 8 * 1_024];
            let mut received = 0;
            loop {
                let read = stream.read(&mut request[received..]).await.unwrap();
                assert_ne!(read, 0, "HTTP request ended before its header block");
                received += read;
                if request[..received]
                    .windows(4)
                    .any(|window| window == b"\r\n\r\n")
                {
                    break;
                }
            }
            let request = String::from_utf8(request[..received].to_vec()).unwrap();
            assert!(request.starts_with("GET /auth/ban-status/closed-only HTTP/1.1\r\n"));
            assert!(request.to_ascii_lowercase().contains(&format!(
                "poly_address: {}\r\n",
                SIGNER.to_ascii_lowercase(),
            )));
            stream
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 21\r\nConnection: close\r\n\r\n{\"closed_only\":false}",
                )
                .await
                .unwrap();
        });

        let ws_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let ws_address = ws_listener.local_addr().unwrap();
        let ws_server = tokio::spawn(async move {
            let (stream, _) = ws_listener.accept().await.unwrap();
            let mut socket = accept_async(stream).await.unwrap();
            let subscription = socket.next().await.unwrap().unwrap();
            let subscription = subscription.into_text().unwrap();
            assert!(subscription.contains(API_KEY));
            assert!(subscription.contains(CONDITION));
            socket.send(Message::text(foreign_frame())).await.unwrap();
        });

        let http_config = PmPrivateHttpConfig::loopback_evidence(
            &format!("http://{http_address}"),
            Duration::from_secs(1),
            Duration::from_secs(2),
            scope(),
        )
        .unwrap();
        let user_ws_config = PmUserWsConfig::loopback_evidence(
            &format!("ws://{ws_address}/ws/user"),
            scope().condition(),
            Duration::from_secs(1),
            Duration::from_secs(2),
            Duration::from_millis(250),
            Duration::from_millis(100),
            4 * 1_024,
            0,
            Duration::from_millis(1),
            8,
            ConnectionEpoch::new(1),
        )
        .unwrap();
        let (http_authority, user_ws_authority, supervisor) =
            test_read_credential_roles(credentials()).unwrap();
        let owner = PmExternalProxyReadConnectivityOwner::loopback_evidence(
            http_config,
            user_ws_config,
            EoaAddress::parse(SIGNER).unwrap(),
            EvmAddress::parse(PROXY).unwrap(),
            http_authority,
            user_ws_authority,
        )
        .unwrap();
        let (mut http, user_ws) = owner.into_read_roles();

        let closed_only = http
            .preflight()
            .closed_only(test_support_read_server_time(1_782_753_357))
            .await
            .unwrap();
        assert!(!closed_only.closed_only());

        let activity = user_ws.activity_view();
        let (_shutdown, shutdown_signal) = pm_user_ws_shutdown_channel();
        let mut sink = EvidenceSink::default();
        let error = user_ws.run(shutdown_signal, &mut sink).await.unwrap_err();
        assert!(matches!(
            error,
            PmUserWsRunError::Transport(PmUserWsTransportError::RetryExhausted {
                final_reason: PmUserWsDisconnectReason::CredentialOwnerMismatch,
                ..
            })
        ));
        assert!(!sink.saw_bound);
        assert_eq!(
            sink.retirement,
            Some(PmUserWsDisconnectReason::CredentialOwnerMismatch),
        );
        assert!(sink.generations.windows(2).all(|pair| pair[0] < pair[1]));
        assert_eq!(activity.high_water(), *sink.generations.last().unwrap());

        drop(http);
        supervisor.shutdown().await.unwrap();
        http_server.await.unwrap();
        ws_server.await.unwrap();
    }
}
