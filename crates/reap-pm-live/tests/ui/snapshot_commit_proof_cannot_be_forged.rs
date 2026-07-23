use reap_pm_core::{
    ConnectionEpoch, IngressSequence, PmInstrumentHandle, SnapshotRevision, VenueEventHash,
};
use reap_pm_state::{PmMetadataFingerprint, PmSnapshotCommitProof};

fn forge(
    instrument: PmInstrumentHandle,
    metadata_fingerprint: PmMetadataFingerprint,
    connection_epoch: ConnectionEpoch,
    metadata_revision: SnapshotRevision,
    snapshot_revision: SnapshotRevision,
    local_ingress_sequence: IngressSequence,
    venue_hash: VenueEventHash,
) -> PmSnapshotCommitProof {
    PmSnapshotCommitProof {
        instrument,
        metadata_fingerprint,
        connection_epoch,
        metadata_revision,
        snapshot_revision,
        local_ingress_sequence,
        venue_hash,
    }
}

fn main() {}
