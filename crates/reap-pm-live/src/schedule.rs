use std::cmp::Ordering;

use reap_pm_core::{PmAccountHandle, PmAccountScope, PmInstrumentHandle, PmOrderSide};
use reap_pm_live_contracts::ConstructedRoleBinding;

use crate::lanes::{PmLaneKind, PmLanePolicy, SaturationAction};

/// Fixed upper bound for coordinator-owned PM quote deadlines.
///
/// The owner allocates this capacity once during construction and never grows
/// the backing store on an admission path.
pub const MAX_PM_SCHEDULED_ACTIONS: usize = 4_096;

/// A timer signal. It carries no order, cancel, journal, or transport authority.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PmScheduledActionKind {
    CancelOwnedQuote,
    ReconciliationRefresh,
    Freshness,
    QuoteEvaluation,
}

impl PmScheduledActionKind {
    const fn rank(self) -> u8 {
        match self {
            Self::CancelOwnedQuote => 0,
            Self::ReconciliationRefresh => 1,
            Self::Freshness => 2,
            Self::QuoteEvaluation => 3,
        }
    }

    /// Frozen within-scheduled-lane rank from the complete scheduler oracle.
    #[must_use]
    pub const fn variant_rank(self) -> u8 {
        self.rank()
    }
}

/// Stable identity of one PM quote deadline.
///
/// Construction is safe to expose: this value is only an identity and cannot
/// enqueue, cancel, service, or otherwise authorize a mutation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PmScheduledActionKey {
    account_scope: PmAccountScope,
    instrument: PmInstrumentHandle,
    side: PmOrderSide,
    kind: PmScheduledActionKind,
}

impl PmScheduledActionKey {
    #[must_use]
    pub const fn new(
        account_scope: PmAccountScope,
        instrument: PmInstrumentHandle,
        side: PmOrderSide,
        kind: PmScheduledActionKind,
    ) -> Self {
        Self {
            account_scope,
            instrument,
            side,
            kind,
        }
    }

    #[must_use]
    pub const fn account_scope(self) -> PmAccountScope {
        self.account_scope
    }

    #[must_use]
    pub const fn account(self) -> PmAccountHandle {
        self.account_scope.handle()
    }

    #[must_use]
    pub const fn instrument(self) -> PmInstrumentHandle {
        self.instrument
    }

    #[must_use]
    pub const fn side(self) -> PmOrderSide {
        self.side
    }

    #[must_use]
    pub const fn kind(self) -> PmScheduledActionKind {
        self.kind
    }
}

impl PartialOrd for PmScheduledActionKey {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for PmScheduledActionKey {
    fn cmp(&self, other: &Self) -> Ordering {
        self.kind
            .rank()
            .cmp(&other.kind.rank())
            .then_with(|| compare_account_scope(self.account_scope, other.account_scope))
            .then_with(|| {
                self.instrument
                    .market()
                    .ordinal()
                    .cmp(&other.instrument.market().ordinal())
            })
            .then_with(|| {
                self.instrument
                    .token()
                    .ordinal()
                    .cmp(&other.instrument.token().ordinal())
            })
            .then_with(|| side_rank(self.side).cmp(&side_rank(other.side)))
    }
}

/// Read-only view of one pending PM deadline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PmScheduledActionView {
    key: PmScheduledActionKey,
    deadline_ns: u64,
    scheduled_at_ns: u64,
    decision_wall_timestamp_ms: u64,
}

impl PmScheduledActionView {
    #[must_use]
    pub const fn key(self) -> PmScheduledActionKey {
        self.key
    }

    #[must_use]
    pub const fn deadline_ns(self) -> u64 {
        self.deadline_ns
    }

    #[must_use]
    pub const fn scheduled_at_ns(self) -> u64 {
        self.scheduled_at_ns
    }

    #[must_use]
    pub const fn decision_wall_timestamp_ms(self) -> u64 {
        self.decision_wall_timestamp_ms
    }
}

/// Bounded scheduler counters and age evidence captured at one monotonic time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PmScheduleMetrics {
    capacity: usize,
    nominal_high_water: usize,
    depth: usize,
    high_water: usize,
    admitted: u64,
    duplicate_suppressed: u64,
    rescheduled: u64,
    removed: u64,
    serviced: u64,
    rejected_full: u64,
    clock_regressions: u64,
    current_due_age_ns: u64,
    maximum_due_age_ns: u64,
    maximum_permitted_due_age_ns: u64,
    fail_closed: bool,
}

impl PmScheduleMetrics {
    #[must_use]
    pub const fn capacity(self) -> usize {
        self.capacity
    }

    #[must_use]
    pub const fn nominal_high_water(self) -> usize {
        self.nominal_high_water
    }

    #[must_use]
    pub const fn depth(self) -> usize {
        self.depth
    }

    #[must_use]
    pub const fn high_water(self) -> usize {
        self.high_water
    }

    #[must_use]
    pub const fn admitted(self) -> u64 {
        self.admitted
    }

    #[must_use]
    pub const fn duplicate_suppressed(self) -> u64 {
        self.duplicate_suppressed
    }

    #[must_use]
    pub const fn rescheduled(self) -> u64 {
        self.rescheduled
    }

    #[must_use]
    pub const fn removed(self) -> u64 {
        self.removed
    }

    #[must_use]
    pub const fn serviced(self) -> u64 {
        self.serviced
    }

    #[must_use]
    pub const fn rejected_full(self) -> u64 {
        self.rejected_full
    }

    #[must_use]
    pub const fn clock_regressions(self) -> u64 {
        self.clock_regressions
    }

    #[must_use]
    pub const fn current_due_age_ns(self) -> u64 {
        self.current_due_age_ns
    }

    #[must_use]
    pub const fn maximum_due_age_ns(self) -> u64 {
        self.maximum_due_age_ns
    }

    #[must_use]
    pub const fn maximum_permitted_due_age_ns(self) -> u64 {
        self.maximum_permitted_due_age_ns
    }

    #[must_use]
    pub const fn fail_closed(self) -> bool {
        self.fail_closed
    }
}

/// Read-only coordinator projection; pending mutation authority stays private.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PmScheduleProjection {
    observed_at_ns: u64,
    next: Option<PmScheduledActionView>,
    metrics: PmScheduleMetrics,
}

impl PmScheduleProjection {
    #[must_use]
    pub const fn observed_at_ns(self) -> u64 {
        self.observed_at_ns
    }

    #[must_use]
    pub const fn next(self) -> Option<PmScheduledActionView> {
        self.next
    }

    #[must_use]
    pub const fn metrics(self) -> PmScheduleMetrics {
        self.metrics
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PmScheduleAdmission {
    Inserted,
    DuplicateSuppressed,
    Rescheduled { previous_deadline_ns: u64 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PmScheduleError {
    ZeroMonotonicTimestamp,
    ZeroWallTimestamp,
    LocalActionSequenceExhausted,
    InstrumentMismatch {
        configured: PmInstrumentHandle,
        attempted: PmInstrumentHandle,
    },
    ClockRegression {
        previous_ns: u64,
        observed_ns: u64,
    },
    Full {
        attempted: PmScheduledActionKey,
        capacity: usize,
        action: SaturationAction,
    },
    Aged {
        pending: PmScheduledActionKey,
        deadline_ns: u64,
        observed_ns: u64,
        due_age_ns: u64,
        maximum_due_age_ns: u64,
        action: SaturationAction,
    },
}

/// A due timer signal, not an order/cancel permission.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct PmDueScheduledAction {
    key: PmScheduledActionKey,
    deadline_ns: u64,
    scheduled_at_ns: u64,
    serviced_at_ns: u64,
    due_age_ns: u64,
    local_action_sequence: u64,
    decision_wall_timestamp_ms: u64,
}

impl PmDueScheduledAction {
    pub(crate) const fn key(&self) -> PmScheduledActionKey {
        self.key
    }

    pub(crate) const fn deadline_ns(&self) -> u64 {
        self.deadline_ns
    }

    pub(crate) const fn scheduled_at_ns(&self) -> u64 {
        self.scheduled_at_ns
    }

    pub(crate) const fn serviced_at_ns(&self) -> u64 {
        self.serviced_at_ns
    }

    pub(crate) const fn due_age_ns(&self) -> u64 {
        self.due_age_ns
    }

    pub(crate) const fn local_action_sequence(&self) -> u64 {
        self.local_action_sequence
    }

    pub(crate) const fn decision_wall_timestamp_ms(&self) -> u64 {
        self.decision_wall_timestamp_ms
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ScheduledEntry {
    key: PmScheduledActionKey,
    deadline_ns: u64,
    scheduled_at_ns: u64,
    decision_wall_timestamp_ms: u64,
}

impl ScheduledEntry {
    const fn view(self) -> PmScheduledActionView {
        PmScheduledActionView {
            key: self.key,
            deadline_ns: self.deadline_ns,
            scheduled_at_ns: self.scheduled_at_ns,
            decision_wall_timestamp_ms: self.decision_wall_timestamp_ms,
        }
    }
}

/// Single owner of every pending quote-replace and owned-cancel deadline.
///
/// A sorted, fixed-capacity `Vec` deliberately combines ordering and duplicate
/// suppression in one allocation. Deadline counts are small and bounded, while
/// avoiding a second hash index makes recovery and equal-time ordering exact.
#[derive(Debug)]
pub(crate) struct PmQuoteScheduleRole {
    instrument: PmInstrumentHandle,
    actions: Vec<ScheduledEntry>,
    high_water: usize,
    admitted: u64,
    duplicate_suppressed: u64,
    rescheduled: u64,
    removed: u64,
    serviced: u64,
    rejected_full: u64,
    clock_regressions: u64,
    maximum_due_age_ns: u64,
    last_observed_ns: u64,
    next_local_action_sequence: u64,
    fail_closed: bool,
}

#[allow(
    clippy::result_large_err,
    reason = "schedule failures retain exact bounded evidence inline on an allocation-free owner path"
)]
impl PmQuoteScheduleRole {
    pub(crate) fn new(instrument: PmInstrumentHandle) -> Self {
        let policy = schedule_policy();
        debug_assert_eq!(policy.capacity(), MAX_PM_SCHEDULED_ACTIONS);
        Self {
            instrument,
            actions: Vec::with_capacity(MAX_PM_SCHEDULED_ACTIONS),
            high_water: 0,
            admitted: 0,
            duplicate_suppressed: 0,
            rescheduled: 0,
            removed: 0,
            serviced: 0,
            rejected_full: 0,
            clock_regressions: 0,
            maximum_due_age_ns: 0,
            last_observed_ns: 0,
            next_local_action_sequence: 1,
            fail_closed: false,
        }
    }

    pub(crate) const fn binding(&self) -> ConstructedRoleBinding {
        ConstructedRoleBinding::quote_schedule(self.instrument)
    }

    pub(crate) const fn instrument(&self) -> PmInstrumentHandle {
        self.instrument
    }

    pub(crate) fn reserved_capacity_bytes(&self) -> usize {
        self.actions.capacity() * std::mem::size_of::<ScheduledEntry>()
    }

    pub(crate) fn schedule(
        &mut self,
        key: PmScheduledActionKey,
        deadline_ns: u64,
        scheduled_at_ns: u64,
        decision_wall_timestamp_ms: u64,
    ) -> Result<PmScheduleAdmission, PmScheduleError> {
        self.observe_clock(scheduled_at_ns)?;
        if deadline_ns == 0 {
            self.fail_closed = true;
            return Err(PmScheduleError::ZeroMonotonicTimestamp);
        }
        if decision_wall_timestamp_ms == 0 {
            self.fail_closed = true;
            return Err(PmScheduleError::ZeroWallTimestamp);
        }
        if key.instrument() != self.instrument {
            self.fail_closed = true;
            return Err(PmScheduleError::InstrumentMismatch {
                configured: self.instrument,
                attempted: key.instrument(),
            });
        }

        if let Some(index) = self.actions.iter().position(|entry| entry.key == key) {
            let previous_deadline_ns = self.actions[index].deadline_ns;
            if previous_deadline_ns == deadline_ns {
                increment_counter(&mut self.duplicate_suppressed);
                return Ok(PmScheduleAdmission::DuplicateSuppressed);
            }
            let mut entry = self.actions.remove(index);
            entry.deadline_ns = deadline_ns;
            entry.scheduled_at_ns = scheduled_at_ns;
            entry.decision_wall_timestamp_ms = decision_wall_timestamp_ms;
            self.insert_sorted(entry);
            increment_counter(&mut self.rescheduled);
            return Ok(PmScheduleAdmission::Rescheduled {
                previous_deadline_ns,
            });
        }

        if self.actions.len() == MAX_PM_SCHEDULED_ACTIONS {
            increment_counter(&mut self.rejected_full);
            self.fail_closed = true;
            return Err(PmScheduleError::Full {
                attempted: key,
                capacity: MAX_PM_SCHEDULED_ACTIONS,
                action: schedule_policy().saturation_action(),
            });
        }

        self.insert_sorted(ScheduledEntry {
            key,
            deadline_ns,
            scheduled_at_ns,
            decision_wall_timestamp_ms,
        });
        self.high_water = self.high_water.max(self.actions.len());
        increment_counter(&mut self.admitted);
        Ok(PmScheduleAdmission::Inserted)
    }

    #[cfg(test)]
    pub(crate) fn remove(&mut self, key: PmScheduledActionKey) -> bool {
        let Some(index) = self.actions.iter().position(|entry| entry.key == key) else {
            return false;
        };
        self.actions.remove(index);
        increment_counter(&mut self.removed);
        true
    }

    pub(crate) fn pop_due(
        &mut self,
        now_ns: u64,
    ) -> Result<Option<PmDueScheduledAction>, PmScheduleError> {
        self.observe_clock(now_ns)?;
        let Some(next) = self.actions.first().copied() else {
            return Ok(None);
        };
        if next.deadline_ns > now_ns {
            return Ok(None);
        }

        let due_age_ns = now_ns - next.deadline_ns;
        self.observe_due_age(due_age_ns);
        if due_age_ns > maximum_due_age_ns() {
            return Err(PmScheduleError::Aged {
                pending: next.key,
                deadline_ns: next.deadline_ns,
                observed_ns: now_ns,
                due_age_ns,
                maximum_due_age_ns: maximum_due_age_ns(),
                action: schedule_policy().saturation_action(),
            });
        }
        let local_action_sequence = self.next_local_action_sequence;
        self.next_local_action_sequence =
            local_action_sequence.checked_add(1).ok_or_else(|| {
                self.fail_closed = true;
                PmScheduleError::LocalActionSequenceExhausted
            })?;
        self.actions.remove(0);
        increment_counter(&mut self.serviced);
        Ok(Some(PmDueScheduledAction {
            key: next.key,
            deadline_ns: next.deadline_ns,
            scheduled_at_ns: next.scheduled_at_ns,
            serviced_at_ns: now_ns,
            due_age_ns,
            local_action_sequence,
            decision_wall_timestamp_ms: next.decision_wall_timestamp_ms,
        }))
    }

    pub(crate) fn projection(
        &mut self,
        now_ns: u64,
    ) -> Result<PmScheduleProjection, PmScheduleError> {
        self.observe_clock(now_ns)?;
        let current_due_age_ns = self
            .actions
            .first()
            .map_or(0, |entry| now_ns.saturating_sub(entry.deadline_ns));
        self.observe_due_age(current_due_age_ns);
        let policy = schedule_policy();
        Ok(PmScheduleProjection {
            observed_at_ns: now_ns,
            next: self.actions.first().copied().map(ScheduledEntry::view),
            metrics: PmScheduleMetrics {
                capacity: MAX_PM_SCHEDULED_ACTIONS,
                nominal_high_water: policy.nominal_high_water(),
                depth: self.actions.len(),
                high_water: self.high_water,
                admitted: self.admitted,
                duplicate_suppressed: self.duplicate_suppressed,
                rescheduled: self.rescheduled,
                removed: self.removed,
                serviced: self.serviced,
                rejected_full: self.rejected_full,
                clock_regressions: self.clock_regressions,
                current_due_age_ns,
                maximum_due_age_ns: self.maximum_due_age_ns,
                maximum_permitted_due_age_ns: maximum_due_age_ns(),
                fail_closed: self.fail_closed,
            },
        })
    }

    fn observe_clock(&mut self, observed_ns: u64) -> Result<(), PmScheduleError> {
        if observed_ns == 0 {
            self.fail_closed = true;
            return Err(PmScheduleError::ZeroMonotonicTimestamp);
        }
        if observed_ns < self.last_observed_ns {
            increment_counter(&mut self.clock_regressions);
            self.fail_closed = true;
            return Err(PmScheduleError::ClockRegression {
                previous_ns: self.last_observed_ns,
                observed_ns,
            });
        }
        self.last_observed_ns = observed_ns;
        Ok(())
    }

    fn observe_due_age(&mut self, due_age_ns: u64) {
        self.maximum_due_age_ns = self.maximum_due_age_ns.max(due_age_ns);
        if due_age_ns > maximum_due_age_ns() {
            self.fail_closed = true;
        }
    }

    fn insert_sorted(&mut self, entry: ScheduledEntry) {
        let index = self
            .actions
            .binary_search_by(|candidate| compare_entries(candidate, &entry))
            .expect_err("a semantic duplicate is handled before sorted insertion");
        self.actions.insert(index, entry);
    }
}

const fn side_rank(side: PmOrderSide) -> u8 {
    match side {
        PmOrderSide::Buy => 0,
        PmOrderSide::Sell => 1,
    }
}

fn compare_account_scope(left: PmAccountScope, right: PmAccountScope) -> Ordering {
    left.environment()
        .cmp(&right.environment())
        .then_with(|| left.chain().cmp(&right.chain()))
        .then_with(|| left.signer().cmp(&right.signer()))
        .then_with(|| left.funder().cmp(&right.funder()))
        .then_with(|| left.handle().cmp(&right.handle()))
}

fn compare_entries(left: &ScheduledEntry, right: &ScheduledEntry) -> Ordering {
    left.deadline_ns
        .cmp(&right.deadline_ns)
        .then_with(|| left.key.cmp(&right.key))
}

const fn schedule_policy() -> PmLanePolicy {
    PmLanePolicy::for_lane(PmLaneKind::Scheduled)
}

fn maximum_due_age_ns() -> u64 {
    schedule_policy()
        .maximum_age_ns()
        .expect("scheduled lane has a finite due-age limit")
}

fn increment_counter(counter: &mut u64) {
    *counter = counter
        .checked_add(1)
        .expect("bounded scheduler observability counter overflow");
}

#[cfg(test)]
mod tests {
    use super::*;
    use reap_pm_core::{
        EvmAddress, PmChainId, PmEnvironmentId, PmFunderId, PmMarketHandle, PmSignerId,
        PmTokenHandle,
    };

    fn instrument(market: u16, token: u16) -> PmInstrumentHandle {
        PmInstrumentHandle::new(
            PmMarketHandle::from_ordinal(market),
            PmTokenHandle::from_ordinal(token),
        )
    }

    fn scope(account: u16) -> PmAccountScope {
        PmAccountScope::new(
            PmEnvironmentId::new("schedule-test").unwrap(),
            PmChainId::new(137).unwrap(),
            PmSignerId::new(EvmAddress::from_bytes([1; 20]).unwrap()),
            PmFunderId::new(EvmAddress::from_bytes([2; 20]).unwrap()),
            PmAccountHandle::from_ordinal(account),
        )
    }

    fn key(
        account: u16,
        instrument: PmInstrumentHandle,
        side: PmOrderSide,
        kind: PmScheduledActionKind,
    ) -> PmScheduledActionKey {
        PmScheduledActionKey::new(scope(account), instrument, side, kind)
    }

    #[test]
    fn duplicate_is_suppressed_and_a_changed_deadline_reschedules_in_place() {
        let instrument = instrument(1, 1);
        let action = key(
            1,
            instrument,
            PmOrderSide::Buy,
            PmScheduledActionKind::QuoteEvaluation,
        );
        let mut owner = PmQuoteScheduleRole::new(instrument);

        assert_eq!(
            owner.schedule(action, 500, 100, 1_000),
            Ok(PmScheduleAdmission::Inserted)
        );
        assert_eq!(
            owner.schedule(action, 500, 110, 1_100),
            Ok(PmScheduleAdmission::DuplicateSuppressed)
        );
        assert_eq!(
            owner.schedule(action, 300, 120, 1_200),
            Ok(PmScheduleAdmission::Rescheduled {
                previous_deadline_ns: 500
            })
        );

        let projection = owner.projection(150).unwrap();
        assert_eq!(projection.metrics().depth(), 1);
        assert_eq!(projection.metrics().high_water(), 1);
        assert_eq!(projection.metrics().admitted(), 1);
        assert_eq!(projection.metrics().duplicate_suppressed(), 1);
        assert_eq!(projection.metrics().rescheduled(), 1);
        assert_eq!(projection.next().unwrap().deadline_ns(), 300);
        assert_eq!(projection.next().unwrap().scheduled_at_ns(), 120);
        assert_eq!(
            projection.next().unwrap().decision_wall_timestamp_ms(),
            1_200
        );
    }

    #[test]
    fn equal_deadlines_are_total_and_cancel_every_owned_quote_before_replace() {
        let instrument = instrument(2, 3);
        let mut owner = PmQuoteScheduleRole::new(instrument);
        let inputs = [
            key(
                2,
                instrument,
                PmOrderSide::Sell,
                PmScheduledActionKind::QuoteEvaluation,
            ),
            key(
                2,
                instrument,
                PmOrderSide::Buy,
                PmScheduledActionKind::CancelOwnedQuote,
            ),
            key(
                1,
                instrument,
                PmOrderSide::Sell,
                PmScheduledActionKind::CancelOwnedQuote,
            ),
            key(
                1,
                instrument,
                PmOrderSide::Buy,
                PmScheduledActionKind::QuoteEvaluation,
            ),
        ];
        for (offset, action) in inputs.into_iter().enumerate() {
            owner
                .schedule(
                    action,
                    1_000,
                    100 + u64::try_from(offset).unwrap(),
                    5_000 + u64::try_from(offset).unwrap(),
                )
                .unwrap();
        }

        let mut observed = Vec::new();
        let mut expected_local_action_sequence = 1;
        while let Some(due) = owner.pop_due(1_000).unwrap() {
            observed.push(due.key());
            assert_eq!(due.deadline_ns(), 1_000);
            assert_eq!(due.serviced_at_ns(), 1_000);
            assert_eq!(due.due_age_ns(), 0);
            assert_eq!(due.local_action_sequence(), expected_local_action_sequence);
            assert!((5_000..5_004).contains(&due.decision_wall_timestamp_ms()));
            expected_local_action_sequence += 1;
        }
        assert_eq!(
            observed,
            [
                key(
                    1,
                    instrument,
                    PmOrderSide::Sell,
                    PmScheduledActionKind::CancelOwnedQuote
                ),
                key(
                    2,
                    instrument,
                    PmOrderSide::Buy,
                    PmScheduledActionKind::CancelOwnedQuote
                ),
                key(
                    1,
                    instrument,
                    PmOrderSide::Buy,
                    PmScheduledActionKind::QuoteEvaluation
                ),
                key(
                    2,
                    instrument,
                    PmOrderSide::Sell,
                    PmScheduledActionKind::QuoteEvaluation
                ),
            ]
        );
    }

    #[test]
    fn due_action_requires_real_wall_time_and_retains_the_action_on_rejection() {
        let instrument = instrument(2, 4);
        let action = key(
            1,
            instrument,
            PmOrderSide::Buy,
            PmScheduledActionKind::QuoteEvaluation,
        );
        let mut owner = PmQuoteScheduleRole::new(instrument);
        owner.schedule(action, 200, 100, 9_999).unwrap();

        assert_eq!(
            owner.schedule(action, 300, 200, 0),
            Err(PmScheduleError::ZeroWallTimestamp)
        );
        let projection = owner.projection(200).unwrap();
        assert_eq!(projection.next().unwrap().key(), action);
        assert_eq!(projection.next().unwrap().deadline_ns(), 200);
        assert_eq!(projection.metrics().depth(), 1);
        assert!(projection.metrics().fail_closed());

        let due = owner
            .pop_due(200)
            .unwrap()
            .expect("rejected action is retained");
        assert_eq!(due.local_action_sequence(), 1);
        assert_eq!(due.decision_wall_timestamp_ms(), 9_999);
    }

    #[test]
    fn deterministic_trace_replays_to_the_same_order_and_metrics() {
        fn run() -> (Vec<PmScheduledActionKey>, PmScheduleMetrics) {
            let instrument = instrument(5, 8);
            let replace_buy = key(
                3,
                instrument,
                PmOrderSide::Buy,
                PmScheduledActionKind::QuoteEvaluation,
            );
            let cancel_buy = key(
                3,
                instrument,
                PmOrderSide::Buy,
                PmScheduledActionKind::CancelOwnedQuote,
            );
            let cancel_sell = key(
                3,
                instrument,
                PmOrderSide::Sell,
                PmScheduledActionKind::CancelOwnedQuote,
            );
            let mut owner = PmQuoteScheduleRole::new(instrument);
            owner.schedule(replace_buy, 900, 100, 1_000).unwrap();
            owner.schedule(cancel_buy, 700, 110, 1_100).unwrap();
            owner.schedule(cancel_sell, 700, 120, 1_200).unwrap();
            owner.schedule(replace_buy, 800, 130, 1_300).unwrap();
            owner.schedule(cancel_sell, 700, 140, 1_400).unwrap();
            assert!(owner.remove(cancel_buy));
            owner.schedule(cancel_buy, 700, 150, 1_500).unwrap();

            let mut output = Vec::new();
            while let Some(due) = owner.pop_due(1_000).unwrap() {
                output.push(due.key());
            }
            let metrics = owner.projection(1_000).unwrap().metrics();
            (output, metrics)
        }

        assert_eq!(run(), run());
    }

    #[test]
    fn due_age_and_clock_regression_latch_fail_closed_evidence() {
        let instrument = instrument(1, 2);
        let action = key(
            1,
            instrument,
            PmOrderSide::Buy,
            PmScheduledActionKind::CancelOwnedQuote,
        );
        let mut owner = PmQuoteScheduleRole::new(instrument);
        owner.schedule(action, 200, 100, 1_000).unwrap();

        let late = owner.projection(200 + maximum_due_age_ns() + 1).unwrap();
        assert_eq!(
            late.metrics().current_due_age_ns(),
            maximum_due_age_ns() + 1
        );
        assert_eq!(
            late.metrics().maximum_due_age_ns(),
            maximum_due_age_ns() + 1
        );
        assert!(late.metrics().fail_closed());

        assert!(matches!(
            owner.projection(199),
            Err(PmScheduleError::ClockRegression { .. })
        ));
        let metrics = owner
            .projection(200 + maximum_due_age_ns() + 2)
            .unwrap()
            .metrics();
        assert_eq!(metrics.clock_regressions(), 1);
        assert!(metrics.fail_closed());
    }

    #[test]
    fn saturation_rejects_without_growth_and_latches_quote_halt() {
        let instrument = instrument(1, 1);
        let mut owner = PmQuoteScheduleRole::new(instrument);
        for account in 0..2_048_u16 {
            for side in [PmOrderSide::Buy, PmOrderSide::Sell] {
                owner
                    .schedule(
                        key(
                            account,
                            instrument,
                            side,
                            PmScheduledActionKind::CancelOwnedQuote,
                        ),
                        10_000 + u64::from(account),
                        100,
                        1_000,
                    )
                    .unwrap();
            }
        }
        assert_eq!(owner.actions.len(), MAX_PM_SCHEDULED_ACTIONS);
        assert_eq!(owner.actions.capacity(), MAX_PM_SCHEDULED_ACTIONS);

        let attempted = key(
            4_000,
            instrument,
            PmOrderSide::Buy,
            PmScheduledActionKind::QuoteEvaluation,
        );
        assert_eq!(
            owner.schedule(attempted, 20_000, 100, 1_000),
            Err(PmScheduleError::Full {
                attempted,
                capacity: MAX_PM_SCHEDULED_ACTIONS,
                action: SaturationAction::SuppressQuoteAndCancelOwned,
            })
        );
        let metrics = owner.projection(100).unwrap().metrics();
        assert_eq!(metrics.depth(), MAX_PM_SCHEDULED_ACTIONS);
        assert_eq!(metrics.high_water(), MAX_PM_SCHEDULED_ACTIONS);
        assert_eq!(metrics.rejected_full(), 1);
        assert!(metrics.fail_closed());
        assert_eq!(owner.actions.capacity(), MAX_PM_SCHEDULED_ACTIONS);
    }

    #[test]
    fn wrong_instrument_and_zero_clock_fail_closed() {
        let configured = instrument(1, 1);
        let attempted = instrument(1, 2);
        let mut owner = PmQuoteScheduleRole::new(configured);
        let action = key(
            1,
            attempted,
            PmOrderSide::Buy,
            PmScheduledActionKind::QuoteEvaluation,
        );
        assert_eq!(
            owner.schedule(action, 10, 0, 1_000),
            Err(PmScheduleError::ZeroMonotonicTimestamp)
        );
        assert_eq!(
            owner.schedule(action, 10, 1, 1_000),
            Err(PmScheduleError::InstrumentMismatch {
                configured,
                attempted,
            })
        );
        assert!(owner.projection(1).unwrap().metrics().fail_closed());
    }

    #[test]
    fn removal_is_exact_and_does_not_reallocate() {
        let instrument = instrument(9, 9);
        let keep = key(
            1,
            instrument,
            PmOrderSide::Buy,
            PmScheduledActionKind::QuoteEvaluation,
        );
        let remove = key(
            1,
            instrument,
            PmOrderSide::Buy,
            PmScheduledActionKind::CancelOwnedQuote,
        );
        let mut owner = PmQuoteScheduleRole::new(instrument);
        owner.schedule(keep, 300, 100, 1_000).unwrap();
        owner.schedule(remove, 200, 100, 1_000).unwrap();
        assert!(owner.remove(remove));
        assert!(!owner.remove(remove));
        let projection = owner.projection(100).unwrap();
        assert_eq!(projection.metrics().removed(), 1);
        assert_eq!(projection.next().unwrap().key(), keep);
        assert_eq!(owner.actions.capacity(), MAX_PM_SCHEDULED_ACTIONS);
    }
}
