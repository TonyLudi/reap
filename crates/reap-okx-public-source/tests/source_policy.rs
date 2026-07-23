use std::collections::BTreeSet;
use std::path::PathBuf;

#[test]
fn production_modules_and_workspace_edges_are_exactly_bounded() {
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
            "lib.rs".to_string(),
            "public_wire.rs".to_string(),
            "reference.rs".to_string(),
            "session.rs".to_string(),
            "subscription.rs".to_string(),
        ])
    );

    let manifest = std::fs::read_to_string(root.join("Cargo.toml")).unwrap();
    assert!(manifest.contains("reap-core.workspace = true"));
    assert!(manifest.contains("reap-transport.workspace = true"));
    for forbidden in [
        "reap-pm-core",
        "reap-venue",
        "reap-order",
        "reap-okx-wire",
        "reqwest",
        "tungstenite",
    ] {
        assert!(!manifest.contains(forbidden), "{forbidden}");
    }
}

#[test]
fn source_exposes_no_secret_mutation_or_arbitrary_network_surface() {
    let source = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src");
    let forbidden = [
        "api_key",
        "apikey",
        "credential",
        "private",
        "secret",
        "signer",
        "login",
        "place_order",
        "cancel_order",
        "orderintent",
        "connect_async",
        "websocketstream",
        "reqwest",
        "client",
    ];
    for entry in std::fs::read_dir(source).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().and_then(|value| value.to_str()) != Some("rs") {
            continue;
        }
        let text = std::fs::read_to_string(&path).unwrap().to_ascii_lowercase();
        for term in forbidden {
            assert!(
                !text.contains(term),
                "{} contains forbidden term {term}",
                path.display()
            );
        }
    }
}

#[test]
fn only_the_session_module_can_reach_neutral_transport() {
    let source = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src");
    for module in ["public_wire.rs", "reference.rs", "subscription.rs"] {
        let text = std::fs::read_to_string(source.join(module)).unwrap();
        assert!(!text.contains("reap_transport"), "{module}");
    }
    let session = std::fs::read_to_string(source.join("session.rs")).unwrap();
    assert!(session.contains("reap_transport"));
}

#[test]
fn raw_receive_evidence_and_public_exports_cannot_be_detached_or_widened() {
    let source = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src");
    let session = std::fs::read_to_string(source.join("session.rs")).unwrap();
    assert!(session.contains("delivery: RawDelivery"));
    assert!(session.contains("OkxPublicEventEvidence"));
    assert!(session.contains("connection_id: ConnId"));
    assert!(session.contains("envelope.conn_id != self.connection_id"));
    assert!(session.contains("pub struct OkxPublicSessionDelivery"));
    assert!(session.contains("connection_id: ConnId"));
    assert!(session.contains("connection_epoch: u64"));
    assert!(session.contains("local_ingress_sequence: u64"));
    assert!(
        !session.contains(
            "pub type OkxPublicSessionDelivery = ImmutableDelivery<OkxPublicSessionEvent>"
        )
    );
    assert!(session.contains("self.connection_epoch.checked_add(1)"));
    assert!(session.contains("pub const fn requires_reconnect"));
    assert!(session.contains("return Err(OkxPublicSessionError::ReconnectRequired)"));
    assert!(session.contains("pub struct OkxPublicUnavailableOccurrence"));
    assert!(session.contains("pub fn take_unavailable"));
    assert!(session.contains("pub fn invalidate_with_receive_evidence"));
    assert!(session.contains("OkxPublicSessionError::UnavailableOccurrencePending"));
    assert!(session.contains("Result<Duration, OkxPublicSessionError>"));
    assert!(session.contains("pub fn classify_captured_payload"));
    assert!(session.contains("venue: Venue::Okx"));
    assert!(session.contains("conn_id: self.connection_id.clone()"));
    assert!(session.contains("channel: Channel::Custom(OKX_INDEX_TICKERS_CHANNEL.to_string())"));
    assert!(session.contains("symbol: Some(self.subscription.instrument().to_string())"));

    let library = std::fs::read_to_string(source.join("lib.rs")).unwrap();
    assert!(!library.contains("pub use reference::*"));
    assert!(!library.contains("pub use session::*"));
    assert!(library.contains("extract_legacy_index_ticker_fields"));
    assert!(!library.contains("extract_index_ticker_fields"));

    let reference = std::fs::read_to_string(source.join("reference.rs")).unwrap();
    assert!(reference.contains("pub(crate) fn configured_reference_from_wire"));
    assert!(!reference.contains("pub fn configured_reference_from_wire"));
}
