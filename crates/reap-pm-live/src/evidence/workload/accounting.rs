use crate::evidence::PmEvidenceError;
use crate::evidence::report::{BootstrapInputMix, InputMixReport, SetupCounters};

pub(super) fn validate_input_mix(
    cycles: usize,
    actual: InputMixReport,
) -> Result<(), PmEvidenceError> {
    let cycles = u64::try_from(cycles)
        .map_err(|_| PmEvidenceError::invariant("fixed cycle count exceeds u64"))?;
    let fills = cycles / 2;
    let cancels = cycles / 2;
    let expected = InputMixReport {
        pm_book_observations: cycles,
        okx_reference_observations: cycles,
        quote_evaluation_timers: cycles,
        quote_intent_acknowledgements: cycles,
        fake_place_acceptances: cycles,
        private_unique_fills: fills,
        private_duplicate_fills: fills,
        fill_order_detail_absences: fills,
        paired_reconciliations: fills,
        fill_freshness_timers: fills,
        replace_timers: cancels,
        cancel_intent_acknowledgements: cancels,
        fake_cancel_acceptances: cancels,
        cancel_order_detail_absences: cancels,
        cancel_freshness_timers: cancels,
    };
    let expected_total = cycles.saturating_mul(10);
    if actual != expected || actual.total() != expected_total {
        return Err(PmEvidenceError::invariant(format!(
            "typed input mix differs: actual={actual:?} (total {}), expected={expected:?} (total {expected_total})",
            actual.total()
        )));
    }
    Ok(())
}

pub(super) fn validate_setup(
    actual: SetupCounters,
    expected_physical_journal_lines: Option<u64>,
) -> Result<(), PmEvidenceError> {
    let expected = SetupCounters {
        bootstrap: BootstrapInputMix {
            private_connection_completion: 1,
            open_orders_snapshot: 1,
            initial_market_metadata: 1,
            initial_pm_book_snapshot: 1,
        },
        journal_header_records: 1,
        w0_paired_reconciliations: 1,
        w0_external_observations: 1,
        w0_internal_fact_acknowledgements: 1,
        w0_owner_reductions: 2,
        w0_journal_records: 1,
        w0_watermark_advances: 1,
        physical_journal_lines: expected_physical_journal_lines,
    };
    if actual != expected {
        return Err(PmEvidenceError::invariant(format!(
            "setup projection differs: actual={actual:?}, expected={expected:?}"
        )));
    }
    Ok(())
}
