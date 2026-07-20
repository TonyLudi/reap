use std::path::Path;

use anyhow::{Context, Result, bail};
use reap_core::{MarketEvent, NormalizedEvent};

use crate::{
    BacktestLatencyClass, BacktestReport, BacktestRunner, BacktestTimeBasis, NS_PER_MS,
    RawCaptureRecordRange, RawReplayBoundary, ScheduledAction, load_events_from_path,
    load_normalized_jsonl_from_path, replay_raw_capture_timed_path_with_boundary,
    replay_raw_capture_timed_range_path, time_ms,
};

impl BacktestRunner {
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
        let path = path.as_ref();
        self.replay.time_basis = BacktestTimeBasis::CaptureReceiveTimestampNs;
        let preload_boundary = replay_raw_capture_timed_path_with_boundary(path, |timed| {
            self.preload_funding_settlement(&timed.event)
        })
        .with_context(|| format!("failed to preload realized funding from {}", path.display()))?;
        if let Some(boundary) = &preload_boundary {
            self.validate_carry_handoff(boundary)?;
        } else if self.replay.carry_source_boundary.is_some() {
            bail!("settled raw carry requires sequenced raw replay input");
        }
        let boundary = replay_raw_capture_timed_path_with_boundary(path, |timed| {
            self.process_replay_event_at(timed.event, timed.recv_ts_ns)
        })
        .with_context(|| format!("failed to replay raw capture from {}", path.display()))?;
        if preload_boundary != boundary {
            bail!("raw capture boundary changed between preload and replay passes");
        }
        if let Some(boundary) = &boundary {
            self.advance_raw_horizon(boundary.maximum_recv_ts_ns)?;
        }
        self.replay.raw_replay_boundary = boundary;
        self.require_all_configured_books()?;
        self.finish_report()
    }

    pub fn run_raw_capture_range_path(
        mut self,
        path: impl AsRef<Path>,
        range: RawCaptureRecordRange,
    ) -> Result<BacktestReport> {
        let path = path.as_ref();
        self.replay.time_basis = BacktestTimeBasis::CaptureReceiveTimestampNs;
        let preload_boundary = replay_raw_capture_timed_range_path(path, range, |timed| {
            self.preload_funding_settlement(&timed.event)
        })
        .with_context(|| {
            format!(
                "failed to preload realized funding from raw capture range {}..={} in {}",
                range.first,
                range.last,
                path.display()
            )
        })?;
        self.validate_carry_handoff(&preload_boundary)?;
        let replay_boundary = replay_raw_capture_timed_range_path(path, range, |timed| {
            self.process_replay_event_at(timed.event, timed.recv_ts_ns)
        })
        .with_context(|| {
            format!(
                "failed to replay raw capture range {}..={} from {}",
                range.first,
                range.last,
                path.display()
            )
        })?;
        if preload_boundary != replay_boundary {
            bail!("raw capture range boundary changed between preload and replay passes");
        }
        self.advance_raw_horizon(replay_boundary.maximum_recv_ts_ns)?;
        self.replay.raw_replay_boundary = Some(replay_boundary);
        self.require_all_configured_books()?;
        self.finish_report()
    }

    pub fn run<I>(&mut self, events: I) -> Result<BacktestReport>
    where
        I: IntoIterator<Item = NormalizedEvent>,
    {
        self.replay.time_basis = BacktestTimeBasis::EventTimestampMs;
        let events = events.into_iter().collect::<Vec<_>>();
        for event in &events {
            self.preload_funding_settlement(event)?;
        }
        for event in events {
            let arrival_ns = event.ts_ms().saturating_mul(NS_PER_MS);
            self.process_replay_event_at(event, arrival_ns)?;
        }

        self.finish_report()
    }

    #[cfg(test)]
    pub(super) fn process_replay_event(&mut self, event: NormalizedEvent) -> Result<()> {
        let arrival_ns = event.ts_ms().saturating_mul(NS_PER_MS);
        self.process_replay_event_at(event, arrival_ns)
    }

    pub(super) fn process_replay_event_at(
        &mut self,
        event: NormalizedEvent,
        candidate_arrival_ns: u64,
    ) -> Result<()> {
        let arrival_ns = self.register_input_arrival(candidate_arrival_ns);
        self.drain_before(arrival_ns)?;
        self.advance_metric_clock(arrival_ns);
        self.replay.now_ns = arrival_ns;
        self.deliver_initial_account_snapshot()?;

        match &event {
            NormalizedEvent::Market(MarketEvent::Depth(book)) => {
                let now_ns = self.replay.now_ns;
                let latency_ms = self
                    .latency_sampler
                    .sample(BacktestLatencyClass::MarketDepth, &book.symbol);
                if let Some(mid) = book.mid().filter(|mid| mid.is_finite() && *mid > 0.0) {
                    self.valuation.depth_marks.insert(book.symbol.clone(), mid);
                }
                let updates = self
                    .matcher_mut(&book.symbol)?
                    .on_depth_at(book.clone(), time_ms(now_ns));
                self.route_exchange_updates(updates)?;
                self.drain_through(now_ns)?;
                self.schedule_after(
                    latency_ms,
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
                let now_ns = self.replay.now_ns;
                let latency_ms = self
                    .latency_sampler
                    .sample(BacktestLatencyClass::HistoricalTrade, symbol);
                let updates = self.matcher_mut(symbol)?.on_trade_at(
                    *price,
                    *qty,
                    *taker_side,
                    time_ms(now_ns),
                );
                self.route_exchange_updates(updates)?;
                self.drain_through(now_ns)?;
                self.schedule_after(
                    latency_ms,
                    ScheduledAction::DeliverTradeStrategy {
                        event: event.into_strategy_event(),
                        arrival_ns,
                    },
                );
                self.drain_through(now_ns)?;
            }
            NormalizedEvent::Market(
                MarketEvent::IndexPrice { symbol, .. } | MarketEvent::BurstSignal { symbol, .. },
            ) => {
                let now_ns = self.replay.now_ns;
                let latency_ms = self
                    .latency_sampler
                    .sample(BacktestLatencyClass::ReferenceData, symbol);
                self.drain_through(now_ns)?;
                self.schedule_after(
                    latency_ms,
                    ScheduledAction::DeliverStrategy(event.into_strategy_event()),
                );
                self.drain_through(now_ns)?;
            }
            NormalizedEvent::Market(MarketEvent::FundingRate {
                symbol,
                rate,
                funding_time_ms,
                ..
            }) => {
                let now_ns = self.replay.now_ns;
                let latency_ms = self
                    .latency_sampler
                    .sample(BacktestLatencyClass::ReferenceData, symbol);
                self.register_funding_rate(symbol, *rate, *funding_time_ms);
                self.drain_through(now_ns)?;
                self.schedule_after(
                    latency_ms,
                    ScheduledAction::DeliverStrategy(event.into_strategy_event()),
                );
                self.drain_through(now_ns)?;
            }
            NormalizedEvent::Market(MarketEvent::PriceLimits {
                symbol, mark_price, ..
            }) => {
                let now_ns = self.replay.now_ns;
                let latency_ms = self
                    .latency_sampler
                    .sample(BacktestLatencyClass::ReferenceData, symbol);
                if mark_price.is_finite() && *mark_price > 0.0 {
                    self.valuation
                        .exchange_marks
                        .insert(symbol.clone(), *mark_price);
                }
                self.drain_through(now_ns)?;
                self.schedule_after(
                    latency_ms,
                    ScheduledAction::DeliverStrategy(event.into_strategy_event()),
                );
                self.drain_through(now_ns)?;
            }
            NormalizedEvent::Order(update) => {
                let now_ns = self.replay.now_ns;
                self.route_exchange_updates(vec![update.clone()])?;
                self.drain_through(now_ns)?;
            }
            NormalizedEvent::Account(_)
            | NormalizedEvent::Timer(_)
            | NormalizedEvent::Control(_)
            | NormalizedEvent::System(_) => {
                let now_ns = self.replay.now_ns;
                self.schedule_at(
                    now_ns,
                    ScheduledAction::DeliverStrategy(event.into_strategy_event()),
                );
                self.drain_through(now_ns)?;
            }
        }
        self.observe_order_entry_readiness();
        self.sample_risk_metrics();
        Ok(())
    }

    fn register_input_arrival(&mut self, candidate_ns: u64) -> u64 {
        self.replay.input_events += 1;
        let arrival_ns = match self.replay.last_arrival_ns {
            Some(last_ns) if candidate_ns < last_ns => {
                let regression_ns = last_ns - candidate_ns;
                self.replay.input_clock_regressions += 1;
                self.replay.max_input_clock_regression_ns =
                    self.replay.max_input_clock_regression_ns.max(regression_ns);
                last_ns
            }
            _ => candidate_ns,
        };
        self.replay.first_arrival_ns.get_or_insert(arrival_ns);
        self.replay.last_arrival_ns = Some(arrival_ns);
        arrival_ns
    }

    pub(super) fn advance_raw_horizon(&mut self, horizon_ns: u64) -> Result<()> {
        if horizon_ns <= self.replay.now_ns {
            return Ok(());
        }
        self.drain_through(horizon_ns)?;
        self.advance_metric_clock(horizon_ns);
        self.replay.now_ns = horizon_ns;
        self.observe_order_entry_readiness();
        self.sample_risk_metrics();
        Ok(())
    }

    pub(super) fn validate_carry_handoff(&self, current: &RawReplayBoundary) -> Result<()> {
        current.validate()?;
        let Some(previous) = &self.replay.carry_source_boundary else {
            return Ok(());
        };
        previous.validate()?;
        if previous.capture_session_id != current.capture_session_id {
            bail!(
                "settled carry crosses capture sessions: previous={}, current={}",
                previous.capture_session_id,
                current.capture_session_id
            );
        }
        let expected = previous
            .last_capture_record_seq
            .checked_add(1)
            .context("previous capture record sequence exhausted")?;
        if current.first_capture_record_seq != expected {
            bail!(
                "settled carry requires the next capture record sequence {expected}, received {}",
                current.first_capture_record_seq
            );
        }
        if current.first_recv_ts_ns < self.replay.now_ns {
            bail!(
                "settled carry receive time regresses: settled_at_ns={}, current_first_recv_ts_ns={}",
                self.replay.now_ns,
                current.first_recv_ts_ns
            );
        }
        Ok(())
    }

    pub(super) fn require_all_configured_books(&self) -> Result<()> {
        let mut missing = self
            .orders
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
}
