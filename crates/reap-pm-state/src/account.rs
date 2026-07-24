use reap_pm_core::{
    EventEnvelope, MAX_PM_ACCOUNT_DIAGNOSTIC_EXTRA_ROWS, PmAllowanceEvent, PmAllowanceValue,
    PmAssetId, PmBalanceEvent, PmCompleteAccountSnapshot, PmPositionAvailability, PmPositionEvent,
    PmProductSource, PmSignedUnits, PmSnapshotEvidence, PmSpenderId, SnapshotRevision, U256,
};
use thiserror::Error;

use crate::private_config::PmPrivateStateConfig;
use crate::private_occurrence::PmPrivateOccurrence;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PmObservedAmount {
    Unavailable,
    ExplicitAbsent,
    Present(U256),
}

impl PmObservedAmount {
    #[must_use]
    pub const fn value(self) -> Option<U256> {
        match self {
            Self::Unavailable => None,
            Self::ExplicitAbsent => Some(U256::ZERO),
            Self::Present(value) => Some(value),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PmAllowanceKnowledge {
    Unconfigured,
    Unavailable,
    ExplicitAbsent,
    Present(PmAllowanceValue),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PmPositionKnowledge {
    Unavailable,
    ExplicitAbsent,
    Tradable(U256),
    ResolvedUnredeemed(U256),
    VenueUnavailable(U256),
}

impl PmPositionKnowledge {
    #[must_use]
    pub const fn tradable_units(self) -> Option<U256> {
        match self {
            Self::ExplicitAbsent => Some(U256::ZERO),
            Self::Tradable(quantity) => Some(quantity),
            Self::Unavailable | Self::ResolvedUnredeemed(_) | Self::VenueUnavailable(_) => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PmAccountSnapshotProjection {
    source: Option<PmProductSource>,
    snapshot: Option<PmSnapshotEvidence>,
    completion: Option<PmPrivateOccurrence>,
    local_wall_receive_ns: Option<u64>,
    monotonic_receive_ns: Option<u64>,
    monotonic_service_ns: Option<u64>,
    collateral: PmObservedAmount,
    outcome_balance: PmObservedAmount,
    position: PmPositionKnowledge,
    unknown_balance_rows: u8,
    unknown_allowance_rows: u8,
    unknown_position_rows: u8,
}

impl PmAccountSnapshotProjection {
    #[must_use]
    pub const fn source(self) -> Option<PmProductSource> {
        self.source
    }

    #[must_use]
    pub const fn snapshot(self) -> Option<PmSnapshotEvidence> {
        self.snapshot
    }

    #[must_use]
    pub const fn completion(self) -> Option<PmPrivateOccurrence> {
        self.completion
    }

    #[must_use]
    pub const fn local_wall_receive_ns(self) -> Option<u64> {
        self.local_wall_receive_ns
    }

    #[must_use]
    pub const fn monotonic_receive_ns(self) -> Option<u64> {
        self.monotonic_receive_ns
    }

    #[must_use]
    pub const fn monotonic_service_ns(self) -> Option<u64> {
        self.monotonic_service_ns
    }

    #[must_use]
    pub const fn collateral(self) -> PmObservedAmount {
        self.collateral
    }

    #[must_use]
    pub const fn outcome_balance(self) -> PmObservedAmount {
        self.outcome_balance
    }

    #[must_use]
    pub const fn position(self) -> PmPositionKnowledge {
        self.position
    }

    #[must_use]
    pub const fn unknown_balance_rows(self) -> u8 {
        self.unknown_balance_rows
    }

    #[must_use]
    pub const fn unknown_allowance_rows(self) -> u8 {
        self.unknown_allowance_rows
    }

    #[must_use]
    pub const fn unknown_position_rows(self) -> u8 {
        self.unknown_position_rows
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PmAccountSnapshotApply {
    Applied {
        revision: SnapshotRevision,
        explicit_absences: u8,
    },
    Duplicate {
        revision: SnapshotRevision,
    },
    IgnoredStale {
        received: SnapshotRevision,
        current: SnapshotRevision,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PmAccountCounters {
    applied: u64,
    duplicates: u64,
    stale: u64,
    contract_violations: u64,
    unknown_rows: u64,
}

impl PmAccountCounters {
    #[must_use]
    pub const fn applied(self) -> u64 {
        self.applied
    }

    #[must_use]
    pub const fn duplicates(self) -> u64 {
        self.duplicates
    }

    #[must_use]
    pub const fn stale(self) -> u64 {
        self.stale
    }

    #[must_use]
    pub const fn contract_violations(self) -> u64 {
        self.contract_violations
    }

    #[must_use]
    pub const fn unknown_rows(self) -> u64 {
        self.unknown_rows
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum PmAccountStateError {
    #[error("complete account snapshot source or account differs from local configuration")]
    ScopeMismatch,
    #[error("complete account snapshot declared expected scopes differ from local configuration")]
    ExpectedScopeMismatch,
    #[error("account snapshot envelope revision differs from its aggregate revision")]
    EnvelopeRevisionMismatch,
    #[error("account snapshot envelope ingress is not its reconciliation completion")]
    CompletionSequenceMismatch,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct AllowanceEntry {
    spender: PmSpenderId,
    knowledge: PmAllowanceKnowledge,
}

pub(crate) struct PmAccountState {
    source: Option<PmProductSource>,
    snapshot: Option<PmSnapshotEvidence>,
    completion: Option<PmPrivateOccurrence>,
    local_wall_receive_ns: Option<u64>,
    monotonic_receive_ns: Option<u64>,
    monotonic_service_ns: Option<u64>,
    collateral: PmObservedAmount,
    outcome_balance: PmObservedAmount,
    position: PmPositionKnowledge,
    allowances: Vec<AllowanceEntry>,
    diagnostic_balances: Vec<PmBalanceEvent>,
    diagnostic_allowances: Vec<PmAllowanceEvent>,
    diagnostic_positions: Vec<PmPositionEvent>,
    unknown_balance_rows: u8,
    unknown_allowance_rows: u8,
    unknown_position_rows: u8,
    counters: PmAccountCounters,
}

impl PmAccountState {
    pub(crate) fn new(config: &PmPrivateStateConfig) -> Self {
        let mut allowances = Vec::with_capacity(config.required_spenders().len());
        allowances.extend(config.required_spenders().iter().copied().map(|spender| {
            AllowanceEntry {
                spender,
                knowledge: PmAllowanceKnowledge::Unavailable,
            }
        }));
        Self {
            source: None,
            snapshot: None,
            completion: None,
            local_wall_receive_ns: None,
            monotonic_receive_ns: None,
            monotonic_service_ns: None,
            collateral: PmObservedAmount::Unavailable,
            outcome_balance: PmObservedAmount::Unavailable,
            position: PmPositionKnowledge::Unavailable,
            allowances,
            diagnostic_balances: Vec::with_capacity(MAX_PM_ACCOUNT_DIAGNOSTIC_EXTRA_ROWS),
            diagnostic_allowances: Vec::with_capacity(MAX_PM_ACCOUNT_DIAGNOSTIC_EXTRA_ROWS),
            diagnostic_positions: Vec::with_capacity(MAX_PM_ACCOUNT_DIAGNOSTIC_EXTRA_ROWS),
            unknown_balance_rows: 0,
            unknown_allowance_rows: 0,
            unknown_position_rows: 0,
            counters: PmAccountCounters::default(),
        }
    }

    pub(crate) fn reserved_capacity_bytes(&self) -> usize {
        self.allowances
            .capacity()
            .saturating_mul(std::mem::size_of::<AllowanceEntry>())
            .saturating_add(
                self.diagnostic_balances
                    .capacity()
                    .saturating_mul(std::mem::size_of::<PmBalanceEvent>()),
            )
            .saturating_add(
                self.diagnostic_allowances
                    .capacity()
                    .saturating_mul(std::mem::size_of::<PmAllowanceEvent>()),
            )
            .saturating_add(
                self.diagnostic_positions
                    .capacity()
                    .saturating_mul(std::mem::size_of::<PmPositionEvent>()),
            )
    }

    pub(crate) fn apply(
        &mut self,
        envelope: &EventEnvelope<PmCompleteAccountSnapshot>,
        config: &PmPrivateStateConfig,
    ) -> Result<PmAccountSnapshotApply, PmAccountStateError> {
        let snapshot = envelope.payload();
        let preview = match self.preview(envelope, config) {
            Ok(preview) => preview,
            Err(error) => {
                self.counters.contract_violations =
                    self.counters.contract_violations.saturating_add(1);
                return Err(error);
            }
        };
        if !matches!(preview, PmAccountSnapshotApply::Applied { .. }) {
            self.count_non_applied(preview);
            return Ok(preview);
        }
        let revision = snapshot.snapshot().revision();
        let collateral = observed_balance(snapshot, config.collateral_asset());
        let outcome_balance = observed_balance(snapshot, config.outcome_asset());
        let position = observed_position(snapshot, config.instrument());
        let explicit_absences =
            count_absences(collateral, outcome_balance, position, snapshot, config);
        self.replace_allowances(snapshot);
        self.replace_diagnostics(snapshot, config);
        self.source = Some(envelope.source());
        self.snapshot = Some(snapshot.snapshot());
        self.completion = Some(PmPrivateOccurrence::new(
            envelope.ordering().connection_epoch(),
            snapshot.boundary().completion_sequence(),
        ));
        self.local_wall_receive_ns = Some(envelope.clock().local_wall_receive_ns());
        self.monotonic_receive_ns = Some(envelope.clock().monotonic_receive_ns());
        self.monotonic_service_ns = Some(envelope.clock().monotonic_service_ns());
        self.collateral = collateral;
        self.outcome_balance = outcome_balance;
        self.position = position;
        self.unknown_balance_rows =
            u8::try_from(self.diagnostic_balances.len()).expect("core account row bound");
        self.unknown_allowance_rows =
            u8::try_from(self.diagnostic_allowances.len()).expect("core account row bound");
        self.unknown_position_rows =
            u8::try_from(self.diagnostic_positions.len()).expect("core account row bound");
        let unknown = u64::from(self.unknown_balance_rows)
            + u64::from(self.unknown_allowance_rows)
            + u64::from(self.unknown_position_rows);
        self.counters.applied = self.counters.applied.saturating_add(1);
        self.counters.unknown_rows = self.counters.unknown_rows.saturating_add(unknown);
        Ok(PmAccountSnapshotApply::Applied {
            revision,
            explicit_absences,
        })
    }

    pub(crate) fn allowance(&self, spender: PmSpenderId) -> PmAllowanceKnowledge {
        self.allowances
            .iter()
            .find(|entry| entry.spender == spender)
            .map_or(PmAllowanceKnowledge::Unconfigured, |entry| entry.knowledge)
    }

    pub(crate) const fn projection(&self) -> PmAccountSnapshotProjection {
        PmAccountSnapshotProjection {
            source: self.source,
            snapshot: self.snapshot,
            completion: self.completion,
            local_wall_receive_ns: self.local_wall_receive_ns,
            monotonic_receive_ns: self.monotonic_receive_ns,
            monotonic_service_ns: self.monotonic_service_ns,
            collateral: self.collateral,
            outcome_balance: self.outcome_balance,
            position: self.position,
            unknown_balance_rows: self.unknown_balance_rows,
            unknown_allowance_rows: self.unknown_allowance_rows,
            unknown_position_rows: self.unknown_position_rows,
        }
    }

    pub(crate) const fn counters(&self) -> PmAccountCounters {
        self.counters
    }

    pub(crate) const fn observed_monotonic_ns(&self) -> Option<u64> {
        self.monotonic_service_ns
    }

    pub(crate) const fn collateral(&self) -> PmObservedAmount {
        self.collateral
    }

    pub(crate) const fn outcome_balance(&self) -> PmObservedAmount {
        self.outcome_balance
    }

    pub(crate) const fn position(&self) -> PmPositionKnowledge {
        self.position
    }

    pub(crate) fn diagnostic_balances(&self) -> impl Iterator<Item = PmBalanceEvent> + '_ {
        self.diagnostic_balances.iter().copied()
    }

    pub(crate) fn diagnostic_allowances(&self) -> impl Iterator<Item = PmAllowanceEvent> + '_ {
        self.diagnostic_allowances.iter().copied()
    }

    pub(crate) fn diagnostic_positions(&self) -> impl Iterator<Item = PmPositionEvent> + '_ {
        self.diagnostic_positions.iter().copied()
    }

    pub(crate) fn preview(
        &self,
        envelope: &EventEnvelope<PmCompleteAccountSnapshot>,
        config: &PmPrivateStateConfig,
    ) -> Result<PmAccountSnapshotApply, PmAccountStateError> {
        validate_envelope(envelope, config)?;
        let received = envelope.payload().snapshot().revision();
        let completion = PmPrivateOccurrence::new(
            envelope.ordering().connection_epoch(),
            envelope.payload().boundary().completion_sequence(),
        );
        let Some(current) = self.snapshot.map(PmSnapshotEvidence::revision) else {
            return Ok(PmAccountSnapshotApply::Applied {
                revision: received,
                explicit_absences: 0,
            });
        };
        if received == current && self.completion == Some(completion) {
            return Ok(PmAccountSnapshotApply::Duplicate { revision: received });
        }
        let stale_occurrence = self.completion.is_some_and(|prior| completion <= prior);
        let stale_revision_same_epoch = self
            .completion
            .is_some_and(|prior| prior.epoch() == completion.epoch() && received <= current);
        if stale_occurrence || stale_revision_same_epoch {
            Ok(PmAccountSnapshotApply::IgnoredStale { received, current })
        } else {
            Ok(PmAccountSnapshotApply::Applied {
                revision: received,
                explicit_absences: 0,
            })
        }
    }

    fn count_non_applied(&mut self, outcome: PmAccountSnapshotApply) {
        match outcome {
            PmAccountSnapshotApply::Duplicate { .. } => {
                self.counters.duplicates = self.counters.duplicates.saturating_add(1);
            }
            PmAccountSnapshotApply::IgnoredStale { .. } => {
                self.counters.stale = self.counters.stale.saturating_add(1);
            }
            PmAccountSnapshotApply::Applied { .. } => {}
        }
    }

    fn replace_allowances(&mut self, snapshot: &PmCompleteAccountSnapshot) {
        for entry in &mut self.allowances {
            entry.knowledge = match snapshot.expected_allowance(entry.spender) {
                Some(Some(row)) => PmAllowanceKnowledge::Present(row.value()),
                Some(None) => PmAllowanceKnowledge::ExplicitAbsent,
                None => unreachable!("local expected scope was validated"),
            };
        }
    }

    fn replace_diagnostics(
        &mut self,
        snapshot: &PmCompleteAccountSnapshot,
        config: &PmPrivateStateConfig,
    ) {
        self.diagnostic_balances.clear();
        self.diagnostic_balances
            .extend(snapshot.balances().iter().copied().filter(|row| {
                row.asset() != config.collateral_asset() && row.asset() != config.outcome_asset()
            }));
        self.diagnostic_allowances.clear();
        self.diagnostic_allowances.extend(
            snapshot
                .allowances()
                .iter()
                .copied()
                .filter(|row| !config.required_spenders().contains(&row.spender())),
        );
        self.diagnostic_positions.clear();
        self.diagnostic_positions.extend(
            snapshot
                .positions()
                .iter()
                .copied()
                .filter(|row| row.instrument() != config.instrument()),
        );
    }
}

fn validate_envelope(
    envelope: &EventEnvelope<PmCompleteAccountSnapshot>,
    config: &PmPrivateStateConfig,
) -> Result<(), PmAccountStateError> {
    let snapshot = envelope.payload();
    if envelope.source() != config.source() || snapshot.account_scope() != config.account_scope() {
        return Err(PmAccountStateError::ScopeMismatch);
    }
    if !config.expected_assets_match(snapshot.expected_assets())
        || !config.expected_spenders_match(snapshot.expected_spenders())
        || !config.expected_instruments_match(snapshot.expected_instruments())
    {
        return Err(PmAccountStateError::ExpectedScopeMismatch);
    }
    if envelope.ordering().snapshot_revision() != Some(snapshot.snapshot().revision()) {
        return Err(PmAccountStateError::EnvelopeRevisionMismatch);
    }
    if envelope.ordering().local_ingress_sequence() != snapshot.boundary().completion_sequence() {
        return Err(PmAccountStateError::CompletionSequenceMismatch);
    }
    Ok(())
}

fn observed_balance(snapshot: &PmCompleteAccountSnapshot, asset: PmAssetId) -> PmObservedAmount {
    match snapshot
        .expected_balance(asset)
        .expect("validated expected asset")
    {
        Some(row) => PmObservedAmount::Present(row.balance()),
        None => PmObservedAmount::ExplicitAbsent,
    }
}

fn observed_position(
    snapshot: &PmCompleteAccountSnapshot,
    instrument: reap_pm_core::PmInstrumentHandle,
) -> PmPositionKnowledge {
    let Some(row) = snapshot
        .expected_position(instrument)
        .expect("validated expected instrument")
    else {
        return PmPositionKnowledge::ExplicitAbsent;
    };
    match row.availability() {
        PmPositionAvailability::Tradable => PmPositionKnowledge::Tradable(row.quantity()),
        PmPositionAvailability::ResolvedUnredeemed => {
            PmPositionKnowledge::ResolvedUnredeemed(row.quantity())
        }
        PmPositionAvailability::Unavailable => {
            PmPositionKnowledge::VenueUnavailable(row.quantity())
        }
    }
}

fn count_absences(
    collateral: PmObservedAmount,
    outcome: PmObservedAmount,
    position: PmPositionKnowledge,
    snapshot: &PmCompleteAccountSnapshot,
    config: &PmPrivateStateConfig,
) -> u8 {
    let missing_allowances = config
        .required_spenders()
        .iter()
        .filter(|spender| matches!(snapshot.expected_allowance(**spender), Some(None)))
        .count();
    u8::from(collateral == PmObservedAmount::ExplicitAbsent)
        + u8::from(outcome == PmObservedAmount::ExplicitAbsent)
        + u8::from(position == PmPositionKnowledge::ExplicitAbsent)
        + u8::try_from(missing_allowances).expect("bounded expected spenders")
}

pub(crate) fn apply_signed(base: U256, delta: PmSignedUnits) -> Option<U256> {
    match delta.sign() {
        reap_pm_core::PmSign::Positive => base.checked_add(delta.magnitude()).ok(),
        reap_pm_core::PmSign::Negative => base.checked_sub(delta.magnitude()).ok(),
    }
}
