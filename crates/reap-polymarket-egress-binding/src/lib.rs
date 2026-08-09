//! Non-authoritative local socket-selection facts for Polymarket transports.
//!
//! This leaf crate deliberately owns no network client, URL, route, method,
//! credential, runtime generation, network-namespace evidence, public-egress
//! claim, authorization, or dispatch capability. Source crates may borrow one
//! validated value only while privately constructing their purpose-closed
//! transports. This value only names an intended binding; it does not observe
//! or attest any connected socket, DNS path, NAT identity, or the egress used
//! by any connection.

#![forbid(unsafe_code)]

use std::{fmt, net::IpAddr};

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
}
