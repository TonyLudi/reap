//! Operator-runnable, credentialed, read-only Polymarket readiness evidence.
//!
//! This package deliberately has no dependency on the PM product coordinator,
//! strategy, journals, the general authentication crate, or an HTTP client.
//! The live adapter owns credential custody and exposes only purpose-specific
//! GET and authenticated user-stream roles.

#![forbid(unsafe_code)]

mod collect;
mod config;
mod credentials;
mod schema;
mod verify;

pub use collect::{
    PmReadOnlySmokeError, collect_pm_read_only_smoke_path,
    resolve_pm_read_only_credentials_directory,
};
pub use config::{
    MAX_PM_READ_ONLY_CONFIG_BYTES, PM_READ_ONLY_CONFIG_SCHEMA_VERSION, PmReadOnlyConfigEvidence,
    PmReadOnlySmokeConfig, PmReadOnlySmokeConfigError, load_pm_read_only_smoke_config_path,
};
pub use credentials::{
    MAX_PM_READ_ONLY_CREDENTIAL_FILE_BYTES, PmReadOnlyCredentialBundle, PmReadOnlyCredentialError,
    PmReadOnlyCredentialKind, load_pm_read_only_credentials,
};
pub use schema::{
    PM_READ_ONLY_ARTIFACT_SCHEMA_VERSION, PmReadOnlyAccountEvidence, PmReadOnlyAllowanceEvidence,
    PmReadOnlyCollectionFailureEvidence, PmReadOnlyMetadataEvidence, PmReadOnlyOrderEvidence,
    PmReadOnlyReconciliationEvidence, PmReadOnlySmokeArtifact, PmReadOnlySmokeSummary,
    PmReadOnlyTeardownEvidence, PmReadOnlyTradeEvidence, PmReadOnlyTradeMakerEvidence,
    PmReadOnlyUserStreamEvidence,
};
pub use verify::{
    MAX_PM_READ_ONLY_ARTIFACT_BYTES, PM_READ_ONLY_LIMITATIONS, PmReadOnlySmokeVerificationError,
    require_pm_read_only_smoke_pass, verify_pm_read_only_smoke_artifact_bytes,
    verify_pm_read_only_smoke_path, verify_pm_read_only_smoke_path_with_anchors,
};
