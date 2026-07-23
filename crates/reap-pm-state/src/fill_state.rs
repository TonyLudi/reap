use reap_pm_core::{
    EventEnvelope, PmAssetId, PmCompleteFillQuery, PmFillEvent, PmFillExecution, PmFillFee,
    PmFillKey, PmFillQueryCursor, PmFillSettlementStatus, PmOrderSide, PmSign, PmSignedUnits, U256,
    exact_order_amounts,
};
use thiserror::Error;

use crate::private_config::PmPrivateStateConfig;
use crate::private_occurrence::PmPrivateOccurrence;

pub const MAX_PM_PRIVATE_FILLS: usize = reap_pm_core::MAX_PM_RECONCILIATION_FILLS;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PmFillFeeState {
    Unknown,
    Incomplete,
    Known {
        asset: PmAssetId,
        delta: PmSignedUnits,
    },
    UnmappedAsset {
        asset: PmAssetId,
        delta: PmSignedUnits,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PmFillProjection {
    key: PmFillKey,
    first_occurrence: PmPrivateOccurrence,
    last_occurrence: PmPrivateOccurrence,
    covered_by_reconciliation: Option<PmPrivateOccurrence>,
    fee: PmFillFeeState,
    settlement: PmFillSettlementStatus,
}

impl PmFillProjection {
    #[must_use]
    pub const fn key(self) -> PmFillKey {
        self.key
    }

    #[must_use]
    pub const fn first_occurrence(self) -> PmPrivateOccurrence {
        self.first_occurrence
    }

    #[must_use]
    pub const fn last_occurrence(self) -> PmPrivateOccurrence {
        self.last_occurrence
    }

    #[must_use]
    pub const fn covered_by_reconciliation(self) -> Option<PmPrivateOccurrence> {
        self.covered_by_reconciliation
    }

    #[must_use]
    pub const fn fee(self) -> PmFillFeeState {
        self.fee
    }

    #[must_use]
    pub const fn settlement(self) -> PmFillSettlementStatus {
        self.settlement
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PmProvisionalDeltas {
    collateral: PmSignedUnits,
    outcome: PmSignedUnits,
    uncovered_fills: u16,
    unresolved_fees: u16,
    unmapped_fee_assets: u16,
}

impl PmProvisionalDeltas {
    #[must_use]
    pub const fn collateral(self) -> PmSignedUnits {
        self.collateral
    }

    #[must_use]
    pub const fn outcome(self) -> PmSignedUnits {
        self.outcome
    }

    #[must_use]
    pub const fn uncovered_fills(self) -> u16 {
        self.uncovered_fills
    }

    #[must_use]
    pub const fn unresolved_fees(self) -> u16 {
        self.unresolved_fees
    }

    #[must_use]
    pub const fn unmapped_fee_assets(self) -> u16 {
        self.unmapped_fee_assets
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PmFillApply {
    PrincipalApplied {
        fee: PmFillFeeState,
        settlement: PmFillSettlementStatus,
    },
    Enriched {
        fee: PmFillFeeState,
        settlement: PmFillSettlementStatus,
    },
    Duplicate,
    IgnoredStale,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PmFillCounters {
    principal_applications: u64,
    fee_enrichments: u64,
    settlement_updates: u64,
    retrying_observations: u64,
    failed_observations: u64,
    duplicates: u64,
    stale: u64,
    conflicts: u64,
    capacity_failures: u64,
    reconciliation_queries: u64,
}

impl PmFillCounters {
    #[must_use]
    pub const fn principal_applications(self) -> u64 {
        self.principal_applications
    }

    #[must_use]
    pub const fn fee_enrichments(self) -> u64 {
        self.fee_enrichments
    }

    #[must_use]
    pub const fn settlement_updates(self) -> u64 {
        self.settlement_updates
    }

    #[must_use]
    pub const fn retrying_observations(self) -> u64 {
        self.retrying_observations
    }

    #[must_use]
    pub const fn failed_observations(self) -> u64 {
        self.failed_observations
    }

    #[must_use]
    pub const fn duplicates(self) -> u64 {
        self.duplicates
    }

    #[must_use]
    pub const fn conflicts(self) -> u64 {
        self.conflicts
    }

    #[must_use]
    pub const fn capacity_failures(self) -> u64 {
        self.capacity_failures
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum PmFillStateError {
    #[error("fill belongs to another configured PM source/account/instrument")]
    ScopeMismatch,
    #[error("private fill violates the locally configured tick or exact amount integrality")]
    MarketContractMismatch,
    #[error("duplicate fill identity carries conflicting principal or fee facts")]
    Conflict,
    #[error("fill principal or fee arithmetic overflowed exact state")]
    ArithmeticOverflow,
    #[error("canonical PM fill storage reached its fixed bound")]
    Capacity,
    #[error("fill-query envelope revision differs from aggregate revision")]
    EnvelopeRevisionMismatch,
    #[error("fill-query envelope ingress differs from reconciliation completion")]
    CompletionSequenceMismatch,
    #[error("fill query does not continue from the exact prior opaque watermark")]
    CursorDiscontinuity,
    #[error("complete fill query omits a locally observed pre-request fill")]
    CutDoesNotCoverObservedFill,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FillEntry {
    event: PmFillEvent,
    first_occurrence: PmPrivateOccurrence,
    last_occurrence: PmPrivateOccurrence,
    covered_by_reconciliation: Option<PmPrivateOccurrence>,
    fee: PmFillFeeState,
    covered_fee: Option<PmFillFeeState>,
    settlement: PmFillSettlementStatus,
    covered_settlement: Option<PmFillSettlementStatus>,
}

impl FillEntry {
    const fn projection(self) -> PmFillProjection {
        PmFillProjection {
            key: self.event.fill_key(),
            first_occurrence: self.first_occurrence,
            last_occurrence: self.last_occurrence,
            covered_by_reconciliation: self.covered_by_reconciliation,
            fee: self.fee,
            settlement: self.settlement,
        }
    }

    fn needs_reconciliation(self) -> bool {
        self.covered_by_reconciliation
            .is_none_or(|covered| self.last_occurrence > covered)
    }

    fn fee_needs_reconciliation(self) -> bool {
        self.covered_fee.is_none_or(|covered| self.fee != covered)
    }

    fn settlement_needs_reconciliation(self) -> bool {
        self.covered_settlement
            .is_none_or(|covered| self.settlement != covered)
    }
}

pub(crate) struct PmFillState {
    entries: Vec<FillEntry>,
    query_keys: Vec<PmFillKey>,
    provisional: PmProvisionalDeltas,
    watermark: Option<PmFillQueryCursor>,
    observed_monotonic_ns: Option<u64>,
    counters: PmFillCounters,
}

impl PmFillState {
    pub(crate) fn new() -> Self {
        Self {
            entries: Vec::with_capacity(MAX_PM_PRIVATE_FILLS),
            query_keys: Vec::with_capacity(MAX_PM_PRIVATE_FILLS),
            provisional: empty_deltas(),
            watermark: None,
            observed_monotonic_ns: None,
            counters: PmFillCounters::default(),
        }
    }

    pub(crate) fn observe(
        &mut self,
        envelope: EventEnvelope<PmFillEvent>,
        config: &PmPrivateStateConfig,
    ) -> Result<PmFillApply, PmFillStateError> {
        let event = *envelope.payload();
        validate_fill(event, envelope.source(), config)?;
        let occurrence = PmPrivateOccurrence::new(
            envelope.ordering().connection_epoch(),
            envelope.ordering().local_ingress_sequence(),
        );
        self.observed_monotonic_ns = Some(envelope.clock().monotonic_service_ns());
        match self.search(event.fill_key()) {
            Ok(index) => self.enrich(index, event, occurrence, config),
            Err(index) => self.insert_observed(index, event, occurrence, config),
        }
    }

    pub(crate) fn preflight_query(
        &mut self,
        envelope: &EventEnvelope<PmCompleteFillQuery>,
        config: &PmPrivateStateConfig,
    ) -> Result<(), PmFillStateError> {
        validate_query_envelope(envelope, config)?;
        let query = envelope.payload();
        self.query_keys.clear();
        self.query_keys
            .extend(query.fills().iter().map(|fill| fill.fill_key()));
        self.query_keys.sort_unstable();
        if query.requested_after() != self.watermark {
            return Err(PmFillStateError::CursorDiscontinuity);
        }
        let request = PmPrivateOccurrence::new(
            envelope.ordering().connection_epoch(),
            query.boundary().request_sequence(),
        );
        let new = query
            .fills()
            .iter()
            .filter(|fill| self.search(fill.fill_key()).is_err())
            .count();
        if self.entries.len().saturating_add(new) > MAX_PM_PRIVATE_FILLS {
            self.counters.capacity_failures = self.counters.capacity_failures.saturating_add(1);
            return Err(PmFillStateError::Capacity);
        }
        for fill in query.fills().iter().copied() {
            validate_fill(fill, envelope.source(), config)?;
            if let Ok(index) = self.search(fill.fill_key()) {
                let entry = self.entries[index];
                validate_same_principal(entry.event, fill)?;
                if entry.last_occurrence > request {
                    validate_historical_fill_compatibility(fill, entry.event, config)?;
                } else {
                    validate_fee_progression(entry.fee, fill.execution().fee(), config)?;
                    settlement_transition(entry.settlement, fill.execution().settlement())?;
                }
            }
        }
        if self.entries.iter().any(|entry| {
            entry.needs_reconciliation()
                && entry.last_occurrence <= request
                && self
                    .query_keys
                    .binary_search(&entry.event.fill_key())
                    .is_err()
        }) {
            return Err(PmFillStateError::CutDoesNotCoverObservedFill);
        }
        let mut prospective = empty_deltas();
        for entry in self.entries.iter().filter(|entry| {
            entry.needs_reconciliation()
                && (entry.last_occurrence > request
                    || self
                        .query_keys
                        .binary_search(&entry.event.fill_key())
                        .is_err())
        }) {
            prospective = add_delta(prospective, entry_provisional_delta(*entry, config)?)?;
        }
        Ok(())
    }

    pub(crate) fn apply_preflighted_query(
        &mut self,
        envelope: EventEnvelope<PmCompleteFillQuery>,
        config: &PmPrivateStateConfig,
    ) -> Result<(), PmFillStateError> {
        let query = envelope.payload();
        let epoch = envelope.ordering().connection_epoch();
        let request = PmPrivateOccurrence::new(epoch, query.boundary().request_sequence());
        let completion = PmPrivateOccurrence::new(epoch, query.boundary().completion_sequence());
        for fill in query.fills().iter().copied() {
            match self.search(fill.fill_key()) {
                Ok(index) => {
                    let entry = &mut self.entries[index];
                    if entry.last_occurrence > request {
                        continue;
                    }
                    entry.event = merge_event(entry.event, fill, config)?;
                    entry.fee = fee_state(entry.event.execution().fee(), config);
                    entry.settlement = entry.event.execution().settlement();
                    entry.covered_by_reconciliation = Some(completion);
                    entry.covered_fee = Some(entry.fee);
                    entry.covered_settlement = Some(entry.settlement);
                    entry.last_occurrence = completion;
                }
                Err(index) => {
                    self.entries.insert(
                        index,
                        FillEntry {
                            event: fill,
                            first_occurrence: request,
                            last_occurrence: completion,
                            covered_by_reconciliation: Some(completion),
                            fee: fee_state(fill.execution().fee(), config),
                            covered_fee: Some(fee_state(fill.execution().fee(), config)),
                            settlement: fill.execution().settlement(),
                            covered_settlement: Some(fill.execution().settlement()),
                        },
                    );
                }
            }
        }
        self.watermark = Some(query.resulting_watermark());
        self.observed_monotonic_ns = Some(envelope.clock().monotonic_service_ns());
        self.provisional = recompute_provisional(&self.entries, config)?;
        self.counters.reconciliation_queries =
            self.counters.reconciliation_queries.saturating_add(1);
        Ok(())
    }

    pub(crate) fn projections(&self) -> impl Iterator<Item = PmFillProjection> + '_ {
        self.entries.iter().copied().map(FillEntry::projection)
    }

    pub(crate) const fn provisional(&self) -> PmProvisionalDeltas {
        self.provisional
    }

    pub(crate) const fn watermark(&self) -> Option<PmFillQueryCursor> {
        self.watermark
    }

    pub(crate) const fn observed_monotonic_ns(&self) -> Option<u64> {
        self.observed_monotonic_ns
    }

    pub(crate) const fn counters(&self) -> PmFillCounters {
        self.counters
    }

    pub(crate) fn unresolved_count(&self) -> u16 {
        u16::try_from(
            self.entries
                .iter()
                .filter(|entry| {
                    (entry.fee_needs_reconciliation()
                        && matches!(
                            entry.fee,
                            PmFillFeeState::Unknown
                                | PmFillFeeState::Incomplete
                                | PmFillFeeState::UnmappedAsset { .. }
                        ))
                        || (entry.settlement_needs_reconciliation()
                            && matches!(
                                entry.settlement,
                                PmFillSettlementStatus::Retrying | PmFillSettlementStatus::Failed
                            ))
                })
                .count(),
        )
        .expect("fill capacity fits u16")
    }

    pub(crate) fn first_unresolved_fee(&self) -> Option<(PmFillKey, PmFillFeeState)> {
        self.entries
            .iter()
            .filter(|entry| entry.fee_needs_reconciliation())
            .find_map(|entry| match entry.fee {
                PmFillFeeState::Unknown
                | PmFillFeeState::Incomplete
                | PmFillFeeState::UnmappedAsset { .. } => Some((entry.event.fill_key(), entry.fee)),
                PmFillFeeState::Known { .. } => None,
            })
    }

    pub(crate) fn first_unresolved_settlement(
        &self,
    ) -> Option<(PmFillKey, PmFillSettlementStatus)> {
        self.entries
            .iter()
            .filter(|entry| entry.settlement_needs_reconciliation())
            .find_map(|entry| {
                matches!(
                    entry.settlement,
                    PmFillSettlementStatus::Retrying | PmFillSettlementStatus::Failed
                )
                .then_some((entry.event.fill_key(), entry.settlement))
            })
    }

    fn search(&self, key: PmFillKey) -> Result<usize, usize> {
        self.entries
            .binary_search_by_key(&key, |entry| entry.event.fill_key())
    }

    fn insert_observed(
        &mut self,
        index: usize,
        event: PmFillEvent,
        occurrence: PmPrivateOccurrence,
        config: &PmPrivateStateConfig,
    ) -> Result<PmFillApply, PmFillStateError> {
        if self.entries.len() == MAX_PM_PRIVATE_FILLS {
            self.counters.capacity_failures = self.counters.capacity_failures.saturating_add(1);
            return Err(PmFillStateError::Capacity);
        }
        let fee = fee_state(event.execution().fee(), config);
        let settlement = event.execution().settlement();
        let delta = event_delta(event, fee, config)?;
        let next = add_delta(self.provisional, delta)?;
        self.entries.insert(
            index,
            FillEntry {
                event,
                first_occurrence: occurrence,
                last_occurrence: occurrence,
                covered_by_reconciliation: None,
                fee,
                covered_fee: None,
                settlement,
                covered_settlement: None,
            },
        );
        self.provisional = next;
        self.counters.principal_applications =
            self.counters.principal_applications.saturating_add(1);
        self.count_settlement_observation(settlement);
        Ok(PmFillApply::PrincipalApplied { fee, settlement })
    }

    fn enrich(
        &mut self,
        index: usize,
        event: PmFillEvent,
        occurrence: PmPrivateOccurrence,
        config: &PmPrivateStateConfig,
    ) -> Result<PmFillApply, PmFillStateError> {
        let prior = self.entries[index];
        if occurrence < prior.last_occurrence {
            self.counters.stale = self.counters.stale.saturating_add(1);
            return Ok(PmFillApply::IgnoredStale);
        }
        if occurrence == prior.last_occurrence {
            if event == prior.event {
                self.counters.duplicates = self.counters.duplicates.saturating_add(1);
                return Ok(PmFillApply::Duplicate);
            }
            self.counters.conflicts = self.counters.conflicts.saturating_add(1);
            return Err(PmFillStateError::Conflict);
        }
        validate_same_principal(prior.event, event).inspect_err(|_| {
            self.counters.conflicts = self.counters.conflicts.saturating_add(1);
        })?;
        let incoming = fee_state(event.execution().fee(), config);
        let fee_update = fee_transition(prior.fee, incoming).inspect_err(|_| {
            self.counters.conflicts = self.counters.conflicts.saturating_add(1);
        })?;
        let settlement_update =
            settlement_transition(prior.settlement, event.execution().settlement()).inspect_err(
                |_| {
                    self.counters.conflicts = self.counters.conflicts.saturating_add(1);
                },
            )?;
        if fee_update.is_none() && settlement_update.is_none() {
            self.counters.duplicates = self.counters.duplicates.saturating_add(1);
            return Ok(PmFillApply::Duplicate);
        }
        let next_fee = fee_update.unwrap_or(prior.fee);
        let next_settlement = settlement_update.unwrap_or(prior.settlement);
        let entry = &mut self.entries[index];
        entry.event = merge_event(prior.event, event, config)?;
        entry.last_occurrence = occurrence;
        entry.fee = next_fee;
        entry.settlement = next_settlement;
        self.provisional = recompute_provisional(&self.entries, config)?;
        if fee_update.is_some() {
            self.counters.fee_enrichments = self.counters.fee_enrichments.saturating_add(1);
        }
        if settlement_update.is_some() {
            self.counters.settlement_updates = self.counters.settlement_updates.saturating_add(1);
            self.count_settlement_observation(next_settlement);
        }
        Ok(PmFillApply::Enriched {
            fee: next_fee,
            settlement: next_settlement,
        })
    }

    fn count_settlement_observation(&mut self, settlement: PmFillSettlementStatus) {
        match settlement {
            PmFillSettlementStatus::Retrying => {
                self.counters.retrying_observations =
                    self.counters.retrying_observations.saturating_add(1);
            }
            PmFillSettlementStatus::Failed => {
                self.counters.failed_observations =
                    self.counters.failed_observations.saturating_add(1);
            }
            PmFillSettlementStatus::Matched
            | PmFillSettlementStatus::Mined
            | PmFillSettlementStatus::Confirmed => {}
        }
    }
}

fn validate_fill(
    fill: PmFillEvent,
    source: reap_pm_core::PmProductSource,
    config: &PmPrivateStateConfig,
) -> Result<(), PmFillStateError> {
    if source != config.source()
        || fill.source() != config.source()
        || fill.account() != config.account()
        || fill.instrument() != config.instrument()
    {
        return Err(PmFillStateError::ScopeMismatch);
    }
    fill.execution()
        .price()
        .validate_tick(config.tick())
        .and_then(|_| {
            exact_order_amounts(
                fill.execution().side(),
                fill.execution().price(),
                fill.execution().quantity(),
            )
            .map(|_| fill.execution().quantity())
        })
        .map_err(|_| PmFillStateError::MarketContractMismatch)?;
    Ok(())
}

fn validate_query_envelope(
    envelope: &EventEnvelope<PmCompleteFillQuery>,
    config: &PmPrivateStateConfig,
) -> Result<(), PmFillStateError> {
    let query = envelope.payload();
    if envelope.source() != config.source() || query.account_scope() != config.account_scope() {
        return Err(PmFillStateError::ScopeMismatch);
    }
    if envelope.ordering().snapshot_revision() != Some(query.snapshot().revision()) {
        return Err(PmFillStateError::EnvelopeRevisionMismatch);
    }
    if envelope.ordering().local_ingress_sequence() != query.boundary().completion_sequence() {
        return Err(PmFillStateError::CompletionSequenceMismatch);
    }
    Ok(())
}

fn validate_same_principal(
    prior: PmFillEvent,
    incoming: PmFillEvent,
) -> Result<(), PmFillStateError> {
    let left = prior.execution();
    let right = incoming.execution();
    if prior.source() != incoming.source()
        || prior.instrument() != incoming.instrument()
        || prior.order() != incoming.order()
        || left.side() != right.side()
        || left.role() != right.role()
        || left.price() != right.price()
        || left.quantity() != right.quantity()
    {
        Err(PmFillStateError::Conflict)
    } else {
        Ok(())
    }
}

fn validate_fee_progression(
    current: PmFillFeeState,
    incoming: PmFillFee,
    config: &PmPrivateStateConfig,
) -> Result<(), PmFillStateError> {
    fee_transition(current, fee_state(incoming, config)).map(|_| ())
}

fn validate_historical_fill_compatibility(
    historical: PmFillEvent,
    later: PmFillEvent,
    config: &PmPrivateStateConfig,
) -> Result<(), PmFillStateError> {
    let historical_fee = fee_state(historical.execution().fee(), config);
    let later_fee = fee_state(later.execution().fee(), config);
    if fee_transition(historical_fee, later_fee).is_err()
        && fee_transition(later_fee, historical_fee).is_err()
    {
        return Err(PmFillStateError::Conflict);
    }
    if !settlement_reachable(
        historical.execution().settlement(),
        later.execution().settlement(),
    ) {
        return Err(PmFillStateError::Conflict);
    }
    Ok(())
}

fn fee_transition(
    current: PmFillFeeState,
    incoming: PmFillFeeState,
) -> Result<Option<PmFillFeeState>, PmFillStateError> {
    use PmFillFeeState::{Incomplete, Known, Unknown, UnmappedAsset};
    match (current, incoming) {
        (Known { .. }, Unknown | Incomplete) | (UnmappedAsset { .. }, Unknown | Incomplete) => {
            Ok(None)
        }
        (Unknown | Incomplete, Known { .. } | UnmappedAsset { .. }) => Ok(Some(incoming)),
        (Unknown, Incomplete)
        | (Incomplete, Unknown)
        | (Unknown, Unknown)
        | (Incomplete, Incomplete) => Ok(None),
        (Known { .. }, Known { .. }) | (UnmappedAsset { .. }, UnmappedAsset { .. })
            if current == incoming =>
        {
            Ok(None)
        }
        (Known { .. }, Known { .. })
        | (Known { .. }, UnmappedAsset { .. })
        | (UnmappedAsset { .. }, Known { .. })
        | (UnmappedAsset { .. }, UnmappedAsset { .. }) => Err(PmFillStateError::Conflict),
    }
}

fn fee_state(fee: PmFillFee, config: &PmPrivateStateConfig) -> PmFillFeeState {
    match fee {
        PmFillFee::Unknown => PmFillFeeState::Unknown,
        PmFillFee::Incomplete => PmFillFeeState::Incomplete,
        PmFillFee::Known { asset, delta }
            if asset == config.collateral_asset() || asset == config.outcome_asset() =>
        {
            PmFillFeeState::Known { asset, delta }
        }
        PmFillFee::Known { asset, delta } => PmFillFeeState::UnmappedAsset { asset, delta },
    }
}

pub(crate) fn settlement_transition(
    current: PmFillSettlementStatus,
    incoming: PmFillSettlementStatus,
) -> Result<Option<PmFillSettlementStatus>, PmFillStateError> {
    use PmFillSettlementStatus::{Confirmed, Failed, Matched, Mined, Retrying};

    match (current, incoming) {
        (Matched, Matched)
        | (Mined, Mined)
        | (Confirmed, Confirmed)
        | (Retrying, Retrying)
        | (Failed, Failed) => Ok(None),
        (Matched, Mined | Retrying)
        | (Mined, Confirmed | Retrying)
        | (Retrying, Mined | Failed) => Ok(Some(incoming)),
        (Matched, Confirmed | Failed)
        | (Mined, Matched | Failed)
        | (Confirmed, Matched | Mined | Retrying | Failed)
        | (Retrying, Matched | Confirmed)
        | (Failed, Matched | Mined | Confirmed | Retrying) => Err(PmFillStateError::Conflict),
    }
}

fn settlement_reachable(earlier: PmFillSettlementStatus, later: PmFillSettlementStatus) -> bool {
    use PmFillSettlementStatus::{Confirmed, Failed, Matched, Mined, Retrying};

    matches!(
        (earlier, later),
        (Matched, Matched | Mined | Confirmed | Retrying | Failed)
            | (Mined, Mined | Confirmed | Retrying | Failed)
            | (Retrying, Retrying | Mined | Confirmed | Failed)
            | (Confirmed, Confirmed)
            | (Failed, Failed)
    )
}

fn merge_event(
    prior: PmFillEvent,
    incoming: PmFillEvent,
    config: &PmPrivateStateConfig,
) -> Result<PmFillEvent, PmFillStateError> {
    let prior_execution = prior.execution();
    let incoming_execution = incoming.execution();
    let fee = match fee_transition(
        fee_state(prior_execution.fee(), config),
        fee_state(incoming_execution.fee(), config),
    )? {
        Some(_) => incoming_execution.fee(),
        None => prior_execution.fee(),
    };
    let settlement = settlement_transition(
        prior_execution.settlement(),
        incoming_execution.settlement(),
    )?
    .unwrap_or(prior_execution.settlement());
    PmFillEvent::new(
        prior.source(),
        prior.instrument(),
        prior.fill_key(),
        prior.order(),
        PmFillExecution::new(
            prior_execution.side(),
            prior_execution.role(),
            settlement,
            prior_execution.price(),
            prior_execution.quantity(),
            fee,
        ),
    )
    .map_err(|_| PmFillStateError::Conflict)
}

fn event_delta(
    event: PmFillEvent,
    fee: PmFillFeeState,
    config: &PmPrivateStateConfig,
) -> Result<PmProvisionalDeltas, PmFillStateError> {
    let execution = event.execution();
    let amounts = exact_order_amounts(execution.side(), execution.price(), execution.quantity())
        .map_err(|_| PmFillStateError::ArithmeticOverflow)?;
    let quantity = execution.quantity().protocol_units();
    let collateral_principal = match execution.side() {
        PmOrderSide::Buy => signed(PmSign::Negative, amounts.maker())?,
        PmOrderSide::Sell => signed(PmSign::Positive, amounts.taker())?,
    };
    let outcome_principal = match execution.side() {
        PmOrderSide::Buy => signed(PmSign::Positive, quantity)?,
        PmOrderSide::Sell => signed(PmSign::Negative, quantity)?,
    };
    let mut delta = PmProvisionalDeltas {
        collateral: collateral_principal,
        outcome: outcome_principal,
        uncovered_fills: 1,
        unresolved_fees: u16::from(matches!(
            fee,
            PmFillFeeState::Unknown | PmFillFeeState::Incomplete
        )),
        unmapped_fee_assets: u16::from(matches!(fee, PmFillFeeState::UnmappedAsset { .. })),
    };
    delta = add_delta(delta, fee_only_delta(fee, config))?;
    Ok(delta)
}

fn fee_only_delta(fee: PmFillFeeState, config: &PmPrivateStateConfig) -> PmProvisionalDeltas {
    let mut delta = empty_deltas();
    match fee {
        PmFillFeeState::Known { asset, delta: fee } if asset == config.collateral_asset() => {
            delta.collateral = fee;
        }
        PmFillFeeState::Known { asset, delta: fee } if asset == config.outcome_asset() => {
            delta.outcome = fee;
        }
        PmFillFeeState::Unknown | PmFillFeeState::Incomplete => {}
        PmFillFeeState::UnmappedAsset { .. } | PmFillFeeState::Known { .. } => {}
    }
    delta
}

fn recompute_provisional(
    entries: &[FillEntry],
    config: &PmPrivateStateConfig,
) -> Result<PmProvisionalDeltas, PmFillStateError> {
    let mut total = empty_deltas();
    for entry in entries
        .iter()
        .copied()
        .filter(|entry| entry.needs_reconciliation())
    {
        total = add_delta(total, entry_provisional_delta(entry, config)?)?;
    }
    Ok(total)
}

fn entry_provisional_delta(
    entry: FillEntry,
    config: &PmPrivateStateConfig,
) -> Result<PmProvisionalDeltas, PmFillStateError> {
    if entry.covered_by_reconciliation.is_none() {
        return event_delta(entry.event, entry.fee, config);
    }
    let mut delta = empty_deltas();
    delta.uncovered_fills = 1;
    if entry.fee_needs_reconciliation() {
        delta = add_delta(delta, fee_only_delta(entry.fee, config))?;
        delta.unresolved_fees = u16::from(matches!(
            entry.fee,
            PmFillFeeState::Unknown | PmFillFeeState::Incomplete
        ));
        delta.unmapped_fee_assets =
            u16::from(matches!(entry.fee, PmFillFeeState::UnmappedAsset { .. }));
    }
    Ok(delta)
}

fn empty_deltas() -> PmProvisionalDeltas {
    PmProvisionalDeltas {
        collateral: PmSignedUnits::ZERO,
        outcome: PmSignedUnits::ZERO,
        uncovered_fills: 0,
        unresolved_fees: 0,
        unmapped_fee_assets: 0,
    }
}

fn add_delta(
    left: PmProvisionalDeltas,
    right: PmProvisionalDeltas,
) -> Result<PmProvisionalDeltas, PmFillStateError> {
    Ok(PmProvisionalDeltas {
        collateral: add_signed(left.collateral, right.collateral)?,
        outcome: add_signed(left.outcome, right.outcome)?,
        uncovered_fills: left
            .uncovered_fills
            .checked_add(right.uncovered_fills)
            .ok_or(PmFillStateError::ArithmeticOverflow)?,
        unresolved_fees: left
            .unresolved_fees
            .checked_add(right.unresolved_fees)
            .ok_or(PmFillStateError::ArithmeticOverflow)?,
        unmapped_fee_assets: left
            .unmapped_fee_assets
            .checked_add(right.unmapped_fee_assets)
            .ok_or(PmFillStateError::ArithmeticOverflow)?,
    })
}

fn signed(sign: PmSign, amount: U256) -> Result<PmSignedUnits, PmFillStateError> {
    PmSignedUnits::from_parts(sign, amount).map_err(|_| PmFillStateError::ArithmeticOverflow)
}

pub(crate) fn add_signed(
    left: PmSignedUnits,
    right: PmSignedUnits,
) -> Result<PmSignedUnits, PmFillStateError> {
    if left.sign() == right.sign() {
        return signed(
            left.sign(),
            left.magnitude()
                .checked_add(right.magnitude())
                .map_err(|_| PmFillStateError::ArithmeticOverflow)?,
        );
    }
    match left.magnitude().cmp(&right.magnitude()) {
        std::cmp::Ordering::Greater => signed(
            left.sign(),
            left.magnitude()
                .checked_sub(right.magnitude())
                .map_err(|_| PmFillStateError::ArithmeticOverflow)?,
        ),
        std::cmp::Ordering::Less => signed(
            right.sign(),
            right
                .magnitude()
                .checked_sub(left.magnitude())
                .map_err(|_| PmFillStateError::ArithmeticOverflow)?,
        ),
        std::cmp::Ordering::Equal => Ok(PmSignedUnits::ZERO),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settlement_transition_graph_is_exact() {
        use PmFillSettlementStatus::{Confirmed, Failed, Matched, Mined, Retrying};

        let statuses = [Matched, Mined, Confirmed, Retrying, Failed];
        for current in statuses {
            for incoming in statuses {
                let transition = settlement_transition(current, incoming);
                if current == incoming {
                    assert_eq!(transition, Ok(None));
                    continue;
                }
                let allowed = matches!(
                    (current, incoming),
                    (Matched, Mined | Retrying)
                        | (Mined, Confirmed | Retrying)
                        | (Retrying, Mined | Failed)
                );
                if allowed {
                    assert_eq!(transition, Ok(Some(incoming)));
                } else {
                    assert_eq!(transition, Err(PmFillStateError::Conflict));
                }
            }
        }
    }
}
