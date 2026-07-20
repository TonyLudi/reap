use reap_pm_core::{PmInstrumentHandle, PmReferenceMapping};

fn checked_mapping_fields_cannot_be_forged(target: PmInstrumentHandle) {
    let _ = PmReferenceMapping {
        target,
        references: [None; 16],
        reference_count: 0,
    };
}

fn main() {}
