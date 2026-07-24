use std::path::PathBuf;

use reap_pm_live_contracts::PmConnectivityConfig;
use reap_pm_state::PmRiskLimits;
use reap_pm_strategy::PmQuoteModel;

use super::*;
use crate::fake_effect::PmFakeEffectRole;
use crate::journal::{PmJournalRecovery, PmSealedJournalProjection};
use crate::private_monitor::PmPrivateMonitorRuntime;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PmEvidenceTerminalLengths {
    pub(crate) critical_lane: usize,
    pub(crate) persistence_lane: usize,
    pub(crate) private_lane: usize,
    pub(crate) scheduled_lane: usize,
    pub(crate) public_lane: usize,
    pub(crate) reconciliation_lane: usize,
    pub(crate) telemetry_lane: usize,
    pub(crate) pending_persistence: usize,
    pub(crate) pending_fake_effects: usize,
    pub(crate) retained_fake_effect_permits: usize,
    pub(crate) pending_durable_consequences: usize,
    pub(crate) reconciliation_reductions: usize,
    pub(crate) canonical_orders: usize,
    pub(crate) owned_orders: usize,
    pub(crate) owned_quote_slots: usize,
    pub(crate) canonical_fills: usize,
    pub(crate) owned_fill_keys: usize,
    pub(crate) unresolved_fills: usize,
    pub(crate) canonical_refresh: usize,
    pub(crate) fill_compaction_pending: usize,
    pub(crate) pending_correlations: usize,
    pub(crate) prepared_correlations: usize,
    pub(crate) tracked_quote_slots: usize,
    pub(crate) pending_schedules: usize,
    pub(crate) copied_outputs: usize,
    pub(crate) retained_admissions: usize,
    pub(crate) refresh_obligations: usize,
}

impl PmEvidenceTerminalLengths {
    pub(crate) fn all_zero(self) -> bool {
        let Self {
            critical_lane,
            persistence_lane,
            private_lane,
            scheduled_lane,
            public_lane,
            reconciliation_lane,
            telemetry_lane,
            pending_persistence,
            pending_fake_effects,
            retained_fake_effect_permits,
            pending_durable_consequences,
            reconciliation_reductions,
            canonical_orders,
            owned_orders,
            owned_quote_slots,
            canonical_fills,
            owned_fill_keys,
            unresolved_fills,
            canonical_refresh,
            fill_compaction_pending,
            pending_correlations,
            prepared_correlations,
            tracked_quote_slots,
            pending_schedules,
            copied_outputs,
            retained_admissions,
            refresh_obligations,
        } = self;
        [
            critical_lane,
            persistence_lane,
            private_lane,
            scheduled_lane,
            public_lane,
            reconciliation_lane,
            telemetry_lane,
            pending_persistence,
            pending_fake_effects,
            retained_fake_effect_permits,
            pending_durable_consequences,
            reconciliation_reductions,
            canonical_orders,
            owned_orders,
            owned_quote_slots,
            canonical_fills,
            owned_fill_keys,
            unresolved_fills,
            canonical_refresh,
            fill_compaction_pending,
            pending_correlations,
            prepared_correlations,
            tracked_quote_slots,
            pending_schedules,
            copied_outputs,
            retained_admissions,
            refresh_obligations,
        ]
        .into_iter()
        .all(|length| length == 0)
    }
}

impl<M: PmQuoteModel> PmCoordinator<M> {
    /// Starts the fixed bare-lane coordinator used by the opaque local
    /// benchmark. The sealed acknowledgement implementation is selected
    /// internally and cannot be supplied by a caller.
    pub(crate) fn start_sealed_evidence(
        config: &PmConnectivityConfig,
        model: M,
        risk_limits: PmRiskLimits,
        policy: PmCoordinatorPolicy,
    ) -> Result<Self, String> {
        let private = Box::new(
            PmPrivateMonitorRuntime::new(config.account(), risk_limits)
                .map_err(|error| error.to_string())?,
        );
        let fake = fixed_fake_role(config);
        let mutation = PmMutationOwner::start_sealed_evidence(config, private, fake)
            .map_err(|error| error.to_string())?;
        Self::from_evidence_parts(config, model, mutation, policy)
    }

    /// Starts the same bare-lane owner with the production filesystem journal.
    /// This is used only by `combined_replay`; no sealed path is available.
    pub(crate) async fn start_real_writer_evidence(
        config: &PmConnectivityConfig,
        model: M,
        risk_limits: PmRiskLimits,
        journal_path: PathBuf,
        policy: PmCoordinatorPolicy,
    ) -> Result<(Self, PmJournalRecovery), String> {
        let private = Box::new(
            PmPrivateMonitorRuntime::new(config.account(), risk_limits)
                .map_err(|error| error.to_string())?,
        );
        let fake = fixed_fake_role(config);
        let (mutation, recovery) = PmMutationOwner::start(config, private, fake, journal_path)
            .await
            .map_err(|error| error.to_string())?;
        let coordinator = Self::from_evidence_parts(config, model, mutation, policy)?;
        Ok((coordinator, recovery))
    }

    fn from_evidence_parts(
        config: &PmConnectivityConfig,
        model: M,
        mutation: PmMutationOwner,
        policy: PmCoordinatorPolicy,
    ) -> Result<Self, String> {
        let decision =
            PmDecisionState::new(config, model, policy).map_err(|error| error.to_string())?;
        let account_route = config.account().account_route();
        Ok(Self {
            decision,
            account_source: account_route.source(),
            account_connection: account_route.connection(),
            account_scope: config.account().account_scope(),
            instrument: config.public().instrument(),
            mutation: Box::new(mutation),
            lanes: Some(Box::new(PmCompleteInputLanes::for_instrument(
                config.public().instrument(),
            ))),
            outputs: PmProductEffectOutput::new(),
            private_readiness_revision: 1,
            last_action_sequence: 0,
            pending_correlations: CorrelationRing::new(),
            prepared_correlations: CorrelationRing::new(),
            tracked_quotes: [None; 2],
            halt: None,
            counters: PmCoordinatorCounters::default(),
            callback_error: None,
            retained_critical: None,
            retained_persistence: None,
            retained_private_admission: None,
            retained_reconciliation_admission: None,
            pending_schedules: PendingSchedules::new(),
            refresh_obligations: refresh_obligations::PmRefreshObligations::new(),
            reconciliation_gate: false,
            reconciliation_recovered: false,
        })
    }

    pub(crate) fn service_reference_evidence(
        &mut self,
        input: PmOkxReferenceInput,
    ) -> Result<(), PmCoordinatorError> {
        let mut effects = PmProductEffectBatch::new();
        let result = self.service_reference(input, &mut effects);
        self.finish_evidence_callback(effects, result)
    }

    pub(crate) fn service_market_evidence(
        &mut self,
        input: PmMarketInput,
    ) -> Result<(), PmCoordinatorError> {
        let mut effects = PmProductEffectBatch::new();
        let result = self.service_market(input, &mut effects);
        self.finish_evidence_callback(effects, result)
    }

    pub(crate) fn service_book_evidence(
        &mut self,
        input: PmBookInput,
    ) -> Result<(), PmCoordinatorError> {
        let mut effects = PmProductEffectBatch::new();
        let result = self.service_book(input, &mut effects);
        self.finish_evidence_callback(effects, result)
    }

    fn finish_evidence_callback(
        &mut self,
        effects: PmProductEffectBatch,
        result: Result<(), PmCoordinatorError>,
    ) -> Result<(), PmCoordinatorError> {
        self.complete_callback(effects, result);
        if let Some(error) = self.callback_error.take() {
            return Err(error);
        }
        self.flush_pending_schedules()
    }

    pub(crate) fn sealed_journal_projection(&self) -> Option<PmSealedJournalProjection> {
        self.mutation
            .sealed_evidence_projection()
            .map(|projection| projection.journal)
    }

    pub(crate) fn begin_sealed_evidence_segment(&mut self) -> bool {
        self.mutation.begin_sealed_evidence_segment()
    }

    pub(crate) fn evidence_terminal_state_lengths(
        &mut self,
        monotonic_now_ns: u64,
    ) -> Result<PmEvidenceTerminalLengths, PmCoordinatorError> {
        let projection = self
            .mutation
            .sealed_evidence_projection()
            .expect("sealed evidence coordinator retains its fixed sealed journal");
        let refresh = self.refresh_obligation_metrics();
        let scheduler = self.scheduler_metrics(monotonic_now_ns)?;
        let depth = |lane| {
            scheduler
                .lane(lane)
                .map_or(0, |metrics| metrics.queue().depth())
        };
        let private = projection.private;
        Ok(PmEvidenceTerminalLengths {
            critical_lane: depth(crate::lanes::PmLaneKind::Critical),
            persistence_lane: depth(crate::lanes::PmLaneKind::Persistence),
            private_lane: depth(crate::lanes::PmLaneKind::Private),
            scheduled_lane: depth(crate::lanes::PmLaneKind::Scheduled),
            public_lane: depth(crate::lanes::PmLaneKind::Public),
            reconciliation_lane: depth(crate::lanes::PmLaneKind::Reconciliation),
            telemetry_lane: depth(crate::lanes::PmLaneKind::Telemetry),
            pending_persistence: projection.pending_persistence,
            pending_fake_effects: projection.pending_fake_effects,
            retained_fake_effect_permits: projection.retained_fake_effect_permits,
            pending_durable_consequences: projection.pending_durable_consequences,
            reconciliation_reductions: projection.reconciliation_reductions,
            canonical_orders: private.canonical_orders,
            owned_orders: private.owned_orders.max(projection.owned_orders),
            owned_quote_slots: private.owned_quote_slots,
            canonical_fills: private.canonical_fills,
            owned_fill_keys: private.owned_fill_keys,
            unresolved_fills: private.unresolved_fills,
            canonical_refresh: private.pending_refresh,
            fill_compaction_pending: usize::from(private.fill_compaction_pending),
            pending_correlations: usize::from(self.pending_correlations.len),
            prepared_correlations: usize::from(self.prepared_correlations.len),
            tracked_quote_slots: self.tracked_quotes.iter().flatten().count(),
            pending_schedules: self.pending_schedules.values.iter().flatten().count(),
            copied_outputs: self.outputs.len(),
            retained_admissions: usize::from(self.retained_critical.is_some())
                + usize::from(self.retained_persistence.is_some())
                + usize::from(self.retained_private_admission.is_some())
                + usize::from(self.retained_reconciliation_admission.is_some()),
            refresh_obligations: refresh
                .fill_observed_pending()
                .saturating_add(refresh.fill_observed_in_flight())
                .saturating_add(refresh.external_ingress_pending())
                .saturating_add(refresh.external_ingress_in_flight()),
        })
    }

    pub(crate) async fn shutdown_evidence(self) -> Result<(), PmMutationError> {
        let Self { mutation, .. } = self;
        mutation.shutdown().await
    }
}

fn fixed_fake_role(config: &PmConnectivityConfig) -> PmFakeEffectRole {
    PmFakeEffectRole::new(
        config.account().account_scope(),
        config.account().instrument(),
        config.account().instrument_id(),
    )
}
