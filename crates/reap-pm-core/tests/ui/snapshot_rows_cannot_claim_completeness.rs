use reap_pm_core::{PmSnapshotEvidence, SnapshotRevision};

fn main() {
    let row_evidence = PmSnapshotEvidence::new(SnapshotRevision::new(1)).unwrap();
    let _ = row_evidence.completeness();
}
