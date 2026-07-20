use std::collections::BTreeSet;
use std::path::Path;

#[test]
fn transport_has_only_the_frozen_modules_and_no_venue_or_authority_protocol() {
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let modules = std::fs::read_dir(&source)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().and_then(|value| value.to_str()) == Some("rs"))
        .map(|path| path.file_name().unwrap().to_string_lossy().into_owned())
        .collect::<BTreeSet<_>>();
    assert_eq!(
        modules,
        BTreeSet::from([
            "backoff.rs".to_string(),
            "bounded.rs".to_string(),
            "health.rs".to_string(),
            "lib.rs".to_string(),
            "shutdown.rs".to_string(),
            "supervisor.rs".to_string(),
        ])
    );

    for entry in modules {
        let text = std::fs::read_to_string(source.join(entry)).unwrap();
        let lower = text.to_ascii_lowercase();
        for forbidden in [
            "polymarket",
            "okx",
            "subscription",
            "login",
            "credential",
            "api_key",
            "privatekey",
            "signedrequest",
            "orderintent",
        ] {
            assert!(
                !lower.contains(forbidden),
                "neutral transport source contains forbidden protocol/authority term {forbidden}"
            );
        }
    }
}

#[test]
fn reconnect_backoff_and_connection_health_are_leaf_modules() {
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    for module in ["backoff.rs", "health.rs"] {
        let text = std::fs::read_to_string(source.join(module)).unwrap();
        assert!(!text.contains("reap_core"));
        assert!(!text.contains("crate::"));
        assert!(!text.contains("super::"));
    }
}

#[test]
fn shutdown_handles_are_opaque_and_expose_no_clearable_watch_authority() {
    let source = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src")
        .join("shutdown.rs");
    let text = std::fs::read_to_string(source).unwrap();

    assert!(!text.contains("pub type Shutdown"));
    assert!(!text.contains("pub inner:"));
    assert!(!text.contains("pub fn borrow"));
    assert!(!text.contains("pub fn send"));
    assert!(!text.contains("watch::Sender<bool>;"));
    assert!(!text.contains("watch::Receiver<bool>;"));
}
