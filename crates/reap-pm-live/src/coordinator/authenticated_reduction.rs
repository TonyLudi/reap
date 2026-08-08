//! Canonical reduction for durable authenticated place/cancel completions.

use reap_pm_state::{
    PmOwnedCancelApply, PmOwnedCancelOutcome, PmOwnedSubmitApply, PmOwnedSubmitResult,
    PmOwnedSubmitState,
};
use thiserror::Error;

use super::live_completion::{PmLiveCancelCompletion, PmLivePlaceCompletion};
use super::mutation::{PmMutationError, PmMutationOwner};
use crate::journal::{PmJournalAuthenticatedClassificationV1, PmJournalSafetyReasonV1};

pub(super) fn preflight_live_place(
    owner: &PmMutationOwner,
    completion: &PmLivePlaceCompletion,
) -> Result<(), PmAuthenticatedReductionError> {
    let result = completion.result();
    let canonical = result.canonical();
    if result.instrument() != owner.instrument_id()
        || canonical.client_order.account() != owner.account_scope().handle()
        || canonical
            .venue_order
            .is_some_and(|venue| venue.account() != owner.account_scope().handle())
    {
        return Err(PmAuthenticatedReductionError::ScopeMismatch);
    }
    let order = owner
        .private()
        .owned_order(canonical.client_order)
        .ok_or(PmAuthenticatedReductionError::UnknownOwnedOrder)?;
    if order.slot().account_scope() != owner.account_scope()
        || order.slot().instrument() != owner.private().instrument()
    {
        return Err(PmAuthenticatedReductionError::ScopeMismatch);
    }
    let state_result = place_state_result(result)?;
    if ambiguous_after_exact_private_accept(result.classification(), order.submit()) {
        let venue = order
            .venue_order()
            .ok_or(PmAuthenticatedReductionError::ResultShapeMismatch)?;
        if exact_order_bytes(venue) != Some(result.expected_order_id()) {
            return Err(PmAuthenticatedReductionError::ResultShapeMismatch);
        }
        return Ok(());
    }
    let expected = owner
        .private()
        .preflight_owned_submit_result(canonical.client_order, state_result)
        .map_err(|_| PmAuthenticatedReductionError::UnexpectedSubmitTransition)?;
    if valid_place_apply(result.classification(), Some(expected), false) {
        Ok(())
    } else {
        Err(PmAuthenticatedReductionError::UnexpectedSubmitTransition)
    }
}

pub(super) fn preflight_live_cancel(
    owner: &PmMutationOwner,
    completion: &PmLiveCancelCompletion,
) -> Result<(), PmAuthenticatedReductionError> {
    let result = completion.result();
    let canonical = result.canonical();
    if result.instrument() != owner.instrument_id()
        || canonical.client_order != completion.client_order()
        || canonical.venue_order != completion.venue_order()
        || canonical.client_order.account() != owner.account_scope().handle()
        || canonical.venue_order.account() != owner.account_scope().handle()
    {
        return Err(PmAuthenticatedReductionError::CancelOwnershipMismatch);
    }
    let order = owner
        .private()
        .owned_order(canonical.client_order)
        .ok_or(PmAuthenticatedReductionError::UnknownOwnedOrder)?;
    if order.slot().account_scope() != owner.account_scope()
        || order.slot().instrument() != owner.private().instrument()
        || order.venue_order() != Some(canonical.venue_order)
    {
        return Err(PmAuthenticatedReductionError::CancelOwnershipMismatch);
    }
    let expected = owner
        .private()
        .preflight_owned_cancel_result(
            completion.intent(),
            cancel_state_outcome(result.classification()),
        )
        .map_err(|_| PmAuthenticatedReductionError::UnexpectedCancelTransition)?;
    if valid_cancel_apply(result.classification(), expected) {
        Ok(())
    } else {
        Err(PmAuthenticatedReductionError::UnexpectedCancelTransition)
    }
}

#[allow(
    clippy::result_large_err,
    reason = "canonical mutation reduction keeps its bounded typed owner error inline to preserve the existing transition API"
)]
pub(super) fn apply_live_place(
    owner: &mut PmMutationOwner,
    completion: PmLivePlaceCompletion,
    monotonic_service_ns: u64,
) -> Result<(), PmMutationError> {
    validate_service_time(owner, monotonic_service_ns)?;
    if let Err(error) = preflight_live_place(owner, &completion) {
        return violation(owner, error, monotonic_service_ns);
    }
    let result = completion.into_result();
    let canonical = result.canonical();
    let order = owner
        .private()
        .owned_order(canonical.client_order)
        .ok_or(PmAuthenticatedReductionError::UnknownOwnedOrder)?;
    let state_result = place_state_result(result)?;
    let accepted_before_ambiguous =
        ambiguous_after_exact_private_accept(result.classification(), order.submit());
    let applied = if accepted_before_ambiguous {
        None
    } else {
        Some(
            owner
                .private_mut()
                .apply_owned_submit_result(canonical.client_order, state_result)?,
        )
    };
    if !valid_place_apply(result.classification(), applied, accepted_before_ambiguous) {
        return violation(
            owner,
            PmAuthenticatedReductionError::UnexpectedSubmitTransition,
            monotonic_service_ns,
        );
    }
    owner.count_place_result();
    if result.classification().requires_reconciliation() {
        owner.require_authenticated_reconciliation();
    }
    Ok(())
}

#[allow(
    clippy::result_large_err,
    reason = "canonical mutation reduction keeps its bounded typed owner error inline to preserve the existing transition API"
)]
pub(super) fn apply_live_cancel(
    owner: &mut PmMutationOwner,
    completion: PmLiveCancelCompletion,
    monotonic_service_ns: u64,
) -> Result<(), PmMutationError> {
    validate_service_time(owner, monotonic_service_ns)?;
    if let Err(error) = preflight_live_cancel(owner, &completion) {
        return violation(owner, error, monotonic_service_ns);
    }
    let (intent, result) = completion.into_parts();
    let state_outcome = cancel_state_outcome(result.classification());
    let applied = owner
        .private_mut()
        .apply_owned_cancel_result(intent, state_outcome)?;
    if !valid_cancel_apply(result.classification(), applied) {
        return violation(
            owner,
            PmAuthenticatedReductionError::UnexpectedCancelTransition,
            monotonic_service_ns,
        );
    }
    if result.classification() == PmJournalAuthenticatedClassificationV1::Accepted
        && applied == PmOwnedCancelApply::Cancelled
    {
        let _required =
            owner.require_refresh(reap_pm_state::PmRefreshReason::MissingOrderDetail)?;
    }
    owner.count_cancel_result();
    if result.classification().requires_reconciliation() {
        owner.require_authenticated_reconciliation();
    }
    Ok(())
}

fn place_state_result(
    result: crate::journal::PmJournalAuthenticatedPlaceResultV1,
) -> Result<PmOwnedSubmitResult, PmAuthenticatedReductionError> {
    Ok(match result.classification() {
        PmJournalAuthenticatedClassificationV1::Accepted => PmOwnedSubmitResult::Accepted(
            result
                .canonical()
                .venue_order
                .ok_or(PmAuthenticatedReductionError::ResultShapeMismatch)?,
        ),
        PmJournalAuthenticatedClassificationV1::Rejected
        | PmJournalAuthenticatedClassificationV1::DefinitelyNotDispatched => {
            PmOwnedSubmitResult::Rejected
        }
        PmJournalAuthenticatedClassificationV1::OutOfProfile
        | PmJournalAuthenticatedClassificationV1::AcknowledgementUnknown => {
            PmOwnedSubmitResult::AmbiguousOwned(expected_venue_order(
                result.canonical().client_order,
                result.expected_order_id(),
            )?)
        }
    })
}

fn expected_venue_order(
    client_order: reap_pm_core::PmClientOrderKey,
    expected: [u8; 32],
) -> Result<reap_pm_core::PmVenueOrderKey, PmAuthenticatedReductionError> {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut identity = String::with_capacity(66);
    identity.push_str("0x");
    for byte in expected {
        identity.push(char::from(HEX[usize::from(byte >> 4)]));
        identity.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    let venue = reap_pm_core::PmVenueOrderId::new(&identity)
        .map_err(|_| PmAuthenticatedReductionError::ResultShapeMismatch)?;
    Ok(reap_pm_core::PmVenueOrderKey::new(
        client_order.account(),
        venue,
    ))
}

const fn cancel_state_outcome(
    classification: PmJournalAuthenticatedClassificationV1,
) -> PmOwnedCancelOutcome {
    match classification {
        PmJournalAuthenticatedClassificationV1::Accepted => PmOwnedCancelOutcome::Accepted,
        PmJournalAuthenticatedClassificationV1::Rejected
        | PmJournalAuthenticatedClassificationV1::DefinitelyNotDispatched => {
            PmOwnedCancelOutcome::Rejected
        }
        PmJournalAuthenticatedClassificationV1::OutOfProfile
        | PmJournalAuthenticatedClassificationV1::AcknowledgementUnknown => {
            PmOwnedCancelOutcome::Ambiguous
        }
    }
}

const fn ambiguous_after_exact_private_accept(
    classification: PmJournalAuthenticatedClassificationV1,
    state: PmOwnedSubmitState,
) -> bool {
    matches!(
        classification,
        PmJournalAuthenticatedClassificationV1::OutOfProfile
            | PmJournalAuthenticatedClassificationV1::AcknowledgementUnknown
    ) && matches!(state, PmOwnedSubmitState::Accepted)
}

const fn valid_place_apply(
    classification: PmJournalAuthenticatedClassificationV1,
    applied: Option<PmOwnedSubmitApply>,
    accepted_before_ambiguous: bool,
) -> bool {
    match classification {
        PmJournalAuthenticatedClassificationV1::Accepted => matches!(
            applied,
            Some(
                PmOwnedSubmitApply::Accepted
                    | PmOwnedSubmitApply::LateAccepted
                    | PmOwnedSubmitApply::Duplicate
            )
        ),
        PmJournalAuthenticatedClassificationV1::Rejected
        | PmJournalAuthenticatedClassificationV1::DefinitelyNotDispatched => matches!(
            applied,
            Some(PmOwnedSubmitApply::Rejected | PmOwnedSubmitApply::Duplicate)
        ),
        PmJournalAuthenticatedClassificationV1::OutOfProfile
        | PmJournalAuthenticatedClassificationV1::AcknowledgementUnknown => {
            accepted_before_ambiguous
                || matches!(
                    applied,
                    Some(PmOwnedSubmitApply::MarkedAmbiguous | PmOwnedSubmitApply::Duplicate)
                )
        }
    }
}

const fn valid_cancel_apply(
    classification: PmJournalAuthenticatedClassificationV1,
    applied: PmOwnedCancelApply,
) -> bool {
    match classification {
        PmJournalAuthenticatedClassificationV1::Accepted => matches!(
            applied,
            PmOwnedCancelApply::Cancelled
                | PmOwnedCancelApply::ConvergedFilled
                | PmOwnedCancelApply::Duplicate
        ),
        PmJournalAuthenticatedClassificationV1::Rejected
        | PmJournalAuthenticatedClassificationV1::DefinitelyNotDispatched => matches!(
            applied,
            PmOwnedCancelApply::Rejected | PmOwnedCancelApply::ConvergedFilled
        ),
        PmJournalAuthenticatedClassificationV1::OutOfProfile
        | PmJournalAuthenticatedClassificationV1::AcknowledgementUnknown => matches!(
            applied,
            PmOwnedCancelApply::MarkedAmbiguous
                | PmOwnedCancelApply::ConvergedFilled
                | PmOwnedCancelApply::Duplicate
        ),
    }
}

fn exact_order_bytes(order: reap_pm_core::PmVenueOrderKey) -> Option<[u8; 32]> {
    let identity = order.id();
    let bytes = identity.as_str().as_bytes();
    if bytes.len() != 66 || &bytes[..2] != b"0x" {
        return None;
    }
    let mut decoded = [0_u8; 32];
    for (index, output) in decoded.iter_mut().enumerate() {
        let high = decode_lower_hex(bytes[2 + index * 2])?;
        let low = decode_lower_hex(bytes[3 + index * 2])?;
        *output = (high << 4) | low;
    }
    Some(decoded)
}

const fn decode_lower_hex(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        _ => None,
    }
}

#[allow(
    clippy::result_large_err,
    reason = "service-time validation returns the bounded typed mutation failure inline for fail-closed owner entry"
)]
fn validate_service_time(
    owner: &mut PmMutationOwner,
    monotonic_service_ns: u64,
) -> Result<(), PmMutationError> {
    if monotonic_service_ns == 0 {
        violation(
            owner,
            PmAuthenticatedReductionError::ZeroMonotonicServiceTime,
            monotonic_service_ns,
        )
    } else {
        Ok(())
    }
}

#[allow(
    clippy::result_large_err,
    reason = "terminal violation handling returns the bounded typed mutation failure inline after latching owner safety"
)]
fn violation<T>(
    owner: &mut PmMutationOwner,
    error: PmAuthenticatedReductionError,
    monotonic_service_ns: u64,
) -> Result<T, PmMutationError> {
    let reason = error.safety_reason();
    let _ = owner.enter_terminal_safety(reason, monotonic_service_ns);
    Err(error.into())
}

trait ReconciliationClassification {
    fn requires_reconciliation(self) -> bool;
}

impl ReconciliationClassification for PmJournalAuthenticatedClassificationV1 {
    fn requires_reconciliation(self) -> bool {
        matches!(self, Self::OutOfProfile | Self::AcknowledgementUnknown)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub(crate) enum PmAuthenticatedReductionError {
    #[error("authenticated PM result reduction requires nonzero monotonic service time")]
    ZeroMonotonicServiceTime,
    #[error("authenticated PM result lies outside the exact mutation-owner scope")]
    ScopeMismatch,
    #[error("authenticated PM result names no canonical owned order")]
    UnknownOwnedOrder,
    #[error("authenticated PM place result has an impossible canonical shape")]
    ResultShapeMismatch,
    #[error("authenticated PM place result received an impossible canonical transition")]
    UnexpectedSubmitTransition,
    #[error("authenticated PM cancel result lost exact locally owned identity")]
    CancelOwnershipMismatch,
    #[error("authenticated PM cancel result received an impossible canonical transition")]
    UnexpectedCancelTransition,
}

impl PmAuthenticatedReductionError {
    pub(super) const fn safety_reason(self) -> PmJournalSafetyReasonV1 {
        if matches!(self, Self::UnknownOwnedOrder) {
            PmJournalSafetyReasonV1::UnresolvedOwnership
        } else {
            PmJournalSafetyReasonV1::ContractViolation
        }
    }
}
