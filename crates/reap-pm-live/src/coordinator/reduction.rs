//! Exact fake-effect result reduction.
//!
//! This module is deliberately downstream of prepared mutation authority.
//! It can only reduce a result that the single mutation owner obtained by
//! consuming a prepared fake effect.

#![allow(
    clippy::result_large_err,
    reason = "the exact inline mutation error preserves allocation-free fail-closed reduction"
)]

use reap_pm_core::{IngressSequence, PmClientOrderKey, PmFillExecution, PmFillKey, U256};
use reap_pm_state::{
    PmOwnedCancelApply, PmOwnedCancelIntent, PmOwnedCancelOutcome, PmOwnedFillApply,
    PmOwnedObservationOccurrence, PmOwnedOrderProjection, PmOwnedSubmitApply, PmOwnedSubmitResult,
    PmOwnedSubmitState,
};
use reap_polymarket_adapter::{
    MAX_PM_FAKE_ACK_FILL_LEGS, PmFakeAckImmediateFillLeg, PmFakeCancelOutcome,
    PmFakeCancelRejectReason, PmFakeCancelResult, PmFakePlaceOutcome, PmFakePlaceRejectReason,
    PmFakePlaceResult,
};
use thiserror::Error;

use super::mutation::{PmMutationError, PmMutationOwner};
use crate::journal::{
    PmJournalCancelOutcomeV1, PmJournalCancelRejectReasonV1, PmJournalCancelResultV1,
    PmJournalFillAppliedV1, PmJournalFillDeliveryV1, PmJournalFillFeeV1, PmJournalFillKeyV1,
    PmJournalFillOccurrenceV1, PmJournalFillRoleV1, PmJournalFillSettlementV1,
    PmJournalFillSourceV1, PmJournalFillV1, PmJournalImmediateFillsV1, PmJournalPlaceOutcomeV1,
    PmJournalPlaceRejectReasonV1, PmJournalPlaceResultV1, PmJournalRecordV1,
    PmJournalSafetyReasonV1, PmJournalSideV1,
};

pub(super) fn reduce_fake_place(
    owner: &mut PmMutationOwner,
    result: PmFakePlaceResult,
    monotonic_service_ns: u64,
) -> Result<(), PmMutationError> {
    validate_service_time(owner, monotonic_service_ns)?;
    let order = validate_place_result_scope(owner, &result, monotonic_service_ns)?;
    let required_facts = match result.outcome() {
        PmFakePlaceOutcome::Acknowledged(acknowledgement) => {
            1_usize.saturating_add(acknowledgement.immediate_fills().len())
        }
        PmFakePlaceOutcome::Rejected(_) | PmFakePlaceOutcome::AcknowledgementUnknown => 1,
    };
    owner.ensure_fact_capacity(required_facts)?;

    match result.outcome() {
        PmFakePlaceOutcome::Acknowledged(acknowledgement) => reduce_acknowledged_place(
            owner,
            &result,
            order,
            acknowledgement.venue_order(),
            acknowledgement.immediate_fills(),
            monotonic_service_ns,
        ),
        PmFakePlaceOutcome::Rejected(reason) => {
            let apply = apply_submit(
                owner,
                result.client_order(),
                PmOwnedSubmitResult::Rejected,
                monotonic_service_ns,
            )?;
            if apply != PmOwnedSubmitApply::Rejected {
                return contract_violation(
                    owner,
                    PmReductionError::UnexpectedSubmitTransition,
                    monotonic_service_ns,
                );
            }
            record_place_result(
                owner,
                PmJournalPlaceResultV1 {
                    client_order: result.client_order(),
                    outcome: PmJournalPlaceOutcomeV1::Rejected,
                    reject_reason: Some(place_reject_reason(*reason)),
                    venue_order: None,
                    immediate_fills: PmJournalImmediateFillsV1::empty(),
                },
                monotonic_service_ns,
            )
        }
        PmFakePlaceOutcome::AcknowledgementUnknown => {
            let apply = apply_submit(
                owner,
                result.client_order(),
                PmOwnedSubmitResult::Ambiguous,
                monotonic_service_ns,
            )?;
            if apply != PmOwnedSubmitApply::MarkedAmbiguous {
                return contract_violation(
                    owner,
                    PmReductionError::UnexpectedSubmitTransition,
                    monotonic_service_ns,
                );
            }
            record_place_result(
                owner,
                PmJournalPlaceResultV1 {
                    client_order: result.client_order(),
                    outcome: PmJournalPlaceOutcomeV1::AmbiguousTimeout,
                    reject_reason: None,
                    venue_order: None,
                    immediate_fills: PmJournalImmediateFillsV1::empty(),
                },
                monotonic_service_ns,
            )
        }
    }
}

fn reduce_acknowledged_place(
    owner: &mut PmMutationOwner,
    result: &PmFakePlaceResult,
    prior: PmOwnedOrderProjection,
    venue_order: reap_pm_core::PmVenueOrderKey,
    fills: &[PmFakeAckImmediateFillLeg],
    monotonic_service_ns: u64,
) -> Result<(), PmMutationError> {
    let expected_apply = match prior.submit() {
        PmOwnedSubmitState::Pending => PmOwnedSubmitApply::Accepted,
        PmOwnedSubmitState::Ambiguous => PmOwnedSubmitApply::LateAccepted,
        PmOwnedSubmitState::Accepted | PmOwnedSubmitState::Rejected => {
            return contract_violation(
                owner,
                PmReductionError::UnexpectedSubmitTransition,
                monotonic_service_ns,
            );
        }
    };
    let apply = apply_submit(
        owner,
        result.client_order(),
        PmOwnedSubmitResult::Accepted(venue_order),
        monotonic_service_ns,
    )?;
    if apply != expected_apply {
        return contract_violation(
            owner,
            PmReductionError::UnexpectedSubmitTransition,
            monotonic_service_ns,
        );
    }

    let immediate_fills = match acknowledgement_fill_keys(fills) {
        Ok(fills) => fills,
        Err(error) => return contract_violation(owner, error, monotonic_service_ns),
    };
    let outcome = match acknowledged_place_outcome(prior.submit(), !fills.is_empty()) {
        Ok(outcome) => outcome,
        Err(error) => return contract_violation(owner, error, monotonic_service_ns),
    };
    record_place_result(
        owner,
        PmJournalPlaceResultV1 {
            client_order: result.client_order(),
            outcome,
            reject_reason: None,
            venue_order: Some(venue_order),
            immediate_fills,
        },
        monotonic_service_ns,
    )?;

    for fill in fills {
        reduce_immediate_fill(owner, result, *fill, monotonic_service_ns)?;
    }
    Ok(())
}

fn acknowledgement_fill_keys(
    fills: &[PmFakeAckImmediateFillLeg],
) -> Result<PmJournalImmediateFillsV1, PmReductionError> {
    if fills.len() > MAX_PM_FAKE_ACK_FILL_LEGS {
        return Err(PmReductionError::TooManyImmediateFills);
    }
    let Some(first) = fills.first() else {
        return Ok(PmJournalImmediateFillsV1::empty());
    };
    let first = journal_fill_key(first.key());
    let mut keys = [first; MAX_PM_FAKE_ACK_FILL_LEGS];
    for (target, fill) in keys.iter_mut().zip(fills) {
        *target = journal_fill_key(fill.key());
    }
    Ok(PmJournalImmediateFillsV1::from_slice(&keys[..fills.len()])?)
}

fn reduce_immediate_fill(
    owner: &mut PmMutationOwner,
    result: &PmFakePlaceResult,
    leg: PmFakeAckImmediateFillLeg,
    monotonic_service_ns: u64,
) -> Result<(), PmMutationError> {
    if leg.key().venue_order().account() != owner.account_scope().handle()
        || leg.execution().side() != result.side()
    {
        return contract_violation(
            owner,
            PmReductionError::FillFactsMismatch,
            monotonic_service_ns,
        );
    }

    let event = match owner.private_mut().owned_fill_event(
        leg.key(),
        result.client_order(),
        leg.execution(),
    ) {
        Ok(event) => event,
        Err(error) => {
            enter_reduction_safety(
                owner,
                PmJournalSafetyReasonV1::ContractViolation,
                monotonic_service_ns,
            );
            return Err(PmReductionError::Event(error).into());
        }
    };
    let ticket = match owner.private_mut().issue_owned_immediate_ack_ticket() {
        Ok(ticket) => ticket,
        Err(error) => {
            enter_reduction_safety(
                owner,
                PmJournalSafetyReasonV1::ContractViolation,
                monotonic_service_ns,
            );
            return Err(error.into());
        }
    };
    let occurrence = ticket.occurrence();
    let apply = match owner
        .private_mut()
        .observe_owned_immediate_fill(ticket, event, None)
    {
        Ok(apply) => apply,
        Err(error) => {
            enter_reduction_safety(
                owner,
                PmJournalSafetyReasonV1::ContractViolation,
                monotonic_service_ns,
            );
            return Err(error.into());
        }
    };

    match apply {
        PmOwnedFillApply::Applied {
            client_order,
            cumulative_filled,
            remaining,
        } => {
            if client_order != result.client_order() {
                return contract_violation(
                    owner,
                    PmReductionError::FillFactsMismatch,
                    monotonic_service_ns,
                );
            }
            let record = immediate_fill_record(
                owner,
                client_order,
                leg.key(),
                leg.execution(),
                occurrence,
                cumulative_filled,
                remaining,
                monotonic_service_ns,
            )?;
            owner.record_fact(PmJournalRecordV1::FillApplied(record), monotonic_service_ns)?;
            owner.count_unique_fill();
            Ok(())
        }
        PmOwnedFillApply::Duplicate { client_order, .. } => {
            if client_order != result.client_order() {
                return contract_violation(
                    owner,
                    PmReductionError::FillFactsMismatch,
                    monotonic_service_ns,
                );
            }
            // The first exact application already owns the one durable
            // FillApplied record. A delivery duplicate only enriches the
            // canonical source mask; it never appends a second mutation fact.
            owner.count_duplicate_fill();
            Ok(())
        }
        PmOwnedFillApply::IgnoredOldEpoch => contract_violation(
            owner,
            PmReductionError::UnexpectedFillTransition,
            monotonic_service_ns,
        ),
    }
}

#[allow(clippy::too_many_arguments)]
fn immediate_fill_record(
    owner: &mut PmMutationOwner,
    client_order: PmClientOrderKey,
    key: PmFillKey,
    execution: PmFillExecution,
    occurrence: PmOwnedObservationOccurrence,
    cumulative: U256,
    remaining: U256,
    monotonic_service_ns: u64,
) -> Result<PmJournalFillAppliedV1, PmMutationError> {
    let owner_sequence = occurrence.reduction_sequence().value();
    if owner_sequence == 0
        || occurrence.private_occurrence().is_some()
        || occurrence.snapshot_revision().is_some()
    {
        return contract_violation(
            owner,
            PmReductionError::ImmediateOccurrenceMismatch,
            monotonic_service_ns,
        );
    }
    Ok(PmJournalFillAppliedV1 {
        fill: PmJournalFillV1 {
            key: journal_fill_key(key),
            client_order,
            instrument: owner.instrument_id(),
            side: PmJournalSideV1::from(execution.side()),
            price_units: execution.price().units(),
            role: PmJournalFillRoleV1::from(execution.role()),
            settlement: PmJournalFillSettlementV1::from(execution.settlement()),
            fee: PmJournalFillFeeV1::from(execution.fee()),
            delta: execution.quantity(),
            authoritative_cumulative: None,
            cumulative,
            remaining,
        },
        source: PmJournalFillSourceV1::PlaceAcknowledgement,
        occurrence: immediate_journal_occurrence(occurrence, monotonic_service_ns)?,
        delivery: PmJournalFillDeliveryV1::Live,
    })
}

pub(super) fn reduce_fake_cancel(
    owner: &mut PmMutationOwner,
    intent: PmOwnedCancelIntent,
    result: PmFakeCancelResult,
    monotonic_service_ns: u64,
) -> Result<(), PmMutationError> {
    validate_service_time(owner, monotonic_service_ns)?;
    validate_cancel_result_scope(owner, intent, &result, monotonic_service_ns)?;
    owner.ensure_fact_capacity(1)?;

    let (state_outcome, journal_outcome, reject_reason) = cancel_outcomes(result.outcome());
    let apply = match owner
        .private_mut()
        .apply_owned_cancel_result(intent, state_outcome)
    {
        Ok(apply) => apply,
        Err(error) => {
            enter_reduction_safety(
                owner,
                PmJournalSafetyReasonV1::ContractViolation,
                monotonic_service_ns,
            );
            return Err(error.into());
        }
    };
    validate_cancel_apply(owner, result.outcome(), apply, monotonic_service_ns)?;
    owner.record_fact(
        PmJournalRecordV1::CancelResult(PmJournalCancelResultV1 {
            client_order: result.client_order(),
            venue_order: result.venue_order(),
            outcome: journal_outcome,
            reject_reason,
        }),
        monotonic_service_ns,
    )?;
    owner.count_cancel_result();
    Ok(())
}

fn validate_cancel_apply(
    owner: &mut PmMutationOwner,
    outcome: PmFakeCancelOutcome,
    apply: PmOwnedCancelApply,
    monotonic_service_ns: u64,
) -> Result<(), PmMutationError> {
    let valid = match outcome {
        PmFakeCancelOutcome::Accepted => matches!(
            apply,
            PmOwnedCancelApply::Cancelled | PmOwnedCancelApply::ConvergedFilled
        ),
        PmFakeCancelOutcome::Rejected(_) => matches!(
            apply,
            PmOwnedCancelApply::Rejected | PmOwnedCancelApply::ConvergedFilled
        ),
        PmFakeCancelOutcome::AlreadyFilled => matches!(
            apply,
            PmOwnedCancelApply::Filled | PmOwnedCancelApply::ConvergedFilled
        ),
        PmFakeCancelOutcome::AcknowledgementUnknown => matches!(
            apply,
            PmOwnedCancelApply::MarkedAmbiguous | PmOwnedCancelApply::ConvergedFilled
        ),
    };
    if valid {
        Ok(())
    } else {
        contract_violation(
            owner,
            PmReductionError::UnexpectedCancelTransition,
            monotonic_service_ns,
        )
    }
}

fn acknowledged_place_outcome(
    prior: PmOwnedSubmitState,
    has_fills: bool,
) -> Result<PmJournalPlaceOutcomeV1, PmReductionError> {
    match (prior, has_fills) {
        (PmOwnedSubmitState::Pending, false) => Ok(PmJournalPlaceOutcomeV1::AcceptedResting),
        (PmOwnedSubmitState::Pending, true) => {
            Ok(PmJournalPlaceOutcomeV1::AcceptedWithImmediateFill)
        }
        (PmOwnedSubmitState::Ambiguous, _) => Ok(PmJournalPlaceOutcomeV1::LateAcknowledgement),
        (PmOwnedSubmitState::Accepted | PmOwnedSubmitState::Rejected, _) => {
            Err(PmReductionError::UnexpectedSubmitTransition)
        }
    }
}

fn cancel_outcomes(
    outcome: PmFakeCancelOutcome,
) -> (
    PmOwnedCancelOutcome,
    PmJournalCancelOutcomeV1,
    Option<PmJournalCancelRejectReasonV1>,
) {
    match outcome {
        PmFakeCancelOutcome::Accepted => (
            PmOwnedCancelOutcome::Accepted,
            PmJournalCancelOutcomeV1::Accepted,
            None,
        ),
        PmFakeCancelOutcome::Rejected(reason) => (
            PmOwnedCancelOutcome::Rejected,
            PmJournalCancelOutcomeV1::Rejected,
            Some(cancel_reject_reason(reason)),
        ),
        PmFakeCancelOutcome::AlreadyFilled => (
            PmOwnedCancelOutcome::AlreadyFilled,
            PmJournalCancelOutcomeV1::AlreadyFilled,
            None,
        ),
        PmFakeCancelOutcome::AcknowledgementUnknown => (
            PmOwnedCancelOutcome::Ambiguous,
            PmJournalCancelOutcomeV1::AmbiguousTimeout,
            None,
        ),
    }
}

fn validate_place_result_scope(
    owner: &mut PmMutationOwner,
    result: &PmFakePlaceResult,
    monotonic_service_ns: u64,
) -> Result<PmOwnedOrderProjection, PmMutationError> {
    let expected_scope = owner.account_scope();
    let expected_instrument_id = owner.instrument_id();
    let expected_instrument = owner.private_mut().instrument();
    if result.account_scope() != expected_scope
        || result.instrument() != expected_instrument
        || result.instrument_id() != expected_instrument_id
        || result.client_order().account() != expected_scope.handle()
    {
        return contract_violation(
            owner,
            PmReductionError::ResultScopeMismatch,
            monotonic_service_ns,
        );
    }
    let Some(order) = owner.private_mut().owned_order(result.client_order()) else {
        return contract_violation(
            owner,
            PmReductionError::UnknownOwnedOrder,
            monotonic_service_ns,
        );
    };
    if order.slot().account_scope() != expected_scope
        || order.slot().instrument() != expected_instrument
        || order.slot().side() != result.side()
        || order.price() != result.price()
        || order.quantity() != result.quantity()
    {
        return contract_violation(
            owner,
            PmReductionError::ResultFactsMismatch,
            monotonic_service_ns,
        );
    }
    Ok(order)
}

fn validate_cancel_result_scope(
    owner: &mut PmMutationOwner,
    intent: PmOwnedCancelIntent,
    result: &PmFakeCancelResult,
    monotonic_service_ns: u64,
) -> Result<(), PmMutationError> {
    let expected_scope = owner.account_scope();
    let expected_instrument_id = owner.instrument_id();
    let expected_instrument = owner.private_mut().instrument();
    if result.account_scope() != expected_scope
        || result.instrument() != expected_instrument
        || result.instrument_id() != expected_instrument_id
        || result.client_order() != intent.client_order()
        || result.venue_order() != intent.venue_order()
    {
        return contract_violation(
            owner,
            PmReductionError::CancelFactsMismatch,
            monotonic_service_ns,
        );
    }
    let Some(order) = owner.private_mut().owned_order(result.client_order()) else {
        return contract_violation(
            owner,
            PmReductionError::UnknownOwnedOrder,
            monotonic_service_ns,
        );
    };
    if order.slot().account_scope() != expected_scope
        || order.slot().instrument() != expected_instrument
        || order.venue_order() != Some(result.venue_order())
    {
        return contract_violation(
            owner,
            PmReductionError::CancelFactsMismatch,
            monotonic_service_ns,
        );
    }
    Ok(())
}

fn apply_submit(
    owner: &mut PmMutationOwner,
    client_order: PmClientOrderKey,
    result: PmOwnedSubmitResult,
    monotonic_service_ns: u64,
) -> Result<PmOwnedSubmitApply, PmMutationError> {
    match owner
        .private_mut()
        .apply_owned_submit_result(client_order, result)
    {
        Ok(apply) => Ok(apply),
        Err(error) => {
            enter_reduction_safety(
                owner,
                PmJournalSafetyReasonV1::ContractViolation,
                monotonic_service_ns,
            );
            Err(error.into())
        }
    }
}

fn immediate_journal_occurrence(
    occurrence: PmOwnedObservationOccurrence,
    monotonic_service_ns: u64,
) -> Result<PmJournalFillOccurrenceV1, PmReductionError> {
    let owner_sequence = occurrence.reduction_sequence().value();
    if owner_sequence == 0
        || monotonic_service_ns == 0
        || occurrence.private_occurrence().is_some()
        || occurrence.snapshot_revision().is_some()
    {
        return Err(PmReductionError::ImmediateOccurrenceMismatch);
    }
    Ok(PmJournalFillOccurrenceV1 {
        owner_sequence: IngressSequence::new(owner_sequence),
        connection: None,
        connection_epoch: None,
        ingress_sequence: None,
        snapshot_revision: None,
        monotonic_service_ns,
    })
}

fn record_place_result(
    owner: &mut PmMutationOwner,
    result: PmJournalPlaceResultV1,
    monotonic_service_ns: u64,
) -> Result<(), PmMutationError> {
    owner.record_fact(PmJournalRecordV1::PlaceResult(result), monotonic_service_ns)?;
    owner.count_place_result();
    Ok(())
}

fn validate_service_time(
    owner: &mut PmMutationOwner,
    monotonic_service_ns: u64,
) -> Result<(), PmMutationError> {
    if monotonic_service_ns == 0 {
        enter_reduction_safety(
            owner,
            PmJournalSafetyReasonV1::ContractViolation,
            monotonic_service_ns,
        );
        Err(PmReductionError::ZeroMonotonicServiceTime.into())
    } else {
        Ok(())
    }
}

fn contract_violation<T>(
    owner: &mut PmMutationOwner,
    error: PmReductionError,
    monotonic_service_ns: u64,
) -> Result<T, PmMutationError> {
    let reason = match &error {
        PmReductionError::UnknownOwnedOrder => PmJournalSafetyReasonV1::UnresolvedOwnership,
        _ => PmJournalSafetyReasonV1::ContractViolation,
    };
    enter_reduction_safety(owner, reason, monotonic_service_ns);
    Err(error.into())
}

fn enter_reduction_safety(
    owner: &mut PmMutationOwner,
    reason: PmJournalSafetyReasonV1,
    monotonic_service_ns: u64,
) {
    let _transition = owner.enter_terminal_safety(reason, monotonic_service_ns);
}

const fn journal_fill_key(key: PmFillKey) -> PmJournalFillKeyV1 {
    PmJournalFillKeyV1 {
        venue_order: key.venue_order(),
        fill_id: key.id(),
    }
}

const fn place_reject_reason(reason: PmFakePlaceRejectReason) -> PmJournalPlaceRejectReasonV1 {
    match reason {
        PmFakePlaceRejectReason::FixtureRejected => PmJournalPlaceRejectReasonV1::FixtureRejected,
        PmFakePlaceRejectReason::PostOnlyWouldTake => {
            PmJournalPlaceRejectReasonV1::PostOnlyWouldTake
        }
    }
}

const fn cancel_reject_reason(reason: PmFakeCancelRejectReason) -> PmJournalCancelRejectReasonV1 {
    match reason {
        PmFakeCancelRejectReason::FixtureRejected => PmJournalCancelRejectReasonV1::FixtureRejected,
    }
}

#[derive(Debug, Error)]
pub(crate) enum PmReductionError {
    #[error("PM fake result reduction requires nonzero monotonic service time")]
    ZeroMonotonicServiceTime,
    #[error("PM fake result lies outside the exact mutation-owner scope")]
    ResultScopeMismatch,
    #[error("PM fake result names no canonical owned order")]
    UnknownOwnedOrder,
    #[error("PM fake place result contradicts the canonical quote facts")]
    ResultFactsMismatch,
    #[error("PM fake place result contradicts the canonical submit transition")]
    UnexpectedSubmitTransition,
    #[error("PM fake acknowledgement exceeds the fixed immediate-fill bound")]
    TooManyImmediateFills,
    #[error("PM fake immediate-fill facts contradict their acknowledged order")]
    FillFactsMismatch,
    #[error("PM fake immediate fill received an impossible canonical transition")]
    UnexpectedFillTransition,
    #[error("PM fake immediate fill carries non-immediate occurrence evidence")]
    ImmediateOccurrenceMismatch,
    #[error("PM fake cancel result contradicts its exact owned-cancel intent")]
    CancelFactsMismatch,
    #[error("PM fake cancel result received an impossible canonical transition")]
    UnexpectedCancelTransition,
    #[error(transparent)]
    Event(#[from] reap_pm_core::PmEventError),
    #[error(transparent)]
    JournalSchema(#[from] crate::journal::PmJournalSchemaError),
}

#[cfg(test)]
mod tests {
    use reap_pm_core::{PmAccountHandle, PmFillId, PmFillKey, PmVenueOrderId, PmVenueOrderKey};
    use reap_pm_state::{PmOwnedObservationOccurrence, PmOwnedReductionSequence};

    use super::*;

    #[test]
    fn acknowledged_place_mapping_distinguishes_resting_filled_and_late() {
        assert_eq!(
            acknowledged_place_outcome(PmOwnedSubmitState::Pending, false)
                .expect("resting outcome"),
            PmJournalPlaceOutcomeV1::AcceptedResting
        );
        assert_eq!(
            acknowledged_place_outcome(PmOwnedSubmitState::Pending, true)
                .expect("immediate-fill outcome"),
            PmJournalPlaceOutcomeV1::AcceptedWithImmediateFill
        );
        assert_eq!(
            acknowledged_place_outcome(PmOwnedSubmitState::Ambiguous, false)
                .expect("late resting acknowledgement"),
            PmJournalPlaceOutcomeV1::LateAcknowledgement
        );
        assert!(matches!(
            acknowledged_place_outcome(PmOwnedSubmitState::Accepted, false),
            Err(PmReductionError::UnexpectedSubmitTransition)
        ));
    }

    #[test]
    fn exact_rejection_reasons_survive_journal_mapping() {
        assert_eq!(
            place_reject_reason(PmFakePlaceRejectReason::FixtureRejected),
            PmJournalPlaceRejectReasonV1::FixtureRejected
        );
        assert_eq!(
            place_reject_reason(PmFakePlaceRejectReason::PostOnlyWouldTake),
            PmJournalPlaceRejectReasonV1::PostOnlyWouldTake
        );
        let (_, outcome, reason) = cancel_outcomes(PmFakeCancelOutcome::Rejected(
            PmFakeCancelRejectReason::FixtureRejected,
        ));
        assert_eq!(outcome, PmJournalCancelOutcomeV1::Rejected);
        assert_eq!(reason, Some(PmJournalCancelRejectReasonV1::FixtureRejected));
    }

    #[test]
    fn every_cancel_result_has_one_exact_state_and_journal_mapping() {
        let cases = [
            (
                PmFakeCancelOutcome::Accepted,
                PmOwnedCancelOutcome::Accepted,
                PmJournalCancelOutcomeV1::Accepted,
            ),
            (
                PmFakeCancelOutcome::AlreadyFilled,
                PmOwnedCancelOutcome::AlreadyFilled,
                PmJournalCancelOutcomeV1::AlreadyFilled,
            ),
            (
                PmFakeCancelOutcome::AcknowledgementUnknown,
                PmOwnedCancelOutcome::Ambiguous,
                PmJournalCancelOutcomeV1::AmbiguousTimeout,
            ),
        ];
        for (input, expected_state, expected_journal) in cases {
            let (state, journal, reason) = cancel_outcomes(input);
            assert_eq!(state, expected_state);
            assert_eq!(journal, expected_journal);
            assert_eq!(reason, None);
        }
    }

    #[test]
    fn acknowledgement_occurrence_persists_owner_order_without_source_invention() {
        let occurrence = PmOwnedObservationOccurrence::immediate(
            PmOwnedReductionSequence::new(17).expect("nonzero reduction sequence"),
        );
        let journal =
            immediate_journal_occurrence(occurrence, 9_001).expect("exact occurrence mapping");
        assert_eq!(journal.owner_sequence, IngressSequence::new(17));
        assert_eq!(journal.connection, None);
        assert_eq!(journal.connection_epoch, None);
        assert_eq!(journal.ingress_sequence, None);
        assert_eq!(journal.snapshot_revision, None);
        assert_eq!(journal.monotonic_service_ns, 9_001);
    }

    #[test]
    fn journal_fill_identity_keeps_the_maker_leg_venue_binding() {
        let account = PmAccountHandle::from_ordinal(4);
        let venue = PmVenueOrderKey::new(
            account,
            PmVenueOrderId::new("maker-leg").expect("venue order"),
        );
        let fill = PmFillKey::new(venue, PmFillId::new("trade-42").expect("fill"));
        assert_eq!(
            journal_fill_key(fill),
            PmJournalFillKeyV1 {
                venue_order: venue,
                fill_id: fill.id(),
            }
        );
    }
}
