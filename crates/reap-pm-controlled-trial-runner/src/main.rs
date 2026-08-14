//! Private PM-T2 controlled-trial runner assembly point.
//!
//! Three commands freeze an offline, explicitly
//! non-authorizing Phase-A candidate/gap report, generate a request that is
//! explicitly not an authorization, or draft the already-defined
//! non-authorizing V4 eligibility envelope. A fourth, explicit production
//! command uses the private one-shot authority and fixed Reap mutation edge to
//! place at most one capped order and then cancel that exact order.

#![forbid(unsafe_code)]
#![allow(
    dead_code,
    reason = "the private authority is wired by later runner session and transport slices"
)]

use std::{error::Error, io::Write as _, net::IpAddr, path::PathBuf};

use clap::{Parser, Subcommand};
use reap_pm_controlled_trial::ReviewedPhaseAEligibilityEnvelopeDraftInputsV4;

mod controlled_trial;
mod phase_a_authorization_request;
mod phase_a_candidate;
mod phase_a_v4_draft;

#[cfg(target_os = "linux")]
use controlled_trial::{
    PredarbExactOrderReconciliationRequestV1, PredarbOwnedFillPositionReconciliationRequestV1,
    PredarbProductionOrderRequestV1, reconcile_predarb_exact_order_v1,
    reconcile_predarb_owned_fill_position_v1, run_authorized_predarb_minimum_fill_v1,
    run_authorized_predarb_place_then_cancel_v1,
};
use phase_a_authorization_request::{
    GeneratePhaseAAuthorizationRequestNotAuthorizationPaths,
    generate_phase_a_authorization_request_not_authorization,
};
use phase_a_candidate::{FreezePhaseACandidatePaths, freeze_phase_a_candidate};
use phase_a_v4_draft::{
    DraftNonAuthorizingPhaseAEligibilityEnvelopeV4Paths,
    draft_non_authorizing_phase_a_eligibility_envelope_v4,
};

#[derive(Debug, Parser)]
#[command(
    name = "reap-pm-controlled-trial-runner",
    about = "PM-T2 offline review gates and explicit one-shot production order trial"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Place one far-away order in the current BTC 5m market, then cancel it.
    #[cfg(target_os = "linux")]
    ProductionPlaceThenExactCancel {
        /// Existing protected Predarb .env; values are never printed or copied.
        #[arg(long, default_value = "../predarb/.env")]
        credential_env: PathBuf,
        /// New or empty owner-only directory; its fixed ledger permits one attempt ever.
        #[arg(long)]
        state_directory: PathBuf,
        /// One currently reviewed clob.polymarket.com IP; no runtime DNS fallback.
        #[arg(long)]
        fixed_peer_ip: String,
        /// Exact Linux interface used for the connection.
        #[arg(long)]
        interface_name: String,
        /// Exact local interface address, including a private address behind NAT.
        #[arg(long)]
        local_source_ip: IpAddr,
        /// Must equal I_ACCEPT_TOTAL_LOSS_AND_ONE_REAL_POLYMARKET_ORDER.
        #[arg(long)]
        authorization_phrase: String,
    },
    /// Buy exactly the current BTC 5m market minimum at the fresh best ask,
    /// then cancel any exact-order remainder and reconcile fills/position.
    #[cfg(target_os = "linux")]
    ProductionMinimumFill {
        /// Existing protected Predarb .env; values are never printed or copied.
        #[arg(long, default_value = "../predarb/.env")]
        credential_env: PathBuf,
        /// New or empty owner-only directory; its fixed ledger permits one attempt ever.
        #[arg(long)]
        state_directory: PathBuf,
        /// One currently reviewed clob.polymarket.com IP; no runtime DNS fallback.
        #[arg(long)]
        fixed_peer_ip: String,
        /// Exact Linux interface used for the connection.
        #[arg(long)]
        interface_name: String,
        /// Exact local interface address, including a private address behind NAT.
        #[arg(long)]
        local_source_ip: IpAddr,
        /// Must equal I_ACCEPT_TOTAL_LOSS_AND_ONE_REAL_POLYMARKET_ORDER.
        #[arg(long)]
        authorization_phrase: String,
    },
    /// Read and classify one exact owned Polymarket order without mutation authority.
    #[cfg(target_os = "linux")]
    ProductionReconcileExactOrder {
        /// Existing protected Predarb .env; values are never printed or copied.
        #[arg(long, default_value = "../predarb/.env")]
        credential_env: PathBuf,
        #[arg(long)]
        condition_id: String,
        #[arg(long)]
        question_id: String,
        #[arg(long)]
        token_id: String,
        #[arg(long)]
        order_id: String,
    },
    /// Re-poll one already-owned fill and reconcile local fill-derived and
    /// authoritative positions. This command has no mutation transport.
    #[cfg(target_os = "linux")]
    ProductionReconcileOwnedFillPosition {
        #[arg(long, default_value = "../predarb/.env")]
        credential_env: PathBuf,
        #[arg(long)]
        condition_id: String,
        #[arg(long)]
        question_id: String,
        #[arg(long)]
        token_id: String,
        #[arg(long)]
        order_id: String,
        #[arg(long)]
        price: String,
        #[arg(long)]
        quantity: String,
        #[arg(long)]
        position_before_protocol_units: String,
    },
    /// Verify static V3 and the V4 eligibility envelope, then print a DENIED candidate.
    FreezePhaseACandidate {
        #[arg(long)]
        repository_root: PathBuf,
        #[arg(long)]
        config: PathBuf,
        #[arg(long)]
        authorization: PathBuf,
        #[arg(long)]
        online_policy_v2: PathBuf,
        #[arg(long)]
        online_authorization_v2: PathBuf,
        #[arg(long)]
        reviewed_production_destination_v1: PathBuf,
        #[arg(long)]
        reviewed_fresh_credential_slot_locator_v1: PathBuf,
        #[arg(long)]
        fresh_credential_delivery_binding_v1: PathBuf,
        #[arg(long)]
        reviewed_signer_proxy_account_identity_v1: PathBuf,
        #[arg(long)]
        reviewed_remote_credential_proof_policy_v1: PathBuf,
        #[arg(long)]
        reviewed_static_online_authorization_v3: PathBuf,
        #[arg(long)]
        reviewed_phase_a_eligibility_envelope_v4: PathBuf,
        #[arg(long)]
        source_manifest: PathBuf,
        #[arg(long)]
        runbook: PathBuf,
    },
    /// Print a DENIED Phase-A request that is explicitly not an authorization.
    GeneratePhaseAAuthorizationRequestNotAuthorization {
        #[arg(long)]
        repository_root: PathBuf,
        #[arg(long)]
        config: PathBuf,
        #[arg(long)]
        authorization: PathBuf,
        #[arg(long)]
        online_policy_v2: PathBuf,
        #[arg(long)]
        online_authorization_v2: PathBuf,
        #[arg(long)]
        reviewed_production_destination_v1: PathBuf,
        #[arg(long)]
        reviewed_fresh_credential_slot_locator_v1: PathBuf,
        #[arg(long)]
        fresh_credential_delivery_binding_v1: PathBuf,
        #[arg(long)]
        reviewed_signer_proxy_account_identity_v1: PathBuf,
        #[arg(long)]
        reviewed_remote_credential_proof_policy_v1: PathBuf,
        #[arg(long)]
        reviewed_static_online_authorization_v3: PathBuf,
        #[arg(long)]
        reviewed_phase_a_eligibility_envelope_v4: PathBuf,
        #[arg(long)]
        reviewed_poly_proxy_control_policy_v1: PathBuf,
        #[arg(long)]
        reviewed_local_operator_cooperative_custody_profile_v1: PathBuf,
        #[arg(long)]
        reviewed_l1_credential_derivation_proof_policy_v1: PathBuf,
        #[arg(long)]
        source_manifest: PathBuf,
        #[arg(long)]
        runbook: PathBuf,
    },
    /// Draft a DENIED V4 envelope as compact JSON on stdout with no newline.
    ///
    /// The reviewer label is unauthenticated display text and all text inputs
    /// must be non-secret. This command creates no file. The caller must
    /// separately install the exact output as one protected 0600 file before
    /// using the canonical V4 loader.
    DraftNonAuthorizingPhaseAEligibilityEnvelopeV4 {
        #[arg(long)]
        config: PathBuf,
        #[arg(long)]
        authorization: PathBuf,
        #[arg(long)]
        online_policy_v2: PathBuf,
        #[arg(long)]
        online_authorization_v2: PathBuf,
        #[arg(long)]
        reviewed_production_destination_v1: PathBuf,
        #[arg(long)]
        reviewed_fresh_credential_slot_locator_v1: PathBuf,
        #[arg(long)]
        fresh_credential_delivery_binding_v1: PathBuf,
        #[arg(long)]
        reviewed_signer_proxy_account_identity_v1: PathBuf,
        #[arg(long)]
        reviewed_remote_credential_proof_policy_v1: PathBuf,
        #[arg(long)]
        reviewed_static_online_authorization_v3: PathBuf,
        #[arg(long)]
        eligibility_record_id: String,
        #[arg(long)]
        reviewer_label: String,
        #[arg(long)]
        reviewed_at_utc: String,
        #[arg(long)]
        not_before_utc: String,
        #[arg(long)]
        expires_at_utc: String,
        #[arg(long)]
        cleanup_not_after_utc: String,
    },
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    match Cli::parse().command {
        #[cfg(target_os = "linux")]
        Command::ProductionPlaceThenExactCancel {
            credential_env,
            state_directory,
            fixed_peer_ip,
            interface_name,
            local_source_ip,
            authorization_phrase,
        } => {
            let report =
                run_authorized_predarb_place_then_cancel_v1(PredarbProductionOrderRequestV1 {
                    credential_env,
                    state_directory,
                    fixed_peer_ip,
                    interface_name,
                    local_source_ip,
                    authorization_phrase,
                })
                .await?;
            let canonical_bytes = serde_json::to_vec(&report)?;
            std::io::stdout().lock().write_all(&canonical_bytes)?;
        }
        #[cfg(target_os = "linux")]
        Command::ProductionMinimumFill {
            credential_env,
            state_directory,
            fixed_peer_ip,
            interface_name,
            local_source_ip,
            authorization_phrase,
        } => {
            let report = run_authorized_predarb_minimum_fill_v1(PredarbProductionOrderRequestV1 {
                credential_env,
                state_directory,
                fixed_peer_ip,
                interface_name,
                local_source_ip,
                authorization_phrase,
            })
            .await?;
            let canonical_bytes = serde_json::to_vec(&report)?;
            std::io::stdout().lock().write_all(&canonical_bytes)?;
        }
        #[cfg(target_os = "linux")]
        Command::ProductionReconcileExactOrder {
            credential_env,
            condition_id,
            question_id,
            token_id,
            order_id,
        } => {
            let report =
                reconcile_predarb_exact_order_v1(PredarbExactOrderReconciliationRequestV1 {
                    credential_env,
                    condition_id,
                    question_id,
                    token_id,
                    order_id,
                })
                .await?;
            let canonical_bytes = serde_json::to_vec(&report)?;
            std::io::stdout().lock().write_all(&canonical_bytes)?;
        }
        #[cfg(target_os = "linux")]
        Command::ProductionReconcileOwnedFillPosition {
            credential_env,
            condition_id,
            question_id,
            token_id,
            order_id,
            price,
            quantity,
            position_before_protocol_units,
        } => {
            let report = reconcile_predarb_owned_fill_position_v1(
                PredarbOwnedFillPositionReconciliationRequestV1 {
                    credential_env,
                    condition_id,
                    question_id,
                    token_id,
                    order_id,
                    price,
                    quantity,
                    position_before_protocol_units,
                },
            )
            .await?;
            let canonical_bytes = serde_json::to_vec(&report)?;
            std::io::stdout().lock().write_all(&canonical_bytes)?;
        }
        Command::FreezePhaseACandidate {
            repository_root,
            config,
            authorization,
            online_policy_v2,
            online_authorization_v2,
            reviewed_production_destination_v1,
            reviewed_fresh_credential_slot_locator_v1,
            fresh_credential_delivery_binding_v1,
            reviewed_signer_proxy_account_identity_v1,
            reviewed_remote_credential_proof_policy_v1,
            reviewed_static_online_authorization_v3,
            reviewed_phase_a_eligibility_envelope_v4,
            source_manifest,
            runbook,
        } => {
            let report = freeze_phase_a_candidate(FreezePhaseACandidatePaths {
                repository_root,
                config,
                authorization,
                online_policy_v2,
                online_authorization_v2,
                reviewed_production_destination_v1,
                reviewed_fresh_credential_slot_locator_v1,
                fresh_credential_delivery_binding_v1,
                reviewed_signer_proxy_account_identity_v1,
                reviewed_remote_credential_proof_policy_v1,
                reviewed_static_online_authorization_v3,
                reviewed_phase_a_eligibility_envelope_v4,
                source_manifest,
                runbook,
            })?;
            let canonical_bytes = serde_json::to_vec(&report)?;
            std::io::stdout().lock().write_all(&canonical_bytes)?;
        }
        Command::GeneratePhaseAAuthorizationRequestNotAuthorization {
            repository_root,
            config,
            authorization,
            online_policy_v2,
            online_authorization_v2,
            reviewed_production_destination_v1,
            reviewed_fresh_credential_slot_locator_v1,
            fresh_credential_delivery_binding_v1,
            reviewed_signer_proxy_account_identity_v1,
            reviewed_remote_credential_proof_policy_v1,
            reviewed_static_online_authorization_v3,
            reviewed_phase_a_eligibility_envelope_v4,
            reviewed_poly_proxy_control_policy_v1,
            reviewed_local_operator_cooperative_custody_profile_v1,
            reviewed_l1_credential_derivation_proof_policy_v1,
            source_manifest,
            runbook,
        } => {
            let request = generate_phase_a_authorization_request_not_authorization(
                GeneratePhaseAAuthorizationRequestNotAuthorizationPaths {
                    repository_root,
                    config,
                    authorization,
                    online_policy_v2,
                    online_authorization_v2,
                    reviewed_production_destination_v1,
                    reviewed_fresh_credential_slot_locator_v1,
                    fresh_credential_delivery_binding_v1,
                    reviewed_signer_proxy_account_identity_v1,
                    reviewed_remote_credential_proof_policy_v1,
                    reviewed_static_online_authorization_v3,
                    reviewed_phase_a_eligibility_envelope_v4,
                    reviewed_poly_proxy_control_policy_v1,
                    reviewed_local_operator_cooperative_custody_profile_v1,
                    reviewed_l1_credential_derivation_proof_policy_v1,
                    source_manifest,
                    runbook,
                },
            )?;
            let canonical_bytes = serde_json::to_vec(&request)?;
            std::io::stdout().lock().write_all(&canonical_bytes)?;
        }
        Command::DraftNonAuthorizingPhaseAEligibilityEnvelopeV4 {
            config,
            authorization,
            online_policy_v2,
            online_authorization_v2,
            reviewed_production_destination_v1,
            reviewed_fresh_credential_slot_locator_v1,
            fresh_credential_delivery_binding_v1,
            reviewed_signer_proxy_account_identity_v1,
            reviewed_remote_credential_proof_policy_v1,
            reviewed_static_online_authorization_v3,
            eligibility_record_id,
            reviewer_label,
            reviewed_at_utc,
            not_before_utc,
            expires_at_utc,
            cleanup_not_after_utc,
        } => {
            let output = draft_non_authorizing_phase_a_eligibility_envelope_v4(
                DraftNonAuthorizingPhaseAEligibilityEnvelopeV4Paths {
                    config,
                    authorization,
                    online_policy_v2,
                    online_authorization_v2,
                    reviewed_production_destination_v1,
                    reviewed_fresh_credential_slot_locator_v1,
                    fresh_credential_delivery_binding_v1,
                    reviewed_signer_proxy_account_identity_v1,
                    reviewed_remote_credential_proof_policy_v1,
                    reviewed_static_online_authorization_v3,
                },
                ReviewedPhaseAEligibilityEnvelopeDraftInputsV4 {
                    eligibility_record_id,
                    reviewer_label,
                    reviewed_at_utc,
                    not_before_utc,
                    expires_at_utc,
                    cleanup_not_after_utc,
                },
            )?;
            std::io::stdout()
                .lock()
                .write_all(output.canonical_bytes())?;
        }
    }
    Ok(())
}
