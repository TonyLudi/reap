use std::{
    fmt,
    sync::Arc,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

#[cfg(any(test, feature = "loopback-evidence"))]
use std::collections::VecDeque;
#[cfg(any(test, feature = "test-support"))]
use std::sync::atomic::{AtomicUsize, Ordering};

use reap_pm_core::ReceivedEventClock;
use reap_polymarket_auth::{
    AuthenticatedOwnedCancelRequest, AuthenticatedPlaceRequest, L2Credentials, L2Timestamp,
    PmAuthError, SerializedOwnedCancelRequest, SerializedPlaceRequest,
};
use thiserror::Error;

use crate::{
    PmPublicWsClockError, PmPublicWsClockSource, PmPublicWsEdgeClock, PmUserWsClockError,
    PmUserWsClockSource, PmUserWsEdgeClock,
};

/// Maximum time between receiving a bounded `/time` response and admitting a
/// mutation dispatch from the durable Goal-F queue.
pub const PM_MUTATION_SERVER_TIME_MAX_AGE: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum PmProductClockError {
    #[error("shared Polymarket product clock is unavailable")]
    Unavailable,
    #[error("shared Polymarket product clock returned an invalid reading")]
    InvalidReading,
    #[error("server-time proof belongs to another product clock domain")]
    WrongDomain,
    #[error("server-time proof belongs to another mutation purpose")]
    WrongMutationPurpose,
    #[error("server-time validation clock precedes its HTTP receive edge")]
    ClockRegression,
    #[error("server-time proof exceeded its fixed pre-dispatch age bound")]
    ServerTimeStale,
    #[error("scripted product clock has no remaining observations")]
    ScriptExhausted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ProductClockReading {
    local_wall_ns: u64,
    monotonic_ns: u64,
}

trait ProductClockSource: Send + Sync + 'static {
    fn sample(&self) -> Result<ProductClockReading, PmProductClockError>;
}

struct SystemProductClock {
    origin: Instant,
}

impl ProductClockSource for SystemProductClock {
    fn sample(&self) -> Result<ProductClockReading, PmProductClockError> {
        let local_wall_ns = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| PmProductClockError::Unavailable)?
            .as_nanos()
            .try_into()
            .map_err(|_| PmProductClockError::Unavailable)?;
        let monotonic_ns = self
            .origin
            .elapsed()
            .as_nanos()
            .saturating_add(1)
            .try_into()
            .map_err(|_| PmProductClockError::Unavailable)?;
        validate_reading(ProductClockReading {
            local_wall_ns,
            monotonic_ns,
        })
    }
}

#[cfg(any(test, feature = "test-support"))]
struct ScriptedProductClock {
    readings: Box<[ProductClockReading]>,
    next: AtomicUsize,
}

#[cfg(any(test, feature = "test-support"))]
impl ProductClockSource for ScriptedProductClock {
    fn sample(&self) -> Result<ProductClockReading, PmProductClockError> {
        let index = self.next.fetch_add(1, Ordering::Relaxed);
        let reading = self
            .readings
            .get(index)
            .copied()
            .ok_or(PmProductClockError::ScriptExhausted)?;
        validate_reading(reading)
    }
}

fn validate_reading(
    reading: ProductClockReading,
) -> Result<ProductClockReading, PmProductClockError> {
    ReceivedEventClock::new(None, reading.local_wall_ns, reading.monotonic_ns)
        .map_err(|_| PmProductClockError::InvalidReading)?;
    Ok(reading)
}

struct ProductClockDomain {
    source: Box<dyn ProductClockSource>,
}

impl ProductClockDomain {
    fn sample(&self) -> Result<ProductClockReading, PmProductClockError> {
        self.source.sample()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MutationTimePurpose {
    Place,
    Cancel,
}

struct MutationTimeAuthority {
    domain: Arc<ProductClockDomain>,
    purpose: MutationTimePurpose,
}

/// Sole owner of one runtime clock origin and its unforgeable domain marker.
///
/// Splitting consumes the owner. Every PM public/user/REST/control view and
/// the OKX ingress view then shares this exact immutable domain. Sampling is
/// lock-free; the production source contains only an immutable `Instant`.
pub struct PmProductClockOwner {
    domain: Arc<ProductClockDomain>,
}

impl PmProductClockOwner {
    #[must_use]
    pub fn system() -> Self {
        Self {
            domain: Arc::new(ProductClockDomain {
                source: Box::new(SystemProductClock {
                    origin: Instant::now(),
                }),
            }),
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn test_support_scripted(readings: &[(u64, u64)]) -> Result<Self, PmProductClockError> {
        if readings.is_empty() {
            return Err(PmProductClockError::ScriptExhausted);
        }
        let readings = readings
            .iter()
            .map(|&(local_wall_ns, monotonic_ns)| {
                validate_reading(ProductClockReading {
                    local_wall_ns,
                    monotonic_ns,
                })
            })
            .collect::<Result<Box<[_]>, _>>()?;
        Ok(Self {
            domain: Arc::new(ProductClockDomain {
                source: Box::new(ScriptedProductClock {
                    readings,
                    next: AtomicUsize::new(0),
                }),
            }),
        })
    }

    #[must_use]
    pub(crate) fn split(self) -> PmProductClockViews {
        let place_time_authority = Arc::new(MutationTimeAuthority {
            domain: Arc::clone(&self.domain),
            purpose: MutationTimePurpose::Place,
        });
        let cancel_time_authority = Arc::new(MutationTimeAuthority {
            domain: Arc::clone(&self.domain),
            purpose: MutationTimePurpose::Cancel,
        });
        PmProductClockViews {
            public_ws: PmPublicWsProductClock {
                domain: Arc::clone(&self.domain),
            },
            user_ws: PmUserWsProductClock {
                domain: Arc::clone(&self.domain),
            },
            public_http: PmPublicHttpProductClock {
                domain: Arc::clone(&self.domain),
            },
            read_server_time_http: PmReadServerTimeProductClock {
                domain: Arc::clone(&self.domain),
            },
            private_read: PmPrivateReadProductClock {
                domain: Arc::clone(&self.domain),
            },
            place_server_time_http: PmPlaceServerTimeProductClock {
                authority: Arc::clone(&place_time_authority),
            },
            place_mutation_time_finalizer: PmPlaceMutationTimeFinalizer {
                authority: place_time_authority,
            },
            cancel_server_time_http: PmCancelServerTimeProductClock {
                authority: Arc::clone(&cancel_time_authority),
            },
            cancel_mutation_time_finalizer: PmCancelMutationTimeFinalizer {
                authority: cancel_time_authority,
            },
            actor: PmActorProductClock {
                domain: Arc::clone(&self.domain),
            },
            okx: PmOkxProductClock {
                domain: Arc::clone(&self.domain),
            },
            #[cfg(test)]
            loopback_place_server_time_http: PmMutationServerTimeProductClock {
                domain: Arc::clone(&self.domain),
            },
            #[cfg(test)]
            loopback_cancel_server_time_http: PmMutationServerTimeProductClock {
                domain: Arc::clone(&self.domain),
            },
            #[cfg(test)]
            loopback_mutation_time_validator: PmMutationServerTimeValidator {
                domain: self.domain,
            },
        }
    }
}

impl Default for PmProductClockOwner {
    fn default() -> Self {
        Self::system()
    }
}

impl fmt::Debug for PmProductClockOwner {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PmProductClockOwner(<opaque-domain>)")
    }
}

/// Move-only split result; each role receives only its purpose-specific view.
pub(crate) struct PmProductClockViews {
    public_ws: PmPublicWsProductClock,
    user_ws: PmUserWsProductClock,
    public_http: PmPublicHttpProductClock,
    read_server_time_http: PmReadServerTimeProductClock,
    private_read: PmPrivateReadProductClock,
    place_server_time_http: PmPlaceServerTimeProductClock,
    place_mutation_time_finalizer: PmPlaceMutationTimeFinalizer,
    cancel_server_time_http: PmCancelServerTimeProductClock,
    cancel_mutation_time_finalizer: PmCancelMutationTimeFinalizer,
    actor: PmActorProductClock,
    okx: PmOkxProductClock,
    #[cfg(test)]
    loopback_place_server_time_http: PmMutationServerTimeProductClock,
    #[cfg(test)]
    loopback_cancel_server_time_http: PmMutationServerTimeProductClock,
    #[cfg(test)]
    loopback_mutation_time_validator: PmMutationServerTimeValidator,
}

impl PmProductClockViews {
    #[must_use]
    pub(crate) fn into_views(
        self,
    ) -> (
        PmPublicWsProductClock,
        PmUserWsProductClock,
        PmPublicHttpProductClock,
        PmReadServerTimeProductClock,
        PmPrivateReadProductClock,
        PmPlaceServerTimeProductClock,
        PmPlaceMutationTimeFinalizer,
        PmCancelServerTimeProductClock,
        PmCancelMutationTimeFinalizer,
        PmActorProductClock,
        PmOkxProductClock,
    ) {
        (
            self.public_ws,
            self.user_ws,
            self.public_http,
            self.read_server_time_http,
            self.private_read,
            self.place_server_time_http,
            self.place_mutation_time_finalizer,
            self.cancel_server_time_http,
            self.cancel_mutation_time_finalizer,
            self.actor,
            self.okx,
        )
    }

    /// Purpose-erased compatibility views for literal-loopback evidence only.
    /// No production connectivity bundle exposes these types.
    #[cfg(test)]
    #[must_use]
    pub(crate) fn into_loopback_mutation_views(
        self,
    ) -> (
        PmPublicWsProductClock,
        PmUserWsProductClock,
        PmPublicHttpProductClock,
        PmReadServerTimeProductClock,
        PmPrivateReadProductClock,
        PmMutationServerTimeProductClock,
        PmMutationServerTimeProductClock,
        PmActorProductClock,
        PmOkxProductClock,
        PmMutationServerTimeValidator,
    ) {
        (
            self.public_ws,
            self.user_ws,
            self.public_http,
            self.read_server_time_http,
            self.private_read,
            self.loopback_place_server_time_http,
            self.loopback_cancel_server_time_http,
            self.actor,
            self.okx,
            self.loopback_mutation_time_validator,
        )
    }
}

// BEGIN OBSERVATION_ONLY_CLOCK_SPLIT
impl PmProductClockOwner {
    /// Split one clock domain into only the receive and control views needed
    /// by read-only product observation.
    #[must_use]
    pub(crate) fn split_observation_only(self) -> PmObservationClockViews {
        PmObservationClockViews {
            public_ws: PmPublicWsProductClock {
                domain: Arc::clone(&self.domain),
            },
            user_ws: PmUserWsProductClock {
                domain: Arc::clone(&self.domain),
            },
            public_http: PmPublicHttpProductClock {
                domain: Arc::clone(&self.domain),
            },
            read_server_time_http: PmReadServerTimeProductClock {
                domain: Arc::clone(&self.domain),
            },
            private_read: PmPrivateReadProductClock {
                domain: Arc::clone(&self.domain),
            },
            actor: PmActorProductClock {
                domain: Arc::clone(&self.domain),
            },
            okx: PmOkxProductClock {
                domain: self.domain,
            },
        }
    }
}

/// Move-only read-observation views from one product clock domain.
pub(crate) struct PmObservationClockViews {
    public_ws: PmPublicWsProductClock,
    user_ws: PmUserWsProductClock,
    public_http: PmPublicHttpProductClock,
    read_server_time_http: PmReadServerTimeProductClock,
    private_read: PmPrivateReadProductClock,
    actor: PmActorProductClock,
    okx: PmOkxProductClock,
}

impl PmObservationClockViews {
    #[must_use]
    pub(crate) fn into_views(
        self,
    ) -> (
        PmPublicWsProductClock,
        PmUserWsProductClock,
        PmPublicHttpProductClock,
        PmReadServerTimeProductClock,
        PmPrivateReadProductClock,
        PmActorProductClock,
        PmOkxProductClock,
    ) {
        (
            self.public_ws,
            self.user_ws,
            self.public_http,
            self.read_server_time_http,
            self.private_read,
            self.actor,
            self.okx,
        )
    }
}
// END OBSERVATION_ONLY_CLOCK_SPLIT

pub struct PmPublicWsProductClock {
    domain: Arc<ProductClockDomain>,
}

impl PmPublicWsClockSource for PmPublicWsProductClock {
    fn observe_public_ws_edge(&mut self) -> Result<PmPublicWsEdgeClock, PmPublicWsClockError> {
        let reading = self
            .domain
            .sample()
            .map_err(|_| PmPublicWsClockError::SystemClockUnavailable)?;
        PmPublicWsEdgeClock::new(reading.local_wall_ns, reading.monotonic_ns)
    }
}

pub struct PmUserWsProductClock {
    domain: Arc<ProductClockDomain>,
}

impl PmUserWsClockSource for PmUserWsProductClock {
    fn observe_user_ws_edge(&mut self) -> Result<PmUserWsEdgeClock, PmUserWsClockError> {
        let reading = self
            .domain
            .sample()
            .map_err(|_| PmUserWsClockError::SystemClockUnavailable)?;
        PmUserWsEdgeClock::new(reading.local_wall_ns, reading.monotonic_ns)
    }
}

pub struct PmPublicHttpProductClock {
    domain: Arc<ProductClockDomain>,
}

impl PmPublicHttpProductClock {
    pub(crate) fn standalone_system() -> Self {
        let owner = PmProductClockOwner::system();
        let (_, _, public_http, _, _, _, _, _, _, _, _) = owner.split().into_views();
        public_http
    }

    pub(crate) fn observe_rest_edge(&self) -> Result<PmRestResponseClock, PmProductClockError> {
        observe_rest_edge(&self.domain)
    }

    #[cfg(any(test, feature = "loopback-evidence"))]
    pub(crate) fn pending_mutation_time(
        &self,
        timestamp: L2Timestamp,
        received: PmRestResponseClock,
    ) -> PmPendingMutationServerTime {
        PmPendingMutationServerTime {
            timestamp,
            received,
            domain: Arc::clone(&self.domain),
        }
    }

    pub(crate) fn read_time(
        &self,
        timestamp: L2Timestamp,
        received: PmRestResponseClock,
    ) -> PmReadServerTime {
        PmReadServerTime {
            timestamp,
            received,
            domain: Arc::clone(&self.domain),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PmPrivateReadEdgeClock {
    received: ReceivedEventClock,
}

impl PmPrivateReadEdgeClock {
    #[must_use]
    pub const fn local_wall_receive_ns(self) -> u64 {
        self.received.local_wall_receive_ns()
    }

    #[must_use]
    pub const fn monotonic_receive_ns(self) -> u64 {
        self.received.monotonic_receive_ns()
    }
}

/// Clock capability owned by the authenticated read worker.
///
/// The worker samples this only after the final authenticated response has
/// been received and parsed. The returned evidence cannot mint server time or
/// be converted into a general product-clock capability.
pub struct PmPrivateReadProductClock {
    domain: Arc<ProductClockDomain>,
}

impl PmPrivateReadProductClock {
    pub fn observe_authenticated_read_complete(
        &mut self,
    ) -> Result<PmPrivateReadEdgeClock, PmProductClockError> {
        let reading = self.domain.sample()?;
        Ok(PmPrivateReadEdgeClock {
            received: ReceivedEventClock::new(None, reading.local_wall_ns, reading.monotonic_ns)
                .map_err(|_| PmProductClockError::InvalidReading)?,
        })
    }
}

/// Clock capability for one authenticated-read `/time` client.
///
/// This view can issue only read-time proofs. It deliberately cannot issue a
/// mutation proof or observe any WebSocket/control edge.
pub struct PmReadServerTimeProductClock {
    domain: Arc<ProductClockDomain>,
}

impl PmReadServerTimeProductClock {
    pub(crate) fn standalone_system() -> Self {
        Self {
            domain: Arc::new(ProductClockDomain {
                source: Box::new(SystemProductClock {
                    origin: Instant::now(),
                }),
            }),
        }
    }

    pub(crate) fn observe_rest_edge(&self) -> Result<PmRestResponseClock, PmProductClockError> {
        observe_rest_edge(&self.domain)
    }

    pub(crate) fn read_time(
        &self,
        timestamp: L2Timestamp,
        received: PmRestResponseClock,
    ) -> PmReadServerTime {
        PmReadServerTime {
            timestamp,
            received,
            domain: Arc::clone(&self.domain),
        }
    }
}

/// Clock capability for the fixed place-only `/time` source.
pub(crate) struct PmPlaceServerTimeProductClock {
    authority: Arc<MutationTimeAuthority>,
}

impl PmPlaceServerTimeProductClock {
    pub(crate) fn observe_rest_edge(&self) -> Result<PmRestResponseClock, PmProductClockError> {
        observe_rest_edge(&self.authority.domain)
    }

    pub(crate) fn place_time_proof(
        &self,
        timestamp: L2Timestamp,
        received: PmRestResponseClock,
    ) -> PmPlaceMutationTimeProof {
        PmPlaceMutationTimeProof {
            core: PmMutationTimeProofCore {
                timestamp,
                received,
                authority: Arc::clone(&self.authority),
                purpose: MutationTimePurpose::Place,
            },
        }
    }
}

/// Clock capability for the fixed cancel-only `/time` source.
pub(crate) struct PmCancelServerTimeProductClock {
    authority: Arc<MutationTimeAuthority>,
}

impl PmCancelServerTimeProductClock {
    pub(crate) fn observe_rest_edge(&self) -> Result<PmRestResponseClock, PmProductClockError> {
        observe_rest_edge(&self.authority.domain)
    }

    pub(crate) fn cancel_time_proof(
        &self,
        timestamp: L2Timestamp,
        received: PmRestResponseClock,
    ) -> PmCancelMutationTimeProof {
        PmCancelMutationTimeProof {
            core: PmMutationTimeProofCore {
                timestamp,
                received,
                authority: Arc::clone(&self.authority),
                purpose: MutationTimePurpose::Cancel,
            },
        }
    }
}

/// Purpose-erased mutation clock retained only for literal-loopback
/// compatibility. Production connectivity never returns this type.
#[cfg(any(test, feature = "loopback-evidence"))]
pub struct PmMutationServerTimeProductClock {
    domain: Arc<ProductClockDomain>,
}

#[cfg(any(test, feature = "loopback-evidence"))]
impl PmMutationServerTimeProductClock {
    pub(crate) fn observe_rest_edge(&self) -> Result<PmRestResponseClock, PmProductClockError> {
        observe_rest_edge(&self.domain)
    }

    pub(crate) fn pending_mutation_time(
        &self,
        timestamp: L2Timestamp,
        received: PmRestResponseClock,
    ) -> PmPendingMutationServerTime {
        PmPendingMutationServerTime {
            timestamp,
            received,
            domain: Arc::clone(&self.domain),
        }
    }
}

fn observe_rest_edge(
    domain: &ProductClockDomain,
) -> Result<PmRestResponseClock, PmProductClockError> {
    let reading = domain.sample()?;
    Ok(PmRestResponseClock {
        received: ReceivedEventClock::new(None, reading.local_wall_ns, reading.monotonic_ns)
            .map_err(|_| PmProductClockError::InvalidReading)?,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PmRestResponseClock {
    received: ReceivedEventClock,
}

impl PmRestResponseClock {
    #[must_use]
    pub const fn local_wall_receive_ns(self) -> u64 {
        self.received.local_wall_receive_ns()
    }

    #[must_use]
    pub const fn monotonic_receive_ns(self) -> u64 {
        self.received.monotonic_receive_ns()
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn test_support_new(
        local_wall_receive_ns: u64,
        monotonic_receive_ns: u64,
    ) -> Result<Self, PmProductClockError> {
        Ok(Self {
            received: ReceivedEventClock::new(None, local_wall_receive_ns, monotonic_receive_ns)
                .map_err(|_| PmProductClockError::InvalidReading)?,
        })
    }
}

pub struct PmReadServerTime {
    timestamp: L2Timestamp,
    received: PmRestResponseClock,
    domain: Arc<ProductClockDomain>,
}

impl PmReadServerTime {
    pub(crate) fn into_l2_timestamp(self) -> Result<L2Timestamp, PmProductClockError> {
        let Self {
            timestamp,
            received,
            domain,
        } = self;
        validate_age(&domain, received)?;
        Ok(timestamp)
    }
}

impl fmt::Debug for PmReadServerTime {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PmReadServerTime(<opaque>)")
    }
}

struct PmMutationTimeProofCore {
    timestamp: L2Timestamp,
    received: PmRestResponseClock,
    authority: Arc<MutationTimeAuthority>,
    purpose: MutationTimePurpose,
}

/// Move-only place-time proof from the fixed place `/time` source.
///
/// The parsed timestamp is deliberately opaque. The proof retains its HTTP
/// receive edge, source domain, exact place authority and purpose until a
/// same-owner finalizer consumes it inside the credential task.
pub struct PmPlaceMutationTimeProof {
    core: PmMutationTimeProofCore,
}

impl fmt::Debug for PmPlaceMutationTimeProof {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PmPlaceMutationTimeProof(<opaque-place-time>)")
    }
}

impl PmPlaceMutationTimeProof {
    pub(crate) const fn observed_l2_timestamp_seconds(&self) -> u64 {
        self.core.timestamp.unix_seconds()
    }
}

/// Move-only cancel-time proof from the fixed cancel `/time` source.
pub struct PmCancelMutationTimeProof {
    core: PmMutationTimeProofCore,
}

impl fmt::Debug for PmCancelMutationTimeProof {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PmCancelMutationTimeProof(<opaque-cancel-time>)")
    }
}

impl PmCancelMutationTimeProof {
    pub(crate) const fn observed_l2_timestamp_seconds(&self) -> u64 {
        self.core.timestamp.unix_seconds()
    }
}

/// Borrowed, non-retainable final place-time view delivered only after the
/// credential-task finalizer has rechecked owner, domain, purpose and age.
pub struct PmFinalPlaceMutationTime<'proof> {
    core: &'proof PmMutationTimeProofCore,
}

impl PmFinalPlaceMutationTime<'_> {
    /// Consume the hidden timestamp only inside the adapter-owned final-HMAC
    /// bridge. It is intentionally crate-private: an external provider cannot
    /// extract a raw timestamp.
    pub(crate) fn consume_l2_timestamp(self) -> Result<L2Timestamp, PmProductClockError> {
        validate_age(&self.core.authority.domain, self.core.received)?;
        Ok(self.core.timestamp)
    }
}

impl fmt::Debug for PmFinalPlaceMutationTime<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PmFinalPlaceMutationTime(<borrowed-opaque-place-time>)")
    }
}

/// Borrowed, non-retainable final cancel-time view delivered only after the
/// credential-task finalizer has rechecked owner, domain, purpose and age.
pub struct PmFinalCancelMutationTime<'proof> {
    core: &'proof PmMutationTimeProofCore,
}

impl PmFinalCancelMutationTime<'_> {
    /// Consume the hidden timestamp only inside the adapter-owned final-HMAC
    /// bridge. It is intentionally crate-private: an external provider cannot
    /// extract a raw timestamp.
    pub(crate) fn consume_l2_timestamp(self) -> Result<L2Timestamp, PmProductClockError> {
        validate_age(&self.core.authority.domain, self.core.received)?;
        Ok(self.core.timestamp)
    }
}

impl fmt::Debug for PmFinalCancelMutationTime<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PmFinalCancelMutationTime(<borrowed-opaque-cancel-time>)")
    }
}

/// Fixed failure vocabulary for the runner-private credential provider.
/// Detailed authentication failures remain inside that task's own channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum PmMutationTimeProviderError {
    #[error("final mutation-time clock validation failed inside credential provider: {0}")]
    FinalClock(PmProductClockError),
    #[error("credential provider rejected final mutation time")]
    Rejected,
}

/// Narrow place-only callback invoked synchronously at the final freshness
/// boundary. The borrowed token has no public timestamp accessor and cannot
/// be retained beyond this call.
pub trait PmPlaceMutationTimeProvider: Send {
    fn consume_final_place_time(
        &mut self,
        time: PmFinalPlaceMutationTime<'_>,
    ) -> Result<(), PmMutationTimeProviderError>;
}

/// Narrow cancel-only callback invoked synchronously at the final freshness
/// boundary.
pub trait PmCancelMutationTimeProvider: Send {
    fn consume_final_cancel_time(
        &mut self,
        time: PmFinalCancelMutationTime<'_>,
    ) -> Result<(), PmMutationTimeProviderError>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum PmMutationTimeConsumeError {
    #[error("final mutation-time clock validation failed: {0}")]
    Clock(#[from] PmProductClockError),
    #[error("final mutation-time provider rejected the proof: {0}")]
    Provider(#[from] PmMutationTimeProviderError),
}

/// Closed, redacted failures for the fixed place-time authentication bridge.
///
/// This vocabulary cannot carry request bytes, credentials, signatures or a
/// raw [`L2Timestamp`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum PmPlaceMutationAuthenticationError {
    #[error("final place-time clock validation failed: {0}")]
    Clock(#[from] PmProductClockError),
    #[error("place-time evidence does not match the retained source proof")]
    ObservedTimestampMismatch,
    #[error("fixed place request authentication failed: {0}")]
    Authentication(#[from] PmAuthError),
}

/// Closed, redacted failures for the exact-owned cancel-time authentication
/// bridge. No variant can carry request bytes, credentials, signatures or a
/// raw [`L2Timestamp`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum PmCancelMutationAuthenticationError {
    #[error("final cancel-time clock validation failed: {0}")]
    Clock(#[from] PmProductClockError),
    #[error("cancel-time evidence does not match the retained source proof")]
    ObservedTimestampMismatch,
    #[error("exact-owned cancel request authentication failed: {0}")]
    Authentication(#[from] PmAuthError),
}

/// Sole place-purpose finalizer paired with one place `/time` source by the
/// public connectivity root.
pub struct PmPlaceMutationTimeFinalizer {
    authority: Arc<MutationTimeAuthority>,
}

impl PmPlaceMutationTimeFinalizer {
    /// Consume one source-issued place proof and authenticate one already
    /// serialized fixed place request at the final freshness boundary.
    ///
    /// `expected_seconds` is evidence correlation only. It must exactly match
    /// the timestamp hidden in `proof`, but it never creates or authorizes an
    /// L2 timestamp. After the first owner/domain/purpose/age check, the hidden
    /// timestamp is rechecked at the immediate HMAC boundary and passed
    /// directly into [`L2Credentials::authenticate_place`].
    pub fn authenticate_exact_place(
        &mut self,
        proof: PmPlaceMutationTimeProof,
        expected_seconds: u64,
        credentials: &L2Credentials,
        request: SerializedPlaceRequest,
    ) -> Result<AuthenticatedPlaceRequest, PmPlaceMutationAuthenticationError> {
        validate_final_mutation_time(&self.authority, &proof.core, MutationTimePurpose::Place)?;
        if proof.core.timestamp.unix_seconds() != expected_seconds {
            return Err(PmPlaceMutationAuthenticationError::ObservedTimestampMismatch);
        }
        let timestamp = PmFinalPlaceMutationTime { core: &proof.core }.consume_l2_timestamp()?;
        Ok(credentials.authenticate_place(timestamp, request)?)
    }

    /// Consume one place proof inside the sole credential task. Validation is
    /// immediately followed by one synchronous, non-retainable provider call.
    /// This compatibility seam is not the production place-authentication
    /// path; production callers use [`Self::authenticate_exact_place`].
    pub fn consume_with(
        &mut self,
        proof: PmPlaceMutationTimeProof,
        provider: &mut dyn PmPlaceMutationTimeProvider,
    ) -> Result<(), PmMutationTimeConsumeError> {
        validate_final_mutation_time(&self.authority, &proof.core, MutationTimePurpose::Place)?;
        provider.consume_final_place_time(PmFinalPlaceMutationTime { core: &proof.core })?;
        Ok(())
    }

    /// Adapt a purpose-bound place proof to the legacy authenticated-loopback
    /// worker token. This compatibility edge is compiled only with literal
    /// loopback mutation support; production compositions cannot name it.
    #[cfg(any(test, feature = "loopback-evidence"))]
    pub fn authorize_loopback_place(
        &mut self,
        proof: PmPlaceMutationTimeProof,
    ) -> Result<PmAuthorizedMutationServerTime, PmProductClockError> {
        validate_final_mutation_time(&self.authority, &proof.core, MutationTimePurpose::Place)?;
        let timestamp = PmFinalPlaceMutationTime { core: &proof.core }.consume_l2_timestamp()?;
        Ok(PmAuthorizedMutationServerTime { timestamp })
    }
}

impl fmt::Debug for PmPlaceMutationTimeFinalizer {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PmPlaceMutationTimeFinalizer(<opaque-place-authority>)")
    }
}

/// Sole cancel-purpose finalizer paired with one cancel `/time` source by the
/// public connectivity root.
pub struct PmCancelMutationTimeFinalizer {
    authority: Arc<MutationTimeAuthority>,
}

impl PmCancelMutationTimeFinalizer {
    /// Consume one source-issued cancel proof and authenticate one already
    /// serialized exact-owned cancel at the final freshness boundary.
    ///
    /// `expected_seconds` is evidence correlation only. It must match the
    /// timestamp hidden in `proof`, but cannot create authentication authority.
    /// The hidden timestamp is rechecked immediately before the fixed HMAC.
    pub fn authenticate_exact_owned_cancel(
        &mut self,
        proof: PmCancelMutationTimeProof,
        expected_seconds: u64,
        credentials: &L2Credentials,
        request: SerializedOwnedCancelRequest,
    ) -> Result<AuthenticatedOwnedCancelRequest, PmCancelMutationAuthenticationError> {
        validate_final_mutation_time(&self.authority, &proof.core, MutationTimePurpose::Cancel)?;
        if proof.core.timestamp.unix_seconds() != expected_seconds {
            return Err(PmCancelMutationAuthenticationError::ObservedTimestampMismatch);
        }
        let timestamp = PmFinalCancelMutationTime { core: &proof.core }.consume_l2_timestamp()?;
        Ok(credentials.authenticate_owned_cancel(timestamp, request)?)
    }

    /// Compatibility-only provider seam. Production exact-owned cancel
    /// authentication uses [`Self::authenticate_exact_owned_cancel`].
    pub fn consume_with(
        &mut self,
        proof: PmCancelMutationTimeProof,
        provider: &mut dyn PmCancelMutationTimeProvider,
    ) -> Result<(), PmMutationTimeConsumeError> {
        validate_final_mutation_time(&self.authority, &proof.core, MutationTimePurpose::Cancel)?;
        provider.consume_final_cancel_time(PmFinalCancelMutationTime { core: &proof.core })?;
        Ok(())
    }

    /// Adapt a purpose-bound cancel proof to the legacy authenticated-loopback
    /// worker token. It is unavailable unless literal loopback mutation
    /// support is explicitly compiled.
    #[cfg(any(test, feature = "loopback-evidence"))]
    pub fn authorize_loopback_cancel(
        &mut self,
        proof: PmCancelMutationTimeProof,
    ) -> Result<PmAuthorizedMutationServerTime, PmProductClockError> {
        validate_final_mutation_time(&self.authority, &proof.core, MutationTimePurpose::Cancel)?;
        let timestamp = PmFinalCancelMutationTime { core: &proof.core }.consume_l2_timestamp()?;
        Ok(PmAuthorizedMutationServerTime { timestamp })
    }
}

impl fmt::Debug for PmCancelMutationTimeFinalizer {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PmCancelMutationTimeFinalizer(<opaque-cancel-authority>)")
    }
}

fn validate_final_mutation_time(
    expected: &Arc<MutationTimeAuthority>,
    proof: &PmMutationTimeProofCore,
    expected_purpose: MutationTimePurpose,
) -> Result<(), PmProductClockError> {
    if expected.purpose != expected_purpose || proof.purpose != expected_purpose {
        return Err(PmProductClockError::WrongMutationPurpose);
    }
    if !Arc::ptr_eq(expected, &proof.authority)
        || !Arc::ptr_eq(&expected.domain, &proof.authority.domain)
    {
        return Err(PmProductClockError::WrongDomain);
    }
    validate_age(&expected.domain, proof.received)
}

/// Purpose-erased proof retained only for literal-loopback compatibility.
#[cfg(any(test, feature = "loopback-evidence"))]
pub struct PmPendingMutationServerTime {
    timestamp: L2Timestamp,
    received: PmRestResponseClock,
    domain: Arc<ProductClockDomain>,
}

#[cfg(any(test, feature = "loopback-evidence"))]
impl fmt::Debug for PmPendingMutationServerTime {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PmPendingMutationServerTime(<opaque>)")
    }
}

/// Purpose-erased proof retained only for literal-loopback compatibility.
#[cfg(any(test, feature = "loopback-evidence"))]
pub struct PmAuthorizedMutationServerTime {
    #[cfg_attr(
        not(any(test, feature = "loopback-evidence")),
        allow(
            dead_code,
            reason = "the opaque timestamp is consumed only by the separately gated authenticated mutation roles"
        )
    )]
    timestamp: L2Timestamp,
}

#[cfg(any(test, feature = "loopback-evidence"))]
impl PmAuthorizedMutationServerTime {
    #[cfg_attr(
        not(any(test, feature = "loopback-evidence")),
        allow(
            dead_code,
            reason = "default and read-only compositions deliberately cannot consume mutation-time authority"
        )
    )]
    pub(crate) const fn into_l2_timestamp(self) -> L2Timestamp {
        self.timestamp
    }
}

#[cfg(any(test, feature = "loopback-evidence"))]
impl fmt::Debug for PmAuthorizedMutationServerTime {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PmAuthorizedMutationServerTime(<opaque>)")
    }
}

/// Purpose-erased validator retained only for literal-loopback compatibility.
#[cfg(any(test, feature = "loopback-evidence"))]
pub struct PmMutationServerTimeValidator {
    domain: Arc<ProductClockDomain>,
}

#[cfg(any(test, feature = "loopback-evidence"))]
impl PmMutationServerTimeValidator {
    pub fn authorize(
        &mut self,
        pending: PmPendingMutationServerTime,
    ) -> Result<PmAuthorizedMutationServerTime, PmProductClockError> {
        if !Arc::ptr_eq(&self.domain, &pending.domain) {
            return Err(PmProductClockError::WrongDomain);
        }
        validate_age(&self.domain, pending.received)?;
        Ok(PmAuthorizedMutationServerTime {
            timestamp: pending.timestamp,
        })
    }
}

fn validate_age(
    domain: &ProductClockDomain,
    received: PmRestResponseClock,
) -> Result<(), PmProductClockError> {
    let now = domain.sample()?;
    let age_ns = now
        .monotonic_ns
        .checked_sub(received.monotonic_receive_ns())
        .ok_or(PmProductClockError::ClockRegression)?;
    if age_ns > PM_MUTATION_SERVER_TIME_MAX_AGE.as_nanos() as u64 {
        return Err(PmProductClockError::ServerTimeStale);
    }
    Ok(())
}

#[cfg(any(test, feature = "loopback-evidence"))]
impl fmt::Debug for PmMutationServerTimeValidator {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PmMutationServerTimeValidator(<opaque-domain>)")
    }
}

/// Bounded, feature-gated proof issuer for deterministic loopback workers.
///
/// It is absent from normal builds and exposes no raw proof constructor or
/// timestamp getter. Each scripted timestamp is consumed once.
#[cfg(any(test, feature = "loopback-evidence"))]
pub struct PmLoopbackServerTimeScript {
    timestamps: VecDeque<L2Timestamp>,
}

#[cfg(any(test, feature = "loopback-evidence"))]
impl PmLoopbackServerTimeScript {
    pub fn new(seconds: &[u64]) -> Result<Self, PmProductClockError> {
        if seconds.is_empty() || seconds.len() > 1_024 {
            return Err(PmProductClockError::ScriptExhausted);
        }
        let timestamps = seconds
            .iter()
            .copied()
            .map(|value| {
                L2Timestamp::from_unix_seconds(value)
                    .map_err(|_| PmProductClockError::InvalidReading)
            })
            .collect::<Result<VecDeque<_>, _>>()?;
        Ok(Self { timestamps })
    }

    pub fn issue_authorized_mutation_server_time(
        &mut self,
    ) -> Result<PmAuthorizedMutationServerTime, PmProductClockError> {
        let timestamp = self
            .timestamps
            .pop_front()
            .ok_or(PmProductClockError::ScriptExhausted)?;
        Ok(PmAuthorizedMutationServerTime { timestamp })
    }
}

#[cfg(any(test, feature = "loopback-evidence"))]
impl fmt::Debug for PmLoopbackServerTimeScript {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PmLoopbackServerTimeScript")
            .field("remaining", &self.timestamps.len())
            .finish()
    }
}

#[cfg(test)]
pub(crate) fn test_support_read_server_time(seconds: u64) -> PmReadServerTime {
    let owner = PmProductClockOwner::test_support_scripted(&[(1_000, 10), (1_001, 11)])
        .expect("valid fixed test clock");
    let (_, _, http, _, _, _, _, _, _, _, _) = owner.split().into_views();
    let timestamp = L2Timestamp::from_unix_seconds(seconds).expect("valid fixed test timestamp");
    http.read_time(
        timestamp,
        http.observe_rest_edge().expect("fixed test REST edge"),
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PmActorClockObservation(ReceivedEventClock);

impl PmActorClockObservation {
    #[must_use]
    pub const fn received_clock(self) -> ReceivedEventClock {
        self.0
    }
}

pub struct PmActorProductClock {
    domain: Arc<ProductClockDomain>,
}

impl PmActorProductClock {
    pub fn observe_control_edge(&mut self) -> Result<PmActorClockObservation, PmProductClockError> {
        let reading = self.domain.sample()?;
        Ok(PmActorClockObservation(
            ReceivedEventClock::new(None, reading.local_wall_ns, reading.monotonic_ns)
                .map_err(|_| PmProductClockError::InvalidReading)?,
        ))
    }
}

pub struct PmOkxProductClock {
    domain: Arc<ProductClockDomain>,
}

impl PmOkxProductClock {
    pub fn observe_okx_edge(&mut self) -> Result<ReceivedEventClock, PmProductClockError> {
        let reading = self.domain.sample()?;
        ReceivedEventClock::new(None, reading.local_wall_ns, reading.monotonic_ns)
            .map_err(|_| PmProductClockError::InvalidReading)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct PlaceProvider {
        timestamp: Option<L2Timestamp>,
    }

    impl PmPlaceMutationTimeProvider for PlaceProvider {
        fn consume_final_place_time(
            &mut self,
            time: PmFinalPlaceMutationTime<'_>,
        ) -> Result<(), PmMutationTimeProviderError> {
            self.timestamp = Some(
                time.consume_l2_timestamp()
                    .map_err(PmMutationTimeProviderError::FinalClock)?,
            );
            Ok(())
        }
    }

    #[derive(Default)]
    struct CancelProvider {
        timestamp: Option<L2Timestamp>,
    }

    impl PmCancelMutationTimeProvider for CancelProvider {
        fn consume_final_cancel_time(
            &mut self,
            time: PmFinalCancelMutationTime<'_>,
        ) -> Result<(), PmMutationTimeProviderError> {
            self.timestamp = Some(
                time.consume_l2_timestamp()
                    .map_err(PmMutationTimeProviderError::FinalClock)?,
            );
            Ok(())
        }
    }

    #[test]
    fn one_script_is_shared_in_strict_cross_role_sample_order() {
        let owner = PmProductClockOwner::test_support_scripted(&[
            (1_000, 10),
            (1_001, 11),
            (1_002, 12),
            (1_003, 13),
        ])
        .unwrap();
        let (mut public, mut user, rest, _, _, _, _, _, _, mut actor, _) =
            owner.split().into_views();
        assert_eq!(
            public
                .observe_public_ws_edge()
                .unwrap()
                .monotonic_receive_ns(),
            10
        );
        assert_eq!(
            user.observe_user_ws_edge().unwrap().monotonic_receive_ns(),
            11
        );
        assert_eq!(rest.observe_rest_edge().unwrap().monotonic_receive_ns(), 12);
        assert_eq!(
            actor
                .observe_control_edge()
                .unwrap()
                .received_clock()
                .monotonic_receive_ns(),
            13
        );
    }

    #[test]
    fn observation_only_views_share_one_strictly_ordered_domain() {
        let owner = PmProductClockOwner::test_support_scripted(&[
            (1_000, 10),
            (1_001, 11),
            (1_002, 12),
            (1_003, 13),
            (1_004, 14),
            (1_005, 15),
            (1_006, 16),
        ])
        .unwrap();
        let (
            mut public_ws,
            mut user_ws,
            public_http,
            read_server_time_http,
            mut private_read,
            mut actor,
            mut okx,
        ) = owner.split_observation_only().into_views();
        assert_eq!(
            public_ws
                .observe_public_ws_edge()
                .unwrap()
                .monotonic_receive_ns(),
            10
        );
        assert_eq!(
            user_ws
                .observe_user_ws_edge()
                .unwrap()
                .monotonic_receive_ns(),
            11
        );
        assert_eq!(
            public_http
                .observe_rest_edge()
                .unwrap()
                .monotonic_receive_ns(),
            12
        );
        assert_eq!(
            read_server_time_http
                .observe_rest_edge()
                .unwrap()
                .monotonic_receive_ns(),
            13
        );
        assert_eq!(
            private_read
                .observe_authenticated_read_complete()
                .unwrap()
                .monotonic_receive_ns(),
            14
        );
        assert_eq!(
            actor
                .observe_control_edge()
                .unwrap()
                .received_clock()
                .monotonic_receive_ns(),
            15
        );
        assert_eq!(okx.observe_okx_edge().unwrap().monotonic_receive_ns(), 16);
    }

    #[test]
    fn final_mutation_time_rejects_wrong_domain_purpose_regression_and_staleness() {
        let first = PmProductClockOwner::test_support_scripted(&[(1_000, 10)]).unwrap();
        let second = PmProductClockOwner::test_support_scripted(&[(2_000, 20)]).unwrap();
        let (_, _, _, _, _, _, mut first_finalizer, _, _, _, _) = first.split().into_views();
        let (_, _, _, _, _, second_place_time, _, _, _, _, _) = second.split().into_views();
        let timestamp = L2Timestamp::from_unix_seconds(1_700_000_000).unwrap();
        let foreign = second_place_time
            .place_time_proof(timestamp, second_place_time.observe_rest_edge().unwrap());
        let mut provider = PlaceProvider::default();
        assert!(matches!(
            first_finalizer.consume_with(foreign, &mut provider),
            Err(PmMutationTimeConsumeError::Clock(
                PmProductClockError::WrongDomain
            ))
        ));
        assert!(provider.timestamp.is_none());

        let wrong_purpose =
            PmProductClockOwner::test_support_scripted(&[(1_000, 10), (1_001, 11)]).unwrap();
        let (_, _, _, _, _, place_time, mut finalizer, _, _, _, _) =
            wrong_purpose.split().into_views();
        let mut proof =
            place_time.place_time_proof(timestamp, place_time.observe_rest_edge().unwrap());
        proof.core.purpose = MutationTimePurpose::Cancel;
        assert!(matches!(
            finalizer.consume_with(proof, &mut provider),
            Err(PmMutationTimeConsumeError::Clock(
                PmProductClockError::WrongMutationPurpose
            ))
        ));

        let regression =
            PmProductClockOwner::test_support_scripted(&[(1_000, 10), (999, 9)]).unwrap();
        let (_, _, _, _, _, place_time, mut finalizer, _, _, _, _) =
            regression.split().into_views();
        let proof = place_time.place_time_proof(timestamp, place_time.observe_rest_edge().unwrap());
        assert!(matches!(
            finalizer.consume_with(proof, &mut provider),
            Err(PmMutationTimeConsumeError::Clock(
                PmProductClockError::ClockRegression
            ))
        ));

        let stale = PmProductClockOwner::test_support_scripted(&[
            (1_000, 10),
            (
                1_001,
                10 + PM_MUTATION_SERVER_TIME_MAX_AGE.as_nanos() as u64 + 1,
            ),
        ])
        .unwrap();
        let (_, _, _, _, _, place_time, mut finalizer, _, _, _, _) = stale.split().into_views();
        let proof = place_time.place_time_proof(timestamp, place_time.observe_rest_edge().unwrap());
        assert!(matches!(
            finalizer.consume_with(proof, &mut provider),
            Err(PmMutationTimeConsumeError::Clock(
                PmProductClockError::ServerTimeStale
            ))
        ));
    }

    #[test]
    fn final_mutation_time_accepts_exact_age_boundary_and_delivers_once() {
        let owner = PmProductClockOwner::test_support_scripted(&[
            (1_000, 10),
            (
                1_001,
                10 + PM_MUTATION_SERVER_TIME_MAX_AGE.as_nanos() as u64,
            ),
            (
                1_002,
                10 + PM_MUTATION_SERVER_TIME_MAX_AGE.as_nanos() as u64,
            ),
        ])
        .unwrap();
        let (_, _, _, _, _, place_time, mut finalizer, _, _, _, _) = owner.split().into_views();
        let timestamp = L2Timestamp::from_unix_seconds(1_700_000_000).unwrap();
        let proof = place_time.place_time_proof(timestamp, place_time.observe_rest_edge().unwrap());
        let mut provider = PlaceProvider::default();
        finalizer.consume_with(proof, &mut provider).unwrap();
        assert_eq!(provider.timestamp, Some(timestamp));
    }

    #[test]
    fn credential_provider_rechecks_age_after_finalizer_delivery_boundary() {
        let owner = PmProductClockOwner::test_support_scripted(&[
            (1_000, 10),
            (1_001, 11),
            (
                1_002,
                10 + PM_MUTATION_SERVER_TIME_MAX_AGE.as_nanos() as u64 + 1,
            ),
        ])
        .unwrap();
        let (_, _, _, _, _, place_time, mut finalizer, _, _, _, _) = owner.split().into_views();
        let timestamp = L2Timestamp::from_unix_seconds(1_700_000_000).unwrap();
        let proof = place_time.place_time_proof(timestamp, place_time.observe_rest_edge().unwrap());
        let mut provider = PlaceProvider::default();
        assert!(matches!(
            finalizer.consume_with(proof, &mut provider),
            Err(PmMutationTimeConsumeError::Provider(
                PmMutationTimeProviderError::FinalClock(PmProductClockError::ServerTimeStale)
            ))
        ));
        assert!(provider.timestamp.is_none());
    }

    #[test]
    fn read_time_is_consumed_once_and_rejects_stale_use() {
        let owner = PmProductClockOwner::test_support_scripted(&[
            (1_000, 10),
            (
                1_001,
                10 + PM_MUTATION_SERVER_TIME_MAX_AGE.as_nanos() as u64 + 1,
            ),
        ])
        .unwrap();
        let (_, _, _, http, _, _, _, _, _, _, _) = owner.split().into_views();
        let timestamp = L2Timestamp::from_unix_seconds(1_700_000_000).unwrap();
        let proof = http.read_time(timestamp, http.observe_rest_edge().unwrap());
        assert!(matches!(
            proof.into_l2_timestamp(),
            Err(PmProductClockError::ServerTimeStale)
        ));
    }

    #[test]
    fn private_read_place_and_cancel_views_share_one_ordered_domain() {
        let owner = PmProductClockOwner::test_support_scripted(&[
            (1_000, 10),
            (1_001, 11),
            (1_002, 12),
            (1_003, 13),
            (1_004, 14),
            (1_005, 15),
            (1_006, 16),
        ])
        .unwrap();
        let (
            _,
            _,
            _,
            _,
            mut private_read,
            place_time,
            mut place_finalizer,
            cancel_time,
            mut cancel_finalizer,
            _,
            _,
        ) = owner.split().into_views();
        assert_eq!(
            private_read
                .observe_authenticated_read_complete()
                .unwrap()
                .monotonic_receive_ns(),
            10
        );
        let timestamp = L2Timestamp::from_unix_seconds(1_700_000_000).unwrap();
        let place = place_time.place_time_proof(timestamp, place_time.observe_rest_edge().unwrap());
        let mut place_provider = PlaceProvider::default();
        place_finalizer
            .consume_with(place, &mut place_provider)
            .unwrap();
        let cancel =
            cancel_time.cancel_time_proof(timestamp, cancel_time.observe_rest_edge().unwrap());
        let mut cancel_provider = CancelProvider::default();
        cancel_finalizer
            .consume_with(cancel, &mut cancel_provider)
            .unwrap();
        assert_eq!(place_provider.timestamp, Some(timestamp));
        assert_eq!(cancel_provider.timestamp, Some(timestamp));
    }
}
