mod account_certification;
mod bill_collection;
mod bootstrap;
mod convergence;
mod coordinator;
mod deadman_certification;
mod economic_statement;
mod fault_campaign;
mod fill_collection;
mod forbidden_orders;
mod host;
mod latency;
mod live_verification;
mod operator;
mod order_ws;
mod production_transition;
mod provenance;
mod regular_execution;
mod runtime;
mod runtime_config;
mod startup;
mod statement;

pub use account_certification::{
    AccountCertificationError, MIN_ACCOUNT_CERTIFICATION_INDEX_INTERVAL_MS,
    collect_account_certification_path, verify_account_certification_artifact_path,
    verify_account_certification_path,
};
pub use bill_collection::{
    BILL_COLLECTION_MANIFEST_NAME, BILL_COLLECTION_SCHEMA_VERSION, BillCollectionCoverage,
    BillCollectionError, BillCollectionManifest, BillCollectionOptions, BillCollectionPageEvidence,
    BillCollectionVerificationSummary, BillCollectionWindow, MAX_BILL_COLLECTION_CLOSE_DELAY_MS,
    MAX_BILL_COLLECTION_CONFIG_BYTES, MAX_BILL_COLLECTION_MANIFEST_BYTES,
    MAX_BILL_COLLECTION_PAGE_BYTES, MAX_BILL_COLLECTION_PAGE_INTERVAL_MS,
    MAX_BILL_COLLECTION_PAGES, MAX_BILL_COLLECTION_TOTAL_BYTES, MAX_BILL_COLLECTION_WINDOW_AGE_MS,
    MIN_BILL_COLLECTION_PAGE_INTERVAL_MS, OKX_ACCOUNT_BILLS_RETENTION_MS, VerifiedBillCollection,
    collect_okx_bills_paths, verify_bill_collection_manifest_path,
};
pub use bootstrap::{
    AccountBootstrapSnapshot, BootstrapValidation, VerifiedBootstrap, VerifiedInstrument,
    okx_instrument_type, verify_bootstrap,
};
pub use coordinator::{
    CancelAction, CoordinatorError, CoordinatorOutput, LiveAction, LiveCoordinator,
    ReconcileAction, ReconciliationResult, SubmitAction,
};
pub use deadman_certification::{
    DEADMAN_EXPIRY_CERTIFICATION_SCHEMA_VERSION, DeadmanBootstrapEvidence,
    DeadmanCertificationFailure, DeadmanCertificationResponseEvidence,
    DeadmanExpiryCertificationArtifact, DeadmanExpiryCertificationCoverage,
    DeadmanExpiryCertificationError, DeadmanExpiryCertificationOptions,
    DeadmanExpiryCertificationSummary, DeadmanJournalEvidence, DeadmanOrderDetailEvidence,
    DeadmanRecoveredOrderEvidence, MAX_DEADMAN_CERTIFICATION_ARTIFACT_BYTES,
    MAX_DEADMAN_CERTIFICATION_CONFIG_BYTES, MAX_DEADMAN_CERTIFICATION_JOURNAL_BYTES,
    MAX_DEADMAN_CERTIFICATION_ORDERS, MAX_DEADMAN_CERTIFICATION_RESPONSE_BYTES,
    MAX_DEADMAN_CERTIFICATION_SPAN_MS, OKX_DEADMAN_CANCEL_SOURCE,
    collect_deadman_expiry_certification_path, verify_deadman_expiry_certification_artifact_path,
    verify_deadman_expiry_certification_path,
};
pub use economic_statement::{
    CurrencyBalanceContinuitySample, DerivativePnlFormulaSample,
    ECONOMIC_RECONCILIATION_SCHEMA_VERSION, EconomicAccountBoundaryEvidence, EconomicIssue,
    EconomicIssueSource, EconomicJournalRecoveryEvidence, EconomicReconciliationCounts,
    EconomicReconciliationError, EconomicReconciliationFailure, EconomicReconciliationOptions,
    EconomicReconciliationReport, EconomicReconciliationScope, EconomicReconciliationTolerances,
    FundingFormulaSample, MAX_ACCOUNT_BOUNDARY_GAP_MS, MAX_ECONOMIC_CONFIG_BYTES,
    MAX_ECONOMIC_DERIVATIVE_PNL_SAMPLES, MAX_ECONOMIC_FUNDING_SAMPLES, MAX_ECONOMIC_JOURNAL_BYTES,
    MAX_ECONOMIC_REPORTED_ISSUES, MAX_FUNDING_BILL_DELAY_MS, MAX_FUNDING_MARK_BRACKET_DISTANCE_MS,
    MAX_TRADE_BILL_DELAY_MS, reconcile_okx_economics_paths,
};
pub use fault_campaign::{
    LIVE_FAULT_MATRIX_MANIFEST_SCHEMA_VERSION, LIVE_FAULT_MATRIX_REPORT_FORMAT_VERSION,
    LiveFaultFileEvidence, LiveFaultMatrixConfigFailure, LiveFaultMatrixError,
    LiveFaultMatrixFailure, LiveFaultMatrixIdentity, LiveFaultMatrixManifest,
    LiveFaultMatrixRunManifest, LiveFaultMatrixRunVerification, LiveFaultMatrixVerificationReport,
    LiveFaultObservedEvidence, LiveFaultProxyEvidenceSummary, LiveFaultScenario,
    LiveFaultScenarioFailure, MAX_LIVE_FAULT_INJECTOR_EVIDENCE_BYTES,
    MAX_LIVE_FAULT_MATRIX_MANIFEST_BYTES, MAX_LIVE_FAULT_MATRIX_RUNS,
    verify_live_fault_matrix_paths,
};
pub use fill_collection::{
    FILL_COLLECTION_MANIFEST_NAME, FILL_COLLECTION_SCHEMA_VERSION, FillCollectionClockEvidence,
    FillCollectionCoverage, FillCollectionError, FillCollectionFileEvidence,
    FillCollectionManifest, FillCollectionOptions, FillCollectionPageEvidence,
    FillCollectionWindow, MAX_FILL_COLLECTION_CLOSE_DELAY_MS, MAX_FILL_COLLECTION_CONFIG_BYTES,
    MAX_FILL_COLLECTION_MANIFEST_BYTES, MAX_FILL_COLLECTION_PAGE_BYTES,
    MAX_FILL_COLLECTION_PAGE_INTERVAL_MS, MAX_FILL_COLLECTION_PAGES,
    MAX_FILL_COLLECTION_TOTAL_BYTES, MAX_FILL_COLLECTION_WINDOW_AGE_MS,
    MIN_FILL_COLLECTION_PAGE_INTERVAL_MS, OKX_RECENT_FILLS_RETENTION_MS, VerifiedFillCollection,
    collect_recent_okx_fills_paths, verify_fill_collection_manifest_path,
};
pub(crate) use host::{HostGuardRuntime, check_host_health, start_host_guard};
pub use host::{HostHealthError, HostHealthSnapshot};
pub(crate) use latency::LiveLatencyCollector;
pub use latency::{
    LIVE_LATENCY_EVIDENCE_SCHEMA_VERSION, LIVE_LATENCY_RESERVOIR_CAPACITY, LiveLatencyEvidence,
    LiveLatencySemantics, LiveLatencySeries, MAX_LIVE_LATENCY_SERIES, MAX_LIVE_LATENCY_US,
};
pub use live_verification::{
    LIVE_RUN_VERIFICATION_FORMAT_VERSION, LiveRunConfigVerification, LiveRunFileEvidence,
    LiveRunVerificationError, LiveRunVerificationFailure, LiveRunVerificationReport,
    MAX_LIVE_RUN_REPORT_BYTES, verify_live_run_paths,
};
pub use operator::{
    OperatorCommand, OperatorError, OperatorResponse, OperatorStatus, send_operator_command,
};
pub(crate) use operator::{OperatorEnvelope, OperatorService, start_operator_service};
pub use production_transition::{
    MAX_REPORTED_TRANSITION_CHANGES, PRODUCTION_TRANSITION_FORMAT_VERSION,
    ProductionTransitionChange, ProductionTransitionConfigEvidence, ProductionTransitionError,
    ProductionTransitionFailure, ProductionTransitionReport, TransitionValueEvidence,
    TransitionValueKind, verify_production_transition_paths,
};
pub use provenance::{current_executable_sha256, host_identity_sha256};
pub use reap_live_contracts::{
    ACCOUNT_CASH_POLICY_VERSION, ACCOUNT_CERTIFICATION_SCHEMA_VERSION,
    ACCOUNT_EQUITY_AGGREGATE_ABS_TOLERANCE_USD, ACCOUNT_EQUITY_AGGREGATE_REL_TOLERANCE,
    ACCOUNT_EQUITY_INDEX_ABS_TOLERANCE_USD, ACCOUNT_EQUITY_INDEX_REL_TOLERANCE,
    AccountCashPolicyEvaluation, AccountCertificationArtifact, AccountCertificationClockEvidence,
    AccountCertificationConfigEvidence, AccountCertificationCoverage,
    AccountCertificationIndexEvidence, AccountCertificationResponseEvidence,
    AccountCertificationSummary, AccountEquityConversionSample, AccountEquityEvaluation,
    MAX_ACCOUNT_CERTIFICATION_ARTIFACT_BYTES, MAX_ACCOUNT_CERTIFICATION_CONFIG_BYTES,
    MAX_ACCOUNT_CERTIFICATION_INDEX_STALENESS_MS, MAX_ACCOUNT_CERTIFICATION_RESPONSE_BYTES,
    MAX_ACCOUNT_CERTIFICATION_SPAN_MS, evaluate_account_cash_policy,
};
pub use reap_live_contracts::{
    AlertConfig, AuthenticatedReadOperation, AuthenticatedReadPlan,
    CHAOS_CONNECTIVITY_PLAN_SCHEMA_VERSION, CapabilitySurface, ChaosAccountRequirements,
    ChaosConnectivityPlan, ChaosConnectivityPlanError, ChaosConnectivityRequirements,
    ConnectivityConsumer, ConnectivityRequirementId, FORBIDDEN_PROOF_DEFAULT_MAX_AGE_MS,
    FORBIDDEN_PROOF_DEFAULT_SCAN_INTERVAL_MS, FORBIDDEN_PROOF_HARD_MAX_AGE_MS,
    ForbiddenOrderCheckPlan, ForbiddenOrderQuery, ForbiddenProofPolicy, HostGuardConfig,
    LiveAccountConfig, LiveConfig, LiveConfigError, LiveConfigFileEvidence, LiveConfigValidation,
    LiveConnectivityRole, LiveConnectivityRolePlan, LiveMode, LiveStorageConfig, LocalTimerPlan,
    MAX_LIVE_CONFIG_BYTES, MAX_ORDER_WEBSOCKET_SESSIONS, MaintenanceProductPlan,
    MaintenanceRelevancePlan, MaintenanceServicePlan, OkxApiKeyPolicyConfig,
    OkxApiKeyPolicyEvaluation, OkxEndpointRegion, OkxTradeModeConfig, OkxVenueConfig,
    OperatorConfig, OrderCommandLanePlan, PrivateChannelBinding, PrivateChannelPlan,
    PrivateStateSessionPlan, PublicChannelPlan, PublicRedundancyConsumer,
    PublicSafetyReadOperation, PublicSafetyReadPlan, PublicSubscriptionPlan,
    RegularMutationOperation, RegularMutationPlan, RequirementUse, RuntimeConfig,
    TradingEnvironment, evaluate_okx_api_key_policy,
};
pub use runtime::{
    LIVE_RUN_REPORT_SCHEMA_VERSION, LiveFailureEvidence, LiveRunOptions, LiveRunReport,
    LiveRuntimeError, LiveStopReason, MAX_LIVE_FAILURE_CODE_BYTES, MAX_LIVE_FAILURE_MESSAGE_BYTES,
    PreparedLiveRun, prepare_live, prepare_live_path, run_live, run_live_path,
};
pub use runtime_config::{
    AlertConfigRuntimeExt, LiveConfigRuntimeExt, OperatorConfigRuntimeExt, load_live_config,
    load_live_config_with_evidence,
};
pub(crate) use runtime_config::{alert_webhook_from_env, operator_secret_from_env};
pub use startup::{LivePhase, ReadinessSnapshot, StartupError, StartupGate};
pub use statement::{
    FILL_STATEMENT_REPORT_SCHEMA_VERSION, FillEvidenceGap, FillFieldMismatch,
    FillJournalRecoveryEvidence, FillRecordIssue, FillStatementComparison, FillStatementCounts,
    FillStatementCoverage, FillStatementError, FillStatementFailure, FillStatementFileEvidence,
    FillStatementReconciliationOptions, FillStatementReconciliationReport, FillStatementScope,
    FillStatementSource, FillStatementTolerances, FillStatementWindow,
    MAX_FILL_STATEMENT_JOURNAL_BYTES, MAX_FILL_STATEMENT_PAGE_BYTES, MAX_FILL_STATEMENT_PAGES,
    MAX_FILL_STATEMENT_TOTAL_PAGE_BYTES, StatementFillKey, reconcile_okx_fill_collection_paths,
    reconcile_okx_fill_statement_paths,
};
