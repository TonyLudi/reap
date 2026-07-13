mod execution;
mod matching;
mod portfolio;
mod replay;

pub use execution::{BacktestConfig, BacktestExecutionConfig, BacktestTimeBasis};
pub use matching::MatchingEngine;
pub use replay::{
    ReplayRow, TimedReplayEvent, load_events_from_path, load_normalized_jsonl,
    load_normalized_jsonl_from_path, replay_raw_capture, replay_raw_capture_path,
    replay_raw_capture_timed, replay_raw_capture_timed_path,
};

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::path::Path;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use crate::portfolio::Portfolio;
use reap_core::{
    AccountUpdate, FillLiquidity, MarketEvent, NormalizedEvent, OrderEvent, OrderIntent,
    OrderUpdate, Position, StrategyEvent, Symbol,
};
use reap_strategy::{ChaosConfig, ChaosStrategy, Strategy};

const MAX_ACTIONS_PER_DRAIN: usize = 1_000_000;
const NS_PER_MS: u64 = 1_000_000;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BacktestReport {
    pub execution: BacktestExecutionConfig,
    pub time_basis: BacktestTimeBasis,
    pub input_events: u64,
    pub first_arrival_ns: Option<u64>,
    pub last_arrival_ns: Option<u64>,
    pub input_clock_regressions: u64,
    pub max_input_clock_regression_ns: u64,
    pub orders_sent: usize,
    pub cancel_requests: usize,
    pub deduplicated_cancel_requests: usize,
    pub ignored_cancel_requests: usize,
    pub exchange_activations: usize,
    pub cancelled_orders: usize,
    pub rejected_orders: usize,
    pub fills: usize,
    pub maker_fills: usize,
    pub taker_fills: usize,
    pub pending_scheduled_actions: usize,
    pub pending_activation_actions: usize,
    pub pending_cancel_actions: usize,
    pub pending_order_update_actions: usize,
    pub pending_strategy_event_actions: usize,
    pub pending_orders: usize,
    pub live_orders: usize,
    pub pending_cancel_requests: usize,
    pub final_delta_usd: f64,
    pub final_pending_delta_usd: f64,
    pub final_equity_usd: f64,
    pub cash_usd: f64,
    pub positions: HashMap<Symbol, f64>,
}

#[derive(Debug)]
enum ScheduledAction {
    ActivateOrder { symbol: Symbol, order_id: String },
    CancelOrder { order_id: String, reason: String },
    DeliverOrder(OrderUpdate),
    DeliverStrategy(StrategyEvent),
}

pub struct BacktestRunner {
    strategy: ChaosStrategy,
    matchers: HashMap<Symbol, MatchingEngine>,
    portfolio: Portfolio,
    execution: BacktestExecutionConfig,
    time_basis: BacktestTimeBasis,
    scheduled: BTreeMap<(u64, u64), ScheduledAction>,
    next_action_seq: u64,
    pending_cancels: HashSet<String>,
    now_ns: u64,
    first_arrival_ns: Option<u64>,
    last_arrival_ns: Option<u64>,
    input_events: u64,
    input_clock_regressions: u64,
    max_input_clock_regression_ns: u64,
    orders_sent: usize,
    cancel_requests: usize,
    deduplicated_cancel_requests: usize,
    ignored_cancel_requests: usize,
    exchange_activations: usize,
    cancelled_orders: usize,
    rejected_orders: usize,
    fills: usize,
    maker_fills: usize,
    taker_fills: usize,
}

impl BacktestRunner {
    pub fn new(config: ChaosConfig) -> Result<Self> {
        Self::with_execution_config(config, BacktestExecutionConfig::default())
    }

    pub fn from_config(config: BacktestConfig) -> Result<Self> {
        Self::with_execution_config(config.strategy, config.backtest)
    }

    pub fn with_execution_config(
        config: ChaosConfig,
        execution: BacktestExecutionConfig,
    ) -> Result<Self> {
        execution.validate()?;
        let depth_fill_conservative_threshold = execution.depth_fill_conservative_threshold;
        let matchers = config
            .instruments
            .iter()
            .map(|inst| {
                (
                    inst.symbol.clone(),
                    MatchingEngine::with_depth_fill_conservative_threshold(
                        inst.clone(),
                        depth_fill_conservative_threshold,
                    ),
                )
            })
            .collect();
        Ok(Self {
            portfolio: Portfolio::new(&config.instruments),
            strategy: ChaosStrategy::new(config).context("invalid chaos/iarb2 configuration")?,
            matchers,
            execution,
            time_basis: BacktestTimeBasis::EventTimestampMs,
            scheduled: BTreeMap::new(),
            next_action_seq: 1,
            pending_cancels: HashSet::new(),
            now_ns: 0,
            first_arrival_ns: None,
            last_arrival_ns: None,
            input_events: 0,
            input_clock_regressions: 0,
            max_input_clock_regression_ns: 0,
            orders_sent: 0,
            cancel_requests: 0,
            deduplicated_cancel_requests: 0,
            ignored_cancel_requests: 0,
            exchange_activations: 0,
            cancelled_orders: 0,
            rejected_orders: 0,
            fills: 0,
            maker_fills: 0,
            taker_fills: 0,
        })
    }

    pub fn run_csv_path(mut self, path: impl AsRef<Path>) -> Result<BacktestReport> {
        let events = load_events_from_path(path.as_ref()).with_context(|| {
            format!(
                "failed to load replay events from {}",
                path.as_ref().display()
            )
        })?;
        self.run(events)
    }

    pub fn run_normalized_jsonl_path(mut self, path: impl AsRef<Path>) -> Result<BacktestReport> {
        let events = load_normalized_jsonl_from_path(path.as_ref()).with_context(|| {
            format!(
                "failed to load normalized replay events from {}",
                path.as_ref().display()
            )
        })?;
        self.run(events)
    }

    pub fn run_raw_capture_path(mut self, path: impl AsRef<Path>) -> Result<BacktestReport> {
        self.time_basis = BacktestTimeBasis::CaptureReceiveTimestampNs;
        replay_raw_capture_timed_path(path.as_ref(), |timed| {
            self.process_replay_event_at(timed.event, timed.recv_ts_ns)
        })
        .with_context(|| {
            format!(
                "failed to replay raw capture from {}",
                path.as_ref().display()
            )
        })?;
        self.require_all_configured_books()?;
        self.finish_report()
    }

    pub fn run<I>(&mut self, events: I) -> Result<BacktestReport>
    where
        I: IntoIterator<Item = NormalizedEvent>,
    {
        self.time_basis = BacktestTimeBasis::EventTimestampMs;
        for event in events {
            let arrival_ns = event.ts_ms().saturating_mul(NS_PER_MS);
            self.process_replay_event_at(event, arrival_ns)?;
        }

        self.finish_report()
    }

    #[cfg(test)]
    fn process_replay_event(&mut self, event: NormalizedEvent) -> Result<()> {
        let arrival_ns = event.ts_ms().saturating_mul(NS_PER_MS);
        self.process_replay_event_at(event, arrival_ns)
    }

    fn process_replay_event_at(
        &mut self,
        event: NormalizedEvent,
        candidate_arrival_ns: u64,
    ) -> Result<()> {
        let arrival_ns = self.register_input_arrival(candidate_arrival_ns);
        self.drain_before(arrival_ns)?;
        self.now_ns = arrival_ns;

        match &event {
            NormalizedEvent::Market(MarketEvent::Depth(book)) => {
                let now_ns = self.now_ns;
                let updates = self
                    .matcher_mut(&book.symbol)?
                    .on_depth_at(book.clone(), time_ms(now_ns));
                self.route_exchange_updates(updates)?;
                self.drain_through(now_ns)?;
                self.schedule_after(
                    self.execution.market_data_latency_ms,
                    ScheduledAction::DeliverStrategy(event.into_strategy_event()),
                );
                self.drain_through(now_ns)?;
            }
            NormalizedEvent::Market(MarketEvent::Trade {
                symbol,
                price,
                qty,
                taker_side,
                ..
            }) => {
                let now_ns = self.now_ns;
                let updates = self.matcher_mut(symbol)?.on_trade_at(
                    *price,
                    *qty,
                    *taker_side,
                    time_ms(now_ns),
                );
                self.route_exchange_updates(updates)?;
                self.drain_through(now_ns)?;
                self.schedule_after(
                    self.execution.market_data_latency_ms,
                    ScheduledAction::DeliverStrategy(event.into_strategy_event()),
                );
                self.drain_through(now_ns)?;
            }
            NormalizedEvent::Market(
                MarketEvent::IndexPrice { .. }
                | MarketEvent::FundingRate { .. }
                | MarketEvent::BurstSignal { .. }
                | MarketEvent::PriceLimits { .. },
            ) => {
                let now_ns = self.now_ns;
                self.drain_through(now_ns)?;
                self.schedule_after(
                    self.execution.market_data_latency_ms,
                    ScheduledAction::DeliverStrategy(event.into_strategy_event()),
                );
                self.drain_through(now_ns)?;
            }
            NormalizedEvent::Order(update) => {
                let now_ns = self.now_ns;
                self.route_exchange_updates(vec![update.clone()])?;
                self.drain_through(now_ns)?;
            }
            NormalizedEvent::Account(_)
            | NormalizedEvent::Timer(_)
            | NormalizedEvent::Control(_)
            | NormalizedEvent::System(_) => {
                let now_ns = self.now_ns;
                self.schedule_at(
                    now_ns,
                    ScheduledAction::DeliverStrategy(event.into_strategy_event()),
                );
                self.drain_through(now_ns)?;
            }
        }
        Ok(())
    }

    fn register_input_arrival(&mut self, candidate_ns: u64) -> u64 {
        self.input_events += 1;
        let arrival_ns = match self.last_arrival_ns {
            Some(last_ns) if candidate_ns < last_ns => {
                let regression_ns = last_ns - candidate_ns;
                self.input_clock_regressions += 1;
                self.max_input_clock_regression_ns =
                    self.max_input_clock_regression_ns.max(regression_ns);
                last_ns
            }
            _ => candidate_ns,
        };
        self.first_arrival_ns.get_or_insert(arrival_ns);
        self.last_arrival_ns = Some(arrival_ns);
        arrival_ns
    }

    fn route_exchange_updates(&mut self, updates: Vec<OrderUpdate>) -> Result<()> {
        for update in updates {
            if update.event == OrderEvent::Cancelled {
                self.cancelled_orders += 1;
            } else if update.event == OrderEvent::Rejected {
                self.rejected_orders += 1;
            }

            let account_update = if update.has_fill() {
                self.fills += 1;
                match update.last_fill_liquidity {
                    Some(FillLiquidity::Maker) => self.maker_fills += 1,
                    Some(FillLiquidity::Taker) => self.taker_fills += 1,
                    None => {}
                }
                self.portfolio.apply_fill(&update);
                Some(self.account_update_after_fill(&update))
            } else {
                None
            };

            self.schedule_after(
                self.execution.order_update_latency_ms,
                ScheduledAction::DeliverOrder(update),
            );
            if let Some(account_update) = account_update {
                self.schedule_after(
                    self.execution.fill_account_latency_ms,
                    ScheduledAction::DeliverStrategy(StrategyEvent::Account(account_update)),
                );
            }
        }
        Ok(())
    }

    fn account_update_after_fill(&self, update: &OrderUpdate) -> AccountUpdate {
        let qty = self
            .portfolio
            .positions()
            .get(&update.symbol)
            .copied()
            .unwrap_or(0.0);
        AccountUpdate {
            ts_ms: time_ms(self.now_ns),
            balances: Vec::new(),
            positions: vec![Position {
                symbol: update.symbol.clone(),
                qty,
                avg_price: if update.avg_fill_price > 0.0 {
                    update.avg_fill_price
                } else {
                    update.last_fill_price
                },
                margin_mode: None,
            }],
            margins: Vec::new(),
        }
    }

    fn accept_intents(&mut self, commands: Vec<OrderIntent>) -> Result<()> {
        let mut queue = VecDeque::from(commands);
        while let Some(command) = queue.pop_front() {
            match command {
                OrderIntent::NewOrder(order) => {
                    self.orders_sent += 1;
                    let symbol = order.symbol.clone();
                    let now_ms = time_ms(self.now_ns);
                    let (order_id, pending) =
                        self.matcher_mut(&symbol)?.prepare_submit(order, now_ms);
                    self.schedule_after(
                        self.execution.order_entry_latency_ms,
                        ScheduledAction::ActivateOrder {
                            symbol,
                            order_id: order_id.clone(),
                        },
                    );

                    let pending = retime_order_update(pending, now_ms);
                    queue.extend(self.strategy.on_event(&StrategyEvent::Order(pending)));
                }
                OrderIntent::CancelOrder { order_id, reason } => {
                    self.cancel_requests += 1;
                    if !self.has_open_order(&order_id) {
                        self.ignored_cancel_requests += 1;
                        continue;
                    }
                    if !self.pending_cancels.insert(order_id.clone()) {
                        self.deduplicated_cancel_requests += 1;
                        continue;
                    }
                    self.schedule_after(
                        self.execution.cancel_latency_ms,
                        ScheduledAction::CancelOrder { order_id, reason },
                    );
                }
            }
        }
        Ok(())
    }

    fn execute_action(&mut self, action: ScheduledAction) -> Result<()> {
        match action {
            ScheduledAction::ActivateOrder { symbol, order_id } => {
                let now_ms = time_ms(self.now_ns);
                let updates = {
                    let matcher = self.matcher_mut(&symbol)?;
                    if !matcher.is_pending(&order_id) {
                        return Ok(());
                    }
                    matcher.activate(&order_id, now_ms)
                };
                self.exchange_activations += 1;
                self.route_exchange_updates(updates)?;
            }
            ScheduledAction::CancelOrder { order_id, reason } => {
                self.pending_cancels.remove(&order_id);
                let now_ms = time_ms(self.now_ns);
                let updates = self
                    .matchers
                    .values_mut()
                    .find(|matcher| matcher.is_open_order(&order_id))
                    .map(|matcher| matcher.cancel_at(&order_id, now_ms, &reason))
                    .unwrap_or_default();
                self.route_exchange_updates(updates)?;
            }
            ScheduledAction::DeliverOrder(update) => {
                let update = retime_order_update(update, time_ms(self.now_ns));
                let commands = self.strategy.on_event(&StrategyEvent::Order(update));
                self.accept_intents(commands)?;
            }
            ScheduledAction::DeliverStrategy(event) => {
                let event = retime_strategy_event(event, time_ms(self.now_ns));
                let commands = self.strategy.on_event(&event);
                self.accept_intents(commands)?;
            }
        }
        Ok(())
    }

    fn schedule_after(&mut self, delay_ms: u64, action: ScheduledAction) {
        let delay_ns = delay_ms.saturating_mul(NS_PER_MS);
        self.schedule_at(self.now_ns.saturating_add(delay_ns), action);
    }

    fn schedule_at(&mut self, due_ns: u64, action: ScheduledAction) {
        let seq = self.next_action_seq;
        self.next_action_seq = self.next_action_seq.saturating_add(1);
        self.scheduled.insert((due_ns, seq), action);
    }

    fn drain_before(&mut self, cutoff_ns: u64) -> Result<()> {
        self.drain_scheduled(cutoff_ns, false)
    }

    fn drain_through(&mut self, cutoff_ns: u64) -> Result<()> {
        self.drain_scheduled(cutoff_ns, true)
    }

    fn drain_scheduled(&mut self, cutoff_ns: u64, inclusive: bool) -> Result<()> {
        let mut processed = 0usize;
        while let Some((&(due_ns, _), _)) = self.scheduled.first_key_value() {
            if due_ns > cutoff_ns || (!inclusive && due_ns == cutoff_ns) {
                break;
            }
            let (_, action) = self
                .scheduled
                .pop_first()
                .expect("first scheduled action must still exist");
            self.now_ns = self.now_ns.max(due_ns);
            self.execute_action(action)?;
            processed += 1;
            if processed > MAX_ACTIONS_PER_DRAIN {
                bail!(
                    "backtest exceeded {MAX_ACTIONS_PER_DRAIN} scheduled actions at {} ns",
                    self.now_ns
                );
            }
        }
        Ok(())
    }

    fn finish_report(&mut self) -> Result<BacktestReport> {
        let now_ns = self.now_ns;
        self.drain_through(now_ns)?;
        let marks = self
            .matchers
            .iter()
            .filter_map(|(symbol, matcher)| Some((symbol.clone(), matcher.depth()?.mid()?)))
            .collect::<HashMap<_, _>>();
        let pending_orders = self
            .matchers
            .values()
            .map(MatchingEngine::pending_order_count)
            .sum();
        let live_orders = self
            .matchers
            .values()
            .map(MatchingEngine::live_order_count)
            .sum();
        let mut pending_activation_actions = 0;
        let mut pending_cancel_actions = 0;
        let mut pending_order_update_actions = 0;
        let mut pending_strategy_event_actions = 0;
        for action in self.scheduled.values() {
            match action {
                ScheduledAction::ActivateOrder { .. } => pending_activation_actions += 1,
                ScheduledAction::CancelOrder { .. } => pending_cancel_actions += 1,
                ScheduledAction::DeliverOrder(_) => pending_order_update_actions += 1,
                ScheduledAction::DeliverStrategy(_) => pending_strategy_event_actions += 1,
            }
        }

        Ok(BacktestReport {
            execution: self.execution.clone(),
            time_basis: self.time_basis,
            input_events: self.input_events,
            first_arrival_ns: self.first_arrival_ns,
            last_arrival_ns: self.last_arrival_ns,
            input_clock_regressions: self.input_clock_regressions,
            max_input_clock_regression_ns: self.max_input_clock_regression_ns,
            orders_sent: self.orders_sent,
            cancel_requests: self.cancel_requests,
            deduplicated_cancel_requests: self.deduplicated_cancel_requests,
            ignored_cancel_requests: self.ignored_cancel_requests,
            exchange_activations: self.exchange_activations,
            cancelled_orders: self.cancelled_orders,
            rejected_orders: self.rejected_orders,
            fills: self.fills,
            maker_fills: self.maker_fills,
            taker_fills: self.taker_fills,
            pending_scheduled_actions: self.scheduled.len(),
            pending_activation_actions,
            pending_cancel_actions,
            pending_order_update_actions,
            pending_strategy_event_actions,
            pending_orders,
            live_orders,
            pending_cancel_requests: self.pending_cancels.len(),
            final_delta_usd: self.strategy.delta_usd(),
            final_pending_delta_usd: self.strategy.pending_delta_usd(),
            final_equity_usd: self.portfolio.equity_usd(&marks),
            cash_usd: self.portfolio.cash_usd(),
            positions: self.portfolio.positions().clone(),
        })
    }

    fn require_all_configured_books(&self) -> Result<()> {
        let mut missing = self
            .matchers
            .iter()
            .filter(|(_, matcher)| matcher.depth().is_none())
            .map(|(symbol, _)| symbol.clone())
            .collect::<Vec<_>>();
        if missing.is_empty() {
            return Ok(());
        }
        missing.sort();
        bail!(
            "raw capture did not produce a valid book for configured symbols: {}",
            missing.join(", ")
        )
    }

    fn matcher_mut(&mut self, symbol: &str) -> Result<&mut MatchingEngine> {
        self.matchers
            .get_mut(symbol)
            .with_context(|| format!("no matcher configured for symbol {symbol}"))
    }

    fn has_open_order(&self, order_id: &str) -> bool {
        self.matchers
            .values()
            .any(|matcher| matcher.is_open_order(order_id))
    }
}

fn time_ms(time_ns: u64) -> u64 {
    time_ns / NS_PER_MS
}

fn retime_order_update(mut update: OrderUpdate, ts_ms: u64) -> OrderUpdate {
    update.ts_ms = ts_ms;
    update
}

fn retime_strategy_event(mut event: StrategyEvent, ts_ms: u64) -> StrategyEvent {
    match &mut event {
        StrategyEvent::Market(MarketEvent::Depth(book)) => book.ts_ms = ts_ms,
        StrategyEvent::Market(
            MarketEvent::Trade {
                ts_ms: event_ts, ..
            }
            | MarketEvent::IndexPrice {
                ts_ms: event_ts, ..
            }
            | MarketEvent::FundingRate {
                ts_ms: event_ts, ..
            }
            | MarketEvent::BurstSignal {
                ts_ms: event_ts, ..
            }
            | MarketEvent::PriceLimits {
                ts_ms: event_ts, ..
            },
        ) => *event_ts = ts_ms,
        StrategyEvent::Order(update) => update.ts_ms = ts_ms,
        StrategyEvent::Account(update) => update.ts_ms = ts_ms,
        StrategyEvent::Timer(event) => event.ts_ms = ts_ms,
        StrategyEvent::Control(event) => event.ts_ms = ts_ms,
        StrategyEvent::System(event) => event.ts_ms = ts_ms,
    }
    event
}

#[cfg(test)]
mod tests {
    use reap_core::{Level, NewOrder, OrderBook, Side, TimeInForce, TimerEvent};
    use reap_strategy::{InstrumentConfig, InstrumentKindConfig, RiskGroupConfig};

    use super::*;

    fn config() -> ChaosConfig {
        ChaosConfig {
            ref_symbol: "BTC-USDT".to_string(),
            active_hedge_threshold_usd: 500.0,
            min_hedge_interval_ms: 0,
            risk_groups: vec![RiskGroupConfig {
                name: "main".to_string(),
                symbols: vec!["BTC-USDT".to_string(), "BTC-PERP".to_string()],
                soft_delta_limit_usd: 50_000.0,
                hard_delta_limit_usd: 75_000.0,
                delta_stop_limit_usd: 100_000.0,
                live_order_limit_usd: 100_000.0,
                ..RiskGroupConfig::default()
            }],
            instruments: vec![
                InstrumentConfig {
                    symbol: "BTC-USDT".to_string(),
                    risk_group: "main".to_string(),
                    kind: InstrumentKindConfig::Spot,
                    max_order_size_usd: 5_000.0,
                    min_order_size_usd: 100.0,
                    max_order_size: 1.0,
                    tick_size: 0.1,
                    lot_size: 0.0001,
                    ..InstrumentConfig::default()
                },
                InstrumentConfig {
                    symbol: "BTC-PERP".to_string(),
                    risk_group: "main".to_string(),
                    kind: InstrumentKindConfig::Future,
                    contract_value: 0.001,
                    max_order_size_usd: 10_000.0,
                    min_order_size_usd: 100.0,
                    max_order_size: 10_000.0,
                    min_trade_size: 1.0,
                    lot_size: 1.0,
                    min_position: -100_000.0,
                    max_position: 100_000.0,
                    ..InstrumentConfig::default()
                },
            ],
            ..ChaosConfig::default()
        }
    }

    fn initial_books() -> Vec<NormalizedEvent> {
        vec![
            NormalizedEvent::from(MarketEvent::Depth(OrderBook::one_level(
                "BTC-USDT",
                1,
                Level::new(50_000.0, 2.0),
                Level::new(50_001.0, 2.0),
            ))),
            NormalizedEvent::from(MarketEvent::Depth(OrderBook::one_level(
                "BTC-PERP",
                1,
                Level::new(50_003.0, 10_000.0),
                Level::new(50_004.0, 10_000.0),
            ))),
        ]
    }

    #[test]
    fn replayed_quote_fill_triggers_hedge_order() {
        let mut runner = BacktestRunner::new(config()).unwrap();
        let mut events = initial_books();
        events.push(NormalizedEvent::from(MarketEvent::Trade {
            ts_ms: 2,
            symbol: "BTC-USDT".to_string(),
            price: 49_000.0,
            qty: 1.0,
            taker_side: Side::Sell,
        }));

        let report = runner.run(events).unwrap();
        assert!(report.orders_sent >= 3);
        assert!(report.fills >= 1);
        assert!(report.taker_fills >= 1);
        assert!(report.final_delta_usd.abs() < 5_000.0);
        assert_eq!(report.execution, BacktestExecutionConfig::default());
    }

    #[test]
    fn normalized_fixture_replays_quote_and_hedge_path() {
        let events = load_normalized_jsonl(
            include_str!("../../../fixtures/normalized/chaos_quote_hedge.jsonl").as_bytes(),
        )
        .unwrap();
        let mut runner = BacktestRunner::new(config()).unwrap();

        let report = runner.run(events).unwrap();

        assert!(report.orders_sent >= 1);
        assert_eq!(report.fills, 2);
        assert_eq!(report.maker_fills, 1);
        assert_eq!(report.taker_fills, 1);
        assert!(report.final_delta_usd.abs() < 1_000.0);
    }

    #[test]
    fn delayed_entry_is_reported_as_pending_at_end_of_data() {
        let execution = BacktestExecutionConfig {
            order_entry_latency_ms: 10,
            ..BacktestExecutionConfig::default()
        };
        let mut runner = BacktestRunner::with_execution_config(config(), execution).unwrap();

        let report = runner.run(initial_books()).unwrap();

        assert!(report.orders_sent > 0);
        assert_eq!(report.exchange_activations, 0);
        assert_eq!(report.pending_orders, report.orders_sent);
        assert_eq!(report.pending_activation_actions, report.pending_orders);
        assert!(report.pending_scheduled_actions >= report.pending_orders);
    }

    #[test]
    fn delayed_market_data_is_not_delivered_past_end_of_data() {
        let execution = BacktestExecutionConfig {
            market_data_latency_ms: 10,
            ..BacktestExecutionConfig::default()
        };
        let mut runner = BacktestRunner::with_execution_config(config(), execution).unwrap();

        let report = runner.run(initial_books()).unwrap();

        assert_eq!(report.orders_sent, 0);
        assert_eq!(report.pending_scheduled_actions, 2);
        assert_eq!(report.pending_strategy_event_actions, 2);
        assert_eq!(report.pending_orders, 0);
    }

    #[test]
    fn input_clock_regressions_are_clamped_and_reported() {
        let mut runner = BacktestRunner::new(config()).unwrap();
        let mut events = initial_books();
        events[0] = NormalizedEvent::from(MarketEvent::Depth(OrderBook::one_level(
            "BTC-USDT",
            10,
            Level::new(50_000.0, 2.0),
            Level::new(50_001.0, 2.0),
        )));

        let report = runner.run(events).unwrap();

        assert_eq!(report.input_clock_regressions, 1);
        assert_eq!(report.max_input_clock_regression_ns, 9 * NS_PER_MS);
        assert_eq!(report.last_arrival_ns, Some(10 * NS_PER_MS));
    }

    #[test]
    fn order_remains_fillable_until_delayed_cancel_is_effective() {
        let execution = BacktestExecutionConfig {
            cancel_latency_ms: 10,
            ..BacktestExecutionConfig::default()
        };
        let mut runner = BacktestRunner::with_execution_config(config(), execution).unwrap();
        runner.now_ns = NS_PER_MS;
        runner.matcher_mut("BTC-USDT").unwrap().on_depth_at(
            OrderBook::one_level(
                "BTC-USDT",
                1,
                Level::new(100.0, 1.0),
                Level::new(101.0, 1.0),
            ),
            1,
        );
        runner
            .accept_intents(vec![OrderIntent::NewOrder(NewOrder {
                symbol: "BTC-USDT".to_string(),
                side: Side::Buy,
                qty: 0.5,
                price: 100.0,
                time_in_force: TimeInForce::PostOnly,
                reduce_only: false,
                self_trade_prevention: None,
                reason: "manual_test".to_string(),
            })])
            .unwrap();
        runner.drain_through(NS_PER_MS).unwrap();
        runner
            .accept_intents(vec![OrderIntent::CancelOrder {
                order_id: "BTC-USDT-1".to_string(),
                reason: "manual_cancel".to_string(),
            }])
            .unwrap();

        runner
            .process_replay_event_at(
                NormalizedEvent::from(MarketEvent::Trade {
                    ts_ms: 5,
                    symbol: "BTC-USDT".to_string(),
                    price: 100.0,
                    qty: 1.5,
                    taker_side: Side::Sell,
                }),
                5 * NS_PER_MS,
            )
            .unwrap();
        runner
            .process_replay_event_at(
                NormalizedEvent::Timer(TimerEvent {
                    ts_ms: 11,
                    name: "advance".to_string(),
                }),
                11 * NS_PER_MS,
            )
            .unwrap();
        let report = runner.finish_report().unwrap();

        assert_eq!(report.fills, 1);
        assert_eq!(report.maker_fills, 1);
        assert_eq!(report.cancel_requests, 1);
        assert_eq!(report.cancelled_orders, 0);
        assert_eq!(report.pending_cancel_requests, 0);
    }

    #[test]
    fn nanosecond_arrival_clock_preserves_cancel_before_next_market_event() {
        let execution = BacktestExecutionConfig {
            cancel_latency_ms: 1,
            ..BacktestExecutionConfig::default()
        };
        let mut runner = BacktestRunner::with_execution_config(config(), execution).unwrap();
        runner.now_ns = 100_100_000;
        runner.matcher_mut("BTC-USDT").unwrap().on_depth_at(
            OrderBook::one_level(
                "BTC-USDT",
                100,
                Level::new(100.0, 1.0),
                Level::new(101.0, 1.0),
            ),
            100,
        );
        runner
            .accept_intents(vec![OrderIntent::NewOrder(NewOrder {
                symbol: "BTC-USDT".to_string(),
                side: Side::Buy,
                qty: 0.5,
                price: 100.0,
                time_in_force: TimeInForce::PostOnly,
                reduce_only: false,
                self_trade_prevention: None,
                reason: "nanosecond_test".to_string(),
            })])
            .unwrap();
        runner.drain_through(100_100_000).unwrap();
        runner
            .accept_intents(vec![OrderIntent::CancelOrder {
                order_id: "BTC-USDT-1".to_string(),
                reason: "cancel_before_trade".to_string(),
            }])
            .unwrap();

        runner
            .process_replay_event_at(
                NormalizedEvent::from(MarketEvent::Trade {
                    ts_ms: 101,
                    symbol: "BTC-USDT".to_string(),
                    price: 100.0,
                    qty: 1.5,
                    taker_side: Side::Sell,
                }),
                101_200_000,
            )
            .unwrap();
        let report = runner.finish_report().unwrap();

        assert_eq!(report.cancelled_orders, 1);
        assert_eq!(report.fills, 0);
        assert_eq!(report.last_arrival_ns, Some(101_200_000));
    }

    #[test]
    fn raw_capture_requires_every_strategy_book() {
        let mut runner = BacktestRunner::new(config()).unwrap();
        replay_raw_capture(
            include_str!("../../../fixtures/raw/okx/depth-gap.jsonl").as_bytes(),
            |event| runner.process_replay_event(event),
        )
        .unwrap();

        let error = runner.require_all_configured_books().unwrap_err();

        assert!(error.to_string().contains("BTC-PERP"));
    }
}
