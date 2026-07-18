use std::collections::BTreeMap;

use crate::{BacktestTimeBasis, RawReplayBoundary, ScheduledAction};

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
