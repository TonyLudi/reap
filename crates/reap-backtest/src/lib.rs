mod matching;
mod portfolio;
mod replay;

pub use matching::MatchingEngine;
pub use replay::{
    ReplayRow, load_events_from_path, load_normalized_jsonl, load_normalized_jsonl_from_path,
    replay_raw_capture, replay_raw_capture_path,
};

use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use crate::portfolio::Portfolio;
use reap_core::{
    AccountUpdate, FillLiquidity, MarketEvent, NewOrder, NormalizedEvent, OrderIntent, OrderUpdate,
    Position, StrategyEvent, Symbol,
};
use reap_strategy::{ChaosConfig, ChaosStrategy, Strategy};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BacktestReport {
    pub orders_sent: usize,
    pub fills: usize,
    pub maker_fills: usize,
    pub taker_fills: usize,
    pub final_delta_usd: f64,
    pub final_pending_delta_usd: f64,
    pub final_equity_usd: f64,
    pub cash_usd: f64,
    pub positions: HashMap<Symbol, f64>,
}

pub struct BacktestRunner {
    strategy: ChaosStrategy,
    matchers: HashMap<Symbol, MatchingEngine>,
    portfolio: Portfolio,
    orders_sent: usize,
    fills: usize,
    maker_fills: usize,
    taker_fills: usize,
}

impl BacktestRunner {
    pub fn new(config: ChaosConfig) -> Result<Self> {
        let matchers = config
            .instruments
            .iter()
            .map(|inst| (inst.symbol.clone(), MatchingEngine::new(inst.clone())))
            .collect();
        Ok(Self {
            portfolio: Portfolio::new(&config.instruments),
            strategy: ChaosStrategy::new(config).context("invalid chaos/iarb2 configuration")?,
            matchers,
            orders_sent: 0,
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
        replay_raw_capture_path(path.as_ref(), |event| self.process_replay_event(event))
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
        for event in events {
            self.process_replay_event(event)?;
        }

        self.finish_report()
    }

    fn process_replay_event(&mut self, event: NormalizedEvent) -> Result<()> {
        match &event {
            NormalizedEvent::Market(MarketEvent::Depth(book)) => {
                let updates = self.matcher_mut(&book.symbol)?.on_depth(book.clone());
                let commands = self.process_updates(updates)?;
                self.process_commands(commands)?;
                self.process_strategy_event(event.into_strategy_event())?;
            }
            NormalizedEvent::Market(MarketEvent::Trade {
                ts_ms,
                symbol,
                price,
                qty,
                taker_side,
                ..
            }) => {
                let updates = self
                    .matcher_mut(symbol)?
                    .on_trade(*ts_ms, *price, *qty, *taker_side);
                let commands = self.process_updates(updates)?;
                self.process_commands(commands)?;
                self.process_strategy_event(event.into_strategy_event())?;
            }
            NormalizedEvent::Market(
                MarketEvent::IndexPrice { .. }
                | MarketEvent::FundingRate { .. }
                | MarketEvent::BurstSignal { .. }
                | MarketEvent::PriceLimits { .. },
            ) => {
                self.process_strategy_event(event.into_strategy_event())?;
            }
            NormalizedEvent::Order(update) => {
                let commands = self.process_updates(vec![update.clone()])?;
                self.process_commands(commands)?;
            }
            NormalizedEvent::Account(_)
            | NormalizedEvent::Timer(_)
            | NormalizedEvent::Control(_)
            | NormalizedEvent::System(_) => {
                self.process_strategy_event(event.into_strategy_event())?;
            }
        }
        Ok(())
    }

    fn finish_report(&self) -> Result<BacktestReport> {
        let marks = self
            .matchers
            .iter()
            .filter_map(|(symbol, matcher)| Some((symbol.clone(), matcher.depth()?.mid()?)))
            .collect::<HashMap<_, _>>();

        Ok(BacktestReport {
            orders_sent: self.orders_sent,
            fills: self.fills,
            maker_fills: self.maker_fills,
            taker_fills: self.taker_fills,
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

    fn process_commands(&mut self, commands: Vec<OrderIntent>) -> Result<()> {
        let mut queue = commands;
        while !queue.is_empty() {
            let mut next = Vec::new();
            for command in queue {
                match command {
                    OrderIntent::NewOrder(order) => {
                        self.orders_sent += 1;
                        let updates = self.submit_order(order)?;
                        next.extend(self.process_updates(updates)?);
                    }
                    OrderIntent::CancelOrder { order_id, reason } => {
                        let updates = self.cancel_order(&order_id, &reason)?;
                        next.extend(self.process_updates(updates)?);
                    }
                }
            }
            queue = next;
        }
        Ok(())
    }

    fn submit_order(&mut self, order: NewOrder) -> Result<Vec<OrderUpdate>> {
        Ok(self.matcher_mut(&order.symbol)?.submit(order))
    }

    fn cancel_order(&mut self, order_id: &str, reason: &str) -> Result<Vec<OrderUpdate>> {
        for matcher in self.matchers.values_mut() {
            if matcher.contains_order(order_id) {
                return Ok(matcher.cancel(order_id, reason));
            }
        }
        Ok(Vec::new())
    }

    fn process_updates(&mut self, updates: Vec<OrderUpdate>) -> Result<Vec<OrderIntent>> {
        let mut commands = Vec::new();
        for update in updates {
            if update.has_fill() {
                self.fills += 1;
                match update.last_fill_liquidity {
                    Some(FillLiquidity::Maker) => self.maker_fills += 1,
                    Some(FillLiquidity::Taker) => self.taker_fills += 1,
                    None => {}
                }
                self.portfolio.apply_fill(&update);
            }
            commands.extend(
                self.strategy
                    .on_event(&StrategyEvent::Order(update.clone())),
            );
            if update.has_fill() {
                let qty = self
                    .portfolio
                    .positions()
                    .get(&update.symbol)
                    .copied()
                    .unwrap_or(0.0);
                commands.extend(
                    self.strategy
                        .on_event(&StrategyEvent::Account(AccountUpdate {
                            ts_ms: update.ts_ms,
                            balances: Vec::new(),
                            positions: vec![Position {
                                symbol: update.symbol,
                                qty,
                                avg_price: if update.avg_fill_price > 0.0 {
                                    update.avg_fill_price
                                } else {
                                    update.last_fill_price
                                },
                                margin_mode: None,
                            }],
                            margins: Vec::new(),
                        })),
                );
            }
        }
        Ok(commands)
    }

    fn process_strategy_event(&mut self, event: StrategyEvent) -> Result<()> {
        let commands = self.strategy.on_event(&event);
        self.process_commands(commands)
    }
}

#[cfg(test)]
mod tests {
    use reap_core::{Level, OrderBook, Side};
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

    #[test]
    fn replayed_quote_fill_triggers_hedge_order() {
        let mut runner = BacktestRunner::new(config()).unwrap();
        let events = vec![
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
            NormalizedEvent::from(MarketEvent::Trade {
                ts_ms: 2,
                symbol: "BTC-USDT".to_string(),
                price: 49_000.0,
                qty: 1.0,
                taker_side: Side::Sell,
            }),
        ];

        let report = runner.run(events).unwrap();
        assert!(report.orders_sent >= 3);
        assert!(report.fills >= 1);
        assert!(report.taker_fills >= 1);
        assert!(report.final_delta_usd.abs() < 5_000.0);
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
