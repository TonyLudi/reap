use std::collections::VecDeque;

use reap_core::TimeMs;

use crate::ChaosExecutionIntent;

use super::ChaosStrategy;

const TRADE_REPRICE_DELAY_NS: u64 = 100_000;
// Every checked-in Iarb2 configuration at the pinned Java revision uses a
// five-millisecond ChaosTimedConflationWorker interval.
const PINNED_IARB2_PRICING_WORKER_INTERVAL_MS: TimeMs = 5;
const NS_PER_MS: u64 = 1_000_000;
const MAX_SCHEDULED_TRADE_REPRICE_ACTIONS: usize = 65_536;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TradeRepriceActionKind {
    Callback,
    PricingWorker { scheduled_time_ms: TimeMs },
}

/// The only scheduled-action representation added for public-trade parity.
/// It is private, non-serializable, and has no constructor outside this
/// module.
#[derive(Debug, Clone, Copy)]
struct ScheduledTradeRepriceAction {
    due_ns: u64,
    sequence: u64,
    kind: TradeRepriceActionKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PendingPricingTimer {
    due_ns: u64,
    scheduled_time_ms: TimeMs,
}

/// Java retains a FIFO timer queue. Keep the common single-timer case inline;
/// the overflow deque allocates only at the rare wall-ms/sub-ms overlap where
/// Java can have more than one timer outstanding.
#[derive(Debug, Clone, Default)]
struct PendingPricingTimers {
    first: Option<PendingPricingTimer>,
    additional: VecDeque<PendingPricingTimer>,
}

impl PendingPricingTimers {
    fn len(&self) -> usize {
        usize::from(self.first.is_some()) + self.additional.len()
    }

    fn front(&self) -> Option<PendingPricingTimer> {
        self.first
    }

    fn get(&self, index: usize) -> Option<PendingPricingTimer> {
        match index {
            0 => self.first,
            index => self.additional.get(index - 1).copied(),
        }
    }

    fn push(&mut self, timer: PendingPricingTimer) -> Result<(), &'static str> {
        if self.len() >= MAX_SCHEDULED_TRADE_REPRICE_ACTIONS {
            return Err("pricing-worker timer queue saturated");
        }
        if self.first.is_none() {
            self.first = Some(timer);
        } else {
            self.additional.push_back(timer);
        }
        Ok(())
    }

    fn pop_front(&mut self) -> Option<PendingPricingTimer> {
        let timer = self.first.take()?;
        self.first = self.additional.pop_front();
        Some(timer)
    }
}

#[derive(Debug, Clone, Default)]
struct PricingWorkerState {
    last_work_ms: TimeMs,
    last_finish_ms: TimeMs,
    next_run_time_ms: TimeMs,
    pending_timers: PendingPricingTimers,
}

#[derive(Debug, Clone, Default)]
pub(super) struct TradeRepriceState {
    scheduled: VecDeque<ScheduledTradeRepriceAction>,
    next_sequence: u64,
    new_wake_deadlines_ns: [u64; 2],
    new_wake_count: usize,
    new_wake_overflow: VecDeque<u64>,
    worker: PricingWorkerState,
    compatibility_worker: PricingWorkerState,
    worker_active: bool,
}

enum CallbackDisposition {
    RunPricing,
    ScheduledOrConflated,
}

impl TradeRepriceState {
    fn begin_public_operation(&mut self) {
        self.new_wake_count = 0;
        self.new_wake_overflow.clear();
    }

    fn schedule_callback(&mut self, arrival_ns: u64) -> Result<(), &'static str> {
        let due_ns = arrival_ns
            .checked_add(TRADE_REPRICE_DELAY_NS)
            .ok_or("public-trade reprice deadline overflow")?;
        self.insert(due_ns, TradeRepriceActionKind::Callback)
    }

    fn insert(&mut self, due_ns: u64, kind: TradeRepriceActionKind) -> Result<(), &'static str> {
        if self.scheduled.len() >= MAX_SCHEDULED_TRADE_REPRICE_ACTIONS {
            return Err("public-trade reprice queue saturated");
        }
        if self.new_wake_count + self.new_wake_overflow.len() >= MAX_SCHEDULED_TRADE_REPRICE_ACTIONS
        {
            return Err("public-trade wake notification batch saturated");
        }
        let sequence = self.next_sequence;
        self.next_sequence = self
            .next_sequence
            .checked_add(1)
            .ok_or("public-trade reprice sequence overflow")?;
        let action = ScheduledTradeRepriceAction {
            due_ns,
            sequence,
            kind,
        };
        if self.scheduled.back().is_none_or(|pending| {
            (pending.due_ns, pending.sequence) <= (action.due_ns, action.sequence)
        }) {
            self.scheduled.push_back(action);
            self.record_wake_deadline(due_ns);
            return Ok(());
        }
        let position = self.scheduled.iter().position(|pending| {
            (pending.due_ns, pending.sequence) > (action.due_ns, action.sequence)
        });
        match position {
            Some(position) => self.scheduled.insert(position, action),
            None => self.scheduled.push_back(action),
        }
        self.record_wake_deadline(due_ns);
        Ok(())
    }

    fn record_wake_deadline(&mut self, due_ns: u64) {
        if let Some(slot) = self.new_wake_deadlines_ns.get_mut(self.new_wake_count) {
            *slot = due_ns;
            self.new_wake_count += 1;
        } else {
            self.new_wake_overflow.push_back(due_ns);
        }
    }

    fn take_new_wake_deadline_ns(&mut self) -> Option<u64> {
        if self.new_wake_count == 0 {
            return None;
        }
        let deadline_ns = self.new_wake_deadlines_ns[0];
        if self.new_wake_count == 2 {
            self.new_wake_deadlines_ns[0] = self.new_wake_deadlines_ns[1];
            if let Some(next) = self.new_wake_overflow.pop_front() {
                self.new_wake_deadlines_ns[1] = next;
            } else {
                self.new_wake_count = 1;
            }
        } else {
            self.new_wake_count = 0;
        }
        Some(deadline_ns)
    }

    fn next_due_ns(&self) -> Option<u64> {
        self.scheduled.front().map(|action| action.due_ns)
    }

    fn pop_due(&mut self, now_ns: u64) -> Option<ScheduledTradeRepriceAction> {
        self.scheduled
            .front()
            .is_some_and(|action| action.due_ns <= now_ns)
            .then(|| {
                self.scheduled
                    .pop_front()
                    .expect("checked scheduled trade-reprice action must exist")
            })
    }

    fn on_worker_trigger(
        &mut self,
        now_ns: u64,
        observed_now_ms: TimeMs,
    ) -> Result<CallbackDisposition, &'static str> {
        // Java drops a trigger while work is in flight. Rust's single writer
        // completes refreshes synchronously, but retain the exact guard.
        if self.worker.last_finish_ms < self.worker.last_work_ms {
            return Ok(CallbackDisposition::ScheduledOrConflated);
        }

        // ChaosConflationWorker.directScheduleWork checks this before its
        // elapsed-time branch. In particular, a regressed millisecond clock
        // joins/drops against the already chosen nextRunTime; it never creates
        // a replacement "regression + interval" timer.
        if observed_now_ms < self.worker.next_run_time_ms {
            return Ok(CallbackDisposition::ScheduledOrConflated);
        }

        let remaining_ms = if observed_now_ms >= self.worker.last_finish_ms {
            let elapsed_ms = observed_now_ms - self.worker.last_finish_ms;
            if elapsed_ms >= PINNED_IARB2_PRICING_WORKER_INTERVAL_MS {
                self.worker.next_run_time_ms = observed_now_ms;
                self.worker.last_work_ms = observed_now_ms;
                return Ok(CallbackDisposition::RunPricing);
            }
            PINNED_IARB2_PRICING_WORKER_INTERVAL_MS - elapsed_ms
        } else {
            // Java retains signed elapsed time. Once `now >= nextRunTime`,
            // a clock regression behind a late finish extends the next delay
            // by the regression instead of dropping the trigger.
            PINNED_IARB2_PRICING_WORKER_INTERVAL_MS
                .checked_add(self.worker.last_finish_ms - observed_now_ms)
                .ok_or("pricing-worker regressed delay overflow")?
        };
        let remaining_ns = remaining_ms
            .checked_mul(NS_PER_MS)
            .ok_or("pricing-worker delay overflow")?;
        let due_ns = now_ns
            .checked_add(remaining_ns)
            .ok_or("pricing-worker deadline overflow")?;
        let scheduled_time_ms = observed_now_ms
            .checked_add(remaining_ms)
            .ok_or("pricing-worker scheduled time overflow")?;
        self.insert(
            due_ns,
            TradeRepriceActionKind::PricingWorker { scheduled_time_ms },
        )?;
        self.worker.pending_timers.push(PendingPricingTimer {
            due_ns,
            scheduled_time_ms,
        })?;
        self.worker.next_run_time_ms = scheduled_time_ms;
        Ok(CallbackDisposition::ScheduledOrConflated)
    }

    fn observe_compatibility_depth(
        &mut self,
        now_ns: u64,
        observed_now_ms: TimeMs,
    ) -> Result<(), &'static str> {
        if self.worker_active {
            return Ok(());
        }
        Self::service_compatibility_worker_if_due(&mut self.compatibility_worker, now_ns);
        let worker = &mut self.compatibility_worker;
        if observed_now_ms < worker.next_run_time_ms {
            return Ok(());
        }

        let remaining_ms = if observed_now_ms >= worker.last_finish_ms {
            let elapsed_ms = observed_now_ms - worker.last_finish_ms;
            if elapsed_ms >= PINNED_IARB2_PRICING_WORKER_INTERVAL_MS {
                worker.next_run_time_ms = observed_now_ms;
                worker.last_work_ms = observed_now_ms;
                worker.last_finish_ms = observed_now_ms;
                return Ok(());
            }
            PINNED_IARB2_PRICING_WORKER_INTERVAL_MS - elapsed_ms
        } else {
            PINNED_IARB2_PRICING_WORKER_INTERVAL_MS
                .checked_add(worker.last_finish_ms - observed_now_ms)
                .ok_or("compatibility pricing-worker regressed delay overflow")?
        };
        let remaining_ns = remaining_ms
            .checked_mul(NS_PER_MS)
            .ok_or("compatibility pricing-worker delay overflow")?;
        let due_ns = now_ns
            .checked_add(remaining_ns)
            .ok_or("compatibility pricing-worker deadline overflow")?;
        let scheduled_time_ms = observed_now_ms
            .checked_add(remaining_ms)
            .ok_or("compatibility pricing-worker scheduled time overflow")?;
        worker.pending_timers.push(PendingPricingTimer {
            due_ns,
            scheduled_time_ms,
        })?;
        worker.next_run_time_ms = scheduled_time_ms;
        Ok(())
    }

    fn service_compatibility_worker_if_due(worker: &mut PricingWorkerState, now_ns: u64) {
        while worker
            .pending_timers
            .front()
            .is_some_and(|timer| timer.due_ns <= now_ns)
        {
            let timer = worker
                .pending_timers
                .pop_front()
                .expect("checked compatibility timer must exist");
            worker.last_work_ms = timer.scheduled_time_ms;
            worker.last_finish_ms = timer.scheduled_time_ms;
        }
    }

    fn activate_worker_from_compatibility(&mut self, now_ns: u64) -> Result<(), &'static str> {
        if self.worker_active {
            return Ok(());
        }
        Self::service_compatibility_worker_if_due(&mut self.compatibility_worker, now_ns);
        self.worker = self.compatibility_worker.clone();
        self.worker_active = true;
        let timer_count = self.worker.pending_timers.len();
        if self
            .scheduled
            .len()
            .checked_add(timer_count)
            .and_then(|count| count.checked_add(1))
            .is_none_or(|count| count > MAX_SCHEDULED_TRADE_REPRICE_ACTIONS)
        {
            return Err("public-trade reprice queue saturated");
        }
        for index in 0..timer_count {
            let timer = self
                .worker
                .pending_timers
                .get(index)
                .expect("checked inherited pricing timer must exist");
            self.insert(
                timer.due_ns,
                TradeRepriceActionKind::PricingWorker {
                    scheduled_time_ms: timer.scheduled_time_ms,
                },
            )?;
        }
        Ok(())
    }

    fn on_pricing_worker_start(
        &mut self,
        observed_now_ms: TimeMs,
        _due_ns: u64,
        scheduled_time_ms: TimeMs,
    ) -> bool {
        if self.worker.pending_timers.pop_front().is_none() {
            return false;
        }
        self.worker.last_work_ms = observed_now_ms.max(scheduled_time_ms);
        true
    }

    fn on_immediate_pricing_worker_start(&mut self, observed_now_ms: TimeMs) {
        // `on_worker_trigger` records the decision/scheduled time from
        // ChaosConflationWorker.runWork. Java samples the wall clock again in
        // runWork(boolean, long) immediately before invoking pricing.
        self.worker.last_work_ms = observed_now_ms.max(self.worker.last_work_ms);
    }

    fn on_pricing_worker_finish(&mut self, observed_now_ms: TimeMs) {
        self.worker.last_finish_ms = observed_now_ms.max(self.worker.last_work_ms);
    }
}

impl ChaosStrategy {
    pub(super) fn schedule_trade_reprice(&mut self, receipt_ns: u64, processing_ns: u64) {
        self.trade_reprice.begin_public_operation();
        let result = self
            .trade_reprice
            .activate_worker_from_compatibility(processing_ns)
            .and_then(|()| self.trade_reprice.schedule_callback(receipt_ns));
        if let Err(reason) = result {
            self.halt_reason = Some(reason.to_string());
        }
    }

    pub(super) fn observe_compatibility_depth_worker(
        &mut self,
        arrival_ns: u64,
        observed_now_ms: TimeMs,
    ) {
        if let Err(reason) = self
            .trade_reprice
            .observe_compatibility_depth(arrival_ns, observed_now_ms)
        {
            self.halt_reason = Some(reason.to_string());
        }
    }

    /// Feeds the reached Live depth callback through the same pinned Java
    /// pricing-worker clock used by deferred public-trade callbacks.
    ///
    /// Returns true when the pinned worker ran synchronously. A false result
    /// means the same private worker action was scheduled or joined.
    pub(super) fn trigger_live_depth_pricing_worker(
        &mut self,
        arrival_ns: u64,
        observed_now_ms: TimeMs,
    ) -> bool {
        self.trade_reprice.begin_public_operation();
        let result = self
            .trade_reprice
            .activate_worker_from_compatibility(arrival_ns)
            .and_then(|()| {
                self.trade_reprice
                    .on_worker_trigger(arrival_ns, observed_now_ms)
            });
        match result {
            Ok(CallbackDisposition::RunPricing) => true,
            Ok(CallbackDisposition::ScheduledOrConflated) => false,
            Err(reason) => {
                self.halt_reason = Some(reason.to_string());
                false
            }
        }
    }

    pub(super) fn finish_live_pricing_worker(&mut self, observed_now_ms: TimeMs) {
        self.trade_reprice.on_pricing_worker_finish(observed_now_ms);
    }

    pub(super) fn start_immediate_live_pricing_worker(&mut self, observed_now_ms: TimeMs) {
        self.trade_reprice
            .on_immediate_pricing_worker_start(observed_now_ms);
    }

    /// Returns the next private deadline without exposing its action value.
    pub fn next_trade_reprice_due_ns(&self) -> Option<u64> {
        self.trade_reprice.next_due_ns()
    }

    /// Takes the transport wake created by the most recent private insertion.
    ///
    /// Replay consumes the notification batch immediately so every insertion
    /// receives its own global scheduler sequence. Two notifications remain
    /// inline; only Java's rare multiple-pending-timer overlap needs the
    /// bounded overflow deque. Live reads the private action queue directly
    /// and resets stale notifications per operation.
    pub fn take_new_trade_reprice_wake_deadline_ns(&mut self) -> Option<u64> {
        self.trade_reprice.take_new_wake_deadline_ns()
    }

    /// Services at most one private action, preserving higher-priority
    /// single-writer work between callbacks with equal deadlines.
    pub fn service_one_due_trade_reprice(
        &mut self,
        now_ns: u64,
        observed_now_ms: reap_core::TimeMs,
        strategy_is_live: bool,
    ) -> Vec<ChaosExecutionIntent> {
        self.service_one_due_trade_reprice_with_finish_clock(
            now_ns,
            observed_now_ms,
            strategy_is_live,
            || observed_now_ms,
        )
    }

    /// Live-clock counterpart that samples the worker finish after pricing
    /// completes, matching Java's conflation-on-work-finish boundary.
    pub fn service_one_due_trade_reprice_with_finish_clock(
        &mut self,
        now_ns: u64,
        observed_now_ms: reap_core::TimeMs,
        strategy_is_live: bool,
        worker_clock: impl FnMut() -> reap_core::TimeMs,
    ) -> Vec<ChaosExecutionIntent> {
        self.service_one_due_trade_reprice_with_clocks(
            || (now_ns, observed_now_ms),
            strategy_is_live,
            worker_clock,
        )
    }

    /// Live counterpart that samples the callback decision/direct-timer start,
    /// the immediate worker start when applicable, and the worker finish at
    /// the corresponding Java boundaries.
    pub fn service_one_due_trade_reprice_with_clocks(
        &mut self,
        start_clock: impl FnOnce() -> (u64, reap_core::TimeMs),
        strategy_is_live: bool,
        mut worker_clock: impl FnMut() -> reap_core::TimeMs,
    ) -> Vec<ChaosExecutionIntent> {
        self.trade_reprice.begin_public_operation();
        let (now_ns, observed_now_ms) = start_clock();
        let Some(action) = self.trade_reprice.pop_due(now_ns) else {
            return Vec::new();
        };
        self.advance_time(observed_now_ms);

        let worker_started = match action.kind {
            TradeRepriceActionKind::Callback => {
                match self
                    .trade_reprice
                    .on_worker_trigger(now_ns, observed_now_ms)
                {
                    Ok(CallbackDisposition::RunPricing) => {
                        let work_start_ms = worker_clock();
                        self.start_immediate_live_pricing_worker(work_start_ms);
                        self.advance_time(work_start_ms);
                        true
                    }
                    Ok(CallbackDisposition::ScheduledOrConflated) => false,
                    Err(reason) => {
                        self.halt_reason = Some(reason.to_string());
                        false
                    }
                }
            }
            TradeRepriceActionKind::PricingWorker { scheduled_time_ms } => self
                .trade_reprice
                .on_pricing_worker_start(observed_now_ms, action.due_ns, scheduled_time_ms),
        };
        if !worker_started {
            return Vec::new();
        }
        let intents = if strategy_is_live {
            self.refresh_quotes()
        } else {
            Vec::new()
        };
        self.finish_live_pricing_worker(worker_clock());
        intents
    }
}

#[cfg(test)]
mod tests {
    use super::{
        MAX_SCHEDULED_TRADE_REPRICE_ACTIONS, NS_PER_MS, PINNED_IARB2_PRICING_WORKER_INTERVAL_MS,
        PendingPricingTimers, PricingWorkerState, TRADE_REPRICE_DELAY_NS, TradeRepriceActionKind,
        TradeRepriceState,
    };

    #[test]
    fn private_queue_is_bounded_and_deadline_overflow_is_explicit() {
        let mut state = TradeRepriceState::default();
        assert_eq!(state.scheduled.capacity(), 0);
        assert_eq!(
            state.schedule_callback(u64::MAX),
            Err("public-trade reprice deadline overflow")
        );
        for arrival in 0..MAX_SCHEDULED_TRADE_REPRICE_ACTIONS as u64 {
            state.schedule_callback(arrival).unwrap();
            assert_eq!(
                state.take_new_wake_deadline_ns(),
                Some(arrival + TRADE_REPRICE_DELAY_NS)
            );
        }
        assert_eq!(
            state.schedule_callback(MAX_SCHEDULED_TRADE_REPRICE_ACTIONS as u64),
            Err("public-trade reprice queue saturated")
        );
        assert_eq!(state.scheduled.len(), MAX_SCHEDULED_TRADE_REPRICE_ACTIONS);
    }

    #[test]
    fn callbacks_are_multiplicative_but_pricing_work_is_trailing_conflated() {
        let mut state = TradeRepriceState {
            scheduled: std::collections::VecDeque::with_capacity(8),
            next_sequence: 0,
            new_wake_deadlines_ns: [0; 2],
            new_wake_count: 0,
            new_wake_overflow: std::collections::VecDeque::new(),
            worker: PricingWorkerState::default(),
            compatibility_worker: PricingWorkerState::default(),
            worker_active: false,
        };
        for arrival in [1_000_000_000, 1_000_200_000, 1_000_250_000] {
            state.schedule_callback(arrival).unwrap();
            assert_eq!(
                state.take_new_wake_deadline_ns(),
                Some(arrival + TRADE_REPRICE_DELAY_NS)
            );
        }

        let first = state
            .pop_due(1_000_000_000 + TRADE_REPRICE_DELAY_NS)
            .unwrap();
        assert_eq!(first.kind, TradeRepriceActionKind::Callback);
        assert!(matches!(
            state.on_worker_trigger(first.due_ns, 1_000),
            Ok(super::CallbackDisposition::RunPricing)
        ));
        state.on_pricing_worker_finish(1_000);
        assert_eq!(state.worker.last_finish_ms, 1_000);

        let second = state.pop_due(1_000_300_000).unwrap();
        assert_eq!(second.kind, TradeRepriceActionKind::Callback);
        assert!(matches!(
            state.on_worker_trigger(second.due_ns, 1_000),
            Ok(super::CallbackDisposition::ScheduledOrConflated)
        ));
        let worker_due = second.due_ns + PINNED_IARB2_PRICING_WORKER_INTERVAL_MS * NS_PER_MS;
        assert_eq!(state.take_new_wake_deadline_ns(), Some(worker_due));
        let third = state.pop_due(1_000_350_000).unwrap();
        assert_eq!(third.kind, TradeRepriceActionKind::Callback);
        assert!(matches!(
            state.on_worker_trigger(third.due_ns, 1_000),
            Ok(super::CallbackDisposition::ScheduledOrConflated)
        ));
        assert_eq!(state.take_new_wake_deadline_ns(), None);

        assert_eq!(state.next_due_ns(), Some(worker_due));
        let worker = state.pop_due(worker_due).unwrap();
        assert_eq!(
            worker.kind,
            TradeRepriceActionKind::PricingWorker {
                scheduled_time_ms: 1_005
            }
        );
        assert!(state.on_pricing_worker_start(1_006, worker.due_ns, 1_005));
        state.on_pricing_worker_finish(1_006);
        assert_eq!(state.worker.last_finish_ms, 1_006);
        assert!(state.scheduled.is_empty());
    }

    #[test]
    fn integer_millisecond_boundary_and_scheduled_time_match_pinned_worker() {
        let mut state = TradeRepriceState::default();
        let callback_now_ns = 9_000_100_000;
        assert_eq!(state.worker.last_finish_ms, 0);

        assert!(matches!(
            state.on_worker_trigger(callback_now_ns, 4),
            Ok(super::CallbackDisposition::ScheduledOrConflated)
        ));
        let worker_due = callback_now_ns + NS_PER_MS;
        assert_eq!(state.next_due_ns(), Some(worker_due));
        let worker = state.pop_due(worker_due).unwrap();
        assert_eq!(
            worker.kind,
            TradeRepriceActionKind::PricingWorker {
                scheduled_time_ms: 5
            }
        );
        assert!(state.on_pricing_worker_start(3, worker.due_ns, 5));
        state.on_pricing_worker_finish(3);
        assert_eq!(state.worker.last_finish_ms, 5);

        assert!(matches!(
            state.on_worker_trigger(worker_due + 1, 10),
            Ok(super::CallbackDisposition::RunPricing)
        ));
        state.on_pricing_worker_finish(10);
        assert_eq!(state.worker.last_finish_ms, 10);
    }

    #[test]
    fn wall_millisecond_boundary_runs_before_submillisecond_pending_timer() {
        let mut state = TradeRepriceState::default();
        let first_start_ns = 1_000_100_000;

        assert!(matches!(
            state.on_worker_trigger(first_start_ns, 1_000),
            Ok(super::CallbackDisposition::RunPricing)
        ));
        state.on_pricing_worker_finish(1_000);
        assert!(matches!(
            state.on_worker_trigger(first_start_ns + NS_PER_MS, 1_001),
            Ok(super::CallbackDisposition::ScheduledOrConflated)
        ));
        let pending_due_ns = 1_005_100_000;
        assert_eq!(
            state
                .worker
                .pending_timers
                .front()
                .map(|timer| timer.due_ns),
            Some(pending_due_ns)
        );

        assert!(matches!(
            state.on_worker_trigger(1_005_000_000, 1_005),
            Ok(super::CallbackDisposition::RunPricing)
        ));
        state.on_pricing_worker_finish(1_005);
        assert_eq!(
            state
                .worker
                .pending_timers
                .front()
                .map(|timer| timer.due_ns),
            Some(pending_due_ns),
            "Java's already-scheduled timer remains after the immediate run"
        );

        assert!(matches!(
            state.on_worker_trigger(1_005_050_000, 1_005),
            Ok(super::CallbackDisposition::ScheduledOrConflated)
        ));
        let second_due_ns = 1_010_050_000;
        assert_eq!(state.worker.pending_timers.len(), 2);
        assert_eq!(
            state.worker.pending_timers.get(1).map(|timer| timer.due_ns),
            Some(second_due_ns)
        );

        let pending = state.pop_due(pending_due_ns).unwrap();
        assert!(state.on_pricing_worker_start(1_005, pending.due_ns, 1_005));
        state.on_pricing_worker_finish(1_005);
        assert_eq!(
            state
                .worker
                .pending_timers
                .front()
                .map(|timer| timer.due_ns),
            Some(second_due_ns),
            "firing the old Java timer must not discard the newer queued timer"
        );
        let second = state.pop_due(second_due_ns).unwrap();
        assert!(state.on_pricing_worker_start(1_010, second.due_ns, 1_010));
        state.on_pricing_worker_finish(1_010);
        assert_eq!(state.worker.pending_timers.front(), None);
    }

    #[test]
    fn compatibility_worker_keeps_submillisecond_timer_after_wall_boundary_run() {
        let mut state = TradeRepriceState::default();
        let first_start_ns = 1_000_100_000;

        state
            .observe_compatibility_depth(first_start_ns, 1_000)
            .unwrap();
        state
            .observe_compatibility_depth(first_start_ns + NS_PER_MS, 1_001)
            .unwrap();
        let pending_due_ns = 1_005_100_000;
        assert_eq!(
            state
                .compatibility_worker
                .pending_timers
                .front()
                .map(|timer| timer.due_ns),
            Some(pending_due_ns)
        );

        state
            .observe_compatibility_depth(1_005_000_000, 1_005)
            .unwrap();
        assert_eq!(state.compatibility_worker.last_finish_ms, 1_005);
        assert_eq!(
            state
                .compatibility_worker
                .pending_timers
                .front()
                .map(|timer| timer.due_ns),
            Some(pending_due_ns),
            "the causal shadow must retain Java's already-scheduled timer"
        );

        state
            .observe_compatibility_depth(1_005_050_000, 1_005)
            .unwrap();
        assert_eq!(state.compatibility_worker.pending_timers.len(), 2);
        assert_eq!(
            state
                .compatibility_worker
                .pending_timers
                .get(1)
                .map(|timer| timer.due_ns),
            Some(1_010_050_000)
        );
    }

    #[test]
    fn regressed_millisecond_clock_is_dropped_before_elapsed_time_math() {
        let mut state = TradeRepriceState {
            worker: PricingWorkerState {
                last_work_ms: 100,
                last_finish_ms: 100,
                next_run_time_ms: 100,
                pending_timers: PendingPricingTimers::default(),
            },
            ..TradeRepriceState::default()
        };
        let callback_now_ns = 9_000_100_000;

        assert!(matches!(
            state.on_worker_trigger(callback_now_ns, 98),
            Ok(super::CallbackDisposition::ScheduledOrConflated)
        ));
        assert_eq!(state.next_due_ns(), None);
        assert_eq!(state.worker.last_finish_ms, 100);
        assert_eq!(state.worker.next_run_time_ms, 100);
    }

    #[test]
    fn live_depth_and_trade_share_one_pending_worker_deadline() {
        let mut state = TradeRepriceState::default();
        let depth_now_ns = 1_000_000_000;

        assert!(matches!(
            state.on_worker_trigger(depth_now_ns, 1_000),
            Ok(super::CallbackDisposition::RunPricing)
        ));
        state.on_pricing_worker_finish(1_000);
        assert_eq!(state.next_due_ns(), None, "first depth runs immediately");

        assert!(matches!(
            state.on_worker_trigger(depth_now_ns + NS_PER_MS, 1_001),
            Ok(super::CallbackDisposition::ScheduledOrConflated)
        ));
        let worker_due = depth_now_ns + 5 * NS_PER_MS;
        assert_eq!(state.next_due_ns(), Some(worker_due));

        assert!(matches!(
            state.on_worker_trigger(depth_now_ns + 2 * NS_PER_MS, 1_002),
            Ok(super::CallbackDisposition::ScheduledOrConflated)
        ));
        assert_eq!(state.next_due_ns(), Some(worker_due));

        let worker = state.pop_due(worker_due).unwrap();
        assert!(state.on_pricing_worker_start(1_005, worker.due_ns, 1_005));
        state.on_pricing_worker_finish(1_005);
        assert_eq!(state.worker.last_finish_ms, 1_005);
        assert_eq!(state.next_due_ns(), None);
    }

    #[test]
    fn depth_only_worker_records_actual_late_finish_before_a_later_trade() {
        let mut state = TradeRepriceState::default();
        let start_ns = 1_000_000_000;
        state.on_worker_trigger(start_ns, 1_000).unwrap();
        state.on_pricing_worker_finish(1_000);
        state
            .on_worker_trigger(start_ns + NS_PER_MS, 1_001)
            .unwrap();

        let worker_due = start_ns + 5 * NS_PER_MS;
        let worker = state.pop_due(worker_due + NS_PER_MS).unwrap();
        assert!(state.on_pricing_worker_start(1_006, worker.due_ns, 1_005));
        state.on_pricing_worker_finish(1_006);
        assert_eq!(state.worker.last_finish_ms, 1_006);

        assert!(matches!(
            state.on_worker_trigger(worker_due + 2 * NS_PER_MS, 1_007),
            Ok(super::CallbackDisposition::ScheduledOrConflated)
        ));
        assert_eq!(state.next_due_ns(), Some(start_ns + 11 * NS_PER_MS));
    }

    #[test]
    fn regression_behind_a_late_finish_extends_the_java_worker_delay() {
        let callback_now_ns = 9_000_100_000;
        let mut state = TradeRepriceState {
            worker: PricingWorkerState {
                last_work_ms: 1_006,
                last_finish_ms: 1_006,
                next_run_time_ms: 1_005,
                pending_timers: PendingPricingTimers::default(),
            },
            ..TradeRepriceState::default()
        };

        assert!(matches!(
            state.on_worker_trigger(callback_now_ns, 1_005),
            Ok(super::CallbackDisposition::ScheduledOrConflated)
        ));
        let expected_due_ns = callback_now_ns + 6 * NS_PER_MS;
        assert_eq!(state.next_due_ns(), Some(expected_due_ns));
        assert_eq!(state.worker.next_run_time_ms, 1_011);
        assert_eq!(
            state
                .worker
                .pending_timers
                .front()
                .map(|timer| timer.due_ns),
            Some(expected_due_ns)
        );
    }

    #[test]
    fn pricing_finish_crossing_a_millisecond_moves_the_next_deadline() {
        let start_ns = 1_000_000_000;
        let mut state = TradeRepriceState::default();

        assert!(matches!(
            state.on_worker_trigger(start_ns, 1_000),
            Ok(super::CallbackDisposition::RunPricing)
        ));
        assert_eq!(state.worker.last_work_ms, 1_000);
        assert_eq!(state.worker.last_finish_ms, 0);
        state.on_pricing_worker_finish(1_001);

        assert!(matches!(
            state.on_worker_trigger(start_ns + 5 * NS_PER_MS, 1_005),
            Ok(super::CallbackDisposition::ScheduledOrConflated)
        ));
        assert_eq!(state.next_due_ns(), Some(start_ns + 6 * NS_PER_MS));
        assert_eq!(state.worker.next_run_time_ms, 1_006);
    }

    #[test]
    fn one_public_operation_keeps_two_wakes_inline_and_preserves_overflow_order() {
        let mut state = TradeRepriceState::default();
        state.schedule_callback(10).unwrap();
        state.schedule_callback(20).unwrap();
        state.schedule_callback(30).unwrap();

        assert_eq!(state.scheduled.len(), 3);
        assert_eq!(
            state.take_new_wake_deadline_ns(),
            Some(10 + TRADE_REPRICE_DELAY_NS)
        );
        assert_eq!(
            state.take_new_wake_deadline_ns(),
            Some(20 + TRADE_REPRICE_DELAY_NS)
        );
        assert_eq!(
            state.take_new_wake_deadline_ns(),
            Some(30 + TRADE_REPRICE_DELAY_NS)
        );
        assert_eq!(state.take_new_wake_deadline_ns(), None);
        assert_eq!(state.next_due_ns(), Some(10 + TRADE_REPRICE_DELAY_NS));
    }
}
