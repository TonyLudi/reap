//! Lowering from one loopback transport observation into the secret-free
//! authenticated-journal result for its durable send grant. Exact-body
//! correlation remains a typed, in-memory comparison and is never converted
//! to bytes or placed in the result record.

use reap_pm_core::PmVenueOrderId;
use reap_polymarket_auth::RuntimeExactBodyCommitment;
use reap_polymarket_live_adapter::{
    PmCancelMutationOutcome, PmMutationClassification, PmPlaceMutationOutcome,
};

use super::PmAuthenticatedExecutionError;
use crate::authenticated_journal::{
    PmAuthenticatedCancelResultV1, PmAuthenticatedCoordinatorIdentityV1,
    PmAuthenticatedPlaceResultV1,
};
use crate::authenticated_journal::{PmAuthenticatedCancelSendGrant, PmAuthenticatedPlaceSendGrant};

#[allow(
    clippy::result_large_err,
    reason = "classification keeps the bounded typed execution failure inline at the exact request-correlation boundary"
)]
pub(super) fn place_result(
    grant: &PmAuthenticatedPlaceSendGrant,
    runtime_exact_body_commitment: RuntimeExactBodyCommitment,
    outcome: &PmPlaceMutationOutcome,
) -> Result<PmAuthenticatedPlaceResultV1, PmAuthenticatedExecutionError> {
    if outcome.runtime_exact_body_commitment() != runtime_exact_body_commitment
        || !grant.matches_retained_request(
            outcome.semantic_request_commitment().bytes(),
            outcome.expected_order_id().bytes(),
            grant.l2_timestamp_seconds(),
        )
    {
        return Err(PmAuthenticatedExecutionError::OutcomeIdentityMismatch);
    }
    let coordinator =
        PmAuthenticatedCoordinatorIdentityV1::new(grant.client_order(), grant.instrument());
    let observed = outcome
        .observed_order_id()
        .map(exact_order_bytes)
        .transpose()?;
    Ok(match outcome.classification() {
        PmMutationClassification::Accepted => {
            let observed = observed.ok_or(PmAuthenticatedExecutionError::OutcomeShapeMismatch)?;
            if observed != grant.expected_order_id() {
                return Err(PmAuthenticatedExecutionError::OutcomeIdentityMismatch);
            }
            PmAuthenticatedPlaceResultV1::accepted(coordinator, grant.grant_sequence(), observed)
        }
        PmMutationClassification::Rejected => {
            if observed.is_some() {
                return Err(PmAuthenticatedExecutionError::OutcomeShapeMismatch);
            }
            PmAuthenticatedPlaceResultV1::rejected(coordinator, grant.grant_sequence())
        }
        PmMutationClassification::DefinitelyNotDispatched => {
            if observed.is_some() {
                return Err(PmAuthenticatedExecutionError::OutcomeShapeMismatch);
            }
            PmAuthenticatedPlaceResultV1::definitely_not_dispatched(
                coordinator,
                grant.grant_sequence(),
            )
        }
        PmMutationClassification::OutOfProfile => PmAuthenticatedPlaceResultV1::out_of_profile(
            coordinator,
            grant.grant_sequence(),
            observed,
        ),
        PmMutationClassification::AcknowledgementUnknown => {
            PmAuthenticatedPlaceResultV1::acknowledgement_unknown(
                coordinator,
                grant.grant_sequence(),
                observed,
            )
        }
    })
}

#[allow(
    clippy::result_large_err,
    reason = "classification keeps the bounded typed execution failure inline at the exact request-correlation boundary"
)]
pub(super) fn cancel_result(
    grant: &PmAuthenticatedCancelSendGrant,
    runtime_exact_body_commitment: RuntimeExactBodyCommitment,
    outcome: &PmCancelMutationOutcome,
) -> Result<PmAuthenticatedCancelResultV1, PmAuthenticatedExecutionError> {
    if outcome.runtime_exact_body_commitment() != runtime_exact_body_commitment
        || !grant.matches_retained_request(
            outcome.semantic_request_commitment().bytes(),
            outcome.order_id().bytes(),
            grant.l2_timestamp_seconds(),
        )
    {
        return Err(PmAuthenticatedExecutionError::OutcomeIdentityMismatch);
    }
    let coordinator =
        PmAuthenticatedCoordinatorIdentityV1::new(grant.client_order(), grant.instrument());
    let observed = outcome
        .observed_order_id()
        .map(exact_order_bytes)
        .transpose()?;
    Ok(match outcome.classification() {
        PmMutationClassification::Accepted => {
            let observed = require_exact_cancel_observed(grant, observed)?;
            PmAuthenticatedCancelResultV1::accepted(
                coordinator,
                grant.venue_order(),
                grant.grant_sequence(),
                observed,
            )
        }
        PmMutationClassification::Rejected => {
            let observed = require_exact_cancel_observed(grant, observed)?;
            PmAuthenticatedCancelResultV1::rejected(
                coordinator,
                grant.venue_order(),
                grant.grant_sequence(),
                observed,
            )
        }
        PmMutationClassification::DefinitelyNotDispatched => {
            if observed.is_some() {
                return Err(PmAuthenticatedExecutionError::OutcomeShapeMismatch);
            }
            PmAuthenticatedCancelResultV1::definitely_not_dispatched(
                coordinator,
                grant.venue_order(),
                grant.grant_sequence(),
            )
        }
        PmMutationClassification::OutOfProfile => PmAuthenticatedCancelResultV1::out_of_profile(
            coordinator,
            grant.venue_order(),
            grant.grant_sequence(),
            observed,
        ),
        PmMutationClassification::AcknowledgementUnknown => {
            PmAuthenticatedCancelResultV1::acknowledgement_unknown(
                coordinator,
                grant.venue_order(),
                grant.grant_sequence(),
                observed,
            )
        }
    })
}

#[allow(
    clippy::result_large_err,
    reason = "exact cancel identity validation returns the bounded typed execution failure without changing its caller contract"
)]
fn require_exact_cancel_observed(
    grant: &PmAuthenticatedCancelSendGrant,
    observed: Option<[u8; 32]>,
) -> Result<[u8; 32], PmAuthenticatedExecutionError> {
    let observed = observed.ok_or(PmAuthenticatedExecutionError::OutcomeShapeMismatch)?;
    if observed == grant.fixed_order_id() {
        Ok(observed)
    } else {
        Err(PmAuthenticatedExecutionError::OutcomeIdentityMismatch)
    }
}

#[allow(
    clippy::result_large_err,
    reason = "exact order parsing returns the bounded typed execution failure without changing its caller contract"
)]
fn exact_order_bytes(identity: PmVenueOrderId) -> Result<[u8; 32], PmAuthenticatedExecutionError> {
    let bytes = identity.as_str().as_bytes();
    if bytes.len() != 66 || &bytes[..2] != b"0x" {
        return Err(PmAuthenticatedExecutionError::OutcomeShapeMismatch);
    }
    let mut decoded = [0_u8; 32];
    for (index, output) in decoded.iter_mut().enumerate() {
        let high = decode_lower_hex(bytes[2 + index * 2])
            .ok_or(PmAuthenticatedExecutionError::OutcomeShapeMismatch)?;
        let low = decode_lower_hex(bytes[3 + index * 2])
            .ok_or(PmAuthenticatedExecutionError::OutcomeShapeMismatch)?;
        *output = (high << 4) | low;
    }
    Ok(decoded)
}

const fn decode_lower_hex(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        _ => None,
    }
}
