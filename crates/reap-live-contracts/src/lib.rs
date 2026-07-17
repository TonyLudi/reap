#![forbid(unsafe_code)]

mod account_certification;
mod config;
mod connectivity_plan;
mod mode;

pub use account_certification::{
    ACCOUNT_CASH_POLICY_VERSION, ACCOUNT_CERTIFICATION_SCHEMA_VERSION,
    ACCOUNT_EQUITY_AGGREGATE_ABS_TOLERANCE_USD, ACCOUNT_EQUITY_AGGREGATE_REL_TOLERANCE,
    ACCOUNT_EQUITY_INDEX_ABS_TOLERANCE_USD, ACCOUNT_EQUITY_INDEX_REL_TOLERANCE,
    AccountCashPolicyEvaluation, AccountCertificationArtifact, AccountCertificationClockEvidence,
    AccountCertificationConfigEvidence, AccountCertificationCoverage,
    AccountCertificationIndexEvidence, AccountCertificationResponseEvidence,
    AccountCertificationSummary, AccountCertificationVerificationError,
    AccountEquityConversionSample, AccountEquityEvaluation,
    MAX_ACCOUNT_CERTIFICATION_ARTIFACT_BYTES, MAX_ACCOUNT_CERTIFICATION_CONFIG_BYTES,
    MAX_ACCOUNT_CERTIFICATION_INDEX_STALENESS_MS, MAX_ACCOUNT_CERTIFICATION_RESPONSE_BYTES,
    MAX_ACCOUNT_CERTIFICATION_SPAN_MS, account_certification_index_ticker_endpoint,
    account_certification_required_index_symbols, build_account_certification_response_evidence,
    derive_account_certification_summary, evaluate_account_cash_policy,
    okx_account_identity_sha256, validate_account_certification_account_id,
    verify_account_certification_artifact_bytes,
};
pub use config::{
    AlertConfig, LiveAccountConfig, LiveConfig, LiveConfigError, LiveConfigFileEvidence,
    LiveConfigValidation, LiveStorageConfig, MAX_LIVE_CONFIG_BYTES, MAX_ORDER_WEBSOCKET_SESSIONS,
    OkxApiKeyPolicyConfig, OkxApiKeyPolicyEvaluation, OkxEndpointRegion, OkxTradeModeConfig,
    OkxVenueConfig, OperatorConfig, RuntimeConfig, TradingEnvironment, evaluate_okx_api_key_policy,
};
pub use connectivity_plan::{
    AuthenticatedReadOperation, AuthenticatedReadPlan, CHAOS_CONNECTIVITY_PLAN_SCHEMA_VERSION,
    CapabilitySurface, ChaosAccountRequirements, ChaosConnectivityPlan, ChaosConnectivityPlanError,
    ChaosConnectivityRequirements, ConnectivityConsumer, ConnectivityRequirementId,
    FORBIDDEN_PROOF_DEFAULT_MAX_AGE_MS, FORBIDDEN_PROOF_DEFAULT_SCAN_INTERVAL_MS,
    FORBIDDEN_PROOF_HARD_MAX_AGE_MS, ForbiddenOrderCheckPlan, ForbiddenOrderQuery,
    ForbiddenProofPolicy, LiveConnectivityRole, LiveConnectivityRolePlan, LocalTimerPlan,
    MaintenanceProductPlan, MaintenanceRelevancePlan, MaintenanceServicePlan, OrderCommandLanePlan,
    PrivateChannelBinding, PrivateChannelPlan, PrivateStateSessionPlan, PublicChannelPlan,
    PublicRedundancyConsumer, PublicSafetyReadOperation, PublicSafetyReadPlan,
    PublicSubscriptionPlan, RegularMutationOperation, RegularMutationPlan, RequirementUse,
};
pub use mode::LiveMode;
pub use reap_core::HostGuardConfig;
