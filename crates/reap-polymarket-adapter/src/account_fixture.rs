use reap_pm_core::{
    ConnectionEpoch, EventEnvelope, IngressSequence, MAX_PM_ACCOUNT_DIAGNOSTIC_EXTRA_ROWS,
    PmAccountHandle, PmAccountScope, PmAggregateError, PmAllowanceEvent, PmAllowanceValue,
    PmAssetId, PmBalanceEvent, PmCompleteAccountSnapshot, PmConnectionId, PmEventError,
    PmGoalFTradingDomain, PmInstrumentHandle, PmPositionAvailability, PmPositionEvent,
    PmProductSource, PmReconciliationRequestBoundary, PmSnapshotEvidence, PmSpenderId, U256,
};
use reap_polymarket_wire::{PmFixtureAllowanceScope, PmLegacyBalanceAllowanceFixture};
use thiserror::Error;

use crate::fixture_delivery::{
    PmFixtureRequestOccurrence, checked_delivery, validate_completion, validate_next_request,
};
use crate::fixture_scope::PmFixtureOwnerId;
use crate::fixture_scope::validate_account_source;
use crate::{
    PmFixtureAccountRoleGrant, PmFixtureAggregateDelivery, PmFixtureCompletionOccurrence,
    PmFixtureDeliveryError, PmFixtureDeliveryScope, PmFixtureInstrumentScope, PmFixtureScopeError,
    PmFixtureServicedAggregate,
};

pub type PmCompleteAccountSnapshotDelivery = PmFixtureAggregateDelivery<PmCompleteAccountSnapshot>;

mod sealed {
    pub trait Sealed {}
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum PmAccountPositionRoleError {
    #[error("account snapshot role requires a Polymarket account source")]
    WrongSource,
    #[error("account snapshot role source belongs to another account")]
    SourceAccountMismatch,
    #[error("account snapshot role chain differs from the exact Goal F domain")]
    DomainChainMismatch,
    #[error("Goal F fixture account signer and funder must be the same exact address")]
    SignerFunderMismatch,
    #[error("observed account row differs from the full configured account scope")]
    AccountScopeMismatch,
    #[error("observed allowance spender belongs to another account")]
    SpenderAccountMismatch,
    #[error("observed allowance spender belongs to another chain")]
    SpenderChainMismatch,
    #[error("observed position differs from the exact configured instrument")]
    InstrumentMismatch,
    #[error("legacy scalar allowance has no asset/token/spender scope and is non-authoritative")]
    UnscopedLegacyAllowance,
    #[error("account fixture request ticket is invalid: {0}")]
    Delivery(#[from] PmFixtureDeliveryError),
    #[error("account fixture event contract failed: {0}")]
    Event(#[from] PmEventError),
    #[error("complete account aggregate contract failed: {0}")]
    Aggregate(#[from] PmAggregateError),
    #[error("account fixture page belongs to another causal request")]
    PageRequestMismatch,
    #[error("account fixture page carries another snapshot revision")]
    PageSnapshotMismatch,
    #[error("account fixture page cursor chain is broken")]
    BrokenCursorChain,
    #[error("account fixture page arrived after the terminal page")]
    PageAfterTerminal,
    #[error("account fixture query has no page and cannot prove explicit emptiness")]
    MissingPage,
    #[error("account fixture query did not reach a terminal cursor")]
    MissingTerminalPage,
    #[error("account fixture query exceeds its fixed page bound")]
    TooManyPages,
}

/// Fixture-only collateral, allowance, inventory, and position snapshot role.
pub trait PmAccountPositionSnapshotRole: sealed::Sealed {
    type BalanceObservation;
    type AllowanceObservation;
    type PositionObservation;
    type CompleteSnapshot;

    fn account_scope(&self) -> PmAccountScope;
    fn account(&self) -> PmAccountHandle;
    fn instrument_scope(&self) -> PmFixtureInstrumentScope;
    fn trading_domain(&self) -> PmGoalFTradingDomain;
    fn source(&self) -> PmProductSource;
    fn connection(&self) -> PmConnectionId;
    fn required_spenders(&self) -> &[PmSpenderId];
}

#[derive(Debug)]
pub struct PmFixtureAccountPositionSnapshot {
    binding: AccountBinding,
    required_spenders: [PmSpenderId; 2],
    last_request: Option<(ConnectionEpoch, IngressSequence)>,
}

impl PmFixtureAccountPositionSnapshot {
    pub fn new(
        grant: PmFixtureAccountRoleGrant,
        account_scope: PmAccountScope,
        instrument: PmFixtureInstrumentScope,
        source: PmProductSource,
        connection: PmConnectionId,
    ) -> Result<Self, PmAccountPositionRoleError> {
        validate_role_source(account_scope, source)?;
        let trading_domain = instrument.trading_domain();
        if trading_domain.chain() != account_scope.chain() {
            return Err(PmAccountPositionRoleError::DomainChainMismatch);
        }
        if account_scope.signer().address() != account_scope.funder().address() {
            return Err(PmAccountPositionRoleError::SignerFunderMismatch);
        }
        let required_spenders = trading_domain
            .required_spenders()
            .map(|requirement| PmSpenderId::new(account_scope.handle(), requirement));
        Ok(Self {
            binding: AccountBinding {
                owner_id: grant.into_owner_id(),
                account_scope,
                instrument,
                trading_domain,
                source,
                connection,
            },
            required_spenders,
            last_request: None,
        })
    }

    #[must_use]
    pub const fn account_scope(&self) -> PmAccountScope {
        self.binding.account_scope
    }

    #[must_use]
    pub const fn account(&self) -> PmAccountHandle {
        self.binding.account_scope.handle()
    }

    #[must_use]
    pub const fn instrument_scope(&self) -> PmFixtureInstrumentScope {
        self.binding.instrument
    }

    #[must_use]
    pub const fn instrument(&self) -> PmInstrumentHandle {
        self.binding.instrument.handle()
    }

    #[must_use]
    pub const fn trading_domain(&self) -> PmGoalFTradingDomain {
        self.binding.trading_domain
    }

    #[must_use]
    pub const fn source(&self) -> PmProductSource {
        self.binding.source
    }

    #[must_use]
    pub const fn connection(&self) -> PmConnectionId {
        self.binding.connection
    }

    #[must_use]
    pub fn required_spenders(&self) -> &[PmSpenderId] {
        &self.required_spenders
    }

    pub fn normalize_balance(
        &self,
        observed_scope: PmAccountScope,
        asset: PmAssetId,
        balance: U256,
        snapshot: PmSnapshotEvidence,
    ) -> Result<PmBalanceEvent, PmAccountPositionRoleError> {
        self.binding
            .normalize_balance(observed_scope, asset, balance, snapshot)
    }

    pub fn normalize_allowance(
        &self,
        observed_scope: PmAccountScope,
        spender: PmSpenderId,
        value: PmAllowanceValue,
        snapshot: PmSnapshotEvidence,
    ) -> Result<PmAllowanceEvent, PmAccountPositionRoleError> {
        self.binding
            .normalize_allowance(observed_scope, spender, value, snapshot)
    }

    pub fn normalize_position(
        &self,
        observed_scope: PmAccountScope,
        instrument: PmFixtureInstrumentScope,
        quantity: U256,
        availability: PmPositionAvailability,
        snapshot: PmSnapshotEvidence,
    ) -> Result<PmPositionEvent, PmAccountPositionRoleError> {
        self.binding.normalize_position(
            observed_scope,
            instrument,
            quantity,
            availability,
            snapshot,
        )
    }

    pub fn reject_legacy_scalar(
        &self,
        fixture: &PmLegacyBalanceAllowanceFixture,
    ) -> Result<(), PmAccountPositionRoleError> {
        debug_assert_eq!(
            fixture.allowance_scope(),
            PmFixtureAllowanceScope::UnscopedLegacyScalar
        );
        Err(PmAccountPositionRoleError::UnscopedLegacyAllowance)
    }

    pub fn request_snapshot(
        &mut self,
        connection_epoch: ConnectionEpoch,
        request_sequence: IngressSequence,
    ) -> Result<PmFixtureAccountSnapshotRequest, PmAccountPositionRoleError> {
        let request = validate_next_request(self.last_request, connection_epoch, request_sequence)?;
        self.last_request = Some((connection_epoch, request_sequence));
        Ok(PmFixtureAccountSnapshotRequest {
            binding: self.binding,
            request,
            expected_spenders: self.required_spenders,
        })
    }

    pub fn reduce_snapshot_delivery<R>(
        &self,
        delivery: PmFixtureServicedAggregate<PmCompleteAccountSnapshot>,
        reduce: impl FnOnce(PmFixtureDeliveryScope, EventEnvelope<PmCompleteAccountSnapshot>) -> R,
    ) -> Result<R, Box<PmFixtureServicedAggregate<PmCompleteAccountSnapshot>>> {
        delivery.reduce_with_owner(self.binding.owner_id, reduce)
    }
}

impl sealed::Sealed for PmFixtureAccountPositionSnapshot {}

impl PmAccountPositionSnapshotRole for PmFixtureAccountPositionSnapshot {
    type BalanceObservation = PmBalanceEvent;
    type AllowanceObservation = PmAllowanceEvent;
    type PositionObservation = PmPositionEvent;
    type CompleteSnapshot = PmCompleteAccountSnapshotDelivery;

    fn account_scope(&self) -> PmAccountScope {
        self.binding.account_scope
    }

    fn account(&self) -> PmAccountHandle {
        self.binding.account_scope.handle()
    }

    fn instrument_scope(&self) -> PmFixtureInstrumentScope {
        self.binding.instrument
    }

    fn trading_domain(&self) -> PmGoalFTradingDomain {
        self.binding.trading_domain
    }

    fn source(&self) -> PmProductSource {
        self.binding.source
    }

    fn connection(&self) -> PmConnectionId {
        self.binding.connection
    }

    fn required_spenders(&self) -> &[PmSpenderId] {
        &self.required_spenders
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PmFixtureBalanceRow {
    asset: PmAssetId,
    balance: U256,
}

impl PmFixtureBalanceRow {
    #[must_use]
    pub const fn new(asset: PmAssetId, balance: U256) -> Self {
        Self { asset, balance }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PmFixtureAllowanceRow {
    spender: PmSpenderId,
    value: PmAllowanceValue,
}

impl PmFixtureAllowanceRow {
    #[must_use]
    pub const fn new(spender: PmSpenderId, value: PmAllowanceValue) -> Self {
        Self { spender, value }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PmFixturePositionRow {
    instrument: PmFixtureInstrumentScope,
    quantity: U256,
    availability: PmPositionAvailability,
}

impl PmFixturePositionRow {
    #[must_use]
    pub const fn new(
        instrument: PmFixtureInstrumentScope,
        quantity: U256,
        availability: PmPositionAvailability,
    ) -> Self {
        Self {
            instrument,
            quantity,
            availability,
        }
    }
}

#[derive(Debug)]
pub struct PmFixtureAccountSnapshotRequest {
    binding: AccountBinding,
    request: PmFixtureRequestOccurrence,
    expected_spenders: [PmSpenderId; 2],
}

impl PmFixtureAccountSnapshotRequest {
    #[must_use]
    pub fn begin(self, snapshot: PmSnapshotEvidence) -> PmFixtureAccountSnapshotAssembly {
        PmFixtureAccountSnapshotAssembly {
            binding: self.binding,
            assembler: AccountSnapshotAssembler::new(
                self.binding,
                self.request,
                snapshot,
                self.expected_spenders,
            ),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn complete(
        self,
        completion: PmFixtureCompletionOccurrence,
        snapshot: PmSnapshotEvidence,
        observed_scope: PmAccountScope,
        balances: &[PmFixtureBalanceRow],
        allowances: &[PmFixtureAllowanceRow],
        positions: &[PmFixturePositionRow],
    ) -> Result<PmCompleteAccountSnapshotDelivery, PmAccountPositionRoleError> {
        let mut assembly = self.begin(snapshot);
        assembly.push_page(observed_scope, None, None, balances, allowances, positions)?;
        assembly.finish(completion)
    }
}

/// Move-only bounded assembly of one atomic account snapshot cursor chain.
pub struct PmFixtureAccountSnapshotAssembly {
    binding: AccountBinding,
    assembler: AccountSnapshotAssembler,
}

impl PmFixtureAccountSnapshotAssembly {
    #[allow(clippy::too_many_arguments)]
    pub fn push_page(
        &mut self,
        observed_scope: PmAccountScope,
        requested_cursor: Option<[u8; 32]>,
        next_cursor: Option<[u8; 32]>,
        balances: &[PmFixtureBalanceRow],
        allowances: &[PmFixtureAllowanceRow],
        positions: &[PmFixturePositionRow],
    ) -> Result<(), PmAccountPositionRoleError> {
        self.assembler.preflight_raw_page(
            requested_cursor,
            next_cursor,
            balances,
            allowances,
            positions,
        )?;
        let snapshot = self.assembler.snapshot;
        let balances = balances
            .iter()
            .map(|row| {
                self.binding
                    .normalize_balance(observed_scope, row.asset, row.balance, snapshot)
            })
            .collect::<Result<Vec<_>, _>>()?;
        let allowances = allowances
            .iter()
            .map(|row| {
                self.binding
                    .normalize_allowance(observed_scope, row.spender, row.value, snapshot)
            })
            .collect::<Result<Vec<_>, _>>()?;
        let positions = positions
            .iter()
            .map(|row| {
                self.binding.normalize_position(
                    observed_scope,
                    row.instrument,
                    row.quantity,
                    row.availability,
                    snapshot,
                )
            })
            .collect::<Result<Vec<_>, _>>()?;
        self.assembler.push_page(AccountPage {
            request_sequence: self.assembler.request_sequence(),
            snapshot,
            requested_cursor,
            next_cursor,
            balances,
            allowances,
            positions,
        })
    }

    pub fn finish(
        self,
        completion: PmFixtureCompletionOccurrence,
    ) -> Result<PmCompleteAccountSnapshotDelivery, PmAccountPositionRoleError> {
        let payload = self.assembler.finish(&completion)?;
        Ok(self.binding.delivery(completion, payload)?)
    }
}

#[derive(Debug, Clone, Copy)]
struct AccountBinding {
    owner_id: PmFixtureOwnerId,
    account_scope: PmAccountScope,
    instrument: PmFixtureInstrumentScope,
    trading_domain: PmGoalFTradingDomain,
    source: PmProductSource,
    connection: PmConnectionId,
}

impl AccountBinding {
    fn validate_observed_scope(
        self,
        observed_scope: PmAccountScope,
    ) -> Result<(), PmAccountPositionRoleError> {
        if observed_scope == self.account_scope {
            Ok(())
        } else {
            Err(PmAccountPositionRoleError::AccountScopeMismatch)
        }
    }

    fn normalize_balance(
        self,
        observed_scope: PmAccountScope,
        asset: PmAssetId,
        balance: U256,
        snapshot: PmSnapshotEvidence,
    ) -> Result<PmBalanceEvent, PmAccountPositionRoleError> {
        self.validate_observed_scope(observed_scope)?;
        Ok(PmBalanceEvent::new(
            self.source,
            self.account_scope.handle(),
            asset,
            balance,
            snapshot,
        )?)
    }

    fn normalize_allowance(
        self,
        observed_scope: PmAccountScope,
        spender: PmSpenderId,
        value: PmAllowanceValue,
        snapshot: PmSnapshotEvidence,
    ) -> Result<PmAllowanceEvent, PmAccountPositionRoleError> {
        self.validate_observed_scope(observed_scope)?;
        if spender.account() != self.account_scope.handle() {
            return Err(PmAccountPositionRoleError::SpenderAccountMismatch);
        }
        if spender.requirement().chain() != self.account_scope.chain() {
            return Err(PmAccountPositionRoleError::SpenderChainMismatch);
        }
        Ok(PmAllowanceEvent::new(
            self.source,
            spender,
            value,
            snapshot,
        )?)
    }

    fn normalize_position(
        self,
        observed_scope: PmAccountScope,
        instrument: PmFixtureInstrumentScope,
        quantity: U256,
        availability: PmPositionAvailability,
        snapshot: PmSnapshotEvidence,
    ) -> Result<PmPositionEvent, PmAccountPositionRoleError> {
        self.validate_observed_scope(observed_scope)?;
        if instrument != self.instrument {
            return Err(PmAccountPositionRoleError::InstrumentMismatch);
        }
        Ok(PmPositionEvent::new(
            self.source,
            self.account_scope.handle(),
            instrument.handle(),
            quantity,
            availability,
            snapshot,
        )?)
    }

    fn delivery(
        self,
        completion: PmFixtureCompletionOccurrence,
        payload: PmCompleteAccountSnapshot,
    ) -> Result<PmCompleteAccountSnapshotDelivery, PmFixtureDeliveryError> {
        checked_delivery(
            self.owner_id,
            self.account_scope,
            self.instrument,
            self.source,
            self.connection,
            completion,
            payload,
        )
    }
}

struct AccountSnapshotAssembler {
    binding: AccountBinding,
    request: PmFixtureRequestOccurrence,
    snapshot: PmSnapshotEvidence,
    expected_spenders: [PmSpenderId; 2],
    chain: AccountPageChain,
    balances: Vec<PmBalanceEvent>,
    allowances: Vec<PmAllowanceEvent>,
    positions: Vec<PmPositionEvent>,
}

impl AccountSnapshotAssembler {
    fn new(
        binding: AccountBinding,
        request: PmFixtureRequestOccurrence,
        snapshot: PmSnapshotEvidence,
        expected_spenders: [PmSpenderId; 2],
    ) -> Self {
        Self {
            binding,
            request,
            snapshot,
            expected_spenders,
            chain: AccountPageChain::new(),
            balances: Vec::new(),
            allowances: Vec::new(),
            positions: Vec::new(),
        }
    }

    fn request_sequence(&self) -> IngressSequence {
        self.request.sequence()
    }

    fn expected_assets(&self) -> [PmAssetId; 2] {
        [
            self.binding.trading_domain.collateral(),
            self.binding.trading_domain.outcome(),
        ]
    }

    fn preflight_raw_page(
        &self,
        requested_cursor: Option<[u8; 32]>,
        next_cursor: Option<[u8; 32]>,
        balances: &[PmFixtureBalanceRow],
        allowances: &[PmFixtureAllowanceRow],
        positions: &[PmFixturePositionRow],
    ) -> Result<(), PmAccountPositionRoleError> {
        let expected_assets = self.expected_assets();
        let expected_instrument = self.binding.instrument.handle();
        self.preflight_page_growth(
            requested_cursor,
            next_cursor,
            RowGrowth {
                current: self.balances.len(),
                additional: balances.len(),
                expected: expected_assets.len(),
                current_diagnostic: self
                    .balances
                    .iter()
                    .filter(|row| !expected_assets.contains(&row.asset()))
                    .count(),
                additional_diagnostic: balances
                    .iter()
                    .filter(|row| !expected_assets.contains(&row.asset))
                    .count(),
                error: PmAggregateError::TooManyBalanceRows,
            },
            RowGrowth {
                current: self.allowances.len(),
                additional: allowances.len(),
                expected: self.expected_spenders.len(),
                current_diagnostic: self
                    .allowances
                    .iter()
                    .filter(|row| !self.expected_spenders.contains(&row.spender()))
                    .count(),
                additional_diagnostic: allowances
                    .iter()
                    .filter(|row| !self.expected_spenders.contains(&row.spender))
                    .count(),
                error: PmAggregateError::TooManyAllowanceRows,
            },
            RowGrowth {
                current: self.positions.len(),
                additional: positions.len(),
                expected: 1,
                current_diagnostic: self
                    .positions
                    .iter()
                    .filter(|row| row.instrument() != expected_instrument)
                    .count(),
                additional_diagnostic: positions
                    .iter()
                    .filter(|row| row.instrument.handle() != expected_instrument)
                    .count(),
                error: PmAggregateError::TooManyPositionRows,
            },
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn preflight_page_growth(
        &self,
        requested_cursor: Option<[u8; 32]>,
        next_cursor: Option<[u8; 32]>,
        balances: RowGrowth,
        allowances: RowGrowth,
        positions: RowGrowth,
    ) -> Result<(), PmAccountPositionRoleError> {
        self.chain.preflight(requested_cursor, next_cursor)?;
        validate_row_growth(balances)?;
        validate_row_growth(allowances)?;
        validate_row_growth(positions)?;
        Ok(())
    }

    fn push_page(&mut self, page: AccountPage) -> Result<(), PmAccountPositionRoleError> {
        validate_page_identity(
            &self.request,
            self.snapshot,
            page.request_sequence,
            page.snapshot,
        )?;
        let expected_assets = self.expected_assets();
        let expected_instrument = self.binding.instrument.handle();
        self.preflight_page_growth(
            page.requested_cursor,
            page.next_cursor,
            RowGrowth {
                current: self.balances.len(),
                additional: page.balances.len(),
                expected: expected_assets.len(),
                current_diagnostic: self
                    .balances
                    .iter()
                    .filter(|row| !expected_assets.contains(&row.asset()))
                    .count(),
                additional_diagnostic: page
                    .balances
                    .iter()
                    .filter(|row| !expected_assets.contains(&row.asset()))
                    .count(),
                error: PmAggregateError::TooManyBalanceRows,
            },
            RowGrowth {
                current: self.allowances.len(),
                additional: page.allowances.len(),
                expected: self.expected_spenders.len(),
                current_diagnostic: self
                    .allowances
                    .iter()
                    .filter(|row| !self.expected_spenders.contains(&row.spender()))
                    .count(),
                additional_diagnostic: page
                    .allowances
                    .iter()
                    .filter(|row| !self.expected_spenders.contains(&row.spender()))
                    .count(),
                error: PmAggregateError::TooManyAllowanceRows,
            },
            RowGrowth {
                current: self.positions.len(),
                additional: page.positions.len(),
                expected: 1,
                current_diagnostic: self
                    .positions
                    .iter()
                    .filter(|row| row.instrument() != expected_instrument)
                    .count(),
                additional_diagnostic: page
                    .positions
                    .iter()
                    .filter(|row| row.instrument() != expected_instrument)
                    .count(),
                error: PmAggregateError::TooManyPositionRows,
            },
        )?;
        if page
            .positions
            .iter()
            .any(|position| position.instrument() != self.binding.instrument.handle())
        {
            return Err(PmAccountPositionRoleError::InstrumentMismatch);
        }
        self.chain.accept(page.requested_cursor, page.next_cursor)?;
        self.balances.extend(page.balances);
        self.allowances.extend(page.allowances);
        self.positions.extend(page.positions);
        Ok(())
    }

    fn finish(
        self,
        completion: &PmFixtureCompletionOccurrence,
    ) -> Result<PmCompleteAccountSnapshot, PmAccountPositionRoleError> {
        self.chain.validate_terminal()?;
        let completion_sequence =
            validate_completion(&self.request, completion, self.snapshot.revision())?;
        let boundary =
            PmReconciliationRequestBoundary::new(self.request.sequence(), completion_sequence)?;
        let expected_assets = [
            self.binding.trading_domain.collateral(),
            self.binding.trading_domain.outcome(),
        ];
        Ok(PmCompleteAccountSnapshot::new(
            self.binding.source,
            self.binding.account_scope,
            self.snapshot,
            boundary,
            Box::new(expected_assets),
            Box::new(self.expected_spenders),
            Box::new([self.binding.instrument.handle()]),
            self.balances.into_boxed_slice(),
            self.allowances.into_boxed_slice(),
            self.positions.into_boxed_slice(),
        )?)
    }
}

struct AccountPage {
    request_sequence: IngressSequence,
    snapshot: PmSnapshotEvidence,
    requested_cursor: Option<[u8; 32]>,
    next_cursor: Option<[u8; 32]>,
    balances: Vec<PmBalanceEvent>,
    allowances: Vec<PmAllowanceEvent>,
    positions: Vec<PmPositionEvent>,
}

struct AccountPageChain {
    expected_cursor: Option<[u8; 32]>,
    seen: Vec<[u8; 32]>,
    started: bool,
    terminal: bool,
    page_count: usize,
}

impl AccountPageChain {
    fn new() -> Self {
        Self {
            expected_cursor: None,
            seen: Vec::new(),
            started: false,
            terminal: false,
            page_count: 0,
        }
    }

    fn preflight(
        &self,
        requested_cursor: Option<[u8; 32]>,
        next_cursor: Option<[u8; 32]>,
    ) -> Result<(), PmAccountPositionRoleError> {
        if self.terminal {
            return Err(PmAccountPositionRoleError::PageAfterTerminal);
        }
        if requested_cursor != self.expected_cursor {
            return Err(PmAccountPositionRoleError::BrokenCursorChain);
        }
        if self.page_count == crate::MAX_PM_FIXTURE_QUERY_PAGES {
            return Err(PmAccountPositionRoleError::TooManyPages);
        }
        if let Some(cursor) = requested_cursor
            && self.seen.contains(&cursor)
        {
            return Err(PmAccountPositionRoleError::BrokenCursorChain);
        }
        if next_cursor
            .is_some_and(|cursor| self.seen.contains(&cursor) || requested_cursor == Some(cursor))
        {
            return Err(PmAccountPositionRoleError::BrokenCursorChain);
        }
        Ok(())
    }

    fn accept(
        &mut self,
        requested_cursor: Option<[u8; 32]>,
        next_cursor: Option<[u8; 32]>,
    ) -> Result<(), PmAccountPositionRoleError> {
        self.preflight(requested_cursor, next_cursor)?;
        if let Some(cursor) = requested_cursor {
            self.seen.push(cursor);
        }
        self.started = true;
        self.page_count += 1;
        self.expected_cursor = next_cursor;
        self.terminal = next_cursor.is_none();
        Ok(())
    }

    fn validate_terminal(&self) -> Result<(), PmAccountPositionRoleError> {
        if !self.started {
            Err(PmAccountPositionRoleError::MissingPage)
        } else if !self.terminal {
            Err(PmAccountPositionRoleError::MissingTerminalPage)
        } else {
            Ok(())
        }
    }
}

fn validate_page_identity(
    request: &PmFixtureRequestOccurrence,
    snapshot: PmSnapshotEvidence,
    page_request_sequence: IngressSequence,
    page_snapshot: PmSnapshotEvidence,
) -> Result<(), PmAccountPositionRoleError> {
    if page_request_sequence != request.sequence() {
        return Err(PmAccountPositionRoleError::PageRequestMismatch);
    }
    if page_snapshot != snapshot {
        return Err(PmAccountPositionRoleError::PageSnapshotMismatch);
    }
    Ok(())
}

#[derive(Clone, Copy)]
struct RowGrowth {
    current: usize,
    additional: usize,
    expected: usize,
    current_diagnostic: usize,
    additional_diagnostic: usize,
    error: PmAggregateError,
}

fn validate_row_growth(growth: RowGrowth) -> Result<(), PmAccountPositionRoleError> {
    let maximum = growth
        .expected
        .saturating_add(MAX_PM_ACCOUNT_DIAGNOSTIC_EXTRA_ROWS);
    let remaining = maximum.saturating_sub(growth.current);
    let remaining_diagnostic =
        MAX_PM_ACCOUNT_DIAGNOSTIC_EXTRA_ROWS.saturating_sub(growth.current_diagnostic);
    if growth.additional > remaining || growth.additional_diagnostic > remaining_diagnostic {
        Err(growth.error.into())
    } else {
        Ok(())
    }
}

fn validate_role_source(
    account_scope: PmAccountScope,
    source: PmProductSource,
) -> Result<(), PmAccountPositionRoleError> {
    match validate_account_source(account_scope, source) {
        Ok(()) => Ok(()),
        Err(PmFixtureScopeError::WrongSource) => Err(PmAccountPositionRoleError::WrongSource),
        Err(PmFixtureScopeError::SourceAccountMismatch) => {
            Err(PmAccountPositionRoleError::SourceAccountMismatch)
        }
    }
}

#[cfg(test)]
mod tests {
    use reap_pm_core::{ConnectionEpoch, IngressSequence, PmSnapshotEvidence, SnapshotRevision};

    use super::{PmAccountPositionRoleError, validate_page_identity};
    use crate::fixture_delivery::PmFixtureRequestOccurrence;

    #[test]
    fn private_page_identity_rejects_another_request_or_snapshot() {
        let request =
            PmFixtureRequestOccurrence::new(ConnectionEpoch::new(1), IngressSequence::new(10))
                .unwrap();
        let snapshot = PmSnapshotEvidence::new(SnapshotRevision::new(7)).unwrap();

        assert_eq!(
            validate_page_identity(&request, snapshot, IngressSequence::new(11), snapshot,),
            Err(PmAccountPositionRoleError::PageRequestMismatch)
        );
        assert_eq!(
            validate_page_identity(
                &request,
                snapshot,
                IngressSequence::new(10),
                PmSnapshotEvidence::new(SnapshotRevision::new(8)).unwrap(),
            ),
            Err(PmAccountPositionRoleError::PageSnapshotMismatch)
        );
    }
}
