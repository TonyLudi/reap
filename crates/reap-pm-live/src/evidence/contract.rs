//! Frozen arithmetic from the Phase-6 local measurement contract.

pub(crate) const WARMUP_CYCLES: usize = 1_000;
pub(crate) const MEASURED_CYCLES: usize = 10_000;
pub(crate) const OBSERVATIONS_PER_CYCLE: usize = 10;
pub(crate) const MEASURED_EXTERNAL_OBSERVATIONS: u64 = 100_000;
pub(crate) const MEASURED_INTERNAL_FACT_ACKS: u64 = 20_010;
pub(crate) const MEASURED_OWNER_REDUCTIONS: u64 = 120_010;
pub(crate) const MEASURED_JOURNAL_RECORDS: u64 = 35_010;
pub(crate) const PHYSICAL_JOURNAL_LINES: u64 = 35_012;
pub(crate) const QUOTE_INTENTS: u64 = 10_000;
pub(crate) const PLACE_RESULTS: u64 = 10_000;
pub(crate) const CANCEL_INTENTS: u64 = 5_000;
pub(crate) const CANCEL_RESULTS: u64 = 5_000;
pub(crate) const UNIQUE_FILLS: u64 = 5_000;
pub(crate) const SUPPRESSED_DUPLICATE_FILLS: u64 = 10_000;
pub(crate) const WATERMARK_ADVANCES: u64 = 10;
pub(crate) const ACTION_SAMPLES: usize = 15_000;
pub(crate) const PARSER_WARMUP_PER_VENUE: usize = 1_000;
pub(crate) const PARSER_SAMPLES_PER_VENUE: usize = 10_000;
pub(crate) const REPEATED_NOMINAL_PASSES: usize = 5;
pub(crate) const MAX_RESERVED_CAPACITY_BYTES: usize = 64 * 1_024 * 1_024;
pub(crate) const MAX_REPLAY_WORKING_BYTES: usize = 16 * 1_024 * 1_024;
pub(crate) const FIXTURE_REVISION: &str = "goal-f-phase6-option1-v1";

pub(crate) const fn is_cancel_cycle(cycle: usize) -> bool {
    cycle % 2 == 1
}

#[cfg(test)]
pub(crate) const fn fill_ordinal(cycle: usize) -> Option<usize> {
    if is_cancel_cycle(cycle) {
        None
    } else {
        Some(cycle / 2)
    }
}

#[cfg(test)]
pub(crate) const fn advances_watermark(cycle: usize) -> bool {
    cycle % 1_000 == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_based_cycle_oracle_pins_cancel_fill_and_cut_positions() {
        assert!(is_cancel_cycle(1));
        assert!(!is_cancel_cycle(2));
        assert!(is_cancel_cycle(999));
        assert!(!is_cancel_cycle(1_000));
        assert!(advances_watermark(1_000));
        assert!(advances_watermark(10_000));
        assert!(!advances_watermark(999));
        assert_eq!(fill_ordinal(2), Some(1));
        assert_eq!(fill_ordinal(1_000), Some(500));

        let cancel_cycles = (1..=MEASURED_CYCLES)
            .filter(|cycle| is_cancel_cycle(*cycle))
            .count();
        let fill_cycles = (1..=MEASURED_CYCLES)
            .filter(|cycle| !is_cancel_cycle(*cycle))
            .count();
        let cuts = (1..=MEASURED_CYCLES)
            .filter(|cycle| advances_watermark(*cycle))
            .count();
        assert_eq!((cancel_cycles, fill_cycles, cuts), (5_000, 5_000, 10));
    }
}
