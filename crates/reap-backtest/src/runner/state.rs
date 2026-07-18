use std::collections::{BTreeMap, HashSet};

use reap_core::Symbol;

use crate::{BacktestTimeBasis, MatchingEngine, RawReplayBoundary, ScheduledAction};

pub(super) struct ReplayState {
    pub(super) time_basis: BacktestTimeBasis,
    pub(super) raw_replay_boundary: Option<RawReplayBoundary>,
    pub(super) carry_source_boundary: Option<RawReplayBoundary>,
    pub(super) now_ns: u64,
    pub(super) first_arrival_ns: Option<u64>,
    pub(super) last_arrival_ns: Option<u64>,
    pub(super) input_events: u64,
    pub(super) input_clock_regressions: u64,
    pub(super) max_input_clock_regression_ns: u64,
}

pub(super) struct ScheduleState {
    pub(super) scheduled: BTreeMap<(u64, u64), ScheduledAction>,
    pub(super) next_action_seq: u64,
}

pub(super) struct OrderLifecycleState {
    pub(super) matchers: BTreeMap<Symbol, MatchingEngine>,
    pub(super) initial_account_snapshot_delivered: bool,
    pub(super) pending_cancels: HashSet<String>,
    pub(super) pending_fill_account_updates: usize,
    pub(super) last_account_publish_ns: Option<u64>,
    pub(super) periodic_account_refreshes: u64,
    pub(super) order_entry_ready_at_ns: Option<u64>,
    pub(super) new_orders_blocked_not_ready: usize,
    pub(super) orders_sent: usize,
    pub(super) cancel_requests: usize,
    pub(super) deduplicated_cancel_requests: usize,
    pub(super) ignored_cancel_requests: usize,
    pub(super) exchange_activations: usize,
    pub(super) cancelled_orders: usize,
    pub(super) rejected_orders: usize,
    pub(super) fills: usize,
    pub(super) maker_fills: usize,
    pub(super) taker_fills: usize,
}
