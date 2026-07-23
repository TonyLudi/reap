use reap_pm_core::{ConnectionEpoch, IngressSequence};

/// Deterministic private-source occurrence within one reconnect epoch.
///
/// Local ingress sequences may restart after reconnect, so comparing a bare
/// ingress sequence is never sufficient.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PmPrivateOccurrence {
    epoch: ConnectionEpoch,
    ingress: IngressSequence,
}

impl PmPrivateOccurrence {
    #[must_use]
    pub const fn new(epoch: ConnectionEpoch, ingress: IngressSequence) -> Self {
        Self { epoch, ingress }
    }

    #[must_use]
    pub const fn epoch(self) -> ConnectionEpoch {
        self.epoch
    }

    #[must_use]
    pub const fn ingress(self) -> IngressSequence {
        self.ingress
    }
}
