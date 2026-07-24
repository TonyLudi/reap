use reap_pm_core::{
    ConnectionEpoch, IngressSequence, OkxReferenceEvent, OkxReferencePrice, PmBookEvent,
    PmBookLevel, PmBookQuantity, PmBookSnapshot, PmBookUpdate, PmConnectionId, PmFillQueryCursor,
    PmPrice, PmProductSource, PmSignedUnits, SnapshotRevision, U256, VenueEventHash,
};
use reap_pm_state::{
    PmBookBatchEvidence, PmBookFreshness, PmBookReducer, PmDomainFingerprint, PmMetadataContract,
    PmMetadataFingerprint, PmMetadataObservation,
};
use reap_polymarket_adapter::{
    PmFixtureAllowanceRow, PmFixtureBalanceRow, PmFixtureFeeEvidence, PmFixtureInstrumentScope,
    PmFixturePositionRow,
};

use super::{event_clock, invariant, ordering};
use crate::coordinator::{PmBookDecisionProjection, PmBookInput, PmOkxReferenceInput};
use crate::evidence::PmEvidenceError;
use crate::evidence::fixture::{allowance_row, authoritative, completion, query_occurrence};

const WALL_TIMESTAMP_BUCKET_NS: u64 = 1_000_000;
const PASS_START_BUCKET_PHASE_NS: u64 = 1_000;

pub(super) struct AccountRows {
    pub(super) scope: reap_pm_core::PmAccountScope,
    pub(super) balances: [PmFixtureBalanceRow; 2],
    pub(super) allowances: [PmFixtureAllowanceRow; 2],
    pub(super) positions: [PmFixturePositionRow; 1],
    pub(super) nominal_fill_fee: PmFixtureFeeEvidence,
}

impl AccountRows {
    pub(super) fn new(
        config: &reap_pm_live_contracts::PmConnectivityConfig,
    ) -> Result<Self, PmEvidenceError> {
        let account = config.account();
        let domain = account.trading_domain();
        let balances = [
            PmFixtureBalanceRow::new(domain.collateral(), U256::from_u64(10_000_000_000)),
            PmFixtureBalanceRow::new(domain.outcome(), U256::from_u64(10_000_000_000)),
        ];
        let spenders = account.required_spenders();
        let allowances = [
            allowance_row(spenders[0], domain.collateral()),
            allowance_row(spenders[1], domain.collateral()),
        ];
        let instrument = PmFixtureInstrumentScope::from_metadata(
            account.instrument(),
            account.expected_metadata(),
        )
        .map_err(invariant)?;
        Ok(Self {
            scope: account.account_scope(),
            balances,
            allowances,
            positions: [PmFixturePositionRow::new(
                instrument,
                U256::from_u64(10_000_000_000),
                reap_pm_core::PmPositionAvailability::Tradable,
            )],
            nominal_fill_fee: PmFixtureFeeEvidence::Known {
                asset: domain.collateral(),
                delta: PmSignedUnits::ZERO,
            },
        })
    }
}

pub(super) struct WorkloadCursor {
    pub(super) absolute_cycle: u64,
    pub(super) monotonic_ns: u64,
    pub(super) internal_fact_acks: u64,
    account_ingress: u64,
    query_revision: u64,
    fill_count: u64,
    fill_cursor_byte: u8,
}

impl WorkloadCursor {
    pub(super) const fn after_setup() -> Self {
        Self {
            absolute_cycle: 0,
            monotonic_ns: 1_000,
            account_ingress: 6,
            query_revision: 3,
            fill_count: 0,
            fill_cursor_byte: 1,
            internal_fact_acks: 0,
        }
    }

    pub(super) fn next_cycle(&mut self) -> u64 {
        self.absolute_cycle = self.absolute_cycle.saturating_add(1);
        self.absolute_cycle
    }

    pub(super) fn align_pass_start(&mut self) -> Result<(), PmEvidenceError> {
        let phase = self.monotonic_ns % WALL_TIMESTAMP_BUCKET_NS;
        let advance = if phase <= PASS_START_BUCKET_PHASE_NS {
            PASS_START_BUCKET_PHASE_NS - phase
        } else {
            WALL_TIMESTAMP_BUCKET_NS - phase + PASS_START_BUCKET_PHASE_NS
        };
        self.monotonic_ns = self
            .monotonic_ns
            .checked_add(advance)
            .ok_or_else(|| PmEvidenceError::invariant("fixed pass clock exhausted"))?;
        debug_assert_eq!(
            self.monotonic_ns % WALL_TIMESTAMP_BUCKET_NS,
            PASS_START_BUCKET_PHASE_NS
        );
        Ok(())
    }

    pub(super) fn next_time(&mut self) -> u64 {
        let current = self.monotonic_ns;
        self.monotonic_ns = self.monotonic_ns.saturating_add(10);
        current
    }

    pub(super) fn next_completion(
        &mut self,
        revision: Option<u64>,
    ) -> reap_polymarket_adapter::PmFixtureCompletionOccurrence {
        let ingress = self.account_ingress;
        self.account_ingress = self.account_ingress.saturating_add(1);
        completion(1, ingress, revision, self.next_time())
    }

    pub(super) fn completion_at(
        &mut self,
        revision: Option<u64>,
        monotonic_ns: u64,
    ) -> reap_polymarket_adapter::PmFixtureCompletionOccurrence {
        let ingress = self.account_ingress;
        self.account_ingress = self.account_ingress.saturating_add(1);
        completion(1, ingress, revision, monotonic_ns)
    }

    pub(super) fn next_query(
        &mut self,
    ) -> Result<crate::private_monitor::PmFixtureQueryOccurrence, PmEvidenceError> {
        let request = self.account_ingress;
        let completion_sequence = request.saturating_add(1);
        self.account_ingress = completion_sequence.saturating_add(1);
        let revision = self.query_revision;
        self.query_revision = revision.saturating_add(1);
        query_occurrence(1, request, completion_sequence, revision, self.next_time())
            .map_err(PmEvidenceError::invariant)
    }

    pub(super) fn fill_cursor(&self, scope: reap_pm_core::PmAccountScope) -> PmFillQueryCursor {
        PmFillQueryCursor::new(scope, [self.fill_cursor_byte; 32])
    }

    pub(super) fn advance_fill_cursor_if_cut(&mut self) -> bool {
        self.fill_count = self.fill_count.saturating_add(1);
        if !self.fill_count.is_multiple_of(500) {
            return false;
        }
        self.fill_cursor_byte = self
            .fill_cursor_byte
            .checked_add(1)
            .expect("fifty fixed measured cursor cuts fit u8");
        true
    }
}

pub(super) struct PublicFixture {
    reducer: PmBookReducer,
    pub(super) instrument: reap_pm_core::PmInstrumentHandle,
    pub(super) pm_source: PmProductSource,
    pub(super) pm_connection: PmConnectionId,
    okx_source: PmProductSource,
    okx_connection: PmConnectionId,
    okx_reference: reap_pm_core::OkxReferenceHandle,
    reference_price: OkxReferencePrice,
    book_sequence: u64,
    reference_sequence: u64,
}

impl PublicFixture {
    pub(super) fn new(
        config: &reap_pm_live_contracts::PmConnectivityConfig,
    ) -> Result<Self, PmEvidenceError> {
        let authority = authoritative();
        let fingerprint =
            PmMetadataFingerprint::new(authority.metadata_fingerprint()).map_err(invariant)?;
        let domain = PmDomainFingerprint::new(authority.domain_fingerprint()).map_err(invariant)?;
        let contract =
            PmMetadataContract::goal_f_clob_v2(config.public().expected_metadata(), domain);
        let mut reducer = PmBookReducer::new(
            config.public().instrument(),
            fingerprint,
            contract,
            PmBookFreshness::new(1_000_000_000, 1_000_000_000).map_err(invariant)?,
        )
        .map_err(invariant)?;
        reducer
            .apply_metadata(
                PmMetadataObservation::new(
                    config.public().instrument(),
                    SnapshotRevision::new(1),
                    fingerprint,
                    contract,
                    50,
                )
                .map_err(invariant)?,
            )
            .map_err(invariant)?;
        reducer
            .begin_epoch(ConnectionEpoch::new(1))
            .map_err(invariant)?;
        Ok(Self {
            reducer,
            instrument: config.public().instrument(),
            pm_source: config.public().polymarket_route().source(),
            pm_connection: config.public().polymarket_route().connection(),
            okx_source: config.public().okx_route().source(),
            okx_connection: config.public().okx_route().connection(),
            okx_reference: config.public().okx_reference(),
            reference_price: OkxReferencePrice::parse_decimal("00050000.125000")
                .map_err(invariant)?,
            book_sequence: 1,
            reference_sequence: 1,
        })
    }

    pub(super) fn snapshot_input(
        &mut self,
        monotonic_ns: u64,
    ) -> Result<PmBookInput, PmEvidenceError> {
        let bid = PmBookLevel::new(
            reap_pm_core::PmBookSide::Bid,
            PmPrice::parse_decimal("0.30").map_err(invariant)?,
            PmBookQuantity::parse_decimal("100").map_err(invariant)?,
        );
        let ask = PmBookLevel::new(
            reap_pm_core::PmBookSide::Ask,
            PmPrice::parse_decimal("0.60").map_err(invariant)?,
            PmBookQuantity::parse_decimal("75").map_err(invariant)?,
        );
        let update = PmBookUpdate::Snapshot(
            PmBookSnapshot::new(vec![bid, ask].into_boxed_slice()).map_err(invariant)?,
        );
        let event = PmBookEvent::new(
            self.pm_source,
            self.instrument,
            SnapshotRevision::new(1),
            update,
        )
        .map_err(invariant)?;
        let hash = VenueEventHash::sha1([1; 20]).map_err(invariant)?;
        let ordering = ordering(Some(1), self.book_sequence, Some(hash))?;
        self.reducer
            .apply_update(
                PmBookBatchEvidence::new(
                    self.instrument,
                    ConnectionEpoch::new(1),
                    SnapshotRevision::new(1),
                    SnapshotRevision::new(1),
                    IngressSequence::new(self.book_sequence),
                    monotonic_ns,
                    Some(hash),
                )
                .map_err(invariant)?,
                event.update(),
            )
            .map_err(invariant)?;
        let projection = PmBookDecisionProjection::from_reduced_owner(
            &self.reducer,
            &event,
            ordering,
            monotonic_ns,
        );
        self.book_sequence = self.book_sequence.saturating_add(1);
        PmBookInput::from_evidence(
            self.pm_connection,
            ordering,
            event_clock(monotonic_ns)?,
            event,
            projection,
        )
        .map_err(invariant)
    }

    pub(super) fn next_book_input(
        &mut self,
        monotonic_ns: u64,
    ) -> Result<PmBookInput, PmEvidenceError> {
        let bid = PmPrice::parse_decimal("0.30").map_err(invariant)?;
        let ask = PmPrice::parse_decimal("0.60").map_err(invariant)?;
        let event = PmBookEvent::new(
            self.pm_source,
            self.instrument,
            SnapshotRevision::new(1),
            PmBookUpdate::TopCheck(reap_pm_core::PmBookTopCheck::new(Some(bid), Some(ask))),
        )
        .map_err(invariant)?;
        let sequence = self.book_sequence;
        let ordering = ordering(Some(1), sequence, None)?;
        self.reducer
            .apply_update(
                PmBookBatchEvidence::new(
                    self.instrument,
                    ConnectionEpoch::new(1),
                    SnapshotRevision::new(1),
                    SnapshotRevision::new(1),
                    IngressSequence::new(sequence),
                    monotonic_ns,
                    None,
                )
                .map_err(invariant)?,
                event.update(),
            )
            .map_err(invariant)?;
        let projection = PmBookDecisionProjection::from_reduced_owner(
            &self.reducer,
            &event,
            ordering,
            monotonic_ns,
        );
        self.book_sequence = sequence.saturating_add(1);
        PmBookInput::from_evidence(
            self.pm_connection,
            ordering,
            event_clock(monotonic_ns)?,
            event,
            projection,
        )
        .map_err(invariant)
    }

    pub(super) fn next_reference_input(
        &mut self,
        monotonic_ns: u64,
    ) -> Result<PmOkxReferenceInput, PmEvidenceError> {
        let sequence = self.reference_sequence;
        self.reference_sequence = sequence.saturating_add(1);
        let event =
            OkxReferenceEvent::new(self.okx_source, self.okx_reference, self.reference_price)
                .map_err(invariant)?;
        Ok(PmOkxReferenceInput::from_evidence(
            self.okx_connection,
            ordering(None, sequence, None)?,
            event_clock(monotonic_ns)?,
            event,
        ))
    }
}
