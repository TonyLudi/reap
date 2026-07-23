use reap_pm_state::PmSnapshotCommitProof;

fn replay(proof: PmSnapshotCommitProof) {
    let _first = proof.clone();
    let _second = proof;
}

fn main() {}
