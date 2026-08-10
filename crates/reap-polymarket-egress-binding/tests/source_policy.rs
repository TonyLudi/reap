const MANIFEST: &str = include_str!("../Cargo.toml");
const SOURCE: &str = include_str!("../src/lib.rs");

#[test]
fn leaf_binding_has_no_transport_or_authority_dependency() {
    assert!(MANIFEST.contains("default = []"));
    assert!(!MANIFEST.contains("default = [\"loopback-evidence\"]"));
    assert!(!MANIFEST.contains("[dependencies]"));
    for forbidden in [
        "reqwest",
        "tokio",
        "url::",
        "http::",
        "Credential",
        "Authorization",
        "Signature",
        "ClientBuilder",
        "Client",
        "Request",
        "Response",
        "Geoblock",
        "generation:",
        "namespace:",
        "pub fn post",
        "pub fn delete",
        "pub fn get",
        "pub fn execute",
        "pub fn mode(",
        "pub const fn mode(",
        "pub fn is_production(",
        "pub const fn is_production(",
        "production_order_entry_authorized(&self) -> bool {\n        true",
    ] {
        assert!(!SOURCE.contains(forbidden), "binding escape: {forbidden}");
    }
    for required in [
        "pub struct PmLocalEgressSelection",
        "pub struct PmFixedTlsPeerSelection",
        "interface_name: Box<str>",
        "local_source_ip: IpAddr",
        "dns_name: Box<str>",
        "peer_addr: SocketAddr",
        "mode: SelectionMode",
        "enum SelectionMode",
        "mode: FixedTlsPeerMode",
        "enum FixedTlsPeerMode",
        "pub fn require_production(&self)",
        "pub fn require_loopback_evidence(&self)",
        "ProductionSelectionRequired",
        "LoopbackEvidenceSelectionRequired",
        "pub fn production(",
        "#[cfg(any(test, feature = \"loopback-evidence\"))]",
        "pub fn loopback_evidence(",
        "ipv6_embeds_ipv4(address)",
        "pub fn production(dns_name: &str, peer_ip: &str)",
        "parsed_peer_ip.to_string() != peer_ip",
        "is_public_global_unicast(parsed_peer_ip)",
        "SocketAddr::new(parsed_peer_ip, 443)",
        "remaining.last().copied() != Some(\"test\")",
        "pub fn dns_name(&self) -> &str",
        "pub const fn peer_addr(&self) -> SocketAddr",
        "pub const fn require_same_address_family(",
        "AddressFamilyMismatch",
        "(IpAddr::V4(_), IpAddr::V4(_)) | (IpAddr::V6(_), IpAddr::V6(_))",
        "pub const fn production_order_entry_authorized(&self) -> bool",
        "false",
        "do not observe",
        "or attest any connected socket, DNS path, NAT identity",
    ] {
        assert!(
            SOURCE.contains(required),
            "missing binding guard: {required}"
        );
    }
}
