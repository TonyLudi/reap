use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

const MANIFEST: &str = include_str!("../Cargo.toml");
const LIB: &str = include_str!("../src/lib.rs");
const CONFIG: &str = include_str!("../src/config.rs");
const DECIMAL: &str = include_str!("../src/decimal.rs");
const ERROR: &str = include_str!("../src/error.rs");
const POSITION: &str = include_str!("../src/position.rs");
const SOURCE: &str = include_str!("../src/source.rs");

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
fn dependency_surface_is_public_read_only_and_credential_free() {
    for forbidden in [
        "reap-polymarket-auth",
        "reap-polymarket-live-adapter",
        "reap-pm-live",
        "hmac",
        "k256",
        "zeroize",
    ] {
        assert!(
            !MANIFEST.contains(forbidden),
            "forbidden dependency: {forbidden}"
        );
    }
    for required in [
        "reap-pm-core.workspace = true",
        "reap-polymarket-egress-binding.workspace = true",
        "reqwest.workspace = true",
        "serde.workspace = true",
        "serde_json.workspace = true",
        "sha2.workspace = true",
    ] {
        assert!(
            MANIFEST.contains(required),
            "missing dependency: {required}"
        );
    }
}

#[test]
fn production_origin_route_query_and_transport_policy_are_closed() {
    for required in [
        "https://data-api.polymarket.com",
        "url.set_path(\"/positions\")",
        ".append_pair(\"user\"",
        ".append_pair(\"market\"",
        ".append_pair(\"sizeThreshold\", \"0\")",
        ".append_pair(\"limit\", \"500\")",
        ".append_pair(\"sortBy\", \"TOKENS\")",
        ".append_pair(\"sortDirection\", \"DESC\")",
        ".redirect(Policy::none())",
        ".retry(reqwest::retry::never())",
        ".no_proxy()",
        ".interface(local_egress.interface_name())",
        ".local_address(local_egress.local_source_ip())",
        ".header(ACCEPT, \"application/json\")",
        ".header(ACCEPT_ENCODING, \"identity\")",
        "MAX_POSITION_PAGE_BODY_BYTES",
        "FullPageAtOffsetCap",
    ] {
        assert!(
            CONFIG.contains(required) || SOURCE.contains(required),
            "missing closed source property: {required}"
        );
    }
    assert_eq!(SOURCE.matches(".get(url)").count(), 1);
    for forbidden in [
        ".post(",
        ".delete(",
        ".put(",
        ".patch(",
        "pub fn request(",
        "pub fn client(",
        "pub fn origin(",
        "pub fn route(",
        "pub fn query(",
        "/order",
        "POLY_",
    ] {
        assert!(
            !SOURCE.contains(forbidden),
            "capability escape: {forbidden}"
        );
    }
}

#[test]
fn selected_local_egress_is_configuration_only_and_preserves_the_fixed_get_surface() {
    for required in [
        "pub fn production_on_selected_local_egress(",
        "local_egress: &PmLocalEgressSelection",
        "PmDataApiPositionConfig::production_on_selected_local_egress(",
        "local_egress.require_production()?;",
        "local_egress.require_loopback_evidence()?;",
        ".interface(local_egress.interface_name())",
        ".local_address(local_egress.local_source_ip())",
        "#[cfg(target_os = \"linux\")]",
        "SelectedLocalEgressUnsupported",
    ] {
        assert!(
            CONFIG.contains(required) || SOURCE.contains(required) || ERROR.contains(required),
            "missing selected source pin: {required}"
        );
    }
    for forbidden in [
        "pub fn client(",
        "pub fn request(",
        "pub fn local_egress(",
        "PmSelectedEgressGeoblockObservation",
        "production_order_entry_authorized: true",
    ] {
        assert!(
            !CONFIG.contains(forbidden) && !SOURCE.contains(forbidden),
            "selected egress escape: {forbidden}"
        );
    }
}

#[test]
fn position_numbers_never_cross_binary_floating_point() {
    for source in [DECIMAL, POSITION, SOURCE] {
        for forbidden in ["f32", "f64", "as_f64", "from_f64"] {
            assert!(
                !source.contains(forbidden),
                "floating-point escape: {forbidden}"
            );
        }
    }
    for required in [
        "Box<RawValue>",
        "PmExactPositionDecimal",
        "coefficient: U256",
        "decimal_exponent: i16",
        "PmTokenId::new(units)",
        "if asset == opposite_asset",
        "if outcome_index > 1",
        "outcome: Box<str>",
        "opposite_outcome: Box<str>",
    ] {
        assert!(
            DECIMAL.contains(required) || POSITION.contains(required),
            "missing exact numeric boundary: {required}"
        );
    }
}

#[test]
fn observation_language_and_authority_remain_narrow() {
    assert!(LIB.contains("PRODUCTION_ORDER_ENTRY_AUTHORIZED: bool = false"));
    assert!(!LIB.contains("PRODUCTION_ORDER_ENTRY_AUTHORIZED: bool = true"));
    for required in [
        "PmMonitoredPositionObservation",
        "PmConfiguredTokenPosition",
        "PmDataApiPositionObservationCommitment",
        "PmDataApiReceiveClockObservation",
        "Absent",
        "Present(Box<PmDataApiPositionEvidence>)",
        "neither funder-wide inventory",
        "nor authority to sell",
    ] {
        assert!(
            POSITION.contains(required),
            "missing limitation: {required}"
        );
    }
    for forbidden in [
        "SellAuthorized",
        "AtomicComplete",
        "atomic_complete: true",
        "sell_authorized: true",
        "production_order_entry_authorized: true",
    ] {
        assert!(
            !POSITION.contains(forbidden),
            "false authority: {forbidden}"
        );
    }
}

#[test]
fn provenance_is_source_computed_secret_free_and_non_authoritative() {
    for required in [
        "reap.polymarket.public-source.position-observation.v1\\0",
        "PositionObservationCommitmentBuilder::new(",
        "commitment.observe_page(offset, &body, &rows, received_clock)",
        "update_position_evidence",
        "received_clock.unix_milliseconds().to_be_bytes()",
        "PmDataApiPositionObservationCommitment::from_source_bytes(",
        "pub const fn commitment(&self) -> PmDataApiPositionObservationCommitment",
    ] {
        assert!(
            POSITION.contains(required) || SOURCE.contains(required),
            "missing provenance boundary: {required}"
        );
    }
    assert!(POSITION.contains("pub(crate) const fn from_source_bytes("));
    for forbidden in [
        "pub const fn from_source_bytes(",
        "pub fn from_source_bytes(",
        "pub fn raw_body(",
        "pub fn response_body(",
        "commitment: [u8; 32]",
        "production_order_entry_authorized(self) -> bool {\n        true",
    ] {
        assert!(
            !POSITION.contains(forbidden) && !SOURCE.contains(forbidden),
            "provenance capability escape: {forbidden}"
        );
    }
}

#[test]
fn arbitrary_origin_is_compile_excluded_outside_unit_tests() {
    assert!(CONFIG.contains("#[cfg(test)]\n    pub(crate) fn numeric_loopback_evidence("));
    assert!(SOURCE.contains("#[cfg(test)]\n    fn numeric_loopback_evidence("));
    assert!(CONFIG.contains("host.parse::<IpAddr>()"));
    assert!(CONFIG.contains("address.is_loopback()"));
    assert!(!MANIFEST.contains("[features]"));
    assert!(MANIFEST.contains(
        "reap-polymarket-egress-binding = { workspace = true, features = [\"loopback-evidence\"] }"
    ));
}

#[test]
fn production_origin_position_is_move_only_verified_before_io_and_has_no_carrier_escape() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let production = production_prefix(SOURCE);
    for required in [
        "pub enum PmProductionDataApiPositionError",
        "OriginRequired",
        "Source(#[from] PmPublicPositionError)",
        "pub struct PmProductionDataApiPositionObservation",
        "observation: PmMonitoredPositionObservation",
        "_production_origin: ProductionDataApiPositionOrigin",
        "struct ProductionDataApiPositionOrigin;",
        "fn verify(mode: OriginMode) -> Result<Self, PmProductionDataApiPositionError>",
        "OriginMode::Production => Ok(Self)",
        "Err(PmProductionDataApiPositionError::OriginRequired)",
        "pub async fn production_observe_configured_token(",
        "ProductionDataApiPositionOrigin::verify(self.transport.mode)?",
        "PmProductionDataApiPositionObservation::from_source(",
        "impl std::fmt::Debug for PmProductionDataApiPositionObservation",
        "<production-origin; monitored-only; sealed>",
    ] {
        assert!(
            production.contains(required),
            "production Data API origin proof lost {required}"
        );
    }
    assert_eq!(
        LIB.matches("PmProductionDataApiPositionObservation")
            .count(),
        1
    );
    assert_eq!(LIB.matches("PmProductionDataApiPositionError").count(), 1);
    assert!(!ERROR.contains("OriginRequired"));
    assert!(MANIFEST.contains("trybuild.workspace = true"));

    let declaration = between(
        production,
        "pub struct PmProductionDataApiPositionObservation {",
        "\n}\n\nimpl PmProductionDataApiPositionObservation",
    );
    assert_eq!(
        declaration.trim(),
        "observation: PmMonitoredPositionObservation,"
    );
    let wrapper_impl = between(
        production,
        "impl PmProductionDataApiPositionObservation {",
        "\n}\n\nimpl std::fmt::Debug for PmProductionDataApiPositionObservation",
    );
    assert_eq!(
        inherent_function_names(wrapper_impl),
        BTreeSet::from([
            "commitment".to_owned(),
            "completed_clock".to_owned(),
            "configured_token".to_owned(),
            "from_source".to_owned(),
            "pages_observed".to_owned(),
            "rows_observed".to_owned(),
            "scope".to_owned(),
        ])
    );

    let method = between(
        production,
        "pub async fn production_observe_configured_token(",
        "\n\n    pub async fn observe_configured_token(",
    );
    let verify = method
        .find("ProductionDataApiPositionOrigin::verify(self.transport.mode)?")
        .expect("private production-origin verification");
    let fetch = method
        .find("self.observe_configured_token().await?")
        .expect("fixed configured-token read");
    let seal = method
        .find("PmProductionDataApiPositionObservation::from_source(")
        .expect("production position seal");
    assert!(verify < fetch && fetch < seal);
    assert_eq!(
        production
            .matches("PmProductionDataApiPositionObservation::from_source(")
            .count(),
        1
    );

    let declaration_attributes = production
        .split_once("pub struct PmProductionDataApiPositionObservation")
        .unwrap()
        .0
        .rsplit("\n\n")
        .next()
        .unwrap();
    for forbidden in ["Clone", "Copy", "Serialize", "Deserialize"] {
        assert!(!declaration_attributes.contains(forbidden));
        assert!(!production.contains(&format!(
            "{forbidden} for PmProductionDataApiPositionObservation"
        )));
    }
    for forbidden in [
        "impl From<PmMonitoredPositionObservation> for PmProductionDataApiPositionObservation",
        "impl TryFrom<PmMonitoredPositionObservation> for PmProductionDataApiPositionObservation",
        "pub fn from_source(",
        "pub fn observation(&self)",
        "pub const fn observation(&self)",
        "pub fn into_observation(",
        "Deref for PmProductionDataApiPositionObservation",
        "AsRef<PmMonitoredPositionObservation> for PmProductionDataApiPositionObservation",
        "production_order_entry_authorized: true",
    ] {
        assert!(
            !production.contains(forbidden),
            "production Data API proof gained carrier escape {forbidden}"
        );
    }
    for test_pin in [
        "numeric_loopback_source_cannot_issue_production_wrapper_before_io",
        "production_origin_proof_accepts_only_production_mode",
        "production_wrapper_preserves_the_exact_observation_and_redacts_debug",
    ] {
        assert!(SOURCE.contains(test_pin));
    }

    let modules = production_modules(&root);
    for (module, contents) in &modules {
        assert_no_wrapper_trait_conversion(
            module,
            contents,
            "PmProductionDataApiPositionObservation",
        );
        assert_authority_bool_methods_are_denied(module, contents);
        if module == "lib.rs" {
            assert_eq!(
                contents
                    .matches("PmProductionDataApiPositionObservation")
                    .count(),
                1,
                "lib.rs must contain only the exact wrapper re-export"
            );
        } else if module != "source.rs" {
            assert!(
                !contents.contains("PmProductionDataApiPositionObservation"),
                "{module} gained production-wrapper extraction outside source.rs"
            );
        }
    }
}
