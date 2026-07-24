//! Exact canonical bootstrap from the checked PM mutation journal.

#![allow(
    clippy::result_large_err,
    reason = "the exact inline recovery error preserves allocation-free fail-closed bootstrap"
)]

use reap_pm_core::{PmFillExecution, PmFillFee, PmFillKey, PmOrderProgress, PmPrice};
use reap_pm_state::{
    PmExactReservation, PmOwnedCancelOutcome, PmOwnedCancelRequestApply, PmOwnedCancelState,
    PmOwnedFillApply, PmOwnedIntentId, PmOwnedObservationOccurrence, PmOwnedObservationSource,
    PmOwnedOrderProgressObservation, PmOwnedProgressApply, PmOwnedQuoteAdmission,
    PmOwnedQuoteIntent, PmOwnedQuoteSlotKey, PmOwnedRecoveryFill, PmOwnedReductionSequence,
    PmOwnedSubmitResult, PmOwnedSubmitState, PmPrivateOccurrence,
};

use super::mutation::PmMutationError;
use crate::journal::{
    PmJournalFillAppliedV1, PmJournalFillOccurrenceV1, PmJournalFillSourceV1,
    PmJournalOrderProgressSourceV1, PmJournalOrderTerminalV1, PmJournalRecoveredObservationV1,
    PmJournalRecoveredOrderV1, PmJournalRecoveredPlaceV1, PmJournalRecovery,
    PmJournalTerminalStatusV1,
};
use crate::private_monitor::PmPrivateMonitorRuntime;

pub(super) fn recover_private_owner(
    private: &mut PmPrivateMonitorRuntime,
    recovery: &PmJournalRecovery,
) -> Result<(), PmMutationError> {
    for row in recovery.recovered_orders() {
        recover_owned_order(private, row)?;
    }
    for observation in recovery.recovered_observations() {
        match observation {
            PmJournalRecoveredObservationV1::FillApplied(applied) => {
                recover_owned_fill(private, applied)?;
            }
            PmJournalRecoveredObservationV1::OrderTerminal(terminal) => {
                recover_owned_terminal(private, terminal)?;
            }
        }
    }
    if recovery.last_owned_observation_sequence() != 0 {
        private.finish_owned_recovery(PmOwnedReductionSequence::new(
            recovery.last_owned_observation_sequence(),
        )?)?;
    }
    for row in recovery.recovered_orders() {
        recover_owned_cancel(private, row)?;
        validate_recovered_order(private, row)?;
    }
    Ok(())
}

fn recover_owned_order(
    private: &mut PmPrivateMonitorRuntime,
    row: PmJournalRecoveredOrderV1,
) -> Result<(), PmMutationError> {
    let journal = row.intent();
    let side = journal.side.into();
    let intent = PmOwnedQuoteIntent::new(
        PmOwnedIntentId::new(journal.intent_id)?,
        PmOwnedQuoteSlotKey::new(private.account_scope(), private.instrument(), side),
        journal.client_order,
        PmPrice::from_units(journal.price_units)?,
        journal.quantity,
        PmExactReservation::policy_approved(journal.reserved_collateral, journal.reserved_outcome)
            .map_err(|_| PmMutationError::RecoveryProjectionMismatch)?,
    )?;
    if private.admit_owned_quote(intent)? != PmOwnedQuoteAdmission::Admitted(journal.client_order) {
        return Err(PmMutationError::RecoveryProjectionMismatch);
    }

    match row.place() {
        PmJournalRecoveredPlaceV1::IntentOnly => {}
        PmJournalRecoveredPlaceV1::Unknown => {
            if row.venue_order().is_some() {
                return Err(PmMutationError::RecoveryProjectionMismatch);
            }
            private
                .apply_owned_submit_result(journal.client_order, PmOwnedSubmitResult::Ambiguous)?;
        }
        PmJournalRecoveredPlaceV1::Bound => {
            let venue = row
                .venue_order()
                .ok_or(PmMutationError::RecoveryProjectionMismatch)?;
            private.apply_owned_submit_result(
                journal.client_order,
                PmOwnedSubmitResult::Accepted(venue),
            )?;
        }
        PmJournalRecoveredPlaceV1::Rejected => {
            if row.venue_order().is_some() {
                return Err(PmMutationError::RecoveryProjectionMismatch);
            }
            private
                .apply_owned_submit_result(journal.client_order, PmOwnedSubmitResult::Rejected)?;
        }
    }
    Ok(())
}

fn recover_owned_fill(
    private: &mut PmPrivateMonitorRuntime,
    applied: PmJournalFillAppliedV1,
) -> Result<(), PmMutationError> {
    let fill = applied.fill;
    let occurrence = recovered_observation_occurrence(applied.occurrence, applied.source)?;
    let source = match applied.source {
        PmJournalFillSourceV1::PlaceAcknowledgement => {
            PmOwnedObservationSource::ImmediateAcknowledgement
        }
        PmJournalFillSourceV1::PrivateWebsocket => PmOwnedObservationSource::PrivateWebSocket,
        PmJournalFillSourceV1::RestReconciliation => PmOwnedObservationSource::RestReconciliation,
    };
    let key = PmFillKey::new(fill.key.venue_order, fill.key.fill_id);
    let execution = PmFillExecution::new(
        fill.side.into(),
        fill.role.into(),
        fill.settlement.into(),
        PmPrice::from_units(fill.price_units)?,
        fill.delta,
        PmFillFee::try_from(fill.fee)?,
    );
    let event = private.owned_fill_event(key, fill.client_order, execution)?;
    let result = private.recover_owned_fill(PmOwnedRecoveryFill::new(
        event,
        fill.authoritative_cumulative,
        occurrence,
        source,
    ))?;
    if !matches!(result, PmOwnedFillApply::Applied { .. }) {
        return Err(PmMutationError::RecoveryProjectionMismatch);
    }
    Ok(())
}

fn recover_owned_terminal(
    private: &mut PmPrivateMonitorRuntime,
    terminal: PmJournalOrderTerminalV1,
) -> Result<(), PmMutationError> {
    let order = private
        .owned_order(terminal.client_order)
        .ok_or(PmMutationError::RecoveryProjectionMismatch)?;
    let progress = PmOrderProgress::new(
        order.quantity(),
        terminal.cumulative,
        match terminal.status {
            PmJournalTerminalStatusV1::Filled => reap_pm_core::PmOrderStatus::Filled,
            PmJournalTerminalStatusV1::Cancelled => reap_pm_core::PmOrderStatus::Cancelled,
            PmJournalTerminalStatusV1::Rejected => reap_pm_core::PmOrderStatus::Rejected,
            PmJournalTerminalStatusV1::Expired => reap_pm_core::PmOrderStatus::Expired,
        },
    )?;
    if progress.remaining_quantity_units() != terminal.remaining
        || order.venue_order() != Some(terminal.venue_order)
    {
        return Err(PmMutationError::RecoveryProjectionMismatch);
    }
    let (source, occurrence_source) = match terminal.source {
        PmJournalOrderProgressSourceV1::PrivateWebsocket => (
            PmOwnedObservationSource::PrivateWebSocket,
            PmJournalFillSourceV1::PrivateWebsocket,
        ),
        PmJournalOrderProgressSourceV1::RestReconciliation => (
            PmOwnedObservationSource::RestReconciliation,
            PmJournalFillSourceV1::RestReconciliation,
        ),
    };
    let occurrence = recovered_observation_occurrence(terminal.occurrence, occurrence_source)?;
    let outcome = private.recover_owned_progress(PmOwnedOrderProgressObservation::new(
        terminal.client_order,
        terminal.venue_order,
        progress,
        occurrence,
        source,
    ))?;
    if !matches!(outcome, PmOwnedProgressApply::Applied { .. }) {
        return Err(PmMutationError::RecoveryProjectionMismatch);
    }
    Ok(())
}

fn recovered_observation_occurrence(
    journal: PmJournalFillOccurrenceV1,
    source: PmJournalFillSourceV1,
) -> Result<PmOwnedObservationOccurrence, PmMutationError> {
    let reduction = PmOwnedReductionSequence::new(journal.owner_sequence.value())?;
    let private = match source {
        PmJournalFillSourceV1::PlaceAcknowledgement => None,
        PmJournalFillSourceV1::PrivateWebsocket | PmJournalFillSourceV1::RestReconciliation => {
            Some(PmPrivateOccurrence::new(
                journal
                    .connection_epoch
                    .ok_or(PmMutationError::RecoveryProjectionMismatch)?,
                journal
                    .ingress_sequence
                    .ok_or(PmMutationError::RecoveryProjectionMismatch)?,
            ))
        }
    };
    Ok(PmOwnedObservationOccurrence::new(
        reduction,
        private,
        journal.snapshot_revision,
    )?)
}

fn recover_owned_cancel(
    private: &mut PmPrivateMonitorRuntime,
    row: PmJournalRecoveredOrderV1,
) -> Result<(), PmMutationError> {
    let outcome = match row.terminal() {
        Some(PmJournalTerminalStatusV1::Cancelled) => Some(PmOwnedCancelOutcome::Accepted),
        Some(PmJournalTerminalStatusV1::Expired) => None,
        Some(PmJournalTerminalStatusV1::Filled | PmJournalTerminalStatusV1::Rejected) => None,
        None if row.cancel_unknown() => Some(PmOwnedCancelOutcome::Ambiguous),
        None => None,
    };
    if !row.cancel_pending() && outcome.is_none() {
        return Ok(());
    }
    let intent = match private.request_owned_cancel(row.intent().client_order)? {
        PmOwnedCancelRequestApply::Issued(intent)
        | PmOwnedCancelRequestApply::Duplicate(intent) => Some(intent),
        PmOwnedCancelRequestApply::AlreadyTerminal => None,
    };
    if let (Some(intent), Some(outcome)) = (intent, outcome) {
        private.apply_owned_cancel_result(intent, outcome)?;
    }
    Ok(())
}

fn validate_recovered_order(
    private: &PmPrivateMonitorRuntime,
    row: PmJournalRecoveredOrderV1,
) -> Result<(), PmMutationError> {
    let journal = row.intent();
    let order = private
        .owned_order(journal.client_order)
        .ok_or(PmMutationError::RecoveryProjectionMismatch)?;
    let expected_submit = match row.place() {
        PmJournalRecoveredPlaceV1::IntentOnly => PmOwnedSubmitState::Pending,
        PmJournalRecoveredPlaceV1::Unknown => PmOwnedSubmitState::Ambiguous,
        PmJournalRecoveredPlaceV1::Bound => PmOwnedSubmitState::Accepted,
        PmJournalRecoveredPlaceV1::Rejected => PmOwnedSubmitState::Rejected,
    };
    let terminal_matches = match row.terminal() {
        None => true,
        Some(PmJournalTerminalStatusV1::Filled) => order
            .status()
            .is_some_and(|status| status == reap_pm_core::PmOrderStatus::Filled),
        Some(PmJournalTerminalStatusV1::Cancelled) => order
            .status()
            .is_some_and(|status| status == reap_pm_core::PmOrderStatus::Cancelled),
        Some(PmJournalTerminalStatusV1::Expired) => order
            .status()
            .is_some_and(|status| status == reap_pm_core::PmOrderStatus::Expired),
        Some(PmJournalTerminalStatusV1::Rejected)
            if row.place() == PmJournalRecoveredPlaceV1::Rejected =>
        {
            order.submit() == PmOwnedSubmitState::Rejected
        }
        Some(PmJournalTerminalStatusV1::Rejected) => order
            .status()
            .is_some_and(|status| status == reap_pm_core::PmOrderStatus::Rejected),
    };
    let cancel_matches = if row.cancel_unknown() && row.terminal().is_none() && !order.is_terminal()
    {
        order.cancel() == PmOwnedCancelState::Ambiguous
    } else if row.cancel_pending() && row.terminal().is_none() && !order.is_terminal() {
        order.cancel() == PmOwnedCancelState::Pending
    } else {
        true
    };
    if order.intent().value() != journal.intent_id
        || order.venue_order() != row.venue_order()
        || order.submit() != expected_submit
        || order.known_fill_total() != row.known_fill_total()
        || order.cumulative_filled() != row.effective_cumulative()
        || order.remaining()
            != journal
                .quantity
                .protocol_units()
                .checked_sub(row.effective_cumulative())
                .map_err(|_| PmMutationError::RecoveryProjectionMismatch)?
        || !terminal_matches
        || !cancel_matches
    {
        return Err(PmMutationError::RecoveryProjectionMismatch);
    }
    Ok(())
}
