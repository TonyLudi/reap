//! Concrete production read roles for the continuously supervised PM actor.
//!
//! This module is deliberately strategy-free. It joins the already reviewed
//! credential-bound user stream, complete authenticated condition-scoped
//! order/trade pagination, and one fixed Data API source per outcome token.
//! Only confirmed trade legs become irreversible fill-derived inventory.

use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
    str::FromStr as _,
};

use async_trait::async_trait;
use reap_pm_core::{EvmAddress, PmBookQuantity, PmConditionId, PmTokenId, U256};
use reap_polymarket_live_adapter::{
    PmAuthenticatedHttpOwner, PmAuthenticatedUserWsRole, PmOpenOrdersCutProgress,
    PmReadServerTimeHttpRole, PmTradesCutProgress, PmUserWsEvent, PmUserWsEventSink,
    PmUserWsShutdownHandle, pm_user_ws_shutdown_channel,
};
use reap_polymarket_public_source::{PmConfiguredTokenPosition, PmDataApiCurrentPositionSource};
use reap_polymarket_wire::{
    PmLiveOpenOrderPage, PmLiveOrder, PmLiveTrade, PmLiveTradePage, PmLiveUserEvent,
    PmLiveUserOrder,
};
use thiserror::Error;
use tokio::{sync::mpsc, task::JoinHandle};

use crate::production_supervisor::{
    MAX_PM_SUPERVISOR_TOKENS, PmProductionSupervisorRoles, PmSupervisorEdgeError, PmSupervisorFill,
    PmSupervisorFixedHeartbeatRole, PmSupervisorOpenOrder, PmSupervisorOrderStatus,
    PmSupervisorPollCut, PmSupervisorPollRole, PmSupervisorPosition,
    PmSupervisorProductionMutationRole, PmSupervisorScope, PmSupervisorWsEvent, PmSupervisorWsRole,
};

const PRODUCTION_WS_EVENT_CAPACITY: usize = 1_024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum PmSupervisorProductionReadError {
    #[error("production supervisor read configuration is invalid")]
    InvalidConfiguration,
}

#[derive(Debug, Clone)]
struct ProductionReadScope {
    condition: PmConditionId,
    tokens: BTreeSet<PmTokenId>,
    expected_signer: EvmAddress,
    expected_maker: EvmAddress,
}

impl ProductionReadScope {
    fn new(
        scope: &PmSupervisorScope,
        expected_signer: EvmAddress,
        expected_maker: EvmAddress,
    ) -> Result<Self, PmSupervisorProductionReadError> {
        let condition = PmConditionId::parse(scope.condition_id())
            .map_err(|_| PmSupervisorProductionReadError::InvalidConfiguration)?;
        if expected_signer.bytes() == [0; 20]
            || expected_maker.bytes() == [0; 20]
            || scope.token_ids().is_empty()
            || scope.token_ids().len() > MAX_PM_SUPERVISOR_TOKENS
        {
            return Err(PmSupervisorProductionReadError::InvalidConfiguration);
        }
        let tokens = scope
            .token_ids()
            .iter()
            .map(|token| {
                U256::from_str(token)
                    .ok()
                    .filter(|units| units.to_string() == *token)
                    .and_then(|units| PmTokenId::new(units).ok())
                    .ok_or(PmSupervisorProductionReadError::InvalidConfiguration)
            })
            .collect::<Result<BTreeSet<_>, _>>()?;
        if tokens.len() != scope.token_ids().len() {
            return Err(PmSupervisorProductionReadError::InvalidConfiguration);
        }
        Ok(Self {
            condition,
            tokens,
            expected_signer,
            expected_maker,
        })
    }

    fn contains_token(&self, token: PmTokenId) -> bool {
        self.tokens.contains(&token)
    }

    fn token_string(token: PmTokenId) -> String {
        token.units().to_string()
    }
}

/// Concrete production order/trade/position polling role.
///
/// Every call finishes all cursor pages for the configured condition and one
/// production-origin position walk for every configured outcome token before
/// releasing a cut. A partial component never advances its sequence.
pub struct PmSupervisorProductionPollRole {
    scope: ProductionReadScope,
    server_time: PmReadServerTimeHttpRole,
    authenticated_http: PmAuthenticatedHttpOwner,
    positions: BTreeMap<PmTokenId, PmDataApiCurrentPositionSource>,
    next_sequence: u64,
}

impl PmSupervisorProductionPollRole {
    fn from_scope(
        scope: ProductionReadScope,
        server_time: PmReadServerTimeHttpRole,
        authenticated_http: PmAuthenticatedHttpOwner,
        positions: impl IntoIterator<Item = (PmTokenId, PmDataApiCurrentPositionSource)>,
    ) -> Result<Self, PmSupervisorProductionReadError> {
        let configured_http_scope = authenticated_http.configured_scope();
        if !server_time.is_production()
            || !authenticated_http.is_production()
            || configured_http_scope.condition() != scope.condition
            || !scope.contains_token(configured_http_scope.token())
            || authenticated_http.configured_l2_signer() != scope.expected_signer
            || authenticated_http.configured_expected_maker() != scope.expected_maker
        {
            return Err(PmSupervisorProductionReadError::InvalidConfiguration);
        }
        let mut position_sources = BTreeMap::new();
        for (token, source) in positions {
            let source_scope = source.configured_scope();
            if !scope.contains_token(token)
                || !source.is_production()
                || source_scope.condition() != scope.condition
                || source_scope.configured_token() != token
                || source_scope.proxy_funder() != scope.expected_maker
                || position_sources.insert(token, source).is_some()
            {
                return Err(PmSupervisorProductionReadError::InvalidConfiguration);
            }
        }
        if position_sources.len() != scope.tokens.len() {
            return Err(PmSupervisorProductionReadError::InvalidConfiguration);
        }
        Ok(Self {
            scope,
            server_time,
            authenticated_http,
            positions: position_sources,
            next_sequence: 1,
        })
    }

    async fn complete_open_orders(
        &mut self,
    ) -> Result<Box<[PmLiveOpenOrderPage]>, PmSupervisorEdgeError> {
        let time = self
            .server_time
            .fresh_read_server_time()
            .await
            .map_err(|_| PmSupervisorEdgeError::Unavailable)?;
        let mut progress = self
            .authenticated_http
            .reconciliation()
            .begin_condition_open_orders(time)
            .await
            .map_err(|_| PmSupervisorEdgeError::Unavailable)?;
        loop {
            match progress {
                PmOpenOrdersCutProgress::Complete(cut) => {
                    return Ok(cut.into_pages());
                }
                PmOpenOrdersCutProgress::Incomplete(assembly) => {
                    let time = self
                        .server_time
                        .fresh_read_server_time()
                        .await
                        .map_err(|_| PmSupervisorEdgeError::Unavailable)?;
                    progress = self
                        .authenticated_http
                        .reconciliation()
                        .continue_open_orders(time, assembly)
                        .await
                        .map_err(|_| PmSupervisorEdgeError::Unavailable)?;
                }
            }
        }
    }

    async fn complete_trades(&mut self) -> Result<Box<[PmLiveTradePage]>, PmSupervisorEdgeError> {
        let time = self
            .server_time
            .fresh_read_server_time()
            .await
            .map_err(|_| PmSupervisorEdgeError::Unavailable)?;
        let mut progress = self
            .authenticated_http
            .reconciliation()
            .begin_condition_trades(time)
            .await
            .map_err(|_| PmSupervisorEdgeError::Unavailable)?;
        loop {
            match progress {
                PmTradesCutProgress::Complete(cut) => {
                    return Ok(cut.into_pages());
                }
                PmTradesCutProgress::Incomplete(assembly) => {
                    let time = self
                        .server_time
                        .fresh_read_server_time()
                        .await
                        .map_err(|_| PmSupervisorEdgeError::Unavailable)?;
                    progress = self
                        .authenticated_http
                        .reconciliation()
                        .continue_trades(time, assembly)
                        .await
                        .map_err(|_| PmSupervisorEdgeError::Unavailable)?;
                }
            }
        }
    }

    async fn complete_positions(
        &mut self,
    ) -> Result<Box<[PmSupervisorPosition]>, PmSupervisorEdgeError> {
        let Self {
            scope,
            positions: sources,
            ..
        } = self;
        let mut positions = Vec::with_capacity(sources.len());
        for (token, source) in sources {
            let observation = source
                .production_observe_configured_token()
                .await
                .map_err(|_| PmSupervisorEdgeError::Unavailable)?;
            let observed_scope = observation.scope();
            if observed_scope.condition() != scope.condition
                || observed_scope.configured_token() != *token
                || observed_scope.proxy_funder() != scope.expected_maker
            {
                return Err(PmSupervisorEdgeError::InvalidObservation);
            }
            let quantity = match observation.configured_token() {
                PmConfiguredTokenPosition::Absent => U256::ZERO,
                PmConfiguredTokenPosition::Present(position) => {
                    if position.asset() != *token {
                        return Err(PmSupervisorEdgeError::InvalidObservation);
                    }
                    position
                        .size_protocol_units()
                        .map_err(|_| PmSupervisorEdgeError::InvalidObservation)?
                }
            };
            positions.push(PmSupervisorPosition {
                token_id: ProductionReadScope::token_string(*token),
                quantity,
            });
        }
        Ok(positions.into_boxed_slice())
    }
}

impl fmt::Debug for PmSupervisorProductionPollRole {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PmSupervisorProductionPollRole")
            .field("condition", &self.scope.condition)
            .field("tokens", &self.scope.tokens)
            .field("authenticated_http", &"[REDACTED]")
            .finish_non_exhaustive()
    }
}

#[async_trait]
impl PmSupervisorPollRole for PmSupervisorProductionPollRole {
    async fn complete_poll(&mut self) -> Result<PmSupervisorPollCut, PmSupervisorEdgeError> {
        let trade_pages = self.complete_trades().await?;
        let open_pages = self.complete_open_orders().await?;
        let open_orders = normalize_open_order_pages(&self.scope, &open_pages)?;
        let fills = normalize_trade_pages(&self.scope, &trade_pages)?;
        let positions = self.complete_positions().await?;
        let sequence = self.next_sequence;
        self.next_sequence = sequence
            .checked_add(1)
            .ok_or(PmSupervisorEdgeError::InvalidObservation)?;
        Ok(PmSupervisorPollCut {
            sequence,
            open_orders,
            fills,
            positions,
        })
    }
}

/// Concrete stream-like adapter over the fixed-endpoint production
/// authenticated user WebSocket. The socket is started lazily by the
/// supervisor task, after journal recovery. Its owned task contains no raw
/// credential; dropping the role requests stream shutdown and aborts the
/// bounded adapter task.
pub struct PmSupervisorProductionWsRole {
    pending: Option<(ProductionReadScope, PmAuthenticatedUserWsRole)>,
    receiver: Option<mpsc::Receiver<Result<PmSupervisorWsEvent, PmSupervisorEdgeError>>>,
    shutdown: Option<PmUserWsShutdownHandle>,
    task: Option<JoinHandle<()>>,
}

impl PmSupervisorProductionWsRole {
    fn from_scope(
        scope: ProductionReadScope,
        role: PmAuthenticatedUserWsRole,
    ) -> Result<Self, PmSupervisorProductionReadError> {
        if !role.is_production()
            || role.condition() != scope.condition
            || role.configured_l2_signer() != scope.expected_signer
            || role.configured_expected_maker() != scope.expected_maker
        {
            return Err(PmSupervisorProductionReadError::InvalidConfiguration);
        }
        Ok(Self {
            pending: Some((scope, role)),
            receiver: None,
            shutdown: None,
            task: None,
        })
    }

    fn start(&mut self) -> Result<(), PmSupervisorEdgeError> {
        if self.receiver.is_some() {
            return Ok(());
        }
        let (scope, role) = self
            .pending
            .take()
            .ok_or(PmSupervisorEdgeError::Unavailable)?;
        let (shutdown, signal) = pm_user_ws_shutdown_channel();
        let (sender, receiver) = mpsc::channel(PRODUCTION_WS_EVENT_CAPACITY);
        let task = tokio::runtime::Handle::try_current()
            .map_err(|_| PmSupervisorEdgeError::Unavailable)?
            .spawn(async move {
                let terminal = sender.clone();
                let mut sink = ProductionWsSink { scope, sender };
                if role.run(signal, &mut sink).await.is_err() {
                    let _ = terminal.send(Err(PmSupervisorEdgeError::Unavailable)).await;
                }
            });
        self.receiver = Some(receiver);
        self.shutdown = Some(shutdown);
        self.task = Some(task);
        Ok(())
    }
}

impl fmt::Debug for PmSupervisorProductionWsRole {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PmSupervisorProductionWsRole(<fixed authenticated stream>)")
    }
}

impl Drop for PmSupervisorProductionWsRole {
    fn drop(&mut self) {
        if let Some(shutdown) = &self.shutdown {
            shutdown.request_shutdown();
        }
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

#[async_trait]
impl PmSupervisorWsRole for PmSupervisorProductionWsRole {
    async fn next_event(&mut self) -> Result<PmSupervisorWsEvent, PmSupervisorEdgeError> {
        self.start()?;
        self.receiver
            .as_mut()
            .ok_or(PmSupervisorEdgeError::Unavailable)?
            .recv()
            .await
            .unwrap_or(Err(PmSupervisorEdgeError::Unavailable))
    }

    async fn shutdown(&mut self) -> Result<(), PmSupervisorEdgeError> {
        self.pending.take();
        let Some(shutdown) = &self.shutdown else {
            return Ok(());
        };
        shutdown.request_shutdown();
        let Some(mut task) = self.task.take() else {
            return Ok(());
        };
        loop {
            tokio::select! {
                joined = &mut task => {
                    return joined.map_err(|_| PmSupervisorEdgeError::Unavailable);
                }
                event = self.receiver.as_mut()
                    .ok_or(PmSupervisorEdgeError::Unavailable)?
                    .recv() => {
                    if event.is_none() && !task.is_finished() {
                        return Err(PmSupervisorEdgeError::Unavailable);
                    }
                }
            }
        }
    }
}

struct ProductionWsSink {
    scope: ProductionReadScope,
    sender: mpsc::Sender<Result<PmSupervisorWsEvent, PmSupervisorEdgeError>>,
}

impl ProductionWsSink {
    async fn send(&self, event: PmSupervisorWsEvent) -> Result<(), PmSupervisorEdgeError> {
        self.sender
            .send(Ok(event))
            .await
            .map_err(|_| PmSupervisorEdgeError::Unavailable)
    }
}

#[async_trait]
impl PmUserWsEventSink for ProductionWsSink {
    type Error = PmSupervisorEdgeError;

    async fn deliver_user_ws_event(&mut self, event: PmUserWsEvent) -> Result<(), Self::Error> {
        match event {
            PmUserWsEvent::SubscriptionSent(_) => self.send(PmSupervisorWsEvent::Connected).await,
            PmUserWsEvent::ConnectionRetired(_)
            | PmUserWsEvent::RetryExhausted(_)
            | PmUserWsEvent::Shutdown(_) => self.send(PmSupervisorWsEvent::Disconnected).await,
            PmUserWsEvent::BoundFrame(frame) => {
                for event in frame.events() {
                    match event {
                        PmLiveUserEvent::Order(order) => {
                            self.send(PmSupervisorWsEvent::Order(normalize_user_order(
                                &self.scope,
                                order,
                            )?))
                            .await?;
                        }
                        PmLiveUserEvent::Trade(trade) => match normalize_trade(&self.scope, trade)?
                        {
                            TradeDisposition::Confirmed(fills) => {
                                for fill in fills {
                                    self.send(PmSupervisorWsEvent::Fill(fill)).await?;
                                }
                            }
                            TradeDisposition::RequiresReconciliation => {
                                self.send(PmSupervisorWsEvent::ReconciliationRequired)
                                    .await?;
                            }
                            TradeDisposition::PendingOrFailed => {}
                        },
                    }
                }
                Ok(())
            }
            PmUserWsEvent::ConnectionOpened(_)
            | PmUserWsEvent::PingSent(_)
            | PmUserWsEvent::Pong(_)
            | PmUserWsEvent::ReconnectScheduled(_) => Ok(()),
        }
    }
}

/// One construction boundary for the complete strategy-neutral private read
/// side used by [`crate::PmProductionSupervisorRoles`].
pub struct PmProductionExecutionReadInfrastructure {
    pub poll: PmSupervisorProductionPollRole,
    pub user_ws: PmSupervisorProductionWsRole,
}

pub type PmConcreteProductionSupervisorRoles<H, M> =
    PmProductionSupervisorRoles<H, PmSupervisorProductionPollRole, PmSupervisorProductionWsRole, M>;

impl PmProductionExecutionReadInfrastructure {
    pub fn new(
        supervisor_scope: &PmSupervisorScope,
        expected_maker: EvmAddress,
        server_time: PmReadServerTimeHttpRole,
        authenticated_http: PmAuthenticatedHttpOwner,
        authenticated_user_ws: PmAuthenticatedUserWsRole,
        positions: impl IntoIterator<Item = (PmTokenId, PmDataApiCurrentPositionSource)>,
    ) -> Result<Self, PmSupervisorProductionReadError> {
        let expected_signer = authenticated_http.configured_l2_signer();
        let scope = ProductionReadScope::new(supervisor_scope, expected_signer, expected_maker)?;
        let poll = PmSupervisorProductionPollRole::from_scope(
            scope.clone(),
            server_time,
            authenticated_http,
            positions,
        )?;
        let user_ws = PmSupervisorProductionWsRole::from_scope(scope, authenticated_user_ws)?;
        Ok(Self { poll, user_ws })
    }

    #[must_use]
    pub fn into_supervisor_roles<H, M>(
        self,
        heartbeat: H,
        mutation: M,
    ) -> PmConcreteProductionSupervisorRoles<H, M> {
        PmProductionSupervisorRoles {
            heartbeat,
            poll: self.poll,
            user_ws: self.user_ws,
            mutation,
        }
    }

    /// Strict production composition for the fixed heartbeat and the
    /// condition's complete token mutation router.
    pub fn into_production_supervisor_roles(
        self,
        supervisor_scope: &PmSupervisorScope,
        heartbeat: PmSupervisorFixedHeartbeatRole,
        mutation: PmSupervisorProductionMutationRole,
    ) -> Result<
        PmConcreteProductionSupervisorRoles<
            PmSupervisorFixedHeartbeatRole,
            PmSupervisorProductionMutationRole,
        >,
        PmSupervisorProductionReadError,
    > {
        let expected_scope = ProductionReadScope::new(
            supervisor_scope,
            self.poll.scope.expected_signer,
            self.poll.scope.expected_maker,
        )?;
        if expected_scope.condition != self.poll.scope.condition
            || expected_scope.tokens != self.poll.scope.tokens
            || heartbeat.configured_l2_signer() != self.poll.scope.expected_signer
            || !mutation.matches_supervisor_scope(
                supervisor_scope,
                self.poll.scope.expected_signer,
                self.poll.scope.expected_maker,
            )
        {
            return Err(PmSupervisorProductionReadError::InvalidConfiguration);
        }
        Ok(self.into_supervisor_roles(heartbeat, mutation))
    }
}

impl fmt::Debug for PmProductionExecutionReadInfrastructure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PmProductionExecutionReadInfrastructure(<complete fixed reads>)")
    }
}

fn normalize_open_order_pages(
    scope: &ProductionReadScope,
    pages: &[PmLiveOpenOrderPage],
) -> Result<Box<[PmSupervisorOpenOrder]>, PmSupervisorEdgeError> {
    if pages.is_empty()
        || !pages.last().is_some_and(PmLiveOpenOrderPage::terminal)
        || pages[..pages.len() - 1]
            .iter()
            .any(PmLiveOpenOrderPage::terminal)
    {
        return Err(PmSupervisorEdgeError::InvalidObservation);
    }
    let mut orders = BTreeMap::new();
    for order in pages.iter().flat_map(|page| page.orders()) {
        let normalized = normalize_rest_order(scope, order)?;
        match orders.get(&normalized.venue_order_id) {
            Some(previous) if previous != &normalized => {
                return Err(PmSupervisorEdgeError::InvalidObservation);
            }
            Some(_) => {}
            None => {
                orders.insert(normalized.venue_order_id.clone(), normalized);
            }
        }
    }
    Ok(orders.into_values().collect::<Vec<_>>().into_boxed_slice())
}

fn normalize_trade_pages(
    scope: &ProductionReadScope,
    pages: &[PmLiveTradePage],
) -> Result<Box<[PmSupervisorFill]>, PmSupervisorEdgeError> {
    if pages.is_empty()
        || !pages.last().is_some_and(PmLiveTradePage::terminal)
        || pages[..pages.len() - 1]
            .iter()
            .any(PmLiveTradePage::terminal)
    {
        return Err(PmSupervisorEdgeError::InvalidObservation);
    }
    let mut fills = BTreeMap::new();
    for trade in pages.iter().flat_map(|page| page.trades()) {
        let TradeDisposition::Confirmed(confirmed) = normalize_trade(scope, trade)? else {
            continue;
        };
        for fill in confirmed {
            let key = (fill.fill_id.clone(), fill.venue_order_id.clone());
            match fills.get(&key) {
                Some(previous) if previous != &fill => {
                    return Err(PmSupervisorEdgeError::InvalidObservation);
                }
                Some(_) => {}
                None => {
                    fills.insert(key, fill);
                }
            }
        }
    }
    Ok(fills.into_values().collect::<Vec<_>>().into_boxed_slice())
}

fn normalize_rest_order(
    scope: &ProductionReadScope,
    order: &PmLiveOrder,
) -> Result<PmSupervisorOpenOrder, PmSupervisorEdgeError> {
    if order.condition() != scope.condition
        || !scope.contains_token(order.token())
        || order.maker() != scope.expected_maker
    {
        return Err(PmSupervisorEdgeError::InvalidObservation);
    }
    normalize_order_facts(
        order.id().as_str(),
        order.token(),
        order.original_size().protocol_units(),
        matched_units(order.size_matched()),
        None,
        Some(order.status()),
    )
}

fn normalize_user_order(
    scope: &ProductionReadScope,
    order: &PmLiveUserOrder,
) -> Result<PmSupervisorOpenOrder, PmSupervisorEdgeError> {
    if order.condition() != scope.condition
        || !scope.contains_token(order.token())
        || order
            .maker()
            .is_some_and(|maker| maker != scope.expected_maker)
    {
        return Err(PmSupervisorEdgeError::InvalidObservation);
    }
    normalize_order_facts(
        order.id().as_str(),
        order.token(),
        order.original_size().protocol_units(),
        matched_units(order.size_matched()),
        Some(order.event_kind()),
        order.status(),
    )
}

fn normalize_order_facts(
    venue_order_id: &str,
    token: PmTokenId,
    original: U256,
    matched: U256,
    event_kind: Option<&str>,
    venue_status: Option<&str>,
) -> Result<PmSupervisorOpenOrder, PmSupervisorEdgeError> {
    if matched > original {
        return Err(PmSupervisorEdgeError::InvalidObservation);
    }
    let cancelled = event_kind == Some("CANCELLATION") || venue_status == Some("CANCELED");
    let status = if matched == original {
        PmSupervisorOrderStatus::Filled
    } else if cancelled {
        PmSupervisorOrderStatus::Cancelled
    } else if !matched.is_zero() {
        PmSupervisorOrderStatus::PartiallyFilled
    } else {
        match venue_status {
            None | Some("LIVE" | "UNMATCHED" | "DELAYED") => PmSupervisorOrderStatus::Live,
            Some("MATCHED") => PmSupervisorOrderStatus::PartiallyFilled,
            Some(_) => return Err(PmSupervisorEdgeError::InvalidObservation),
        }
    };
    Ok(PmSupervisorOpenOrder {
        venue_order_id: venue_order_id.to_owned(),
        token_id: ProductionReadScope::token_string(token),
        status,
        cumulative_filled: matched,
    })
}

fn matched_units(quantity: PmBookQuantity) -> U256 {
    match quantity {
        PmBookQuantity::Delete => U256::ZERO,
        PmBookQuantity::Quantity(quantity) => quantity.protocol_units(),
    }
}

enum TradeDisposition {
    Confirmed(Vec<PmSupervisorFill>),
    RequiresReconciliation,
    PendingOrFailed,
}

fn normalize_trade(
    scope: &ProductionReadScope,
    trade: &PmLiveTrade,
) -> Result<TradeDisposition, PmSupervisorEdgeError> {
    if trade.condition() != scope.condition {
        return Err(PmSupervisorEdgeError::InvalidObservation);
    }
    let status = trade
        .status()
        .strip_prefix("TRADE_STATUS_")
        .unwrap_or(trade.status());
    match status {
        "MATCHED_NOT_BROADCASTED" | "MATCHED" | "MINED" | "RETRYING" => {
            return Ok(TradeDisposition::PendingOrFailed);
        }
        "FAILED" => return Ok(TradeDisposition::RequiresReconciliation),
        "CONFIRMED" => {}
        _ => return Err(PmSupervisorEdgeError::InvalidObservation),
    }

    let mut fills = Vec::new();
    match trade.trader_side() {
        Some("TAKER") => {
            if !scope.contains_token(trade.token()) {
                return Err(PmSupervisorEdgeError::InvalidObservation);
            }
            let venue_order = match (trade.order_id(), trade.taker_order_id()) {
                (Some(order), None) | (None, Some(order)) => order,
                (Some(order), Some(taker)) if order == taker => order,
                _ => return Err(PmSupervisorEdgeError::InvalidObservation),
            };
            fills.push(PmSupervisorFill {
                fill_id: trade.id().as_str().to_owned(),
                venue_order_id: venue_order.as_str().to_owned(),
                token_id: ProductionReadScope::token_string(trade.token()),
                side: trade.side(),
                quantity: trade.size().protocol_units(),
            });
        }
        Some("MAKER") => {
            for maker in trade.maker_orders() {
                if maker.maker() != scope.expected_maker {
                    continue;
                }
                if !scope.contains_token(maker.token()) {
                    return Err(PmSupervisorEdgeError::InvalidObservation);
                }
                fills.push(PmSupervisorFill {
                    fill_id: trade.id().as_str().to_owned(),
                    venue_order_id: maker.order_id().as_str().to_owned(),
                    token_id: ProductionReadScope::token_string(maker.token()),
                    side: maker.side(),
                    quantity: maker.matched_amount().protocol_units(),
                });
            }
            if fills.is_empty() {
                return Err(PmSupervisorEdgeError::InvalidObservation);
            }
        }
        _ => return Err(PmSupervisorEdgeError::InvalidObservation),
    }
    Ok(TradeDisposition::Confirmed(fills))
}

#[cfg(test)]
mod tests {
    use reap_polymarket_wire::{PmLiveUserFrame, parse_live_user_frame};

    use super::*;

    const CONDITION: &str = "0x1111111111111111111111111111111111111111111111111111111111111111";
    const MAKER: &str = "0x2222222222222222222222222222222222222222";
    const SIGNER: &str = "0x3333333333333333333333333333333333333333";
    const OWNER: &str = "00000000-0000-4000-8000-000000000001";
    const ORDER_1: &str = "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const ORDER_2: &str = "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

    fn scope() -> ProductionReadScope {
        ProductionReadScope::new(
            &PmSupervisorScope::new(CONDITION, ["101".to_owned(), "102".to_owned()]).unwrap(),
            EvmAddress::parse(SIGNER).unwrap(),
            EvmAddress::parse(MAKER).unwrap(),
        )
        .unwrap()
    }

    fn one_event(raw: String) -> PmLiveUserFrame {
        parse_live_user_frame(raw.as_bytes()).unwrap()
    }

    #[test]
    fn user_order_mapping_preserves_partial_progress_and_terminal_cancel() {
        let partial = one_event(format!(
            r#"{{"event_type":"order","id":"{ORDER_1}","owner":"{OWNER}","market":"{CONDITION}","asset_id":"101","side":"BUY","original_size":"10","size_matched":"2.5","price":"0.42","type":"UPDATE","status":"LIVE","maker_address":"{MAKER}","timestamp":"1782753357257"}}"#,
        ));
        let PmLiveUserEvent::Order(order) = &partial.events()[0] else {
            panic!("order event");
        };
        let normalized = normalize_user_order(&scope(), order).unwrap();
        assert_eq!(normalized.status, PmSupervisorOrderStatus::PartiallyFilled);
        assert_eq!(normalized.cumulative_filled, U256::from_u64(2_500_000));

        let cancelled = one_event(format!(
            r#"{{"event_type":"order","id":"{ORDER_1}","owner":"{OWNER}","market":"{CONDITION}","asset_id":"101","side":"BUY","original_size":"10","size_matched":"2.5","price":"0.42","type":"CANCELLATION","status":"CANCELED","maker_address":"{MAKER}","timestamp":"1782753357258"}}"#,
        ));
        let PmLiveUserEvent::Order(order) = &cancelled.events()[0] else {
            panic!("order event");
        };
        let normalized = normalize_user_order(&scope(), order).unwrap();
        assert_eq!(normalized.status, PmSupervisorOrderStatus::Cancelled);
        assert_eq!(normalized.cumulative_filled, U256::from_u64(2_500_000));
    }

    #[test]
    fn confirmed_maker_trade_emits_every_owned_leg_with_composite_identity() {
        let frame = one_event(format!(
            r#"{{"event_type":"trade","type":"TRADE","id":"trade-1","owner":"{OWNER}","trade_owner":"{OWNER}","market":"{CONDITION}","asset_id":"101","side":"SELL","size":"3","price":"0.42","status":"CONFIRMED","maker_orders":[{{"order_id":"{ORDER_1}","owner":"{OWNER}","maker_address":"{MAKER}","matched_amount":"1","price":"0.42","asset_id":"101","side":"SELL"}},{{"order_id":"{ORDER_2}","owner":"{OWNER}","maker_address":"{MAKER}","matched_amount":"2","price":"0.42","asset_id":"101","side":"SELL"}}],"maker_address":"{MAKER}","timestamp":"1782753357257","matchtime":"1782753357257","last_update":"1782753357258","transaction_hash":"","trader_side":"MAKER"}}"#,
        ));
        let PmLiveUserEvent::Trade(trade) = &frame.events()[0] else {
            panic!("trade event");
        };
        let TradeDisposition::Confirmed(fills) = normalize_trade(&scope(), trade).unwrap() else {
            panic!("confirmed trade");
        };
        assert_eq!(fills.len(), 2);
        assert_eq!(fills[0].fill_id, fills[1].fill_id);
        assert_ne!(fills[0].venue_order_id, fills[1].venue_order_id);
        assert_eq!(fills[0].quantity, U256::from_u64(1_000_000));
        assert_eq!(fills[1].quantity, U256::from_u64(2_000_000));
    }

    #[test]
    fn nonfinal_trade_never_becomes_fill_and_failed_requires_reconciliation() {
        for (status, expects_reconciliation) in
            [("MATCHED", false), ("MINED", false), ("FAILED", true)]
        {
            let frame = one_event(format!(
                r#"{{"event_type":"trade","type":"TRADE","id":"trade-{status}","owner":"{OWNER}","trade_owner":"{OWNER}","market":"{CONDITION}","asset_id":"101","side":"BUY","size":"1","price":"0.42","status":"{status}","taker_order_id":"{ORDER_1}","maker_orders":[],"maker_address":"{MAKER}","timestamp":"1782753357257","matchtime":"1782753357257","last_update":"1782753357258","transaction_hash":"","trader_side":"TAKER"}}"#,
            ));
            let PmLiveUserEvent::Trade(trade) = &frame.events()[0] else {
                panic!("trade event");
            };
            assert_eq!(
                matches!(
                    normalize_trade(&scope(), trade).unwrap(),
                    TradeDisposition::RequiresReconciliation
                ),
                expects_reconciliation
            );
        }
    }

    #[test]
    fn prefixed_confirmed_taker_trade_accepts_equal_direct_order_references() {
        let frame = one_event(format!(
            r#"{{"event_type":"trade","type":"TRADE","id":"trade-confirmed","owner":"{OWNER}","trade_owner":"{OWNER}","market":"{CONDITION}","asset_id":"101","side":"BUY","size":"1","price":"0.42","status":"TRADE_STATUS_CONFIRMED","order_id":"{ORDER_1}","taker_order_id":"{ORDER_1}","maker_orders":[],"maker_address":"{MAKER}","timestamp":"1782753357257","matchtime":"1782753357257","last_update":"1782753357258","transaction_hash":"","trader_side":"TAKER"}}"#,
        ));
        let PmLiveUserEvent::Trade(trade) = &frame.events()[0] else {
            panic!("trade event");
        };
        let TradeDisposition::Confirmed(fills) = normalize_trade(&scope(), trade).unwrap() else {
            panic!("confirmed trade");
        };
        assert_eq!(fills.len(), 1);
        assert_eq!(fills[0].venue_order_id, ORDER_1);
        assert_eq!(fills[0].quantity, U256::from_u64(1_000_000));
    }

    #[test]
    fn read_scope_requires_canonical_condition_tokens_and_nonzero_maker() {
        let invalid_condition = PmSupervisorScope::new("condition", ["101".to_owned()]).unwrap();
        assert_eq!(
            ProductionReadScope::new(
                &invalid_condition,
                EvmAddress::parse(SIGNER).unwrap(),
                EvmAddress::parse(MAKER).unwrap(),
            )
            .unwrap_err(),
            PmSupervisorProductionReadError::InvalidConfiguration
        );
        let invalid_token = PmSupervisorScope::new(CONDITION, ["01".to_owned()]).unwrap();
        assert_eq!(
            ProductionReadScope::new(
                &invalid_token,
                EvmAddress::parse(SIGNER).unwrap(),
                EvmAddress::parse(MAKER).unwrap(),
            )
            .unwrap_err(),
            PmSupervisorProductionReadError::InvalidConfiguration
        );
    }
}
