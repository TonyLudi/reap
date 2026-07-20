use reap_pm_core::{ConnectionEpoch, IngressSequence, PmSourceHandle};
use reap_pm_live::PmServiceKey;

fn main() {
    let _ = PmServiceKey {
        monotonic_receive_ns: 1,
        source: PmSourceHandle::from_ordinal(0),
        connection_epoch: ConnectionEpoch::new(1),
        local_ingress_sequence: IngressSequence::new(1),
        variant_rank: 255,
    };
}
