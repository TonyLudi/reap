use super::*;
use reap_pm_core::{
    ConnectionEpoch, EventOrdering, EvmAddress, IngressSequence, PmAccountHandle, PmAccountScope,
    PmChainId, PmConnectionId, PmEnvironmentId, PmFunderId, PmInstrumentHandle, PmMarketHandle,
    PmSignerId, PmSourceHandle, PmTokenHandle, ReceivedEventClock,
};

pub(super) const OWNER_MEMORY_BOUND_BYTES: usize = 64 * 1024 * 1024;

pub(super) fn instrument() -> PmInstrumentHandle {
    PmInstrumentHandle::new(
        PmMarketHandle::from_ordinal(3),
        PmTokenHandle::from_ordinal(5),
    )
}

pub(super) fn account_scope(account: u16) -> PmAccountScope {
    PmAccountScope::new(
        PmEnvironmentId::new("phase6-overload").expect("environment"),
        PmChainId::new(137).expect("chain"),
        PmSignerId::new(EvmAddress::from_bytes([1; 20]).expect("signer")),
        PmFunderId::new(EvmAddress::from_bytes([2; 20]).expect("funder")),
        PmAccountHandle::from_ordinal(account),
    )
}

fn ordering(sequence: u64) -> EventOrdering {
    EventOrdering::new(
        ConnectionEpoch::new(1),
        None,
        None,
        None,
        IngressSequence::new(sequence),
    )
    .expect("ordering")
}

fn clock(receive_ns: u64) -> ReceivedEventClock {
    ReceivedEventClock::new(None, receive_ns + 10_000, receive_ns).expect("clock")
}

fn connection() -> PmConnectionId {
    PmConnectionId::new("phase6-overload").expect("connection")
}

pub(super) fn internal_ingress(sequence: u64, receive_ns: u64) -> PmCompleteIngress {
    PmCompleteIngress::internal(
        PmSourceHandle::from_ordinal(9),
        connection(),
        ordering(sequence),
        clock(receive_ns),
    )
}

pub(super) fn account_ingress(sequence: u64, receive_ns: u64) -> PmCompleteIngress {
    PmCompleteIngress::product(
        reap_pm_core::PmProductSource::polymarket_account(
            PmSourceHandle::from_ordinal(7),
            PmAccountHandle::from_ordinal(1),
        ),
        connection(),
        ordering(sequence),
        clock(receive_ns),
    )
}
