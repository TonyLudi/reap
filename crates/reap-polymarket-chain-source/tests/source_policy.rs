use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

fn production_prefix(source: &str) -> &str {
    [
        "\n#[cfg(test)]\nmod tests",
        "\n#[cfg(test)]\npub(crate) mod tests",
    ]
    .into_iter()
    .filter_map(|marker| source.find(marker))
    .min()
    .map_or(source, |end| &source[..end])
}

fn between<'a>(source: &'a str, start: &str, end: &str) -> &'a str {
    source
        .split_once(start)
        .and_then(|(_, tail)| tail.split_once(end).map(|(value, _)| value))
        .expect("source-policy markers must exist in order")
}

fn production_modules(root: &Path) -> Vec<(String, String)> {
    let mut modules = std::fs::read_dir(root.join("src"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().and_then(|value| value.to_str()) == Some("rs"))
        .map(|path| {
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            let source = std::fs::read_to_string(path).unwrap();
            (name, production_prefix(&source).to_owned())
        })
        .collect::<Vec<_>>();
    modules.sort_by(|left, right| left.0.cmp(&right.0));
    modules
}

fn inherent_function_names(source: &str) -> BTreeSet<String> {
    source
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            (!line.starts_with("//")).then(|| {
                line.split_once("fn ")
                    .and_then(|(_, tail)| tail.split_once('(').map(|(name, _)| name.to_owned()))
            })?
        })
        .collect()
}

fn assert_no_wrapper_trait_conversion(module: &str, source: &str, wrapper: &str) {
    let normalized = source.split_whitespace().collect::<Vec<_>>().join(" ");
    let forbidden_trait_heads = [
        "From<",
        "Into<",
        "TryFrom<",
        "TryInto<",
        "AsRef<",
        "Deref",
        "std::convert::From<",
        "std::convert::Into<",
        "std::convert::TryFrom<",
        "std::convert::TryInto<",
        "std::convert::AsRef<",
        "core::convert::From<",
        "core::convert::Into<",
        "core::convert::TryFrom<",
        "core::convert::TryInto<",
        "core::convert::AsRef<",
        "std::ops::Deref",
        "core::ops::Deref",
    ];
    let allowed_inherent = format!("impl{wrapper}");
    let allowed_debug = format!("implstd::fmt::Debugfor{wrapper}");
    let mut remaining = normalized.as_str();
    while let Some(index) = remaining.find("impl") {
        let candidate = &remaining[index..];
        let after_impl = &candidate["impl".len()..];
        if after_impl.starts_with(' ') || after_impl.starts_with('<') {
            let header = candidate
                .split_once('{')
                .map_or(candidate, |(head, _)| head);
            if header.contains(wrapper) {
                let compact = header
                    .chars()
                    .filter(|value| !value.is_whitespace())
                    .collect::<String>();
                for forbidden in forbidden_trait_heads {
                    assert!(
                        !compact.contains(forbidden),
                        "{module} gained wrapper conversion or dereference {forbidden}: {header}"
                    );
                }
                assert!(
                    compact == allowed_inherent || compact == allowed_debug,
                    "{module} gained an unreviewed wrapper trait implementation: {header}"
                );
            }
        }
        remaining = after_impl;
    }
}

fn assert_authority_bool_methods_are_denied(module: &str, source: &str) {
    let mut remaining = source;
    while let Some(index) = remaining.find("fn ") {
        let function = &remaining[index + 3..];
        let Some((name, _)) = function.split_once('(') else {
            break;
        };
        let Some(open_brace) = function.find('{') else {
            break;
        };
        if let Some(semicolon) = function.find(';')
            && semicolon < open_brace
        {
            remaining = &function[semicolon + 1..];
            continue;
        }
        let signature = &function[..open_brace];
        if signature.contains("-> bool")
            && [
                "authoriz",
                "permit",
                "allowed",
                "can_place",
                "can_cancel",
                "can_dispatch",
                "can_send",
                "can_mutate",
            ]
            .iter()
            .any(|keyword| name.contains(keyword))
        {
            let body = function[open_brace + 1..]
                .split_once('}')
                .map_or("", |(body, _)| body)
                .split_whitespace()
                .collect::<String>();
            assert_eq!(
                body, "false",
                "{module} contains a positive or non-constant authority method {name}"
            );
        }
        remaining = &function[open_brace + 1..];
    }
}

#[test]
fn production_module_surface_is_closed_and_exact() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let source = root.join("src");
    let modules = std::fs::read_dir(&source)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().and_then(|value| value.to_str()) == Some("rs"))
        .map(|path| path.file_name().unwrap().to_string_lossy().into_owned())
        .collect::<BTreeSet<_>>();
    assert_eq!(
        modules,
        BTreeSet::from([
            "contract.rs".to_string(),
            "lib.rs".to_string(),
            "rpc.rs".to_string(),
            "source.rs".to_string(),
        ])
    );

    let library = std::fs::read_to_string(source.join("lib.rs")).unwrap();
    assert!(library.contains("#![forbid(unsafe_code)]"));
    assert!(!library.contains("pub use rpc"));
    assert!(!library.contains("PmPolygonRpcTransport"));
}

#[test]
fn rpc_method_contract_is_the_exact_five_request_read_only_cut() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src");
    let rpc = std::fs::read_to_string(root.join("rpc.rs")).unwrap();
    assert_eq!(rpc.matches(r#""eth_chainId""#).count(), 1);
    assert_eq!(rpc.matches(r#""eth_getBlockByNumber""#).count(), 1);
    assert_eq!(rpc.matches(r#""eth_call""#).count(), 1);
    assert_eq!(rpc.matches(r#""finalized""#).count(), 1);
    assert_eq!(rpc.matches(r#""dd62ed3e""#).count(), 1);
    assert_eq!(rpc.matches(r#""e985e9c5""#).count(), 1);
    assert!(rpc.contains("id: CHAIN_ID_REQUEST_ID"));
    assert!(rpc.contains("id: FINALIZED_BLOCK_REQUEST_ID"));
    assert!(rpc.contains("id: BLOCK_REREAD_REQUEST_ID"));
    assert!(rpc.contains("PUSD_ALLOWANCE_REQUEST_ID"));
    assert!(rpc.contains("CONDITIONAL_TOKENS_APPROVAL_REQUEST_ID"));

    for forbidden in [
        "eth_getBalance",
        "eth_sendTransaction",
        "eth_sendRawTransaction",
        "eth_estimateGas",
        "eth_getTransactionCount",
        "personal_sign",
        "eth_sign",
        "wallet_",
        "batch",
    ] {
        assert!(
            !rpc.contains(forbidden),
            "forbidden RPC surface {forbidden}"
        );
    }
    assert!(!rpc.contains("pub fn eth_call_request"));
    assert!(!rpc.contains("pub(crate) fn eth_call_request"));
}

#[test]
fn origin_transport_and_clock_policy_cannot_be_widened_silently() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src");
    let contract = std::fs::read_to_string(root.join("contract.rs")).unwrap();
    let source = std::fs::read_to_string(root.join("source.rs")).unwrap();

    assert_eq!(
        contract.matches("https://polygon.drpc.org").count(),
        1,
        "one fixed production origin"
    );
    assert!(source.contains("pub fn production()"));
    assert_eq!(source.matches("pub fn production()").count(), 1);
    assert!(source.contains("pub fn production_on_selected_local_egress("));
    assert!(source.contains("pub fn production_on_fixed_tls_peer_and_selected_local_egress("));
    assert!(source.contains("local_egress: &PmLocalEgressSelection"));
    assert!(source.contains("local_egress.require_production()?;"));
    assert!(source.contains("local_egress.require_loopback_evidence()?;"));
    assert!(source.contains("#[cfg(any(test, feature = \"loopback-evidence\"))]"));
    assert!(source.contains("pub fn loopback_evidence("));
    assert!(
        source.contains("pub fn loopback_evidence_on_fixed_tls_peer_and_selected_local_egress(")
    );
    assert!(source.contains("Host::Ipv4"));
    assert!(source.contains("Host::Ipv6"));
    assert!(source.contains("ip.is_loopback()"));
    assert!(source.contains(".redirect(Policy::none())"));
    assert!(source.contains(".retry(reqwest::retry::never())"));
    assert!(source.contains(".no_proxy()"));
    assert!(source.contains(".interface(local_egress.interface_name())"));
    assert!(source.contains(".local_address(local_egress.local_source_ip())"));
    assert!(source.contains("builder.resolve(fixed_peer.dns_name(), fixed_peer.peer_addr())"));
    assert!(source.contains("response.remote_addr() != Some(expected_peer)"));
    assert!(source.contains("#[cfg(target_os = \"linux\")]"));
    assert!(source.contains("SelectedLocalEgressUnsupported"));
    assert!(source.contains("builder.https_only(true)"));
    assert!(source.contains("MAX_JSON_RPC_RESPONSE_BYTES"));
    assert!(source.contains("ClockSource::System"));
    assert!(source.contains("MAX_FINALIZED_BLOCK_AGE_SECONDS: u64 = 30"));
    assert!(source.contains("MAX_FINALIZED_BLOCK_FUTURE_SECONDS: u64 = 5"));
}

#[test]
fn selected_local_egress_is_configuration_only_and_preserves_the_fixed_rpc_surface() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let manifest = std::fs::read_to_string(root.join("Cargo.toml")).unwrap();
    let source = std::fs::read_to_string(root.join("src/source.rs")).unwrap();
    assert!(manifest.contains("reap-polymarket-egress-binding.workspace = true"));
    for required in [
        "Self::production_with_local_egress(None)",
        "Self::production_with_local_egress(Some(local_egress))",
        "local_egress.require_production()?;",
        "local_egress.require_loopback_evidence()?;",
        "PmPolygonRpcTransport::build(",
        "pub fn production_on_fixed_tls_peer_and_selected_local_egress(",
        "pub fn loopback_evidence_on_fixed_tls_peer_and_selected_local_egress(",
        "fixed_peer: &PmFixedTlsPeerSelection",
        "fixed_peer.require_production()?;",
        "fixed_peer.require_loopback_evidence()?;",
        "fixed_peer.require_same_address_family(local_egress)?;",
        "origin.host_str() != Some(fixed_peer.dns_name())",
        ".interface(local_egress.interface_name())",
        ".local_address(local_egress.local_source_ip())",
        ".resolve(fixed_peer.dns_name(), fixed_peer.peer_addr())",
        "expected_peer: Option<std::net::SocketAddr>",
        "response.remote_addr() != Some(expected_peer)",
        "return Err(PmPolygonChainSourceError::RequestFailed)",
        "pub enum PmPolygonFixedPeerSourceError",
        "FixedTlsPeerSelection(#[from] PmFixedTlsPeerSelectionError)",
        "LocalEgressSelection(#[from] PmLocalEgressSelectionError)",
        "DnsNameMismatch",
        "Source(#[from] PmPolygonChainSourceError)",
    ] {
        assert!(
            source.contains(required),
            "missing selected source pin: {required}"
        );
    }
    for forbidden in [
        "pub fn client(",
        "pub fn request(",
        "pub fn post(",
        "pub fn local_egress(",
        "pub fn fixed_peer(",
        ".resolve_to_addrs(",
        "PmSelectedEgressGeoblockObservation",
        "production_order_entry_authorized: true",
    ] {
        assert!(
            !source.contains(forbidden),
            "selected egress escape: {forbidden}"
        );
    }

    let raw_error = between(
        &source,
        "pub enum PmPolygonChainSourceError {",
        "\n}\n\n/// Construction errors confined",
    );
    for fixed_peer_only in [
        "FixedTlsPeerSelection",
        "DnsNameMismatch",
        "RemotePeerMismatch",
    ] {
        assert!(
            !raw_error.contains(fixed_peer_only),
            "raw Polygon error enum expanded with {fixed_peer_only}"
        );
    }

    let source_tests = std::fs::read_to_string(root.join("src/source/tests.rs")).unwrap();
    for required in [
        "fixed_peer_keeps_hostname_source_ip_exact_peer_and_avoids_decoy",
        "fixed_peer_response_rejects_a_different_expected_remote_socket",
        "127.0.0.2",
        "polygon-source.test",
        "decoy.accept()",
        "PmFixedTlsPeerSelectionError::AddressFamilyMismatch",
    ] {
        assert!(
            source_tests.contains(required),
            "missing dynamic fixed-peer pin: {required}"
        );
    }
}

#[test]
fn production_origin_cut_is_move_only_verified_before_io_and_has_no_carrier_escape() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let source = std::fs::read_to_string(root.join("src/source.rs")).unwrap();
    let source_tests = std::fs::read_to_string(root.join("src/source/tests.rs")).unwrap();
    let library = std::fs::read_to_string(root.join("src/lib.rs")).unwrap();
    let manifest = std::fs::read_to_string(root.join("Cargo.toml")).unwrap();
    let production = production_prefix(&source);

    for required in [
        "pub struct PmProductionPolygonFinalizedAuthorizationCut",
        "cut: PmPolygonFinalizedAuthorizationCut",
        "pub enum PmProductionPolygonFinalizedAuthorizationError",
        "OriginRequired",
        "Source(#[from] PmPolygonChainSourceError)",
        "_production_origin: ProductionPolygonOrigin",
        "struct ProductionPolygonOrigin;",
        ") -> Result<Self, PmProductionPolygonFinalizedAuthorizationError>",
        "SourceMode::Production => Ok(Self)",
        "SourceMode::LoopbackEvidence =>",
        "Err(PmProductionPolygonFinalizedAuthorizationError::OriginRequired)",
        "pub async fn production_finalized_authorization_cut(",
        "let production_origin = ProductionPolygonOrigin::verify(self.mode)?;",
        "PmProductionPolygonFinalizedAuthorizationCut::from_source(",
        "impl std::fmt::Debug for PmProductionPolygonFinalizedAuthorizationCut",
        "<production-origin; read-only; sealed>",
    ] {
        assert!(
            production.contains(required),
            "production Polygon origin proof lost {required}"
        );
    }
    assert_eq!(
        library
            .matches("PmProductionPolygonFinalizedAuthorizationCut")
            .count(),
        1
    );
    assert_eq!(
        library
            .matches("PmProductionPolygonFinalizedAuthorizationError")
            .count(),
        1
    );
    assert!(manifest.contains("trybuild.workspace = true"));
    let raw_error = between(
        production,
        "pub enum PmPolygonChainSourceError {",
        "\n}\n\n/// Errors specific to obtaining a production-origin proof",
    );
    assert!(!raw_error.contains("OriginRequired"));

    let declaration = between(
        production,
        "pub struct PmProductionPolygonFinalizedAuthorizationCut {",
        "\n}\n\nimpl PmProductionPolygonFinalizedAuthorizationCut",
    );
    assert_eq!(
        declaration.trim(),
        "cut: PmPolygonFinalizedAuthorizationCut,"
    );
    let wrapper_impl = between(
        production,
        "impl PmProductionPolygonFinalizedAuthorizationCut {",
        "\n}\n\nimpl std::fmt::Debug for PmProductionPolygonFinalizedAuthorizationCut",
    );
    assert_eq!(
        inherent_function_names(wrapper_impl),
        BTreeSet::from([
            "block".to_owned(),
            "commitment".to_owned(),
            "conditional_tokens_approval".to_owned(),
            "from_source".to_owned(),
            "observed_clock".to_owned(),
            "pusd_allowance".to_owned(),
            "scope".to_owned(),
        ])
    );

    let method = between(
        production,
        "pub async fn production_finalized_authorization_cut(",
        "\n\n    /// Reads the exact five-request cut",
    );
    let verify = method
        .find("ProductionPolygonOrigin::verify(self.mode)?")
        .expect("private production-origin verification");
    let fetch = method
        .find("self.finalized_authorization_cut(scope).await?")
        .expect("fixed finalized authorization read");
    let seal = method
        .find("PmProductionPolygonFinalizedAuthorizationCut::from_source(")
        .expect("production cut seal");
    assert!(verify < fetch && fetch < seal);
    assert_eq!(
        production
            .matches("PmProductionPolygonFinalizedAuthorizationCut::from_source(")
            .count(),
        1
    );

    let declaration_attributes = production
        .split_once("pub struct PmProductionPolygonFinalizedAuthorizationCut")
        .unwrap()
        .0
        .rsplit("\n\n")
        .next()
        .unwrap();
    for forbidden in ["Clone", "Copy", "Serialize", "Deserialize"] {
        assert!(!declaration_attributes.contains(forbidden));
        assert!(!production.contains(&format!(
            "{forbidden} for PmProductionPolygonFinalizedAuthorizationCut"
        )));
    }
    for forbidden in [
        "impl From<PmPolygonFinalizedAuthorizationCut> for PmProductionPolygonFinalizedAuthorizationCut",
        "impl TryFrom<PmPolygonFinalizedAuthorizationCut> for PmProductionPolygonFinalizedAuthorizationCut",
        "pub fn from_source(",
        "pub fn cut(&self)",
        "pub const fn cut(&self)",
        "pub fn into_cut(",
        "Deref for PmProductionPolygonFinalizedAuthorizationCut",
        "AsRef<PmPolygonFinalizedAuthorizationCut> for PmProductionPolygonFinalizedAuthorizationCut",
        "production_order_entry_authorized: true",
    ] {
        assert!(
            !production.contains(forbidden),
            "production Polygon proof gained carrier escape {forbidden}"
        );
    }
    for test_pin in [
        "loopback_source_cannot_issue_production_wrapper_before_io",
        "production_origin_proof_accepts_only_production_mode",
        "production_wrapper_preserves_the_exact_cut_and_redacts_debug",
    ] {
        assert!(source_tests.contains(test_pin));
    }

    let modules = production_modules(&root);
    for (module, contents) in &modules {
        assert_no_wrapper_trait_conversion(
            module,
            contents,
            "PmProductionPolygonFinalizedAuthorizationCut",
        );
        assert_authority_bool_methods_are_denied(module, contents);
        if module == "lib.rs" {
            assert_eq!(
                contents
                    .matches("PmProductionPolygonFinalizedAuthorizationCut")
                    .count(),
                1,
                "lib.rs must contain only the exact wrapper re-export"
            );
        } else if module != "source.rs" {
            assert!(
                !contents.contains("PmProductionPolygonFinalizedAuthorizationCut"),
                "{module} gained production-wrapper extraction outside source.rs"
            );
        }
    }
}

#[test]
fn contracts_owner_spenders_and_assets_are_frozen() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src");
    let contract = std::fs::read_to_string(root.join("contract.rs")).unwrap();
    let source = std::fs::read_to_string(root.join("source.rs")).unwrap();

    for exact in [
        "0xE111180000d2663C0091e4f400237545B87B996B",
        "0xe2222d279d744050d28e00520010520000310F59",
        "0xC011a7E12a19f7B1f670d46F03B03f3342E82DFB",
        "0x4D97DCd97eC945f40cF65F87097ACe5EA0476045",
    ] {
        assert_eq!(contract.matches(exact).count(), 1, "{exact}");
    }
    assert!(contract.contains("self.account_scope.funder().address()"));
    assert!(contract.contains("not independent signature-type attestation"));
    assert!(contract.contains("outer connectivity/preflight contract"));
    assert!(contract.contains("Self::StandardV2"));
    assert!(contract.contains("Self::NegativeRiskV2"));
    assert!(source.contains("let owner = scope.owner();"));
    assert!(source.contains("let spender = scope.spender().address();"));
    assert!(!source.contains("getBalance"));
    assert!(!source.contains("sendTransaction"));
    assert!(contract.contains("production_order_entry_authorized"));
    assert!(contract.contains("false"));
}

#[test]
fn provenance_is_source_computed_exact_and_non_authoritative() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src");
    let contract = std::fs::read_to_string(root.join("contract.rs")).unwrap();
    let source = std::fs::read_to_string(root.join("source.rs")).unwrap();

    for required in [
        "reap.polymarket.chain-source.finalized-authorization-cut.v1\\0",
        "AUTHORIZATION_CUT_REQUEST_COUNT: u8 = 5",
        "AuthorizationCommitmentBasis {",
        "response_bodies:",
        "update_rpc_observation(",
        "update_block(&mut hasher, basis.finalized)",
        "basis.pusd_allowance.to_be_bytes()",
        "basis.conditional_tokens_approval.is_approved()",
        "basis.observed_clock.unix_seconds().to_be_bytes()",
        "PmPolygonFinalizedAuthorizationCommitment::from_source_bytes(",
    ] {
        assert!(
            source.contains(required),
            "missing provenance binding: {required}"
        );
    }
    assert!(contract.contains("pub(crate) const fn from_source_bytes("));
    assert!(
        contract
            .contains("pub const fn commitment(self) -> PmPolygonFinalizedAuthorizationCommitment")
    );
    for forbidden in [
        "pub const fn from_source_bytes(",
        "pub fn from_source_bytes(",
        "pub fn raw_body(",
        "pub fn response_body(",
        "commitment: [u8; 32]",
        "production_order_entry_authorized(self) -> bool {\n        true",
    ] {
        assert!(
            !contract.contains(forbidden) && !source.contains(forbidden),
            "provenance capability escape: {forbidden}"
        );
    }
}
