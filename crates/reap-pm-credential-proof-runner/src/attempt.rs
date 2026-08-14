//! Private, loopback-only, permanently denied credential-proof attempt.
//!
//! This module is compiled only for tests or the explicit loopback-evidence
//! feature and is not exported from the crate. It loads no credential files,
//! exposes no constructor to another crate, has no command or binary, and can
//! select only the reserved loopback transport seam. The caller/test supplies
//! already-owned holders. Success retains the same L2 holder behind the opaque
//! auth dispatch owner and still returns only `DENIED` evidence.

mod lineage;

use std::{fmt, path::Path};

use reap_polymarket_auth::{
    EoaAddress, FixedEoaSigner, L1CredentialDerivationMatchedClosedOnlyDispatch,
    L1CredentialDerivationNonce, L1CredentialDerivationTimestamp, L2Credentials, L2Timestamp,
};
use reap_polymarket_egress_binding::{PmFixedTlsPeerSelection, PmLocalEgressSelection};

use self::lineage::{
    AttemptPublicBinding, FinalAttemptLineage, LineageError, PreparedAttemptLineage, commitment,
};
use crate::transport::{
    ClosedOnlyFalseLoopbackEvidence, CredentialProofTransportError,
    LoopbackCredentialProofAttemptTransport, LoopbackServerTimeObservation,
};

const POLICY_COMMITMENT_DOMAIN: &[u8] =
    b"reap.pm-t2.phase-a.credential-proof.policy-selection.v1\0";
const SOURCE_COMMITMENT_DOMAIN: &[u8] =
    b"reap.pm-t2.phase-a.credential-proof.source-selection.v1\0";
const DESTINATION_COMMITMENT_DOMAIN: &[u8] =
    b"reap.pm-t2.phase-a.credential-proof.destination-selection.v1\0";
const LOCAL_EGRESS_COMMITMENT_DOMAIN: &[u8] =
    b"reap.pm-t2.phase-a.credential-proof.local-egress-selection.v1\0";
const SIGNER_IDENTITY_COMMITMENT_DOMAIN: &[u8] =
    b"reap.pm-t2.phase-a.credential-proof.signer-identity.v1\0";
const ACTOR_GENERATION_COMMITMENT_DOMAIN: &[u8] =
    b"reap.pm-t2.phase-a.credential-proof.actor-generation.v1\0";
const ATTEMPT_COMMITMENT_DOMAIN: &[u8] = b"reap.pm-t2.phase-a.credential-proof.attempt.v1\0";

pub(crate) struct CredentialProofAttemptCommitmentInputs {
    policy: [u8; 32],
    source: [u8; 32],
    selected_actor_generation: [u8; 32],
    attempt: [u8; 32],
}

impl CredentialProofAttemptCommitmentInputs {
    #[cfg(test)]
    fn synthetic_for_tests(
        policy: [u8; 32],
        source: [u8; 32],
        selected_actor_generation: [u8; 32],
        attempt: [u8; 32],
    ) -> Self {
        Self {
            policy,
            source,
            selected_actor_generation,
            attempt,
        }
    }
}

pub(crate) struct DeniedCredentialProofAttempt {
    _closed_only_dispatch:
        L1CredentialDerivationMatchedClosedOnlyDispatch<ClosedOnlyFalseLoopbackEvidence>,
    _transport: LoopbackCredentialProofAttemptTransport,
    _lineage: FinalAttemptLineage,
}

impl DeniedCredentialProofAttempt {
    pub(crate) const fn authorization(&self) -> &'static str {
        "DENIED"
    }

    pub(crate) const fn production_permit(&self) -> bool {
        false
    }

    pub(crate) const fn resume_allowed(&self) -> bool {
        false
    }
}

impl fmt::Debug for DeniedCredentialProofAttempt {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(
            "DeniedCredentialProofAttempt(<DENIED; LOOPBACK_ONLY; NO_REMOTE_OR_MUTATION_AUTHORITY>)",
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CredentialProofAttemptError {
    SignerAndL2ActorMismatch,
    InvalidCommitmentInput,
    DurableLineage,
    AttemptAlreadyBurned,
    Transport,
    Authentication,
    ServerTimeRegressed,
}

impl fmt::Display for CredentialProofAttemptError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::SignerAndL2ActorMismatch => "credential-proof actor identity mismatch",
            Self::InvalidCommitmentInput => "credential-proof commitment input rejected",
            Self::DurableLineage => "credential-proof durable lineage rejected",
            Self::AttemptAlreadyBurned => "credential-proof attempt is already burned",
            Self::Transport => "credential-proof loopback transport rejected",
            Self::Authentication => "credential-proof authentication transition rejected",
            Self::ServerTimeRegressed => "credential-proof server time regressed",
        })
    }
}

impl std::error::Error for CredentialProofAttemptError {}

pub(crate) fn execute_private_loopback_credential_proof_attempt(
    artifact_directory: &Path,
    fixed_peer: PmFixedTlsPeerSelection,
    selected_local_egress: PmLocalEgressSelection,
    commitment_inputs: CredentialProofAttemptCommitmentInputs,
    signer: FixedEoaSigner,
    l2_credentials: L2Credentials,
) -> Result<DeniedCredentialProofAttempt, CredentialProofAttemptError> {
    if signer.address() != l2_credentials.address() {
        return Err(CredentialProofAttemptError::SignerAndL2ActorMismatch);
    }
    require_nonzero_inputs(&commitment_inputs)?;
    let binding = public_binding(
        &fixed_peer,
        &selected_local_egress,
        signer.address(),
        &commitment_inputs,
    );
    let mut transport = LoopbackCredentialProofAttemptTransport::loopback_evidence(
        fixed_peer,
        selected_local_egress,
    )
    .map_err(map_transport)?;

    let prepared =
        PreparedAttemptLineage::create_new(artifact_directory, binding).map_err(map_lineage)?;
    let mut burned = prepared.burn().map_err(map_lineage)?;

    burned.validate_exact().map_err(map_lineage)?;
    let first_time_observation = transport.first_server_time().map_err(map_transport)?;
    let first_time_seconds = source_owned_seconds(&first_time_observation);
    let l1_timestamp = L1CredentialDerivationTimestamp::from_unix_seconds(first_time_seconds)
        .map_err(|_| CredentialProofAttemptError::Authentication)?;
    let derive_request = signer
        .consume_into_l1_credential_derivation_request(
            l1_timestamp,
            L1CredentialDerivationNonce::from_u64(0),
        )
        .map_err(|_| CredentialProofAttemptError::Authentication)?;

    burned.validate_exact().map_err(map_lineage)?;
    let derive_response = derive_request
        .dispatch(&mut transport)
        .map_err(map_transport)?;
    let matched_l2 = l2_credentials
        .consume_with_l1_credential_derivation_response(derive_response)
        .map_err(|_| CredentialProofAttemptError::Authentication)?;

    burned.validate_exact().map_err(map_lineage)?;
    let second_time_observation = transport.second_server_time().map_err(map_transport)?;
    let second_time_seconds = source_owned_seconds(&second_time_observation);
    if second_time_seconds < first_time_seconds {
        return Err(CredentialProofAttemptError::ServerTimeRegressed);
    }
    let l2_timestamp = L2Timestamp::from_unix_seconds(second_time_seconds)
        .map_err(|_| CredentialProofAttemptError::Authentication)?;
    let closed_only_request = matched_l2
        .consume_into_authenticated_closed_only(l2_timestamp)
        .map_err(|_| CredentialProofAttemptError::Authentication)?;

    burned.validate_exact().map_err(map_lineage)?;
    let closed_only_dispatch = closed_only_request
        .dispatch(&mut transport)
        .map_err(map_transport)?;
    let lineage = burned.finish().map_err(map_lineage)?;
    Ok(DeniedCredentialProofAttempt {
        _closed_only_dispatch: closed_only_dispatch,
        _transport: transport,
        _lineage: lineage,
    })
}

fn source_owned_seconds(observation: &LoopbackServerTimeObservation) -> u64 {
    observation.unix_seconds()
}

fn require_nonzero_inputs(
    inputs: &CredentialProofAttemptCommitmentInputs,
) -> Result<(), CredentialProofAttemptError> {
    if [
        &inputs.policy,
        &inputs.source,
        &inputs.selected_actor_generation,
        &inputs.attempt,
    ]
    .into_iter()
    .any(|value| value.iter().all(|byte| *byte == 0))
    {
        return Err(CredentialProofAttemptError::InvalidCommitmentInput);
    }
    Ok(())
}

fn public_binding(
    fixed_peer: &PmFixedTlsPeerSelection,
    local_egress: &PmLocalEgressSelection,
    signer: EoaAddress,
    inputs: &CredentialProofAttemptCommitmentInputs,
) -> AttemptPublicBinding {
    let destination = format!("{}|{}", fixed_peer.dns_name(), fixed_peer.peer_addr());
    let local = format!(
        "{}|{}",
        local_egress.interface_name(),
        local_egress.local_source_ip()
    );
    AttemptPublicBinding {
        policy_commitment: commitment(POLICY_COMMITMENT_DOMAIN, &inputs.policy),
        source_commitment: commitment(SOURCE_COMMITMENT_DOMAIN, &inputs.source),
        destination_selection_commitment: commitment(
            DESTINATION_COMMITMENT_DOMAIN,
            destination.as_bytes(),
        ),
        local_egress_selection_commitment: commitment(
            LOCAL_EGRESS_COMMITMENT_DOMAIN,
            local.as_bytes(),
        ),
        signer_identity_commitment: commitment(
            SIGNER_IDENTITY_COMMITMENT_DOMAIN,
            signer.to_string().as_bytes(),
        ),
        selected_actor_generation_commitment: commitment(
            ACTOR_GENERATION_COMMITMENT_DOMAIN,
            &inputs.selected_actor_generation,
        ),
        attempt_commitment: commitment(ATTEMPT_COMMITMENT_DOMAIN, &inputs.attempt),
    }
}

fn map_lineage(error: LineageError) -> CredentialProofAttemptError {
    match error {
        LineageError::AlreadyBurned => CredentialProofAttemptError::AttemptAlreadyBurned,
        _ => CredentialProofAttemptError::DurableLineage,
    }
}

fn map_transport(_: CredentialProofTransportError) -> CredentialProofAttemptError {
    CredentialProofAttemptError::Transport
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    include!("attempt/tests.rs");
}
