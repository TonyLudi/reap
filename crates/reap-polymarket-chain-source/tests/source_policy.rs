use std::collections::BTreeSet;
use std::path::PathBuf;

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
    assert!(source.contains("#[cfg(any(test, feature = \"loopback-evidence\"))]"));
    assert!(source.contains("pub fn loopback_evidence("));
    assert!(source.contains("Host::Ipv4"));
    assert!(source.contains("Host::Ipv6"));
    assert!(source.contains("ip.is_loopback()"));
    assert!(source.contains(".redirect(Policy::none())"));
    assert!(source.contains(".retry(reqwest::retry::never())"));
    assert!(source.contains(".no_proxy()"));
    assert!(source.contains("builder.https_only(true)"));
    assert!(source.contains("MAX_JSON_RPC_RESPONSE_BYTES"));
    assert!(source.contains("ClockSource::System"));
    assert!(source.contains("MAX_FINALIZED_BLOCK_AGE_SECONDS: u64 = 30"));
    assert!(source.contains("MAX_FINALIZED_BLOCK_FUTURE_SECONDS: u64 = 5"));
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
