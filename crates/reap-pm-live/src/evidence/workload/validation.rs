use super::{CounterCut, PassRawProjection, delta};
use crate::evidence::PmEvidenceError;
use crate::evidence::contract::{
    CANCEL_INTENTS, CANCEL_RESULTS, MEASURED_CYCLES, MEASURED_EXTERNAL_OBSERVATIONS,
    MEASURED_INTERNAL_FACT_ACKS, MEASURED_JOURNAL_RECORDS, MEASURED_OWNER_REDUCTIONS,
    OBSERVATIONS_PER_CYCLE, PLACE_RESULTS, QUOTE_INTENTS, SUPPRESSED_DUPLICATE_FILLS, UNIQUE_FILLS,
    WATERMARK_ADVANCES,
};
use crate::evidence::report::NominalCounters;

pub(super) fn validate_repeated_passes(
    passes: &[PassRawProjection],
) -> Result<(), PmEvidenceError> {
    let Some(primary) = passes.first() else {
        return Err(PmEvidenceError::invariant("nominal pass list is empty"));
    };
    if !primary.terminal_state_lengths_zero {
        return Err(PmEvidenceError::invariant(format!(
            "first nominal pass retained terminal owner state: {:?}",
            primary.terminal_state_lengths
        )));
    }
    for (index, pass) in passes.iter().enumerate().skip(1) {
        let difference = if pass.input_mix != primary.input_mix {
            Some("input mix")
        } else if pass.counters != primary.counters {
            Some("nominal counters")
        } else if pass.journal_record_delta != primary.journal_record_delta {
            Some("journal record delta")
        } else if pass.journal_hash != primary.journal_hash {
            Some("journal hash")
        } else if pass.logical_hash != primary.logical_hash {
            Some("logical hash")
        } else if pass.public_hash != primary.public_hash {
            Some("public hash")
        } else if pass.reserved_capacity_bytes != primary.reserved_capacity_bytes {
            Some("reserved capacity")
        } else if pass.allocator_live_bytes != primary.allocator_live_bytes {
            Some("allocator live-byte delta")
        } else if pass.terminal_state_lengths != primary.terminal_state_lengths
            || !pass.terminal_state_lengths_zero
        {
            Some("terminal state lengths")
        } else {
            None
        };
        if let Some(difference) = difference {
            return Err(PmEvidenceError::invariant(format!(
                "nominal pass {} differs from the first normalized terminal projection in {difference}; first allocator delta={}, actual allocator delta={}, first capacity={}, actual capacity={}, first terminal={:?}, actual terminal={:?}",
                index + 1,
                primary.allocator_live_bytes,
                pass.allocator_live_bytes,
                primary.reserved_capacity_bytes,
                pass.reserved_capacity_bytes,
                primary.terminal_state_lengths,
                pass.terminal_state_lengths,
            )));
        }
    }
    if passes.iter().any(|pass| pass.allocator_live_bytes > 0) {
        return Err(PmEvidenceError::invariant(
            "a normalized nominal pass grew allocator live bytes",
        ));
    }
    Ok(())
}

pub(super) fn validate_nominal(
    cycles: usize,
    counters: NominalCounters,
    before: CounterCut,
    after: CounterCut,
) -> Result<(), PmEvidenceError> {
    let cycles = u64::try_from(cycles).expect("fixed cycle count fits u64");
    let fills = cycles / 2;
    let cancels = cycles / 2;
    let cuts = cycles / 1_000;
    let expected = NominalCounters {
        external_observations: cycles
            * u64::try_from(OBSERVATIONS_PER_CYCLE).expect("fixed observations per cycle fit u64"),
        internal_fact_acknowledgements: cycles * 2 + cuts,
        owner_reductions: cycles * 12 + cuts,
        journal_records: cycles * 3 + fills + cuts,
        quote_evaluations: cycles + cancels,
        quote_candidates_evaluated: cycles + cancels,
        quote_intents: cycles,
        place_results: cycles,
        prepared_quote_projections: cycles,
        executed_quote_projections: cycles,
        cancel_decisions: cancels,
        cancel_intents: cancels,
        cancel_results: cancels,
        prepared_cancel_projections: cancels,
        executed_cancel_projections: cancels,
        unique_fills: fills,
        duplicate_fills: fills * 2,
        filled_orders: fills,
        cancelled_orders: cancels,
        paired_reconciliations: fills,
        watermark_advances: cuts,
        owned_lifecycle_rows_compacted: cycles,
        canonical_order_rows_compacted: cycles,
        owned_fill_keys_compacted: fills,
        canonical_fill_rows_compacted: fills,
        refresh_tickets_inserted: fills,
        refresh_tickets_admitted: fills,
        refresh_effects: fills,
        refresh_tickets_completed: fills,
        refresh_ticket_high_water: 1,
        refresh_duplicate_or_superseded: 0,
        queue_saturations: 0,
        state_bearing_drops: 0,
    };
    if counters != expected {
        return Err(PmEvidenceError::invariant(format!(
            "nominal counter projection differs: actual={counters:?}, expected={expected:?}"
        )));
    }
    if cycles == u64::try_from(MEASURED_CYCLES).expect("fixed measured cycles fit u64") {
        for (label, actual, expected) in [
            (
                "external observations",
                counters.external_observations,
                MEASURED_EXTERNAL_OBSERVATIONS,
            ),
            (
                "internal fact acknowledgements",
                counters.internal_fact_acknowledgements,
                MEASURED_INTERNAL_FACT_ACKS,
            ),
            (
                "owner reductions",
                counters.owner_reductions,
                MEASURED_OWNER_REDUCTIONS,
            ),
            (
                "journal records",
                counters.journal_records,
                MEASURED_JOURNAL_RECORDS,
            ),
            ("quote intents", counters.quote_intents, QUOTE_INTENTS),
            ("place results", counters.place_results, PLACE_RESULTS),
            ("cancel intents", counters.cancel_intents, CANCEL_INTENTS),
            ("cancel results", counters.cancel_results, CANCEL_RESULTS),
            ("unique fills", counters.unique_fills, UNIQUE_FILLS),
            (
                "suppressed duplicate fills",
                counters.duplicate_fills,
                SUPPRESSED_DUPLICATE_FILLS,
            ),
            (
                "watermark advances",
                counters.watermark_advances,
                WATERMARK_ADVANCES,
            ),
        ] {
            if actual != expected {
                return Err(PmEvidenceError::invariant(format!(
                    "measured {label} is {actual}, expected {expected}"
                )));
            }
        }
    }
    for (label, actual, expected) in [
        (
            "owned lifecycle rows compacted",
            delta(
                after.mutation.owned_lifecycle_rows_compacted(),
                before.mutation.owned_lifecycle_rows_compacted(),
            ),
            cycles,
        ),
        (
            "canonical order rows compacted",
            delta(
                after.mutation.canonical_order_rows_compacted(),
                before.mutation.canonical_order_rows_compacted(),
            ),
            cycles,
        ),
        (
            "owned fill keys compacted",
            delta(
                after.mutation.owned_fill_keys_compacted(),
                before.mutation.owned_fill_keys_compacted(),
            ),
            fills,
        ),
        (
            "canonical fill rows compacted",
            delta(
                after.mutation.canonical_fill_rows_compacted(),
                before.mutation.canonical_fill_rows_compacted(),
            ),
            fills,
        ),
    ] {
        if actual != expected {
            return Err(PmEvidenceError::invariant(format!(
                "{label} is {actual}, expected {expected}"
            )));
        }
    }
    Ok(())
}
