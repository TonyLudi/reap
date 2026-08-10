//! Non-authoritative production-selected WebSocket ownership.
//!
//! This module can bind the public and authenticated-user WebSocket roles to
//! one validated caller-provided fixed peer and one selected Linux
//! interface/source pair. It does not observe DNS, a public NAT address, a
//! network namespace, a Linux TID, an orchestration actor generation, or an
//! authorization window. Split role values can also be recombined with values
//! from another split, so their private allocation lineage is only a local
//! construction guard and never provenance or authority.

use std::{
    fmt,
    net::SocketAddr,
    rc::Rc,
    thread::{self, ThreadId},
};

use reap_polymarket_egress_binding::{PmFixedTlsPeerSelection, PmLocalEgressSelection};

use crate::{
    PmAuthenticatedUserWsRole, PmLiveAdapterError, PmPublicMarketWsRole,
    public_ws::PmProductionSelectedPublicWsRole, user_ws::PmProductionSelectedUserWsRole,
    ws_transport::PmFixedWsRoute,
};

#[cfg(target_os = "linux")]
const PRODUCTION_WS_DNS_NAME: &str = "ws-subscriptions-clob.polymarket.com";
const LINUX_INTERFACE_NAME_MAX_BYTES: usize = 15;

/// Read-only facts copied from one successfully upgraded selected socket.
///
/// The interface bytes are the exact post-handshake `SO_BINDTODEVICE`
/// readback. The addresses are the exact local and peer socket addresses.
/// These facts do not prove the packet route, network namespace, Linux TID,
/// DNS path, public NAT identity, actor generation, or authorization.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct PmSelectedWsSocketFacts {
    interface_name: [u8; LINUX_INTERFACE_NAME_MAX_BYTES],
    interface_name_len: u8,
    local_addr: SocketAddr,
    peer_addr: SocketAddr,
}

impl PmSelectedWsSocketFacts {
    pub(crate) fn from_verified_socket(
        interface_name: &[u8],
        local_addr: SocketAddr,
        peer_addr: SocketAddr,
    ) -> Option<Self> {
        if interface_name.is_empty()
            || interface_name.len() > LINUX_INTERFACE_NAME_MAX_BYTES
            || !interface_name.is_ascii()
        {
            return None;
        }
        let mut copied_name = [0_u8; LINUX_INTERFACE_NAME_MAX_BYTES];
        copied_name[..interface_name.len()].copy_from_slice(interface_name);
        Some(Self {
            interface_name: copied_name,
            interface_name_len: interface_name.len() as u8,
            local_addr,
            peer_addr,
        })
    }

    /// Exact Linux interface name read back from the connected socket.
    #[must_use]
    pub fn interface_name(&self) -> &str {
        std::str::from_utf8(&self.interface_name[..usize::from(self.interface_name_len)])
            .expect("selected WebSocket interface names are validated ASCII")
    }

    #[must_use]
    pub const fn local_addr(self) -> SocketAddr {
        self.local_addr
    }

    #[must_use]
    pub const fn peer_addr(self) -> SocketAddr {
        self.peer_addr
    }

    /// Selected socket facts can never authorize production order entry.
    #[must_use]
    pub const fn production_order_entry_authorized(self) -> bool {
        false
    }
}

impl fmt::Debug for PmSelectedWsSocketFacts {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .write_str("PmSelectedWsSocketFacts(<non-authoritative; device/local/peer redacted>)")
    }
}

struct PmProductionSelectedWsBundleIdentity {
    creating_process_id: u32,
    creating_thread_id: ThreadId,
    fixed_tls_peer: PmFixedTlsPeerSelection,
    selected_local_egress: PmLocalEgressSelection,
}

pub(crate) struct PmProductionSelectedWsRouteBinding {
    route: PmFixedWsRoute,
    identity: Rc<PmProductionSelectedWsBundleIdentity>,
}

impl PmProductionSelectedWsRouteBinding {
    pub(crate) const fn route(&self) -> PmFixedWsRoute {
        self.route
    }

    pub(crate) fn fixed_tls_peer(&self) -> &PmFixedTlsPeerSelection {
        &self.identity.fixed_tls_peer
    }

    pub(crate) fn selected_local_egress(&self) -> &PmLocalEgressSelection {
        &self.identity.selected_local_egress
    }

    pub(crate) fn revalidate_process_and_thread(&self) -> bool {
        self.identity.creating_process_id == std::process::id()
            && self.identity.creating_thread_id == thread::current().id()
    }
}

struct PmProductionSelectedWsBundleBinding {
    identity: Rc<PmProductionSelectedWsBundleIdentity>,
}

impl PmProductionSelectedWsBundleBinding {
    fn new(
        fixed_tls_peer: PmFixedTlsPeerSelection,
        selected_local_egress: PmLocalEgressSelection,
    ) -> Result<Self, PmLiveAdapterError> {
        #[cfg(not(target_os = "linux"))]
        {
            let _ = (fixed_tls_peer, selected_local_egress);
            return Err(PmLiveAdapterError::InvalidConfiguration(
                "production selected WebSocket egress requires Linux",
            ));
        }

        #[cfg(target_os = "linux")]
        {
            fixed_tls_peer.require_production().map_err(|_| {
                PmLiveAdapterError::InvalidConfiguration(
                    "production selected WebSocket requires a production fixed TLS peer",
                )
            })?;
            selected_local_egress.require_production().map_err(|_| {
                PmLiveAdapterError::InvalidConfiguration(
                    "production selected WebSocket requires production local egress",
                )
            })?;
            fixed_tls_peer
                .require_same_address_family(&selected_local_egress)
                .map_err(|_| {
                    PmLiveAdapterError::InvalidConfiguration(
                        "production selected WebSocket peer and local egress families differ",
                    )
                })?;
            if fixed_tls_peer.dns_name() != PRODUCTION_WS_DNS_NAME
                || fixed_tls_peer.peer_addr().port() != 443
            {
                return Err(PmLiveAdapterError::InvalidConfiguration(
                    "production selected WebSocket fixed peer has the wrong TLS identity",
                ));
            }
            Ok(Self {
                identity: Rc::new(PmProductionSelectedWsBundleIdentity {
                    creating_process_id: std::process::id(),
                    creating_thread_id: thread::current().id(),
                    fixed_tls_peer,
                    selected_local_egress,
                }),
            })
        }
    }

    fn into_routes(
        self,
    ) -> (
        PmProductionSelectedWsRouteBinding,
        PmProductionSelectedWsRouteBinding,
    ) {
        let public = PmProductionSelectedWsRouteBinding {
            route: PmFixedWsRoute::PublicMarket,
            identity: Rc::clone(&self.identity),
        };
        let user = PmProductionSelectedWsRouteBinding {
            route: PmFixedWsRoute::AuthenticatedUser,
            identity: self.identity,
        };
        (public, user)
    }
}

/// Paired, move-only construction owner for production-selected public and
/// authenticated-user WebSockets.
///
/// The owner accepts no endpoint, resolver, socket, dial strategy, or raw
/// credential. Its private `Rc` keeps the pair on its constructing OS thread,
/// but is not an actor-generation or same-pair proof after [`Self::into_roles`].
pub struct PmProductionSelectedWsOwner {
    public: PmPublicMarketWsRole,
    user: PmAuthenticatedUserWsRole,
    binding: PmProductionSelectedWsBundleBinding,
}

impl PmProductionSelectedWsOwner {
    pub fn new(
        public: PmPublicMarketWsRole,
        user: PmAuthenticatedUserWsRole,
        fixed_tls_peer: PmFixedTlsPeerSelection,
        selected_local_egress: PmLocalEgressSelection,
    ) -> Result<Self, PmLiveAdapterError> {
        if !public.is_production() || !user.is_production() {
            return Err(PmLiveAdapterError::InvalidConfiguration(
                "production selected WebSocket owner requires production roles",
            ));
        }
        if public.scope().condition() != user.condition() {
            return Err(PmLiveAdapterError::InvalidConfiguration(
                "production selected WebSocket roles must bind one condition",
            ));
        }
        let binding =
            PmProductionSelectedWsBundleBinding::new(fixed_tls_peer, selected_local_egress)?;
        Ok(Self {
            public,
            user,
            binding,
        })
    }

    #[must_use]
    pub fn into_roles(
        self,
    ) -> (
        PmProductionSelectedPublicWsRole,
        PmProductionSelectedUserWsRole,
    ) {
        let (public_binding, user_binding) = self.binding.into_routes();
        (
            PmProductionSelectedPublicWsRole::from_role_and_binding(self.public, public_binding),
            PmProductionSelectedUserWsRole::from_role_and_binding(self.user, user_binding),
        )
    }

    /// Pairing selected read transports never authorizes order entry.
    #[must_use]
    pub const fn production_order_entry_authorized(&self) -> bool {
        false
    }
}

impl fmt::Debug for PmProductionSelectedWsOwner {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(
            "PmProductionSelectedWsOwner(<non-authoritative; paired selected WebSockets>)",
        )
    }
}

#[cfg(test)]
mod tests {
    #[cfg(target_os = "linux")]
    use std::time::Duration;

    #[cfg(target_os = "linux")]
    use async_trait::async_trait;
    #[cfg(target_os = "linux")]
    use reap_pm_core::{ConnectionEpoch, PmConditionId, PmMarketId, PmTokenId, U256};
    #[cfg(target_os = "linux")]
    use reap_polymarket_auth::{AuthenticatedUserSubscription, CredentialOwnedUserFrame};
    #[cfg(target_os = "linux")]
    use reap_polymarket_wire::{PmLiveUserFrame, PmWireScope};

    #[cfg(target_os = "linux")]
    use crate::{PmPublicWsBounds, PmPublicWsConfig, PmUserWsBounds, PmUserWsConfig};

    use super::*;

    #[cfg(target_os = "linux")]
    struct UnusedUserAuthority;

    #[cfg(target_os = "linux")]
    #[async_trait]
    impl crate::PmUserWsReadAuthorityProvider for UnusedUserAuthority {
        async fn authenticate_user_subscription(
            &mut self,
            _condition: PmConditionId,
        ) -> Result<AuthenticatedUserSubscription, PmLiveAdapterError> {
            panic!("selected owner construction does not authenticate")
        }

        async fn bind_user_frame(
            &mut self,
            _frame: PmLiveUserFrame,
        ) -> Result<CredentialOwnedUserFrame, PmLiveAdapterError> {
            panic!("selected owner construction does not bind frames")
        }
    }

    #[cfg(target_os = "linux")]
    fn condition(byte: u8) -> PmConditionId {
        PmConditionId::from_bytes([byte; 32]).unwrap()
    }

    #[cfg(target_os = "linux")]
    fn scope(condition: PmConditionId) -> PmWireScope {
        PmWireScope::new(
            condition,
            PmMarketId::from_bytes([0x22; 32]).unwrap(),
            PmTokenId::new(U256::from_u64(7)).unwrap(),
        )
    }

    #[cfg(target_os = "linux")]
    fn public_role(condition: PmConditionId) -> PmPublicMarketWsRole {
        let bounds = PmPublicWsBounds::new(
            Duration::from_secs(2),
            Duration::from_secs(30),
            Duration::from_secs(5),
            64 * 1_024,
            2,
            Duration::from_millis(100),
            16,
            ConnectionEpoch::new(1),
        )
        .unwrap();
        PmPublicMarketWsRole::new(PmPublicWsConfig::production(scope(condition), bounds).unwrap())
            .unwrap()
    }

    #[cfg(target_os = "linux")]
    fn user_role(condition: PmConditionId) -> PmAuthenticatedUserWsRole {
        let bounds = PmUserWsBounds::new(
            Duration::from_secs(2),
            Duration::from_secs(30),
            Duration::from_secs(5),
            64 * 1_024,
            2,
            Duration::from_millis(100),
            16,
            ConnectionEpoch::new(1),
        )
        .unwrap();
        PmAuthenticatedUserWsRole::from_external_authority(
            PmUserWsConfig::production(condition, bounds).unwrap(),
            Box::new(UnusedUserAuthority),
        )
    }

    #[cfg(target_os = "linux")]
    fn fixed_peer(ip: &str) -> PmFixedTlsPeerSelection {
        PmFixedTlsPeerSelection::production(PRODUCTION_WS_DNS_NAME, ip).unwrap()
    }

    #[cfg(target_os = "linux")]
    fn local_v4() -> PmLocalEgressSelection {
        PmLocalEgressSelection::production("pm-tunnel0", "192.0.2.10".parse().unwrap()).unwrap()
    }

    #[test]
    fn selected_socket_facts_are_copy_read_only_and_non_authoritative() {
        let local = "192.0.2.10:43210".parse().unwrap();
        let peer = "8.8.8.8:443".parse().unwrap();
        let facts = PmSelectedWsSocketFacts::from_verified_socket(b"pm-tunnel0", local, peer)
            .expect("valid verified facts");
        assert_eq!(facts.interface_name(), "pm-tunnel0");
        assert_eq!(facts.local_addr(), local);
        assert_eq!(facts.peer_addr(), peer);
        assert!(!facts.production_order_entry_authorized());
        assert_eq!(
            format!("{facts:?}"),
            "PmSelectedWsSocketFacts(<non-authoritative; device/local/peer redacted>)"
        );
        assert!(PmSelectedWsSocketFacts::from_verified_socket(b"", local, peer).is_none());
        assert!(
            PmSelectedWsSocketFacts::from_verified_socket(b"interface-name-too-long", local, peer)
                .is_none()
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn paired_owner_requires_one_condition_and_one_exact_binding() {
        let exact_condition = condition(0x11);
        let owner = PmProductionSelectedWsOwner::new(
            public_role(exact_condition),
            user_role(exact_condition),
            fixed_peer("8.8.8.8"),
            local_v4(),
        )
        .unwrap();
        assert!(!owner.production_order_entry_authorized());
        assert_eq!(
            format!("{owner:?}"),
            "PmProductionSelectedWsOwner(<non-authoritative; paired selected WebSockets>)"
        );
        let (public, user) = owner.into_roles();
        assert_eq!(public.scope().condition(), exact_condition);
        assert_eq!(user.condition(), exact_condition);
        assert!(!public.production_order_entry_authorized());
        assert!(!user.production_order_entry_authorized());

        assert!(
            PmProductionSelectedWsOwner::new(
                public_role(condition(0x31)),
                user_role(condition(0x32)),
                fixed_peer("8.8.8.8"),
                local_v4(),
            )
            .is_err()
        );
        assert!(
            PmProductionSelectedWsOwner::new(
                public_role(exact_condition),
                user_role(exact_condition),
                PmFixedTlsPeerSelection::production("clob.polymarket.com", "8.8.8.8").unwrap(),
                local_v4(),
            )
            .is_err()
        );
        assert!(
            PmProductionSelectedWsOwner::new(
                public_role(exact_condition),
                user_role(exact_condition),
                fixed_peer("2606:4700:4700::1111"),
                local_v4(),
            )
            .is_err()
        );
        assert!(
            PmProductionSelectedWsOwner::new(
                public_role(exact_condition),
                user_role(exact_condition),
                PmFixedTlsPeerSelection::loopback_evidence(
                    "ws-subscriptions-clob.polymarket.test",
                    "127.0.0.1:443".parse().unwrap(),
                )
                .unwrap(),
                local_v4(),
            )
            .is_err()
        );
    }
}
