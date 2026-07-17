use std::future::Future;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use futures_util::FutureExt;
use futures_util::future::BoxFuture;
use futures_util::stream::{FuturesUnordered, StreamExt};
use reap_okx_live_adapter::ForbiddenOrderObserver;
use reap_order::{PacingPolicy, RequestKind, RequestPacer};
use reap_venue::okx::{
    OkxAlgoOrderPage, OkxAlgoOrderPagination, OkxAlgoOrderQuery, OkxSpreadOrderPage,
    OkxSpreadOrderPagination, RestError,
};
use tokio::sync::mpsc;
use tokio::time::{Instant, MissedTickBehavior};

use crate::{FORBIDDEN_PROOF_HARD_MAX_AGE_MS, ForbiddenProofPolicy};

const MAX_CONCURRENT_FORBIDDEN_SCANS: usize = 16;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ForbiddenOrderEvent {
    pub account_id: String,
    pub observed_at_ms: u64,
    pub state: ForbiddenOrderState,
}

impl ForbiddenOrderEvent {
    pub(crate) fn expire_delayed_zero_proof(&mut self, now_ms: u64) {
        let expires_at_ms = match &self.state {
            ForbiddenOrderState::VerifiedZero { expires_at_ms } => *expires_at_ms,
            ForbiddenOrderState::NonZero { .. }
            | ForbiddenOrderState::Unverifiable { .. }
            | ForbiddenOrderState::Expired { .. } => return,
        };
        if now_ms < expires_at_ms {
            return;
        }
        self.state = ForbiddenOrderState::Expired {
            last_verified_at_ms: self.observed_at_ms,
            max_age_ms: expires_at_ms.saturating_sub(self.observed_at_ms),
        };
        self.observed_at_ms = now_ms;
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ForbiddenOrderState {
    VerifiedZero {
        expires_at_ms: u64,
    },
    NonZero {
        algo_orders_observed: Option<usize>,
        spread_orders_observed: Option<usize>,
    },
    Unverifiable {
        domain: ForbiddenOrderDomain,
        reason: String,
    },
    Expired {
        last_verified_at_ms: u64,
        max_age_ms: u64,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ForbiddenOrderDomain {
    Algo,
    Spread,
}

impl ForbiddenOrderDomain {
    fn as_str(self) -> &'static str {
        match self {
            Self::Algo => "algo",
            Self::Spread => "spread",
        }
    }
}

impl ForbiddenOrderState {
    pub(crate) fn is_verified_zero(&self) -> bool {
        matches!(self, Self::VerifiedZero { .. })
    }

    pub(crate) fn failure_reason(&self) -> Option<String> {
        match self {
            Self::VerifiedZero { .. } => None,
            Self::NonZero {
                algo_orders_observed,
                spread_orders_observed,
            } => Some(format!(
                "forbidden pending orders are nonzero (algo_observed={}, spread_observed={})",
                observed_count(*algo_orders_observed),
                observed_count(*spread_orders_observed),
            )),
            Self::Unverifiable { domain, reason } => Some(format!(
                "forbidden {} pending orders are unverifiable: {reason}",
                domain.as_str(),
            )),
            Self::Expired {
                last_verified_at_ms,
                max_age_ms,
            } => Some(format!(
                "forbidden-order zero proof expired after {max_age_ms}ms (last verified at {last_verified_at_ms})"
            )),
        }
    }

    pub(crate) fn alert_code(&self) -> Option<&'static str> {
        match self {
            Self::VerifiedZero { .. } => None,
            Self::NonZero { .. } => Some("forbidden_orders_nonzero"),
            Self::Unverifiable { .. } => Some("forbidden_orders_unverifiable"),
            Self::Expired { .. } => Some("forbidden_orders_proof_expired"),
        }
    }
}

fn observed_count(count: Option<usize>) -> String {
    count.map_or_else(|| "unobserved".to_string(), |count| count.to_string())
}

#[derive(Debug, Clone)]
pub(crate) struct ForbiddenSentinelPolicy {
    max_age: Duration,
    scan_interval: Duration,
    domain_timeout: Duration,
    max_concurrent_scans: usize,
    max_pages: usize,
    pacing_policy: PacingPolicy,
}

impl ForbiddenSentinelPolicy {
    pub(crate) fn from_plan(
        policy: &ForbiddenProofPolicy,
        max_pages: usize,
        pacing_policy: PacingPolicy,
    ) -> Result<Self, String> {
        Self::new(
            policy.max_age_ms(),
            policy.scan_interval_ms(),
            policy.hard_max_age_ms(),
            max_pages,
            pacing_policy,
        )
    }

    fn new(
        max_age_ms: u64,
        scan_interval_ms: u64,
        domain_timeout_ms: u64,
        max_pages: usize,
        pacing_policy: PacingPolicy,
    ) -> Result<Self, String> {
        if max_age_ms == 0 || max_age_ms > FORBIDDEN_PROOF_HARD_MAX_AGE_MS {
            return Err(format!(
                "forbidden proof maximum age must be between 1 and {FORBIDDEN_PROOF_HARD_MAX_AGE_MS}ms"
            ));
        }
        if scan_interval_ms == 0 || scan_interval_ms > max_age_ms / 2 {
            return Err(format!(
                "forbidden proof scan interval must be between 1ms and half of the {max_age_ms}ms maximum age"
            ));
        }
        if domain_timeout_ms == 0 || domain_timeout_ms > FORBIDDEN_PROOF_HARD_MAX_AGE_MS {
            return Err(format!(
                "forbidden scan timeout must be between 1 and {FORBIDDEN_PROOF_HARD_MAX_AGE_MS}ms"
            ));
        }
        let max_concurrent_scans = concurrent_scan_bound(
            Duration::from_millis(domain_timeout_ms),
            Duration::from_millis(scan_interval_ms),
        );
        if max_concurrent_scans > MAX_CONCURRENT_FORBIDDEN_SCANS {
            return Err(format!(
                "forbidden scan timeout/interval ratio requires {max_concurrent_scans} concurrent scans, above the hard maximum {MAX_CONCURRENT_FORBIDDEN_SCANS}"
            ));
        }
        if max_pages == 0 {
            return Err("forbidden scan page cap must be positive".to_string());
        }
        Ok(Self {
            max_age: Duration::from_millis(max_age_ms),
            scan_interval: Duration::from_millis(scan_interval_ms),
            domain_timeout: Duration::from_millis(domain_timeout_ms),
            max_concurrent_scans,
            max_pages,
            pacing_policy,
        })
    }
}

fn concurrent_scan_bound(domain_timeout: Duration, scan_interval: Duration) -> usize {
    let timeout_ms = domain_timeout.as_millis();
    let interval_ms = scan_interval.as_millis().max(1);
    usize::try_from(timeout_ms / interval_ms + 1).unwrap_or(usize::MAX)
}

#[async_trait]
pub(crate) trait ForbiddenOrderObserverPort: Send + Sync {
    async fn algo_pending_page(
        &self,
        query: OkxAlgoOrderQuery,
        after: Option<&str>,
    ) -> Result<OkxAlgoOrderPage, RestError>;

    async fn spread_pending_page(
        &self,
        end_id: Option<&str>,
    ) -> Result<OkxSpreadOrderPage, RestError>;
}

#[async_trait]
impl ForbiddenOrderObserverPort for ForbiddenOrderObserver {
    async fn algo_pending_page(
        &self,
        query: OkxAlgoOrderQuery,
        after: Option<&str>,
    ) -> Result<OkxAlgoOrderPage, RestError> {
        ForbiddenOrderObserver::algo_pending_page(self, query, after).await
    }

    async fn spread_pending_page(
        &self,
        end_id: Option<&str>,
    ) -> Result<OkxSpreadOrderPage, RestError> {
        ForbiddenOrderObserver::spread_pending_page(self, end_id).await
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ForbiddenScanResult {
    VerifiedZero,
    NonZero {
        algo_orders_observed: Option<usize>,
        spread_orders_observed: Option<usize>,
    },
    Unverifiable {
        domain: ForbiddenOrderDomain,
        reason: String,
    },
}

struct TaggedForbiddenScanResult {
    generation: u64,
    completed_at: Instant,
    observed_at_ms: u64,
    result: ForbiddenScanResult,
}

#[derive(Debug, Default)]
struct ForbiddenScanOrder {
    zero_generation_floor: u64,
    latest_accepted_zero_generation: Option<u64>,
}

impl ForbiddenScanOrder {
    fn accept(
        &mut self,
        generation: u64,
        next_generation: u64,
        result: ForbiddenScanResult,
    ) -> Option<ForbiddenScanResult> {
        if result != ForbiddenScanResult::VerifiedZero {
            self.invalidate(next_generation);
            return Some(result);
        }
        if generation < self.zero_generation_floor
            || self
                .latest_accepted_zero_generation
                .is_some_and(|latest| generation <= latest)
        {
            return None;
        }
        self.latest_accepted_zero_generation = Some(generation);
        Some(result)
    }

    fn invalidate(&mut self, next_generation: u64) {
        self.zero_generation_floor = next_generation;
        self.latest_accepted_zero_generation = None;
    }
}

#[derive(Default)]
struct ForbiddenEventOutbox {
    failure: Option<ForbiddenOrderEvent>,
    recovery: Option<ForbiddenOrderEvent>,
}

impl ForbiddenEventOutbox {
    fn push(&mut self, event: ForbiddenOrderEvent) {
        if event.state.is_verified_zero() {
            self.recovery = Some(event);
            return;
        }
        self.recovery = None;
        let replace_failure = self.failure.as_ref().is_none_or(|pending| {
            failure_priority(&event.state) >= failure_priority(&pending.state)
        });
        if replace_failure {
            self.failure = Some(event);
        }
    }

    fn is_empty(&self) -> bool {
        self.failure.is_none() && self.recovery.is_none()
    }

    fn pop_front(&mut self) -> Option<ForbiddenOrderEvent> {
        self.failure.take().or_else(|| self.recovery.take())
    }
}

fn failure_priority(state: &ForbiddenOrderState) -> u8 {
    match state {
        ForbiddenOrderState::VerifiedZero { .. } => 0,
        ForbiddenOrderState::Expired { .. } => 1,
        ForbiddenOrderState::Unverifiable { .. } => 2,
        ForbiddenOrderState::NonZero { .. } => 3,
    }
}

pub(crate) async fn run_forbidden_order_sentinel(
    account_id: String,
    observer: Arc<dyn ForbiddenOrderObserverPort>,
    policy: ForbiddenSentinelPolicy,
    events: mpsc::Sender<ForbiddenOrderEvent>,
) {
    let algo_pacer = RequestPacer::new(policy.pacing_policy.clone());
    let spread_pacer = RequestPacer::new(policy.pacing_policy.clone());
    let mut last_verified: Option<(Instant, u64)> = None;
    let mut scans = FuturesUnordered::<BoxFuture<'static, TaggedForbiddenScanResult>>::new();
    let mut scan_order = ForbiddenScanOrder::default();
    let mut outbox = ForbiddenEventOutbox::default();
    let mut next_generation = 0_u64;
    let mut scan_starts = tokio::time::interval_at(Instant::now(), policy.scan_interval);
    scan_starts.set_missed_tick_behavior(MissedTickBehavior::Skip);

    loop {
        let expiry = last_verified.map(|(verified_at, _)| verified_at + policy.max_age);
        tokio::select! {
            biased;
            _ = wait_until(expiry), if expiry.is_some() => {
                let (_, last_verified_at_ms) =
                    last_verified.take().expect("guarded forbidden proof expiry");
                scan_order.invalidate(next_generation);
                outbox.push(ForbiddenOrderEvent {
                    account_id: account_id.clone(),
                    observed_at_ms: unix_time_ms(),
                    state: ForbiddenOrderState::Expired {
                        last_verified_at_ms,
                        max_age_ms: duration_ms(policy.max_age),
                    },
                });
            }
            Some(completed) = scans.next(), if !scans.is_empty() => {
                let Some(result) =
                    scan_order.accept(completed.generation, next_generation, completed.result)
                else {
                    continue;
                };
                let state = match result {
                    ForbiddenScanResult::VerifiedZero => {
                        if Instant::now() >= completed.completed_at + policy.max_age {
                            last_verified = None;
                            scan_order.invalidate(next_generation);
                            ForbiddenOrderState::Expired {
                                last_verified_at_ms: completed.observed_at_ms,
                                max_age_ms: duration_ms(policy.max_age),
                            }
                        } else {
                            last_verified =
                                Some((completed.completed_at, completed.observed_at_ms));
                            ForbiddenOrderState::VerifiedZero {
                                expires_at_ms: completed
                                    .observed_at_ms
                                    .saturating_add(duration_ms(policy.max_age)),
                            }
                        }
                    }
                    ForbiddenScanResult::NonZero {
                        algo_orders_observed,
                        spread_orders_observed,
                    } => {
                        last_verified = None;
                        ForbiddenOrderState::NonZero {
                            algo_orders_observed,
                            spread_orders_observed,
                        }
                    }
                    ForbiddenScanResult::Unverifiable { domain, reason } => {
                        last_verified = None;
                        ForbiddenOrderState::Unverifiable { domain, reason }
                    }
                };
                outbox.push(ForbiddenOrderEvent {
                    account_id: account_id.clone(),
                    observed_at_ms: completed.observed_at_ms,
                    state,
                });
            }
            _ = scan_starts.tick() => {
                if scans.len() >= policy.max_concurrent_scans {
                    scans = FuturesUnordered::new();
                    last_verified = None;
                    scan_order.invalidate(next_generation);
                    outbox.push(ForbiddenOrderEvent {
                        account_id: account_id.clone(),
                        observed_at_ms: unix_time_ms(),
                        state: ForbiddenOrderState::Unverifiable {
                            domain: ForbiddenOrderDomain::Algo,
                            reason: format!(
                                "forbidden scan scheduler reached its bounded concurrency limit {}",
                                policy.max_concurrent_scans
                            ),
                        },
                    });
                }
                let generation = next_generation;
                next_generation = next_generation.saturating_add(1);
                scans.push(
                    launch_forbidden_scan(
                        generation,
                        Arc::clone(&observer),
                        policy.max_pages,
                        policy.domain_timeout,
                        algo_pacer.clone(),
                        spread_pacer.clone(),
                    )
                    .boxed(),
                );
            }
            permit = events.reserve(), if !outbox.is_empty() => {
                match permit {
                    Ok(permit) => {
                        permit.send(
                            outbox
                                .pop_front()
                                .expect("guarded forbidden event outbox"),
                        );
                    }
                    Err(_) => return,
                }
            }
            _ = events.closed() => return,
        }
    }
}

async fn launch_forbidden_scan(
    generation: u64,
    observer: Arc<dyn ForbiddenOrderObserverPort>,
    max_pages: usize,
    domain_timeout: Duration,
    algo_pacer: RequestPacer,
    spread_pacer: RequestPacer,
) -> TaggedForbiddenScanResult {
    let result = scan_forbidden_orders(
        observer.as_ref(),
        max_pages,
        domain_timeout,
        &algo_pacer,
        &spread_pacer,
    )
    .await;
    TaggedForbiddenScanResult {
        generation,
        completed_at: Instant::now(),
        observed_at_ms: unix_time_ms(),
        result,
    }
}

async fn wait_until(deadline: Option<Instant>) {
    match deadline {
        Some(deadline) => tokio::time::sleep_until(deadline).await,
        None => std::future::pending::<()>().await,
    }
}

async fn scan_forbidden_orders(
    observer: &dyn ForbiddenOrderObserverPort,
    max_pages: usize,
    domain_timeout: Duration,
    algo_pacer: &RequestPacer,
    spread_pacer: &RequestPacer,
) -> ForbiddenScanResult {
    let algo = scan_domain(
        ForbiddenOrderDomain::Algo,
        domain_timeout,
        scan_algo_orders(observer, max_pages, algo_pacer),
    );
    let spread = scan_domain(
        ForbiddenOrderDomain::Spread,
        domain_timeout,
        scan_spread_orders(observer, max_pages, spread_pacer),
    );
    tokio::pin!(algo);
    tokio::pin!(spread);
    let mut algo_zero = false;
    let mut spread_zero = false;

    loop {
        tokio::select! {
            biased;
            result = &mut algo, if !algo_zero => {
                match result {
                    Ok(0) => algo_zero = true,
                    Ok(count) => {
                        return ForbiddenScanResult::NonZero {
                            algo_orders_observed: Some(count),
                            spread_orders_observed: spread_zero.then_some(0),
                        };
                    }
                    Err(reason) => {
                        return ForbiddenScanResult::Unverifiable {
                            domain: ForbiddenOrderDomain::Algo,
                            reason,
                        };
                    }
                }
            }
            result = &mut spread, if !spread_zero => {
                match result {
                    Ok(0) => spread_zero = true,
                    Ok(count) => {
                        return ForbiddenScanResult::NonZero {
                            algo_orders_observed: algo_zero.then_some(0),
                            spread_orders_observed: Some(count),
                        };
                    }
                    Err(reason) => {
                        return ForbiddenScanResult::Unverifiable {
                            domain: ForbiddenOrderDomain::Spread,
                            reason,
                        };
                    }
                }
            }
        }
        if algo_zero && spread_zero {
            return ForbiddenScanResult::VerifiedZero;
        }
    }
}

async fn scan_domain(
    domain: ForbiddenOrderDomain,
    timeout: Duration,
    scan: impl Future<Output = Result<usize, RestError>>,
) -> Result<usize, String> {
    tokio::time::timeout(timeout, scan)
        .await
        .map_err(|_| {
            format!(
                "scan exceeded its independent {}ms deadline",
                duration_ms(timeout)
            )
        })?
        .map_err(|error| format!("{} scan failed: {error}", domain.as_str()))
}

async fn scan_algo_orders(
    observer: &dyn ForbiddenOrderObserverPort,
    max_pages: usize,
    pacer: &RequestPacer,
) -> Result<usize, RestError> {
    for query in OkxAlgoOrderQuery::ALL {
        let mut pagination = OkxAlgoOrderPagination::new(max_pages)?;
        loop {
            pacer.pace(RequestKind::Reconcile, "forbidden-algo").await;
            let page = observer
                .algo_pending_page(query, pagination.after())
                .await?;
            let observed_orders = page.orders.len();
            let complete = pagination.accept(page)?;
            if observed_orders > 0 {
                return Ok(observed_orders);
            }
            if complete {
                break;
            }
        }
    }
    Ok(0)
}

async fn scan_spread_orders(
    observer: &dyn ForbiddenOrderObserverPort,
    max_pages: usize,
    pacer: &RequestPacer,
) -> Result<usize, RestError> {
    let mut pagination = OkxSpreadOrderPagination::new(max_pages)?;
    loop {
        pacer.pace(RequestKind::Reconcile, "forbidden-spread").await;
        let page = observer.spread_pending_page(pagination.end_id()).await?;
        let observed_orders = page.orders.len();
        let complete = pagination.accept(page)?;
        if observed_orders > 0 {
            return Ok(observed_orders);
        }
        if complete {
            break;
        }
    }
    Ok(0)
}

fn duration_ms(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

fn unix_time_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, VecDeque};
    use std::sync::Mutex;

    use reap_venue::okx::{
        OKX_DEFAULT_MAX_PENDING_ORDER_PAGES, OkxAlgoOrder, OkxAlgoOrderType, OkxSpreadOrder,
    };

    use super::*;

    enum AlgoResponse {
        Page(OkxAlgoOrderPage),
        Error(&'static str),
        Hang,
    }

    enum SpreadResponse {
        Page(OkxSpreadOrderPage),
        Error(&'static str),
        Hang,
    }

    #[derive(Default)]
    struct ObserverMock {
        algo: Mutex<BTreeMap<String, VecDeque<AlgoResponse>>>,
        spread: Mutex<VecDeque<SpreadResponse>>,
        calls: Mutex<Vec<String>>,
    }

    impl ObserverMock {
        fn algo_response(&self, query: OkxAlgoOrderQuery, response: AlgoResponse) {
            self.algo
                .lock()
                .unwrap()
                .entry(query.as_str().to_string())
                .or_default()
                .push_back(response);
        }

        fn spread_response(&self, response: SpreadResponse) {
            self.spread.lock().unwrap().push_back(response);
        }
    }

    #[async_trait]
    impl ForbiddenOrderObserverPort for ObserverMock {
        async fn algo_pending_page(
            &self,
            query: OkxAlgoOrderQuery,
            after: Option<&str>,
        ) -> Result<OkxAlgoOrderPage, RestError> {
            self.calls
                .lock()
                .unwrap()
                .push(format!("algo:{}:{after:?}", query.as_str()));
            let response = {
                self.algo
                    .lock()
                    .unwrap()
                    .get_mut(query.as_str())
                    .and_then(VecDeque::pop_front)
            };
            match response {
                Some(AlgoResponse::Page(page)) => Ok(page),
                Some(AlgoResponse::Error(reason)) => Err(RestError::Transport(reason.to_string())),
                Some(AlgoResponse::Hang) => std::future::pending().await,
                None => Ok(OkxAlgoOrderPage {
                    orders: Vec::new(),
                    next_after: None,
                }),
            }
        }

        async fn spread_pending_page(
            &self,
            end_id: Option<&str>,
        ) -> Result<OkxSpreadOrderPage, RestError> {
            self.calls
                .lock()
                .unwrap()
                .push(format!("spread:{end_id:?}"));
            let response = { self.spread.lock().unwrap().pop_front() };
            match response {
                Some(SpreadResponse::Page(page)) => Ok(page),
                Some(SpreadResponse::Error(reason)) => {
                    Err(RestError::Transport(reason.to_string()))
                }
                Some(SpreadResponse::Hang) => std::future::pending().await,
                None => Ok(OkxSpreadOrderPage {
                    orders: Vec::new(),
                    next_end_id: None,
                }),
            }
        }
    }

    fn no_pacing() -> PacingPolicy {
        PacingPolicy {
            submit_requests: 100,
            cancel_requests: 100,
            reconcile_requests: 100,
            window: Duration::from_millis(1),
        }
    }

    fn algo_order(id: &str, order_type: OkxAlgoOrderType) -> OkxAlgoOrder {
        OkxAlgoOrder {
            algo_id: id.to_string(),
            client_order_id: String::new(),
            symbol: "BTC-USDT".to_string(),
            order_type,
            state: "effective".to_string(),
        }
    }

    fn spread_order(id: &str) -> OkxSpreadOrder {
        OkxSpreadOrder {
            spread_id: format!("spread-{id}"),
            exchange_order_id: id.to_string(),
            client_order_id: String::new(),
            state: "live".to_string(),
        }
    }

    async fn scan(observer: &ObserverMock, max_pages: usize) -> ForbiddenScanResult {
        scan_forbidden_orders(
            observer,
            max_pages,
            Duration::from_millis(50),
            &RequestPacer::new(no_pacing()),
            &RequestPacer::new(no_pacing()),
        )
        .await
    }

    fn unverifiable_reason(
        result: ForbiddenScanResult,
        expected_domain: ForbiddenOrderDomain,
    ) -> String {
        match result {
            ForbiddenScanResult::Unverifiable { domain, reason } => {
                assert_eq!(domain, expected_domain);
                reason
            }
            other => panic!("expected unverifiable {expected_domain:?}, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn complete_zero_scan_visits_all_seven_algo_families_and_spread() {
        let observer = ObserverMock::default();
        assert_eq!(
            scan(&observer, OKX_DEFAULT_MAX_PENDING_ORDER_PAGES).await,
            ForbiddenScanResult::VerifiedZero
        );
        let calls = observer.calls.lock().unwrap().clone();
        for query in OkxAlgoOrderQuery::ALL {
            assert!(
                calls
                    .iter()
                    .any(|call| call == &format!("algo:{}:None", query.as_str()))
            );
        }
        assert!(calls.iter().any(|call| call == "spread:None"));
        assert_eq!(calls.len(), 8);
    }

    #[tokio::test]
    async fn first_known_nonzero_does_not_claim_the_peer_domain_was_observed() {
        let observer = ObserverMock::default();
        observer.algo_response(
            OkxAlgoOrderQuery::Trigger,
            AlgoResponse::Page(OkxAlgoOrderPage {
                orders: vec![algo_order("algo-1", OkxAlgoOrderType::Trigger)],
                next_after: None,
            }),
        );
        observer.spread_response(SpreadResponse::Page(OkxSpreadOrderPage {
            orders: vec![spread_order("spread-order-1")],
            next_end_id: None,
        }));
        assert_eq!(
            scan(&observer, OKX_DEFAULT_MAX_PENDING_ORDER_PAGES).await,
            ForbiddenScanResult::NonZero {
                algo_orders_observed: Some(1),
                spread_orders_observed: None,
            }
        );
    }

    #[tokio::test]
    async fn duplicate_algo_id_across_query_families_can_never_produce_a_zero_proof() {
        let observer = ObserverMock::default();
        for (query, order_type) in [
            (
                OkxAlgoOrderQuery::ConditionalAndOco,
                OkxAlgoOrderType::Conditional,
            ),
            (OkxAlgoOrderQuery::Trigger, OkxAlgoOrderType::Trigger),
        ] {
            observer.algo_response(
                query,
                AlgoResponse::Page(OkxAlgoOrderPage {
                    orders: vec![algo_order("duplicate", order_type)],
                    next_after: None,
                }),
            );
        }
        assert_eq!(
            scan(&observer, OKX_DEFAULT_MAX_PENDING_ORDER_PAGES).await,
            ForbiddenScanResult::NonZero {
                algo_orders_observed: Some(1),
                spread_orders_observed: None,
            }
        );
    }

    #[tokio::test]
    async fn repeated_cursor_and_page_cap_errors_propagate_as_unverifiable() {
        let repeated = ObserverMock::default();
        for _ in 0..2 {
            repeated.algo_response(
                OkxAlgoOrderQuery::Chase,
                AlgoResponse::Page(OkxAlgoOrderPage {
                    orders: Vec::new(),
                    next_after: Some("same".to_string()),
                }),
            );
        }
        let error = unverifiable_reason(scan(&repeated, 4).await, ForbiddenOrderDomain::Algo);
        assert!(error.contains("OKX algo pending-order pagination repeated cursor same"));

        let capped = ObserverMock::default();
        capped.spread_response(SpreadResponse::Page(OkxSpreadOrderPage {
            orders: Vec::new(),
            next_end_id: Some("more".to_string()),
        }));
        let error = unverifiable_reason(scan(&capped, 1).await, ForbiddenOrderDomain::Spread);
        assert!(error.contains(
            "OKX spread pending-order pagination reached the configured limit after 1 pages"
        ));
    }

    #[tokio::test]
    async fn endpoint_failure_and_independent_domain_timeout_fail_closed() {
        let failed = ObserverMock::default();
        failed.algo_response(
            OkxAlgoOrderQuery::Iceberg,
            AlgoResponse::Error("malformed or unknown enum"),
        );
        assert!(
            unverifiable_reason(
                scan(&failed, OKX_DEFAULT_MAX_PENDING_ORDER_PAGES).await,
                ForbiddenOrderDomain::Algo,
            )
            .contains("malformed or unknown enum")
        );

        let hung = ObserverMock::default();
        hung.spread_response(SpreadResponse::Hang);
        let error = unverifiable_reason(
            scan_forbidden_orders(
                &hung,
                OKX_DEFAULT_MAX_PENDING_ORDER_PAGES,
                Duration::from_millis(5),
                &RequestPacer::new(no_pacing()),
                &RequestPacer::new(no_pacing()),
            )
            .await,
            ForbiddenOrderDomain::Spread,
        );
        assert!(error.contains("scan exceeded its independent 5ms deadline"));
        assert!(
            hung.calls
                .lock()
                .unwrap()
                .iter()
                .any(|call| call.starts_with("algo:"))
        );
    }

    #[tokio::test]
    async fn known_invalid_domain_returns_without_waiting_for_hung_peer() {
        let nonzero = ObserverMock::default();
        nonzero.algo_response(
            OkxAlgoOrderQuery::Trigger,
            AlgoResponse::Page(OkxAlgoOrderPage {
                orders: vec![algo_order("algo-1", OkxAlgoOrderType::Trigger)],
                next_after: None,
            }),
        );
        nonzero.spread_response(SpreadResponse::Hang);
        let result = tokio::time::timeout(
            Duration::from_millis(50),
            scan_forbidden_orders(
                &nonzero,
                OKX_DEFAULT_MAX_PENDING_ORDER_PAGES,
                Duration::from_secs(5),
                &RequestPacer::new(no_pacing()),
                &RequestPacer::new(no_pacing()),
            ),
        )
        .await
        .expect("known algo nonzero must not await the hung spread scan");
        assert_eq!(
            result,
            ForbiddenScanResult::NonZero {
                algo_orders_observed: Some(1),
                spread_orders_observed: None,
            }
        );

        let failed = ObserverMock::default();
        failed.algo_response(
            OkxAlgoOrderQuery::ConditionalAndOco,
            AlgoResponse::Error("algo endpoint failed"),
        );
        failed.spread_response(SpreadResponse::Hang);
        let result = tokio::time::timeout(
            Duration::from_millis(50),
            scan_forbidden_orders(
                &failed,
                OKX_DEFAULT_MAX_PENDING_ORDER_PAGES,
                Duration::from_secs(5),
                &RequestPacer::new(no_pacing()),
                &RequestPacer::new(no_pacing()),
            ),
        )
        .await
        .expect("known algo failure must not await the hung spread scan");
        assert!(
            unverifiable_reason(result, ForbiddenOrderDomain::Algo)
                .contains("algo endpoint failed")
        );
    }

    #[tokio::test]
    async fn spread_invalid_returns_without_waiting_for_hung_algo_peer() {
        let nonzero = ObserverMock::default();
        nonzero.algo_response(OkxAlgoOrderQuery::ConditionalAndOco, AlgoResponse::Hang);
        nonzero.spread_response(SpreadResponse::Page(OkxSpreadOrderPage {
            orders: vec![spread_order("spread-order-1")],
            next_end_id: None,
        }));
        let result = tokio::time::timeout(
            Duration::from_millis(50),
            scan_forbidden_orders(
                &nonzero,
                OKX_DEFAULT_MAX_PENDING_ORDER_PAGES,
                Duration::from_secs(5),
                &RequestPacer::new(no_pacing()),
                &RequestPacer::new(no_pacing()),
            ),
        )
        .await
        .expect("known spread nonzero must not await the hung algo scan");
        assert_eq!(
            result,
            ForbiddenScanResult::NonZero {
                algo_orders_observed: None,
                spread_orders_observed: Some(1),
            }
        );

        let failed = ObserverMock::default();
        failed.algo_response(OkxAlgoOrderQuery::ConditionalAndOco, AlgoResponse::Hang);
        failed.spread_response(SpreadResponse::Error("spread endpoint failed"));
        let result = tokio::time::timeout(
            Duration::from_millis(50),
            scan_forbidden_orders(
                &failed,
                OKX_DEFAULT_MAX_PENDING_ORDER_PAGES,
                Duration::from_secs(5),
                &RequestPacer::new(no_pacing()),
                &RequestPacer::new(no_pacing()),
            ),
        )
        .await
        .expect("known spread failure must not await the hung algo scan");
        assert!(
            unverifiable_reason(result, ForbiddenOrderDomain::Spread)
                .contains("spread endpoint failed")
        );
    }

    #[test]
    fn policy_enforces_default_shape_hard_cap_and_half_age_start() {
        let policy = ForbiddenSentinelPolicy::new(30_000, 15_000, 60_000, 64, no_pacing())
            .expect("normative policy");
        assert_eq!(policy.max_age, Duration::from_secs(30));
        assert_eq!(policy.scan_interval, Duration::from_secs(15));
        assert_eq!(policy.domain_timeout, Duration::from_secs(60));
        assert_eq!(policy.max_concurrent_scans, 5);
        assert!(ForbiddenSentinelPolicy::new(60_001, 15_000, 60_000, 64, no_pacing()).is_err());
        assert!(ForbiddenSentinelPolicy::new(30_000, 15_001, 60_000, 64, no_pacing()).is_err());
        assert!(ForbiddenSentinelPolicy::new(30_000, 15_000, 60_001, 64, no_pacing()).is_err());
        assert!(ForbiddenSentinelPolicy::new(2, 1, 60_000, 64, no_pacing()).is_err());
    }

    #[tokio::test]
    async fn task_requires_initial_zero_and_expires_while_overlapping_scans_hang() {
        let observer = Arc::new(ObserverMock::default());
        observer.spread_response(SpreadResponse::Page(OkxSpreadOrderPage {
            orders: Vec::new(),
            next_end_id: None,
        }));
        for _ in 0..8 {
            observer.spread_response(SpreadResponse::Hang);
        }
        let (tx, mut rx) = mpsc::channel(8);
        let task = tokio::spawn(run_forbidden_order_sentinel(
            "main".to_string(),
            observer,
            ForbiddenSentinelPolicy::new(12, 4, 20, 4, no_pacing()).unwrap(),
            tx,
        ));

        let first = tokio::time::timeout(Duration::from_millis(50), rx.recv())
            .await
            .unwrap()
            .unwrap();
        assert!(first.state.is_verified_zero());
        let expired = tokio::time::timeout(Duration::from_millis(50), rx.recv())
            .await
            .unwrap()
            .unwrap();
        assert!(matches!(expired.state, ForbiddenOrderState::Expired { .. }));
        let unverifiable = tokio::time::timeout(Duration::from_millis(50), rx.recv())
            .await
            .unwrap()
            .unwrap();
        assert!(matches!(
            unverifiable.state,
            ForbiddenOrderState::Unverifiable { .. }
        ));

        task.abort();
        let _ = task.await;
    }

    async fn settle_spawned_tasks() {
        for _ in 0..4 {
            tokio::task::yield_now().await;
        }
    }

    fn call_count(observer: &ObserverMock, expected: &str) -> usize {
        observer
            .calls
            .lock()
            .unwrap()
            .iter()
            .filter(|call| call.as_str() == expected)
            .count()
    }

    #[tokio::test(start_paused = true)]
    async fn hung_scans_do_not_delay_half_age_scan_starts() {
        let observer = Arc::new(ObserverMock::default());
        for _ in 0..8 {
            observer.algo_response(OkxAlgoOrderQuery::ConditionalAndOco, AlgoResponse::Hang);
            observer.spread_response(SpreadResponse::Hang);
        }
        let (tx, _rx) = mpsc::channel(8);
        let task = tokio::spawn(run_forbidden_order_sentinel(
            "main".to_string(),
            Arc::clone(&observer) as Arc<dyn ForbiddenOrderObserverPort>,
            ForbiddenSentinelPolicy::new(40, 10, 100, 4, no_pacing()).unwrap(),
            tx,
        ));

        settle_spawned_tasks().await;
        for expected_starts in 1..=5 {
            assert_eq!(
                call_count(&observer, "algo:conditional,oco:None"),
                expected_starts
            );
            assert_eq!(call_count(&observer, "spread:None"), expected_starts);
            if expected_starts < 5 {
                tokio::time::advance(Duration::from_millis(10)).await;
                settle_spawned_tasks().await;
            }
        }

        task.abort();
        let _ = task.await;
    }

    #[tokio::test(start_paused = true)]
    async fn full_output_channel_does_not_delay_scan_starts() {
        let observer = Arc::new(ObserverMock::default());
        let (tx, mut rx) = mpsc::channel(1);
        tx.try_send(ForbiddenOrderEvent {
            account_id: "occupied".to_string(),
            observed_at_ms: 0,
            state: ForbiddenOrderState::Expired {
                last_verified_at_ms: 0,
                max_age_ms: 1,
            },
        })
        .unwrap();
        let task = tokio::spawn(run_forbidden_order_sentinel(
            "main".to_string(),
            Arc::clone(&observer) as Arc<dyn ForbiddenOrderObserverPort>,
            ForbiddenSentinelPolicy::new(40, 10, 100, 4, no_pacing()).unwrap(),
            tx,
        ));

        settle_spawned_tasks().await;
        for expected_starts in 1..=5 {
            assert_eq!(call_count(&observer, "spread:None"), expected_starts);
            if expected_starts < 5 {
                tokio::time::advance(Duration::from_millis(10)).await;
                settle_spawned_tasks().await;
            }
        }
        assert_eq!(rx.try_recv().unwrap().account_id, "occupied");
        assert!(rx.try_recv().is_err());

        task.abort();
        let _ = task.await;
    }

    #[test]
    fn out_of_order_zero_cannot_rearm_after_a_newer_failure() {
        let mut order = ForbiddenScanOrder::default();
        assert!(matches!(
            order.accept(
                1,
                2,
                ForbiddenScanResult::Unverifiable {
                    domain: ForbiddenOrderDomain::Spread,
                    reason: "newer scan failed".to_string(),
                },
            ),
            Some(ForbiddenScanResult::Unverifiable { .. })
        ));
        assert_eq!(order.accept(0, 2, ForbiddenScanResult::VerifiedZero), None);
        assert_eq!(order.accept(1, 2, ForbiddenScanResult::VerifiedZero), None);
        assert_eq!(
            order.accept(2, 3, ForbiddenScanResult::VerifiedZero),
            Some(ForbiddenScanResult::VerifiedZero)
        );
    }

    #[test]
    fn older_failure_invalidates_a_newer_zero_until_a_post_failure_scan() {
        let mut order = ForbiddenScanOrder::default();
        assert_eq!(
            order.accept(3, 4, ForbiddenScanResult::VerifiedZero),
            Some(ForbiddenScanResult::VerifiedZero)
        );
        assert!(matches!(
            order.accept(
                1,
                4,
                ForbiddenScanResult::Unverifiable {
                    domain: ForbiddenOrderDomain::Algo,
                    reason: "older scan completed late".to_string(),
                },
            ),
            Some(ForbiddenScanResult::Unverifiable { .. })
        ));
        assert_eq!(order.accept(3, 4, ForbiddenScanResult::VerifiedZero), None);
        assert_eq!(
            order.accept(4, 5, ForbiddenScanResult::VerifiedZero),
            Some(ForbiddenScanResult::VerifiedZero)
        );
    }

    #[test]
    fn outbox_delivers_failure_before_a_later_recovery_and_drops_stale_zero() {
        let zero = ForbiddenOrderEvent {
            account_id: "main".to_string(),
            observed_at_ms: 1,
            state: ForbiddenOrderState::VerifiedZero { expires_at_ms: 31 },
        };
        let failure = ForbiddenOrderEvent {
            account_id: "main".to_string(),
            observed_at_ms: 2,
            state: ForbiddenOrderState::Unverifiable {
                domain: ForbiddenOrderDomain::Algo,
                reason: "timeout".to_string(),
            },
        };
        let recovery = ForbiddenOrderEvent {
            account_id: "main".to_string(),
            observed_at_ms: 3,
            state: ForbiddenOrderState::VerifiedZero { expires_at_ms: 33 },
        };
        let mut outbox = ForbiddenEventOutbox::default();
        outbox.push(zero);
        outbox.push(failure.clone());
        outbox.push(recovery.clone());
        assert_eq!(outbox.pop_front(), Some(failure));
        assert_eq!(outbox.pop_front(), Some(recovery));
        assert!(outbox.is_empty());
    }

    #[test]
    fn typed_states_have_deterministic_alert_codes_and_operator_recovery_text() {
        let state = ForbiddenOrderState::NonZero {
            algo_orders_observed: Some(2),
            spread_orders_observed: None,
        };
        assert_eq!(state.alert_code(), Some("forbidden_orders_nonzero"));
        assert_eq!(
            state.failure_reason().unwrap(),
            "forbidden pending orders are nonzero (algo_observed=2, spread_observed=unobserved)"
        );
        assert!(
            ForbiddenOrderState::VerifiedZero { expires_at_ms: 30 }
                .failure_reason()
                .is_none()
        );

        let mut delayed = ForbiddenOrderEvent {
            account_id: "main".to_string(),
            observed_at_ms: 100,
            state: ForbiddenOrderState::VerifiedZero { expires_at_ms: 130 },
        };
        delayed.expire_delayed_zero_proof(130);
        assert_eq!(
            delayed,
            ForbiddenOrderEvent {
                account_id: "main".to_string(),
                observed_at_ms: 130,
                state: ForbiddenOrderState::Expired {
                    last_verified_at_ms: 100,
                    max_age_ms: 30,
                },
            }
        );
    }
}
