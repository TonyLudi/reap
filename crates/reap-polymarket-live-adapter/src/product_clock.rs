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
use reap_polymarket_auth::L2Timestamp;
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
    pub fn split(self) -> PmProductClockViews {
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
            place_server_time_http: PmMutationServerTimeProductClock {
                domain: Arc::clone(&self.domain),
            },
            cancel_server_time_http: PmMutationServerTimeProductClock {
                domain: Arc::clone(&self.domain),
            },
            actor: PmActorProductClock {
                domain: Arc::clone(&self.domain),
            },
            okx: PmOkxProductClock {
                domain: Arc::clone(&self.domain),
            },
            mutation_time_validator: PmMutationServerTimeValidator {
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
pub struct PmProductClockViews {
    public_ws: PmPublicWsProductClock,
    user_ws: PmUserWsProductClock,
    public_http: PmPublicHttpProductClock,
    read_server_time_http: PmReadServerTimeProductClock,
    private_read: PmPrivateReadProductClock,
    place_server_time_http: PmMutationServerTimeProductClock,
    cancel_server_time_http: PmMutationServerTimeProductClock,
    actor: PmActorProductClock,
    okx: PmOkxProductClock,
    mutation_time_validator: PmMutationServerTimeValidator,
}

impl PmProductClockViews {
    #[must_use]
    pub fn into_views(
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
            self.place_server_time_http,
            self.cancel_server_time_http,
            self.actor,
            self.okx,
            self.mutation_time_validator,
        )
    }
}

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
        let (_, _, public_http, _, _, _, _, _, _, _) = owner.split().into_views();
        public_http
    }

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

/// Clock capability for exactly one mutation-purpose `/time` client.
///
/// Separate instances are issued for place and cancel so neither HTTP client
/// is shared across independently supervised mutation paths.
pub struct PmMutationServerTimeProductClock {
    domain: Arc<ProductClockDomain>,
}

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

pub struct PmPendingMutationServerTime {
    timestamp: L2Timestamp,
    received: PmRestResponseClock,
    domain: Arc<ProductClockDomain>,
}

impl fmt::Debug for PmPendingMutationServerTime {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PmPendingMutationServerTime(<opaque>)")
    }
}

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

impl fmt::Debug for PmAuthorizedMutationServerTime {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PmAuthorizedMutationServerTime(<opaque>)")
    }
}

pub struct PmMutationServerTimeValidator {
    domain: Arc<ProductClockDomain>,
}

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
    let (_, _, http, _, _, _, _, _, _, _) = owner.split().into_views();
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

    #[test]
    fn one_script_is_shared_in_strict_cross_role_sample_order() {
        let owner = PmProductClockOwner::test_support_scripted(&[
            (1_000, 10),
            (1_001, 11),
            (1_002, 12),
            (1_003, 13),
        ])
        .unwrap();
        let (mut public, mut user, rest, _, _, _, _, mut actor, _, _) = owner.split().into_views();
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
    fn mutation_time_rejects_wrong_domain_regression_and_staleness() {
        let first =
            PmProductClockOwner::test_support_scripted(&[(1_000, 10), (1_001, 11)]).unwrap();
        let second = PmProductClockOwner::test_support_scripted(&[(2_000, 20)]).unwrap();
        let (_, _, _, _, _, _first_place_time, _, _, _, mut first_validator) =
            first.split().into_views();
        let (_, _, _, _, _, second_place_time, _, _, _, _) = second.split().into_views();
        let timestamp = L2Timestamp::from_unix_seconds(1_700_000_000).unwrap();
        let foreign = second_place_time
            .pending_mutation_time(timestamp, second_place_time.observe_rest_edge().unwrap());
        assert!(matches!(
            first_validator.authorize(foreign),
            Err(PmProductClockError::WrongDomain)
        ));

        let regression =
            PmProductClockOwner::test_support_scripted(&[(1_000, 10), (999, 9)]).unwrap();
        let (_, _, _, _, _, place_time, _, _, _, mut validator) = regression.split().into_views();
        let pending =
            place_time.pending_mutation_time(timestamp, place_time.observe_rest_edge().unwrap());
        assert!(matches!(
            validator.authorize(pending),
            Err(PmProductClockError::ClockRegression)
        ));

        let stale = PmProductClockOwner::test_support_scripted(&[
            (1_000, 10),
            (
                1_001,
                10 + PM_MUTATION_SERVER_TIME_MAX_AGE.as_nanos() as u64 + 1,
            ),
        ])
        .unwrap();
        let (_, _, _, _, _, place_time, _, _, _, mut validator) = stale.split().into_views();
        let pending =
            place_time.pending_mutation_time(timestamp, place_time.observe_rest_edge().unwrap());
        assert!(matches!(
            validator.authorize(pending),
            Err(PmProductClockError::ServerTimeStale)
        ));
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
        let (_, _, _, http, _, _, _, _, _, _) = owner.split().into_views();
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
        ])
        .unwrap();
        let (_, _, _, _, mut private_read, place_time, cancel_time, _, _, mut validator) =
            owner.split().into_views();
        assert_eq!(
            private_read
                .observe_authenticated_read_complete()
                .unwrap()
                .monotonic_receive_ns(),
            10
        );
        let timestamp = L2Timestamp::from_unix_seconds(1_700_000_000).unwrap();
        let place =
            place_time.pending_mutation_time(timestamp, place_time.observe_rest_edge().unwrap());
        validator.authorize(place).unwrap();
        let cancel =
            cancel_time.pending_mutation_time(timestamp, cancel_time.observe_rest_edge().unwrap());
        validator.authorize(cancel).unwrap();
    }
}
