//! Non-authoritative local socket-selection facts for Polymarket transports.
//!
//! This leaf crate deliberately owns no network client, URL, route, method,
//! credential, runtime generation, network-namespace evidence, public-egress
//! claim, authorization, or dispatch capability. Source crates may borrow
//! validated values only while privately constructing their purpose-closed
//! transports. These values only name intended bindings; they do not observe
//! or attest any connected socket, DNS path, NAT identity, or the egress used
//! by any connection.

#![forbid(unsafe_code)]

use std::{
    fmt,
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
};

const LINUX_INTERFACE_NAME_MAX_BYTES: usize = 15;

/// Closed validation errors for one non-authoritative local socket selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PmLocalEgressSelectionError {
    InvalidInterfaceName,
    InvalidLocalSourceIp,
    ScopedIpv6Unsupported,
    ProductionSelectionRequired,
    #[cfg(any(test, feature = "loopback-evidence"))]
    LoopbackEvidenceSelectionRequired,
}

impl fmt::Display for PmLocalEgressSelectionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidInterfaceName => "invalid Linux interface name",
            Self::InvalidLocalSourceIp => "invalid local source IP",
            Self::ScopedIpv6Unsupported => {
                "IPv6 link-local source selection requires an unsupported scope identifier"
            }
            Self::ProductionSelectionRequired => {
                "production transport requires a production local-egress selection"
            }
            #[cfg(any(test, feature = "loopback-evidence"))]
            Self::LoopbackEvidenceSelectionRequired => {
                "loopback evidence transport requires a loopback-evidence local-egress selection"
            }
        })
    }
}

impl std::error::Error for PmLocalEgressSelectionError {}

/// Closed validation errors for one non-authoritative fixed TLS peer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PmFixedTlsPeerSelectionError {
    InvalidDnsName,
    InvalidPeerAddress,
    AddressFamilyMismatch,
    ProductionSelectionRequired,
    #[cfg(any(test, feature = "loopback-evidence"))]
    LoopbackEvidenceSelectionRequired,
}

impl fmt::Display for PmFixedTlsPeerSelectionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidDnsName => "invalid canonical fixed-peer DNS name",
            Self::InvalidPeerAddress => "invalid fixed TLS peer address",
            Self::AddressFamilyMismatch => {
                "fixed TLS peer and selected local source IP families differ"
            }
            Self::ProductionSelectionRequired => {
                "production transport requires a production fixed TLS peer"
            }
            #[cfg(any(test, feature = "loopback-evidence"))]
            Self::LoopbackEvidenceSelectionRequired => {
                "loopback evidence transport requires a loopback-evidence fixed TLS peer"
            }
        })
    }
}

impl std::error::Error for PmFixedTlsPeerSelectionError {}

/// One exact DNS identity and one exact TCP peer for private source setup.
///
/// This cloneable value is reviewed configuration only. It performs no DNS
/// lookup, opens no socket, proves no connected peer, and grants no network or
/// order-entry authority. Production construction fixes port 443 and accepts
/// only a canonical DNS name plus a canonical ordinary global-unicast IP.
#[derive(Clone, PartialEq, Eq)]
pub struct PmFixedTlsPeerSelection {
    dns_name: Box<str>,
    peer_addr: SocketAddr,
    mode: FixedTlsPeerMode,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum FixedTlsPeerMode {
    Production,
    #[cfg(any(test, feature = "loopback-evidence"))]
    LoopbackEvidence,
}

impl PmFixedTlsPeerSelection {
    /// Validate one production DNS identity and exact peer IP at TCP port 443.
    pub fn production(dns_name: &str, peer_ip: &str) -> Result<Self, PmFixedTlsPeerSelectionError> {
        validate_canonical_dns_name(dns_name, false)?;
        let parsed_peer_ip = peer_ip
            .parse::<IpAddr>()
            .map_err(|_| PmFixedTlsPeerSelectionError::InvalidPeerAddress)?;
        if parsed_peer_ip.to_string() != peer_ip || !is_public_global_unicast(parsed_peer_ip) {
            return Err(PmFixedTlsPeerSelectionError::InvalidPeerAddress);
        }
        Ok(Self {
            dns_name: dns_name.into(),
            peer_addr: SocketAddr::new(parsed_peer_ip, 443),
            mode: FixedTlsPeerMode::Production,
        })
    }

    /// Evidence-only fixed hostname and literal loopback peer.
    ///
    /// The hostname must be below the reserved `.test` suffix. This seam is
    /// absent from normal production builds unless explicitly feature-enabled.
    #[cfg(any(test, feature = "loopback-evidence"))]
    pub fn loopback_evidence(
        dns_name: &str,
        peer_addr: SocketAddr,
    ) -> Result<Self, PmFixedTlsPeerSelectionError> {
        validate_canonical_dns_name(dns_name, true)?;
        if peer_addr.port() == 0
            || !peer_addr.ip().is_loopback()
            || matches!(peer_addr, SocketAddr::V6(address) if address.flowinfo() != 0 || address.scope_id() != 0)
        {
            return Err(PmFixedTlsPeerSelectionError::InvalidPeerAddress);
        }
        Ok(Self {
            dns_name: dns_name.into(),
            peer_addr,
            mode: FixedTlsPeerMode::LoopbackEvidence,
        })
    }

    /// Fail closed unless this value came from [`Self::production`].
    pub fn require_production(&self) -> Result<(), PmFixedTlsPeerSelectionError> {
        match self.mode {
            FixedTlsPeerMode::Production => Ok(()),
            #[cfg(any(test, feature = "loopback-evidence"))]
            FixedTlsPeerMode::LoopbackEvidence => {
                Err(PmFixedTlsPeerSelectionError::ProductionSelectionRequired)
            }
        }
    }

    /// Fail closed unless this value came from [`Self::loopback_evidence`].
    #[cfg(any(test, feature = "loopback-evidence"))]
    pub fn require_loopback_evidence(&self) -> Result<(), PmFixedTlsPeerSelectionError> {
        match self.mode {
            FixedTlsPeerMode::LoopbackEvidence => Ok(()),
            FixedTlsPeerMode::Production => {
                Err(PmFixedTlsPeerSelectionError::LoopbackEvidenceSelectionRequired)
            }
        }
    }

    #[must_use]
    pub fn dns_name(&self) -> &str {
        &self.dns_name
    }

    #[must_use]
    pub const fn peer_addr(&self) -> SocketAddr {
        self.peer_addr
    }

    /// Require this peer and one local source selection to use the same IP
    /// family. This is configuration validation, not connected-socket proof.
    pub const fn require_same_address_family(
        &self,
        local_egress: &PmLocalEgressSelection,
    ) -> Result<(), PmFixedTlsPeerSelectionError> {
        if matches!(
            (self.peer_addr.ip(), local_egress.local_source_ip()),
            (IpAddr::V4(_), IpAddr::V4(_)) | (IpAddr::V6(_), IpAddr::V6(_))
        ) {
            Ok(())
        } else {
            Err(PmFixedTlsPeerSelectionError::AddressFamilyMismatch)
        }
    }

    /// Fixed peer configuration can never authorize production order entry.
    #[must_use]
    pub const fn production_order_entry_authorized(&self) -> bool {
        false
    }
}

impl fmt::Debug for PmFixedTlsPeerSelection {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PmFixedTlsPeerSelection(<non-authoritative; fixed-host-and-peer>)")
    }
}

fn validate_canonical_dns_name(
    dns_name: &str,
    require_test_suffix: bool,
) -> Result<(), PmFixedTlsPeerSelectionError> {
    let mut labels = dns_name.split('.');
    let first = labels
        .next()
        .ok_or(PmFixedTlsPeerSelectionError::InvalidDnsName)?;
    let remaining = labels.collect::<Vec<_>>();
    if dns_name.is_empty()
        || dns_name.len() > 253
        || remaining.is_empty()
        || dns_name.parse::<IpAddr>().is_ok()
        || !valid_dns_label(first)
        || remaining.iter().any(|label| !valid_dns_label(label))
        || (require_test_suffix && remaining.last().copied() != Some("test"))
    {
        return Err(PmFixedTlsPeerSelectionError::InvalidDnsName);
    }
    Ok(())
}

fn valid_dns_label(label: &str) -> bool {
    !label.is_empty()
        && label.len() <= 63
        && !label.starts_with('-')
        && !label.ends_with('-')
        && label
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

/// Conservative, explicit ordinary public-global-unicast predicate.
///
/// This deliberately avoids toolchain-dependent `is_global` semantics and
/// rejects private, loopback, link-local, shared, benchmark, documentation,
/// multicast, mapped/compatible IPv6, and other special-purpose ranges.
fn is_public_global_unicast(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(address) => is_public_global_unicast_v4(address),
        IpAddr::V6(address) => is_public_global_unicast_v6(address),
    }
}

fn is_public_global_unicast_v4(address: Ipv4Addr) -> bool {
    let [a, b, c, _] = address.octets();
    !(a == 0
        || a == 10
        || (a == 100 && (64..=127).contains(&b))
        || a == 127
        || (a == 169 && b == 254)
        || (a == 172 && (16..=31).contains(&b))
        || (a == 192 && b == 0 && c == 0)
        || (a == 192 && b == 0 && c == 2)
        || (a == 192 && b == 88 && c == 99)
        || (a == 192 && b == 168)
        || (a == 198 && matches!(b, 18 | 19))
        || (a == 198 && b == 51 && c == 100)
        || (a == 203 && b == 0 && c == 113)
        || a >= 224)
}

fn is_public_global_unicast_v6(address: Ipv6Addr) -> bool {
    let segments = address.segments();
    segments[0] & 0xe000 == 0x2000
        && !(segments[0] == 0x2001 && segments[1] <= 0x01ff)
        && !(segments[0] == 0x2001 && segments[1] == 0x0db8)
        && segments[0] != 0x2002
        && !(segments[0] == 0x3fff && segments[1] & 0xf000 == 0)
}

/// Validated interface name and local source IP configuration for private
/// socket setup.
///
/// This cloneable value is configuration, not provenance or authority. In
/// particular it carries no trusted generation and cannot construct a
/// selected-egress observation or make any network request.
#[derive(Clone, PartialEq, Eq)]
pub struct PmLocalEgressSelection {
    interface_name: Box<str>,
    local_source_ip: IpAddr,
    mode: SelectionMode,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SelectionMode {
    Production,
    #[cfg(any(test, feature = "loopback-evidence"))]
    LoopbackEvidence,
}

impl PmLocalEgressSelection {
    /// Validate a configured production local socket selection.
    pub fn production(
        interface_name: &str,
        local_source_ip: IpAddr,
    ) -> Result<Self, PmLocalEgressSelectionError> {
        validate_interface_name(interface_name)?;
        validate_production_ip(local_source_ip)?;
        Ok(Self {
            interface_name: interface_name.into(),
            local_source_ip,
            mode: SelectionMode::Production,
        })
    }

    /// Test/evidence-only configuration for a literal loopback source IP.
    #[cfg(any(test, feature = "loopback-evidence"))]
    pub fn loopback_evidence(
        interface_name: &str,
        local_source_ip: IpAddr,
    ) -> Result<Self, PmLocalEgressSelectionError> {
        validate_interface_name(interface_name)?;
        if !local_source_ip.is_loopback() {
            return Err(PmLocalEgressSelectionError::InvalidLocalSourceIp);
        }
        Ok(Self {
            interface_name: interface_name.into(),
            local_source_ip,
            mode: SelectionMode::LoopbackEvidence,
        })
    }

    /// Fail closed unless this value came from [`Self::production`].
    ///
    /// This mode check grants no authority and makes no socket observation.
    pub fn require_production(&self) -> Result<(), PmLocalEgressSelectionError> {
        match self.mode {
            SelectionMode::Production => Ok(()),
            #[cfg(any(test, feature = "loopback-evidence"))]
            SelectionMode::LoopbackEvidence => {
                Err(PmLocalEgressSelectionError::ProductionSelectionRequired)
            }
        }
    }

    /// Fail closed unless this value came from [`Self::loopback_evidence`].
    #[cfg(any(test, feature = "loopback-evidence"))]
    pub fn require_loopback_evidence(&self) -> Result<(), PmLocalEgressSelectionError> {
        match self.mode {
            SelectionMode::LoopbackEvidence => Ok(()),
            SelectionMode::Production => {
                Err(PmLocalEgressSelectionError::LoopbackEvidenceSelectionRequired)
            }
        }
    }

    #[must_use]
    pub fn interface_name(&self) -> &str {
        &self.interface_name
    }

    #[must_use]
    pub const fn local_source_ip(&self) -> IpAddr {
        self.local_source_ip
    }

    /// Local socket selection can never authorize production order entry.
    #[must_use]
    pub const fn production_order_entry_authorized(&self) -> bool {
        false
    }
}

impl fmt::Debug for PmLocalEgressSelection {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .write_str("PmLocalEgressSelection(<non-authoritative; interface-and-local-ip-only>)")
    }
}

fn validate_interface_name(interface_name: &str) -> Result<(), PmLocalEgressSelectionError> {
    if interface_name.is_empty()
        || interface_name.len() > LINUX_INTERFACE_NAME_MAX_BYTES
        || interface_name
            .bytes()
            .any(|byte| !byte.is_ascii_alphanumeric() && !matches!(byte, b'_' | b'.' | b'-'))
    {
        return Err(PmLocalEgressSelectionError::InvalidInterfaceName);
    }
    Ok(())
}

fn validate_production_ip(local_source_ip: IpAddr) -> Result<(), PmLocalEgressSelectionError> {
    if local_source_ip.is_unspecified()
        || local_source_ip.is_loopback()
        || local_source_ip.is_multicast()
        || matches!(local_source_ip, IpAddr::V4(address) if address.is_broadcast())
    {
        return Err(PmLocalEgressSelectionError::InvalidLocalSourceIp);
    }
    if matches!(local_source_ip, IpAddr::V6(address) if address.is_unicast_link_local()) {
        return Err(PmLocalEgressSelectionError::ScopedIpv6Unsupported);
    }
    if matches!(local_source_ip, IpAddr::V6(address) if ipv6_embeds_ipv4(address)) {
        return Err(PmLocalEgressSelectionError::InvalidLocalSourceIp);
    }
    Ok(())
}

fn ipv6_embeds_ipv4(address: std::net::Ipv6Addr) -> bool {
    let segments = address.segments();
    segments[..6].iter().all(|segment| *segment == 0)
        || (segments[..5].iter().all(|segment| *segment == 0) && segments[5] == u16::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn production_accepts_only_bounded_interface_and_unicast_source_ip() {
        let selection =
            PmLocalEgressSelection::production("pm-tunnel0", "192.0.2.10".parse().unwrap())
                .unwrap();
        assert_eq!(selection.interface_name(), "pm-tunnel0");
        assert_eq!(
            selection.local_source_ip(),
            "192.0.2.10".parse::<IpAddr>().unwrap()
        );
        assert!(!selection.production_order_entry_authorized());
        assert_eq!(selection.require_production(), Ok(()));
        assert_eq!(
            format!("{selection:?}"),
            "PmLocalEgressSelection(<non-authoritative; interface-and-local-ip-only>)"
        );

        for invalid in ["", "interface-name-too-long", "pm/tunnel", "pm tunnel"] {
            assert_eq!(
                PmLocalEgressSelection::production(invalid, "192.0.2.10".parse().unwrap()),
                Err(PmLocalEgressSelectionError::InvalidInterfaceName)
            );
        }
        for invalid in [
            "0.0.0.0",
            "127.0.0.1",
            "224.0.0.1",
            "255.255.255.255",
            "::127.0.0.1",
            "::ffff:0.0.0.0",
            "::ffff:127.0.0.1",
            "::ffff:192.0.2.10",
        ] {
            assert_eq!(
                PmLocalEgressSelection::production("pm0", invalid.parse().unwrap()),
                Err(PmLocalEgressSelectionError::InvalidLocalSourceIp)
            );
        }
        assert_eq!(
            PmLocalEgressSelection::production("pm0", "fe80::1".parse().unwrap()),
            Err(PmLocalEgressSelectionError::ScopedIpv6Unsupported)
        );
    }

    #[test]
    fn loopback_selection_is_separate_and_feature_closed() {
        let selection =
            PmLocalEgressSelection::loopback_evidence("lo", "127.0.0.1".parse().unwrap()).unwrap();
        assert_eq!(selection.interface_name(), "lo");
        assert_eq!(
            selection.local_source_ip(),
            "127.0.0.1".parse::<IpAddr>().unwrap()
        );
        assert!(!selection.production_order_entry_authorized());
        assert_eq!(selection.require_loopback_evidence(), Ok(()));
        assert_eq!(
            selection.require_production(),
            Err(PmLocalEgressSelectionError::ProductionSelectionRequired)
        );
        assert!(
            PmLocalEgressSelection::loopback_evidence("lo", "192.0.2.10".parse().unwrap()).is_err()
        );
    }

    #[test]
    fn production_and_loopback_modes_cannot_cross() {
        let production =
            PmLocalEgressSelection::production("pm0", "192.0.2.10".parse().unwrap()).unwrap();
        assert_eq!(
            production.require_loopback_evidence(),
            Err(PmLocalEgressSelectionError::LoopbackEvidenceSelectionRequired)
        );
        let loopback =
            PmLocalEgressSelection::loopback_evidence("lo", "127.0.0.2".parse().unwrap()).unwrap();
        assert_eq!(
            loopback.require_production(),
            Err(PmLocalEgressSelectionError::ProductionSelectionRequired)
        );
    }

    #[test]
    fn production_fixed_tls_peer_is_exact_canonical_and_non_authoritative() {
        let selection = PmFixedTlsPeerSelection::production("polygon.drpc.org", "8.8.8.8").unwrap();
        assert_eq!(selection.dns_name(), "polygon.drpc.org");
        assert_eq!(selection.peer_addr(), "8.8.8.8:443".parse().unwrap());
        assert_eq!(selection.require_production(), Ok(()));
        let ipv4_local =
            PmLocalEgressSelection::production("pm0", "192.0.2.10".parse().unwrap()).unwrap();
        assert_eq!(selection.require_same_address_family(&ipv4_local), Ok(()));
        let ipv6_local =
            PmLocalEgressSelection::production("pm0", "2001:4860:4860::8844".parse().unwrap())
                .unwrap();
        assert_eq!(
            selection.require_same_address_family(&ipv6_local),
            Err(PmFixedTlsPeerSelectionError::AddressFamilyMismatch)
        );
        assert!(!selection.production_order_entry_authorized());
        assert_eq!(
            format!("{selection:?}"),
            "PmFixedTlsPeerSelection(<non-authoritative; fixed-host-and-peer>)"
        );

        for invalid in [
            "polygon",
            "Polygon.drpc.org",
            "polygon.drpc.org.",
            "polygon..drpc.org",
            "-polygon.drpc.org",
            "polygon-.drpc.org",
            "polygon_drpc.org",
            "127.0.0.1",
        ] {
            assert_eq!(
                PmFixedTlsPeerSelection::production(invalid, "8.8.8.8"),
                Err(PmFixedTlsPeerSelectionError::InvalidDnsName),
                "accepted DNS name {invalid}"
            );
        }

        for invalid in [
            "0.0.0.0",
            "10.0.0.1",
            "100.64.0.1",
            "127.0.0.1",
            "169.254.0.1",
            "172.16.0.1",
            "192.0.2.1",
            "192.168.0.1",
            "198.18.0.1",
            "198.51.100.1",
            "203.0.113.1",
            "224.0.0.1",
            "255.255.255.255",
            "8.8.8.08",
            "::1",
            "::8.8.8.8",
            "::ffff:8.8.8.8",
            "2001:db8::1",
            "2001:4860:4860:0:0:0:0:8888",
        ] {
            assert_eq!(
                PmFixedTlsPeerSelection::production("polygon.drpc.org", invalid),
                Err(PmFixedTlsPeerSelectionError::InvalidPeerAddress),
                "accepted peer IP {invalid}"
            );
        }

        let ipv6 = PmFixedTlsPeerSelection::production("polygon.drpc.org", "2001:4860:4860::8888")
            .unwrap();
        assert_eq!(
            ipv6.peer_addr(),
            "[2001:4860:4860::8888]:443".parse().unwrap()
        );
    }

    #[test]
    fn loopback_fixed_peer_is_feature_closed_and_modes_cannot_cross() {
        let loopback = PmFixedTlsPeerSelection::loopback_evidence(
            "polygon-source.test",
            "127.0.0.1:31443".parse().unwrap(),
        )
        .unwrap();
        assert_eq!(loopback.dns_name(), "polygon-source.test");
        assert_eq!(loopback.peer_addr(), "127.0.0.1:31443".parse().unwrap());
        assert_eq!(loopback.require_loopback_evidence(), Ok(()));
        let ipv6_local =
            PmLocalEgressSelection::loopback_evidence("lo", "::1".parse().unwrap()).unwrap();
        assert_eq!(
            loopback.require_same_address_family(&ipv6_local),
            Err(PmFixedTlsPeerSelectionError::AddressFamilyMismatch)
        );
        assert_eq!(
            loopback.require_production(),
            Err(PmFixedTlsPeerSelectionError::ProductionSelectionRequired)
        );
        assert!(!loopback.production_order_entry_authorized());

        let production =
            PmFixedTlsPeerSelection::production("polygon.drpc.org", "8.8.8.8").unwrap();
        assert_eq!(
            production.require_loopback_evidence(),
            Err(PmFixedTlsPeerSelectionError::LoopbackEvidenceSelectionRequired)
        );
        for (dns_name, peer_addr) in [
            ("polygon.example", "127.0.0.1:31443"),
            ("Polygon.test", "127.0.0.1:31443"),
            ("polygon.test", "192.0.2.1:31443"),
            ("polygon.test", "127.0.0.1:0"),
        ] {
            assert!(
                PmFixedTlsPeerSelection::loopback_evidence(dns_name, peer_addr.parse().unwrap())
                    .is_err(),
                "accepted {dns_name} at {peer_addr}"
            );
        }
    }
}
