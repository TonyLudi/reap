//! Durable consequences of canonical private and reconciliation reductions.
//!
//! This module does not own state. It joins exact reduction values emitted by
//! the sole private owner to the PM mutation journal, preserving original
//! connection, occurrence, source, and service-time evidence.

#![allow(
    clippy::result_large_err,
    reason = "the exact inline mutation error preserves allocation-free fail-closed reduction"
)]

use reap_pm_core::{
    EventClock, EventEnvelope, EventOrdering, IngressSequence, PmCompleteAccountSnapshot,
    PmCompleteFillQuery, PmConnectionId, PmOrderStatus, PmProductSource, SnapshotRevision,
};
use reap_pm_state::{
    PmOwnedFillApply, PmOwnedFillReduction, PmOwnedObservationOccurrence, PmOwnedObservationSource,
    PmOwnedProgressApply, PmPrivateFillReduction, PmPrivateOrderReduction, PmReconciliationApply,
    PmReconciliationFillDisposition,
};
use reap_polymarket_adapter::PmPrivateLifecycleObservation;
use thiserror::Error;

use super::mutation::{PmMutationError, PmMutationOwner};
use crate::journal::{
    PmJournalFillAppliedV1, PmJournalFillCursorV1, PmJournalFillDeliveryV1, PmJournalFillFeeV1,
    PmJournalFillKeyV1, PmJournalFillOccurrenceV1, PmJournalFillRoleV1, PmJournalFillSettlementV1,
    PmJournalFillSourceV1, PmJournalFillV1, PmJournalFillWatermarkV1, PmJournalFingerprintV1,
    PmJournalOrderProgressSourceV1, PmJournalOrderTerminalV1, PmJournalRecordV1,
    PmJournalSafetyReasonV1, PmJournalSideV1, PmJournalTerminalStatusV1,
};
use crate::private_monitor::PmServicedPrivateReduction;

#[allow(clippy::too_many_arguments)]
pub(super) fn reduce_private_observation(
    owner: &mut PmMutationOwner,
    source: PmProductSource,
    connection: PmConnectionId,
    clock: EventClock,
    ordering: EventOrdering,
    observation: PmPrivateLifecycleObservation,
) -> Result<PmServicedPrivateReduction, PmMutationError> {
    // Every reached private observation has at most one newly durable owned
    // consequence. Capacity is checked before canonical mutation.
    owner.ensure_fact_capacity(1)?;
    let monotonic_service_ns = clock.monotonic_service_ns();
    let reduction = match owner.private_mut().reduce_serviced_private_observation(
        source,
        connection,
        clock,
        ordering,
        observation,
    ) {
        Ok(reduction) => reduction,
        Err(error) => {
            let _transition = owner.enter_terminal_safety(
                PmJournalSafetyReasonV1::ContractViolation,
                monotonic_service_ns,
            );
            return Err(error.into());
        }
    };
    match reduction {
        PmServicedPrivateReduction::Order(order) => record_private_order(owner, order)?,
        PmServicedPrivateReduction::Fill(fill) => record_private_fill(owner, fill)?,
        PmServicedPrivateReduction::Unresolved(_) => {}
    }
    Ok(reduction)
}

pub(super) fn reduce_reconciliation(
    owner: &mut PmMutationOwner,
    account: EventEnvelope<PmCompleteAccountSnapshot>,
    fills: EventEnvelope<PmCompleteFillQuery>,
) -> Result<PmReconciliationApply, PmMutationError> {
    let expected = ExpectedOccurrence {
        source: PmOwnedObservationSource::RestReconciliation,
        connection: fills.connection_id(),
        epoch: fills.ordering().connection_epoch(),
        ingress: fills.payload().boundary().completion_sequence(),
        snapshot: Some(fills.payload().snapshot().revision()),
    };
    let row_count = owner.reconciliation_fact_upper_bound(fills.payload().fills());
    let requested_after = fills.payload().requested_after();
    let resulting_watermark = fills.payload().resulting_watermark();
    let watermark_advanced = requested_after != Some(resulting_watermark);
    let monotonic_service_ns = fills.clock().monotonic_service_ns();
    // The journal queue is deliberately bounded at 1,024 facts. Known owned
    // fill keys are duplicate/enrichment rows and cannot emit another durable
    // FillApplied fact; every other row remains a conservative upper bound.
    // A larger possibly-new cut is rejected before state mutation and must
    // remain unready/retry; it cannot become partially durable.
    let possible_watermark_record = usize::from(watermark_advanced);
    owner.ensure_fact_capacity(row_count.saturating_add(possible_watermark_record))?;
    if watermark_advanced {
        owner.ensure_fill_watermark_compaction_available()?;
    }
    let outcome = match owner.reduce_private_reconciliation(account, fills) {
        Ok(outcome) => outcome,
        Err(error) => {
            owner.clear_reconciliation_reductions();
            let _transition = owner.enter_terminal_safety(
                PmJournalSafetyReasonV1::ContractViolation,
                monotonic_service_ns,
            );
            return Err(error.into());
        }
    };
    project_reconciliation_consequences(owner, expected, monotonic_service_ns)?;
    if matches!(outcome, PmReconciliationApply::Applied { .. }) && watermark_advanced {
        let compaction = owner.prepare_fill_watermark_compaction()?;
        owner.record_fill_watermark_fact(
            PmJournalRecordV1::FillWatermarkAdvanced(PmJournalFillWatermarkV1 {
                cursor: PmJournalFillCursorV1 {
                    account_scope: resulting_watermark.account_scope(),
                    opaque: PmJournalFingerprintV1::from_bytes(resulting_watermark.opaque()),
                },
            }),
            compaction,
            monotonic_service_ns,
        )?;
    }
    Ok(outcome)
}

fn project_reconciliation_consequences(
    owner: &mut PmMutationOwner,
    expected: ExpectedOccurrence,
    monotonic_service_ns: u64,
) -> Result<(), PmMutationError> {
    let reduction_count = owner.reconciliation_reduction_count();
    let reduction_result = (|| {
        for index in 0..reduction_count {
            let row = owner.reconciliation_reduction(index).ok_or_else(|| {
                fail(
                    owner,
                    PmPrivateReductionError::ScratchMismatch,
                    monotonic_service_ns,
                )
            })?;
            match row.disposition() {
                PmReconciliationFillDisposition::OwnedApplied(owned) => {
                    record_owned_fill(
                        owner,
                        row.envelope(),
                        owned,
                        PmJournalFillSourceV1::RestReconciliation,
                        expected,
                    )?;
                }
                PmReconciliationFillDisposition::OwnedDuplicate(_)
                | PmReconciliationFillDisposition::OwnedStale(_) => {
                    owner.count_duplicate_fill();
                }
                PmReconciliationFillDisposition::Unowned(_) => {}
            }
        }
        Ok::<(), PmMutationError>(())
    })();
    owner.clear_reconciliation_reductions();
    reduction_result
}

fn record_private_order(
    owner: &mut PmMutationOwner,
    reduction: PmPrivateOrderReduction,
) -> Result<(), PmMutationError> {
    let Some(owned) = reduction.owned() else {
        return Ok(());
    };
    let envelope = reduction.envelope();
    let occurrence = journal_occurrence(
        envelope.clock(),
        owned.occurrence(),
        owned.source(),
        ExpectedOccurrence {
            source: PmOwnedObservationSource::PrivateWebSocket,
            connection: envelope.connection_id(),
            epoch: envelope.ordering().connection_epoch(),
            ingress: envelope.ordering().local_ingress_sequence(),
            snapshot: envelope.ordering().snapshot_revision(),
        },
    )
    .map_err(|error| fail(owner, error, envelope.clock().monotonic_service_ns()))?;

    let PmOwnedProgressApply::Applied {
        status,
        cumulative_filled,
        remaining,
    } = owned.apply()
    else {
        return Ok(());
    };
    if !status.is_terminal() {
        return Ok(());
    }

    let observation = owned.observation();
    let Some(order) = owner.private_mut().owned_order(observation.client_order()) else {
        return Err(fail(
            owner,
            PmPrivateReductionError::UnknownOwnedOrder,
            envelope.clock().monotonic_service_ns(),
        ));
    };
    if order.client_order() != observation.client_order()
        || order.venue_order() != Some(observation.venue_order())
        || order.status() != Some(status)
        || order.cumulative_filled() != cumulative_filled
        || order.remaining() != remaining
    {
        return Err(fail(
            owner,
            PmPrivateReductionError::OrderFactsMismatch,
            envelope.clock().monotonic_service_ns(),
        ));
    }

    // Terminal progress can arrive before its exact fill legs. Only a
    // complete principal cut is admitted to recovery or releases reservation.
    if order.known_fill_total() != cumulative_filled {
        return Ok(());
    }
    let Some(status) = terminal_status(status) else {
        return Err(fail(
            owner,
            PmPrivateReductionError::OrderFactsMismatch,
            envelope.clock().monotonic_service_ns(),
        ));
    };
    owner.record_fact(
        PmJournalRecordV1::OrderTerminal(PmJournalOrderTerminalV1 {
            client_order: observation.client_order(),
            venue_order: observation.venue_order(),
            status,
            cumulative: cumulative_filled,
            remaining,
            source: PmJournalOrderProgressSourceV1::PrivateWebsocket,
            occurrence,
        }),
        envelope.clock().monotonic_service_ns(),
    )
}

fn record_private_fill(
    owner: &mut PmMutationOwner,
    reduction: PmPrivateFillReduction,
) -> Result<(), PmMutationError> {
    let Some(owned) = reduction.owned() else {
        return Ok(());
    };
    let envelope = reduction.envelope();
    record_owned_fill(
        owner,
        envelope,
        owned,
        PmJournalFillSourceV1::PrivateWebsocket,
        ExpectedOccurrence {
            source: PmOwnedObservationSource::PrivateWebSocket,
            connection: envelope.connection_id(),
            epoch: envelope.ordering().connection_epoch(),
            ingress: envelope.ordering().local_ingress_sequence(),
            snapshot: envelope.ordering().snapshot_revision(),
        },
    )
}

fn record_owned_fill(
    owner: &mut PmMutationOwner,
    envelope: EventEnvelope<reap_pm_core::PmFillEvent>,
    owned: PmOwnedFillReduction,
    journal_source: PmJournalFillSourceV1,
    expected: ExpectedOccurrence,
) -> Result<(), PmMutationError> {
    let event = *envelope.payload();
    let observation = owned.observation();
    if observation.key() != event.fill_key()
        || observation.quantity() != event.execution().quantity()
    {
        return Err(fail(
            owner,
            PmPrivateReductionError::FillFactsMismatch,
            envelope.clock().monotonic_service_ns(),
        ));
    }
    let occurrence = journal_occurrence(
        envelope.clock(),
        owned.occurrence(),
        owned.source(),
        expected,
    )
    .map_err(|error| fail(owner, error, envelope.clock().monotonic_service_ns()))?;
    match owned.apply() {
        PmOwnedFillApply::Applied {
            client_order,
            cumulative_filled,
            remaining,
        } => {
            let execution = event.execution();
            owner.record_fact(
                PmJournalRecordV1::FillApplied(PmJournalFillAppliedV1 {
                    fill: PmJournalFillV1 {
                        key: PmJournalFillKeyV1 {
                            venue_order: event.fill_key().venue_order(),
                            fill_id: event.fill_key().id(),
                        },
                        client_order,
                        instrument: owner.instrument_id(),
                        side: PmJournalSideV1::from(execution.side()),
                        price_units: execution.price().units(),
                        role: PmJournalFillRoleV1::from(execution.role()),
                        settlement: PmJournalFillSettlementV1::from(execution.settlement()),
                        fee: PmJournalFillFeeV1::from(execution.fee()),
                        delta: execution.quantity(),
                        authoritative_cumulative: observation.reported_cumulative(),
                        cumulative: cumulative_filled,
                        remaining,
                    },
                    source: journal_source,
                    occurrence,
                    delivery: PmJournalFillDeliveryV1::Live,
                }),
                envelope.clock().monotonic_service_ns(),
            )?;
            owner.count_unique_fill();
            Ok(())
        }
        PmOwnedFillApply::Duplicate { .. } => {
            owner.count_duplicate_fill();
            Ok(())
        }
        PmOwnedFillApply::IgnoredOldEpoch => Err(fail(
            owner,
            PmPrivateReductionError::FillFactsMismatch,
            envelope.clock().monotonic_service_ns(),
        )),
    }
}

#[derive(Clone, Copy)]
struct ExpectedOccurrence {
    source: PmOwnedObservationSource,
    connection: PmConnectionId,
    epoch: reap_pm_core::ConnectionEpoch,
    ingress: IngressSequence,
    snapshot: Option<SnapshotRevision>,
}

fn validate_occurrence(
    clock: EventClock,
    occurrence: PmOwnedObservationOccurrence,
    source: PmOwnedObservationSource,
    expected: ExpectedOccurrence,
) -> Result<(), PmPrivateReductionError> {
    let Some(private) = occurrence.private_occurrence() else {
        return Err(PmPrivateReductionError::OccurrenceMismatch);
    };
    if source != expected.source
        || clock.monotonic_service_ns() == 0
        || occurrence.snapshot_revision() != expected.snapshot
        || private.epoch() != expected.epoch
        || private.ingress() != expected.ingress
    {
        return Err(PmPrivateReductionError::OccurrenceMismatch);
    }
    Ok(())
}

fn journal_occurrence(
    clock: EventClock,
    occurrence: PmOwnedObservationOccurrence,
    source: PmOwnedObservationSource,
    expected: ExpectedOccurrence,
) -> Result<PmJournalFillOccurrenceV1, PmPrivateReductionError> {
    validate_occurrence(clock, occurrence, source, expected)?;
    if occurrence.reduction_sequence().value() == 0 {
        return Err(PmPrivateReductionError::OccurrenceMismatch);
    }
    Ok(PmJournalFillOccurrenceV1 {
        owner_sequence: IngressSequence::new(occurrence.reduction_sequence().value()),
        connection: Some(expected.connection),
        connection_epoch: Some(expected.epoch),
        ingress_sequence: Some(expected.ingress),
        snapshot_revision: expected.snapshot,
        monotonic_service_ns: clock.monotonic_service_ns(),
    })
}

const fn terminal_status(status: PmOrderStatus) -> Option<PmJournalTerminalStatusV1> {
    match status {
        PmOrderStatus::Filled => Some(PmJournalTerminalStatusV1::Filled),
        PmOrderStatus::Cancelled => Some(PmJournalTerminalStatusV1::Cancelled),
        PmOrderStatus::Rejected => Some(PmJournalTerminalStatusV1::Rejected),
        PmOrderStatus::Expired => Some(PmJournalTerminalStatusV1::Expired),
        PmOrderStatus::Pending | PmOrderStatus::Open | PmOrderStatus::PartiallyFilled => None,
    }
}

fn fail(
    owner: &mut PmMutationOwner,
    error: PmPrivateReductionError,
    monotonic_service_ns: u64,
) -> PmMutationError {
    let reason = match &error {
        PmPrivateReductionError::UnknownOwnedOrder => PmJournalSafetyReasonV1::UnresolvedOwnership,
        _ => PmJournalSafetyReasonV1::ContractViolation,
    };
    let _transition = owner.enter_terminal_safety(reason, monotonic_service_ns);
    PmMutationError::PrivateReduction(error)
}

#[derive(Debug, Error)]
pub(crate) enum PmPrivateReductionError {
    #[error("PM serviced private reduction lost its exact occurrence evidence")]
    OccurrenceMismatch,
    #[error("PM serviced private reduction names no canonical owned order")]
    UnknownOwnedOrder,
    #[error("PM serviced private order facts contradict canonical ownership")]
    OrderFactsMismatch,
    #[error("PM serviced private fill facts contradict canonical ownership")]
    FillFactsMismatch,
    #[error("PM reconciliation reduction scratch disagrees with its committed cut")]
    ScratchMismatch,
}

#[cfg(test)]
mod tests;
