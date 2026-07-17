use reap_live::{
    AlertConfig, AlertConfigRuntimeExt, LiveConfig, LiveConfigRuntimeExt, OperatorConfig,
    OperatorConfigRuntimeExt,
};

#[test]
fn moved_config_types_retain_runtime_method_compatibility() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("live.toml");
    std::fs::write(
        &path,
        include_bytes!("../../../examples/live-okx-demo.toml"),
    )
    .unwrap();

    let config = LiveConfig::load(&path).unwrap();
    let (with_evidence, evidence) = LiveConfig::load_with_evidence(&path).unwrap();

    assert_eq!(
        config.evidence_fingerprint().unwrap(),
        with_evidence.evidence_fingerprint().unwrap()
    );
    assert_eq!(evidence.source_path, path.canonicalize().unwrap());
    assert!(
        OperatorConfig::default()
            .secret_from_env()
            .unwrap()
            .is_none()
    );
    assert!(AlertConfig::default().webhook_from_env().unwrap().is_none());
}
