use reap_pm_live_contracts::PmConnectivityConfig;

use super::*;
use crate::journal::PmSealedJournalProjection;

/// Fixed read-only projection used by the opaque Phase-6 evidence runner.
///
/// It exposes no receipt, acknowledgement, record, or execution authority.
#[derive(Debug, Clone, Copy)]
pub(crate) struct PmMutationEvidenceProjection {
    pub(crate) journal: PmSealedJournalProjection,
    pub(crate) pending_persistence: usize,
    pub(crate) pending_fake_effects: usize,
    pub(crate) retained_fake_effect_permits: usize,
    pub(crate) pending_durable_consequences: usize,
    pub(crate) owned_orders: usize,
    pub(crate) private: reap_pm_state::PmPrivateCardinalities,
    pub(crate) reconciliation_reductions: usize,
}

impl PmMutationOwner {
    /// Starts the one internally selected sealed journal for the fixed,
    /// zero-input benchmark runner.
    ///
    /// The caller cannot select a backend or supply records, sequences,
    /// acknowledgements, or effects.
    pub(crate) fn start_sealed_evidence(
        config: &PmConnectivityConfig,
        private: Box<PmPrivateMonitorRuntime>,
        fake: PmFakeEffectRole,
    ) -> Result<Self, PmMutationError> {
        let scope = PmJournalScopeV1::from_config(config)?;
        let instrument_scope = PmFixtureInstrumentScope::from_metadata(
            config.account().instrument(),
            config.account().expected_metadata(),
        )?;
        let instrument_id = config.account().instrument_id();
        if private.account_scope() != scope.account_scope()
            || private.instrument() != config.account().instrument()
            || fake.account_scope() != scope.account_scope()
            || fake.instrument() != config.account().instrument()
            || fake.instrument_id() != instrument_id
        {
            return Err(PmMutationError::CompositionScopeMismatch);
        }
        Ok(Self {
            scope: scope.clone(),
            instrument_scope,
            instrument_id,
            private,
            fake,
            journal: PmMutationJournal::start_sealed_evidence(scope),
            persistence: PmPersistenceQueue::new(),
            effects: PmFakeEffectQueue::new()?,
            durable_consequences: VecDeque::with_capacity(PM_PENDING_PERSISTENCE_CAPACITY),
            reconciliation_reductions: PmReconciliationReductions::new(),
            next_intent_id: 1,
            current_revisions: None,
            halt: None,
            counters: PmMutationCounters::default(),
        })
    }

    pub(crate) fn sealed_evidence_projection(&self) -> Option<PmMutationEvidenceProjection> {
        Some(PmMutationEvidenceProjection {
            journal: self.journal.sealed_evidence_projection()?,
            pending_persistence: self.persistence.len(),
            pending_fake_effects: self.effects.queued_len(),
            retained_fake_effect_permits: self.effects.retained_permits(),
            pending_durable_consequences: self.durable_consequences.len(),
            owned_orders: self.private.owned_orders().count(),
            private: self.private.cardinalities(),
            reconciliation_reductions: self.reconciliation_reductions.len(),
        })
    }

    pub(crate) fn begin_sealed_evidence_segment(&mut self) -> bool {
        self.journal.begin_sealed_evidence_segment()
    }
}
