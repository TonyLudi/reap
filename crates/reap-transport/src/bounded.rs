use reap_core::RawEnvelope;
use thiserror::Error;
use tokio::sync::mpsc;

/// Creates a finite Tokio channel while preserving the historical convention
/// that a requested zero capacity means the minimum capacity of one.
#[must_use]
pub fn bounded_channel<T>(requested_capacity: usize) -> (mpsc::Sender<T>, mpsc::Receiver<T>) {
    mpsc::channel(requested_capacity.max(1))
}

/// An immutable value stamped at its monotonic ingress boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImmutableDelivery<T> {
    payload: T,
    monotonic_receive_ns: u64,
}

impl<T> ImmutableDelivery<T> {
    pub fn new(payload: T, monotonic_receive_ns: u64) -> Result<Self, DeliveryClockError> {
        if monotonic_receive_ns == 0 {
            return Err(DeliveryClockError::ZeroMonotonicReceive);
        }
        Ok(Self {
            payload,
            monotonic_receive_ns,
        })
    }

    #[must_use]
    pub const fn monotonic_receive_ns(&self) -> u64 {
        self.monotonic_receive_ns
    }

    #[must_use]
    pub const fn payload(&self) -> &T {
        &self.payload
    }

    #[must_use]
    pub fn into_payload(self) -> T {
        self.payload
    }

    /// Transforms the delivered value without detaching it from the checked
    /// monotonic receive evidence established at ingress.
    pub fn try_map<U, E>(
        self,
        transform: impl FnOnce(T) -> Result<U, E>,
    ) -> Result<ImmutableDelivery<U>, E> {
        Ok(ImmutableDelivery {
            payload: transform(self.payload)?,
            monotonic_receive_ns: self.monotonic_receive_ns,
        })
    }

    pub fn queue_age_ns(&self, monotonic_service_ns: u64) -> Result<u64, DeliveryClockError> {
        monotonic_service_ns
            .checked_sub(self.monotonic_receive_ns)
            .ok_or(DeliveryClockError::ServiceBeforeReceive)
    }
}

/// Legacy OKX raw-ingress alias.
///
/// [`ImmutableDelivery`] remains venue-neutral; sibling products carry their
/// own statically typed payload rather than widening this OKX wire boundary.
pub type RawDelivery = ImmutableDelivery<RawEnvelope>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum DeliveryClockError {
    #[error("monotonic receive timestamp must be positive")]
    ZeroMonotonicReceive,
    #[error("monotonic service timestamp precedes receive timestamp")]
    ServiceBeforeReceive,
}
