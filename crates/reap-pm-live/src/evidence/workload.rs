use std::fmt::Write as _;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use reap_benchmark_allocator::{AllocationSnapshot, MeasurementWindow};
use reap_pm_core::{
    ConnectionEpoch, EventClock, EventOrdering, IngressSequence, OkxReferencePrice,
    PmFillQueryCursor, PmMarketEvent, PmOrderSide, PmVenueOrderId, PmVenueOrderKey,
    SnapshotRevision, VenueEventHash,
};
use reap_polymarket_adapter::{
    PmFakeCancelScript, PmFakePlaceScript, PmFixtureCompletionOccurrence, PmFixtureFeeEvidence,
};
use sha2::{Digest, Sha256};

use super::PmEvidenceError;
use super::contract::{
    ACTION_SAMPLES, MEASURED_CYCLES, REPEATED_NOMINAL_PASSES, WARMUP_CYCLES, is_cancel_cycle,
};
use super::fixture::{
    MARKET, PM_FUNDER, Phase6Model, completion, connectivity_config, coordinator_policy,
    market_metadata, model, query_occurrence, risk_limits,
};
use super::report::{
    AllocationReport, CapacityReport, InputMixReport, NominalCounters, PassProjection,
    SetupCounters, TerminalStateLengths, hex,
};
use crate::coordinator::{
    PmBookDecisionProjection, PmCoordinator, PmCoordinatorCounters, PmMarketInput,
    PmMutationCounters, PmRefreshObligationMetrics,
};
use crate::journal::{PmJournalScopeV1, PmSealedJournalProjection, PmSealedJournalRecordCounts};
use crate::private_monitor::{
    PmOpenOrdersFixtureInput, PmOrderDetailFixtureInput, PmReconciliationFixtureInput,
};
use crate::schedule::PmScheduledActionKind;

mod accounting;
mod capacity;
mod fixtures;
mod projection;
mod validation;

use accounting::{validate_input_mix, validate_setup};
use fixtures::{AccountRows, PublicFixture, WorkloadCursor};
use projection::EffectProjection;
use validation::{validate_nominal, validate_repeated_passes};

const WALL_BASE: u64 = 1_700_000_000_000_000_000;
const DURABILITY_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, Copy)]
pub(crate) struct BenchmarkWarmup {
    pub(crate) setup: SetupCounters,
    pub(crate) input_mix: InputMixReport,
    pub(crate) counters: NominalCounters,
}

pub(crate) struct BenchmarkOutcome {
    pub(crate) warmup: BenchmarkWarmup,
    pub(crate) recorded_setup: SetupCounters,
    pub(crate) measured_input_mix: InputMixReport,
    pub(crate) measured: NominalCounters,
    pub(crate) repeated_passes: Vec<PassProjection>,
    pub(crate) action_latencies_ns: Vec<u64>,
    pub(crate) owner_public_projection_sha256: String,
    pub(crate) capacities: CapacityReport,
    pub(crate) total_elapsed_ns: u128,
    pub(crate) owner_allocations: AllocationReport,
}

pub(crate) struct RealWriterOutcome {
    pub(crate) setup: SetupCounters,
    pub(crate) input_mix: InputMixReport,
    pub(crate) counters: NominalCounters,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DurabilityMode {
    Sealed,
    RealWriter,
}

#[derive(Debug, Clone, Copy)]
struct PassRawProjection {
    input_mix: InputMixReport,
    counters: NominalCounters,
    journal_record_delta: u64,
    journal_hash: [u8; 32],
    logical_hash: [u8; 32],
    public_hash: [u8; 32],
    reserved_capacity_bytes: usize,
    terminal_state_lengths_zero: bool,
    terminal_state_lengths: TerminalStateLengths,
    allocator_live_bytes: i64,
    owner_elapsed_ns: u128,
}

struct EvidenceRun {
    owner: PmCoordinator<Phase6Model>,
    public: PublicFixture,
    cursor: WorkloadCursor,
    account: AccountRows,
    journal_fingerprint: crate::journal::PmJournalFingerprintV1,
    raw_fill: String,
    venue_id: String,
    setup: SetupCounters,
}

pub(crate) fn run_benchmark_warmup() -> Result<BenchmarkWarmup, PmEvidenceError> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| PmEvidenceError::invariant(error.to_string()))?;
    runtime.block_on(Box::pin(run_benchmark_warmup_async()))
}

async fn run_benchmark_warmup_async() -> Result<BenchmarkWarmup, PmEvidenceError> {
    let mut warmup = EvidenceRun::start_sealed()?;
    let warmup_result = warmup
        .run_pass(WARMUP_CYCLES, DurabilityMode::Sealed, None, None)
        .await?;
    warmup.owner.shutdown_evidence().await.map_err(invariant)?;
    Ok(BenchmarkWarmup {
        setup: warmup.setup,
        input_mix: warmup_result.input_mix,
        counters: warmup_result.counters,
    })
}

pub(crate) fn run_benchmark_workload(
    warmup: BenchmarkWarmup,
) -> Result<BenchmarkOutcome, PmEvidenceError> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| PmEvidenceError::invariant(error.to_string()))?;
    runtime.block_on(Box::pin(run_benchmark_workload_async(warmup)))
}

async fn run_benchmark_workload_async(
    warmup: BenchmarkWarmup,
) -> Result<BenchmarkOutcome, PmEvidenceError> {
    verify_tracking_allocator()?;
    let mut measured = EvidenceRun::start_sealed()?;
    let mut action_latencies = Vec::with_capacity(ACTION_SAMPLES);
    let mut raw_passes = Vec::with_capacity(REPEATED_NOMINAL_PASSES);
    let mut allocation_window = reap_benchmark_allocator::start_measurement().map_err(invariant)?;
    let mut previous_checkpoint = AllocationSnapshot::default();
    let mut primary_elapsed = 0_u128;
    let mut primary_public_hash = [0_u8; 32];
    for ordinal in 1..=REPEATED_NOMINAL_PASSES {
        let latency = (ordinal == 1).then_some(&mut action_latencies);
        let mut pass = measured
            .run_pass(
                MEASURED_CYCLES,
                DurabilityMode::Sealed,
                Some(&mut allocation_window),
                latency,
            )
            .await?;
        let checkpoint = allocation_window.checkpoint().map_err(invariant)?;
        pass.allocator_live_bytes = checkpoint
            .live_bytes_delta
            .saturating_sub(previous_checkpoint.live_bytes_delta);
        previous_checkpoint = checkpoint;
        if ordinal == 1 {
            primary_elapsed = pass.owner_elapsed_ns;
            primary_public_hash = pass.public_hash;
        }
        raw_passes.push(pass);
    }
    let owner_snapshot = allocation_window.stop().map_err(invariant)?;
    if owner_snapshot != previous_checkpoint {
        return Err(PmEvidenceError::invariant(
            "stopped allocation snapshot differs from the last terminal checkpoint",
        ));
    }
    if owner_snapshot.allocation_calls != 0 || owner_snapshot.allocated_bytes != 0 {
        return Err(PmEvidenceError::invariant(format!(
            "normalized owner path requested an allocation: {owner_snapshot:?}"
        )));
    }
    if action_latencies.len() != ACTION_SAMPLES {
        return Err(PmEvidenceError::invariant(format!(
            "action sample count {}, expected {ACTION_SAMPLES}",
            action_latencies.len()
        )));
    }
    validate_repeated_passes(&raw_passes)?;
    let capacities = measured.capacity_report()?;
    let measured_counters = raw_passes
        .first()
        .map(|pass| pass.counters)
        .ok_or_else(|| PmEvidenceError::invariant("nominal pass list is empty"))?;
    let measured_input_mix = raw_passes
        .first()
        .map(|pass| pass.input_mix)
        .ok_or_else(|| PmEvidenceError::invariant("nominal pass list is empty"))?;
    let repeated_passes = raw_passes
        .into_iter()
        .enumerate()
        .map(|(index, pass)| PassProjection {
            ordinal: index + 1,
            input_mix: pass.input_mix,
            counters: pass.counters,
            journal_record_delta: pass.journal_record_delta,
            journal_hash: hex(pass.journal_hash),
            logical_hash: hex(pass.logical_hash),
            reserved_capacity_bytes: pass.reserved_capacity_bytes,
            terminal_state_lengths_zero: pass.terminal_state_lengths_zero,
            terminal_state_lengths: pass.terminal_state_lengths,
            allocator_live_bytes: pass.allocator_live_bytes,
        })
        .collect();
    measured
        .owner
        .shutdown_evidence()
        .await
        .map_err(invariant)?;
    Ok(BenchmarkOutcome {
        warmup,
        recorded_setup: measured.setup,
        measured_input_mix,
        measured: measured_counters,
        repeated_passes,
        action_latencies_ns: action_latencies,
        owner_public_projection_sha256: hex(primary_public_hash),
        capacities,
        total_elapsed_ns: primary_elapsed,
        owner_allocations: owner_snapshot.into(),
    })
}

fn verify_tracking_allocator() -> Result<(), PmEvidenceError> {
    let window = reap_benchmark_allocator::start_measurement().map_err(invariant)?;
    let probe = Box::new([0_u8; 64]);
    std::hint::black_box(&probe);
    drop(probe);
    let snapshot = window.stop().map_err(invariant)?;
    if snapshot.allocation_calls == 0 || snapshot.deallocation_calls == 0 {
        return Err(PmEvidenceError::invariant(
            "benchmark target did not install TrackingAllocator",
        ));
    }
    Ok(())
}

pub(crate) async fn run_real_writer_workload(
    journal_path: PathBuf,
) -> Result<RealWriterOutcome, PmEvidenceError> {
    let mut run = EvidenceRun::start_real(journal_path).await?;
    let pass = run
        .run_pass(MEASURED_CYCLES, DurabilityMode::RealWriter, None, None)
        .await?;
    run.owner.shutdown_evidence().await.map_err(invariant)?;
    Ok(RealWriterOutcome {
        setup: run.setup,
        input_mix: pass.input_mix,
        counters: pass.counters,
    })
}

pub(crate) fn journal_scope() -> Result<PmJournalScopeV1, PmEvidenceError> {
    PmJournalScopeV1::from_config(&connectivity_config()).map_err(invariant)
}

impl EvidenceRun {
    fn start_sealed() -> Result<Self, PmEvidenceError> {
        let config = connectivity_config();
        let owner = PmCoordinator::start_sealed_evidence(
            &config,
            model(),
            risk_limits(),
            coordinator_policy(),
        )
        .map_err(PmEvidenceError::invariant)?;
        let mut run = Self::new(owner, &config)?;
        run.prepare_private_sealed()?;
        run.prepare_public()?;
        validate_setup(run.setup, None)?;
        Ok(run)
    }

    async fn start_real(journal_path: PathBuf) -> Result<Self, PmEvidenceError> {
        let config = connectivity_config();
        let (owner, recovery) = PmCoordinator::start_real_writer_evidence(
            &config,
            model(),
            risk_limits(),
            journal_path,
            coordinator_policy(),
        )
        .await
        .map_err(PmEvidenceError::invariant)?;
        if recovery.record_count() != 0 {
            return Err(PmEvidenceError::invariant(
                "fresh combined replay journal recovered existing records",
            ));
        }
        let mut run = Self::new(owner, &config)?;
        run.prepare_private_real().await?;
        run.prepare_public()?;
        validate_setup(run.setup, Some(2))?;
        Ok(run)
    }

    fn new(
        owner: PmCoordinator<Phase6Model>,
        config: &reap_pm_live_contracts::PmConnectivityConfig,
    ) -> Result<Self, PmEvidenceError> {
        let sealed = owner.sealed_journal_projection();
        if let Some(projection) = sealed {
            validate_sealed_header(projection)?;
        }
        Ok(Self {
            owner,
            public: PublicFixture::new(config)?,
            cursor: WorkloadCursor::after_setup(),
            account: AccountRows::new(config)?,
            journal_fingerprint: PmJournalScopeV1::from_config(config)
                .map_err(invariant)?
                .fingerprint(),
            raw_fill: String::with_capacity(512),
            venue_id: String::with_capacity(32),
            setup: SetupCounters {
                journal_header_records: 1,
                physical_journal_lines: sealed.is_none().then_some(2),
                ..SetupCounters::default()
            },
        })
    }

    fn prepare_private_sealed(&mut self) -> Result<(), PmEvidenceError> {
        self.prepare_private_inputs()?;
        let reductions = self.acknowledge_one_sealed(150)?;
        self.observe_setup_ack(reductions)?;
        self.drain_effects(&mut EffectProjection::new());
        self.validate_sealed_setup()?;
        Ok(())
    }

    async fn prepare_private_real(&mut self) -> Result<(), PmEvidenceError> {
        self.prepare_private_inputs()?;
        let mut effects = EffectProjection::new();
        let reductions = self.acknowledge_setup_real(150, &mut effects).await?;
        self.observe_setup_ack(reductions)?;
        Ok(())
    }

    fn prepare_private_inputs(&mut self) -> Result<(), PmEvidenceError> {
        self.owner
            .connect_private_fixture(completion(1, 1, None, 120))
            .map_err(invariant)?;
        self.settle(121, &mut EffectProjection::new())?;
        self.setup.bootstrap.private_connection_completion = self
            .setup
            .bootstrap
            .private_connection_completion
            .saturating_add(1);

        let empty: [&[u8]; 0] = [];
        self.owner
            .ingest_open_orders_fixture(PmOpenOrdersFixtureInput::new(
                query_occurrence(1, 2, 3, 1, 130).map_err(PmEvidenceError::invariant)?,
                &empty,
            ))
            .map_err(invariant)?;
        self.settle(132, &mut EffectProjection::new())?;
        self.setup.bootstrap.open_orders_snapshot =
            self.setup.bootstrap.open_orders_snapshot.saturating_add(1);

        let no_fills: [&[u8]; 0] = [];
        let input = PmReconciliationFixtureInput::new(
            query_occurrence(1, 4, 5, 2, 140).map_err(PmEvidenceError::invariant)?,
            &self.account.balances,
            &self.account.allowances,
            &self.account.positions,
            None,
            PmFillQueryCursor::new(self.account.scope, [1; 32]),
            &no_fills,
            PmFixtureFeeEvidence::Unknown,
        );
        self.owner
            .ingest_reconciliation_fixture(input)
            .map_err(invariant)?;
        self.setup.w0_paired_reconciliations =
            self.setup.w0_paired_reconciliations.saturating_add(1);
        self.setup.w0_external_observations = self.setup.w0_external_observations.saturating_add(1);
        let reductions = self.settle_count(142, &mut EffectProjection::new())?;
        self.setup.w0_owner_reductions = self.setup.w0_owner_reductions.saturating_add(reductions);
        Ok(())
    }

    fn prepare_public(&mut self) -> Result<(), PmEvidenceError> {
        let market_clock = event_clock(180)?;
        let market_ordering = ordering(None, 1, None)?;
        let market = PmMarketEvent::new(
            self.public.pm_source,
            self.public.instrument,
            SnapshotRevision::new(1),
            market_metadata(),
        )
        .map_err(invariant)?;
        self.owner
            .service_market_evidence(PmMarketInput::from_evidence(
                self.public.pm_connection,
                market_ordering,
                market_clock,
                market,
            ))
            .map_err(invariant)?;
        self.setup.bootstrap.initial_market_metadata = self
            .setup
            .bootstrap
            .initial_market_metadata
            .saturating_add(1);
        let snapshot = self.public.snapshot_input(190)?;
        self.owner
            .service_book_evidence(snapshot)
            .map_err(invariant)?;
        self.setup.bootstrap.initial_pm_book_snapshot = self
            .setup
            .bootstrap
            .initial_pm_book_snapshot
            .saturating_add(1);
        self.drain_effects(&mut EffectProjection::new());
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    async fn run_pass(
        &mut self,
        cycles: usize,
        durability: DurabilityMode,
        mut allocation: Option<&mut MeasurementWindow>,
        mut action_latencies: Option<&mut Vec<u64>>,
    ) -> Result<PassRawProjection, PmEvidenceError> {
        self.cursor.align_pass_start()?;
        if durability == DurabilityMode::Sealed && !self.owner.begin_sealed_evidence_segment() {
            return Err(PmEvidenceError::invariant(
                "fixed action-path owner did not retain its sealed evidence ledger",
            ));
        }
        let before = CounterCut::capture(&self.owner, self.cursor.internal_fact_acks);
        let journal_before = self.owner.sealed_journal_projection();
        let capacity_before = self.owner.reserved_capacity_bytes();
        let mut effects = EffectProjection::for_pass(
            self.journal_fingerprint,
            self.account.scope.handle(),
            self.cursor.absolute_cycle.saturating_add(1),
            before.mutation.quote_intents().saturating_add(1),
        );
        let mut public_projection = Sha256::new();
        let mut input_mix = InputMixReport::default();
        let pass_started = Instant::now();
        let mut excluded_elapsed_ns = 0_u128;
        for cycle in 1..=cycles {
            self.run_cycle(
                cycle,
                durability,
                allocation.as_deref_mut(),
                action_latencies.as_deref_mut(),
                &mut effects,
                &mut input_mix,
                &mut public_projection,
                &mut excluded_elapsed_ns,
            )
            .await?;
        }
        let owner_elapsed_ns = pass_started
            .elapsed()
            .as_nanos()
            .saturating_sub(excluded_elapsed_ns);
        let after = CounterCut::capture(&self.owner, self.cursor.internal_fact_acks);
        let journal_after = self.owner.sealed_journal_projection();
        validate_input_mix(cycles, input_mix)?;
        let counters = nominal_delta(before, after, &effects, input_mix);
        validate_nominal(cycles, counters, before, after)?;
        let capacity_after = self.owner.reserved_capacity_bytes();
        if capacity_after != capacity_before {
            return Err(PmEvidenceError::invariant(format!(
                "reserved owner capacity changed from {capacity_before} to {capacity_after}"
            )));
        }
        let (journal_record_delta, journal_hash) = journal_delta(
            journal_before,
            journal_after,
            cycles,
            counters.journal_records,
        )?;
        let digest = public_projection.finalize();
        let mut public_hash = [0; 32];
        public_hash.copy_from_slice(&digest);
        let mut logical_hash = effects.finish_hash()?;
        for (target, byte) in logical_hash.iter_mut().zip(digest.iter()) {
            *target ^= *byte;
        }
        let (terminal_state_lengths, terminal_state_lengths_zero) =
            if durability == DurabilityMode::Sealed {
                let lengths = self
                    .owner
                    .evidence_terminal_state_lengths(self.cursor.monotonic_ns)
                    .map_err(invariant)?;
                (TerminalStateLengths::from(lengths), lengths.all_zero())
            } else {
                (
                    TerminalStateLengths::default(),
                    self.owner.persistence_metrics().depth() == 0
                        && self.owner.fake_effect_metrics().depth() == 0
                        && self.owner.pending_effect_outputs() == 0,
                )
            };
        Ok(PassRawProjection {
            input_mix,
            counters,
            journal_record_delta,
            journal_hash,
            logical_hash,
            public_hash,
            reserved_capacity_bytes: capacity_after,
            terminal_state_lengths_zero,
            terminal_state_lengths,
            allocator_live_bytes: 0,
            owner_elapsed_ns,
        })
    }

    #[allow(clippy::too_many_arguments)]
    async fn run_cycle(
        &mut self,
        cycle: usize,
        durability: DurabilityMode,
        allocation: Option<&mut MeasurementWindow>,
        action_latencies: Option<&mut Vec<u64>>,
        effects: &mut EffectProjection,
        input_mix: &mut InputMixReport,
        public_projection: &mut Sha256,
        excluded_elapsed_ns: &mut u128,
    ) -> Result<(), PmEvidenceError> {
        let mut allocation = allocation;
        let mut action_latencies = action_latencies;
        let absolute_cycle = self.cursor.next_cycle();
        let venue = self.build_cycle_fixtures(
            absolute_cycle,
            allocation.as_deref_mut(),
            excluded_elapsed_ns,
        )?;

        let book_ns = self.cursor.next_time();
        let book = with_paused(allocation.as_deref_mut(), excluded_elapsed_ns, || {
            self.public.next_book_input(book_ns)
        })?;
        hash_book_projection(public_projection, book.projection())?;
        self.owner.service_book_evidence(book).map_err(invariant)?;
        self.drain_effects(effects);
        input_mix.pm_book_observations = input_mix.pm_book_observations.saturating_add(1);

        let reference_ns = self.cursor.next_time();
        let reference = with_paused(allocation.as_deref_mut(), excluded_elapsed_ns, || {
            self.public.next_reference_input(reference_ns)
        })?;
        hash_reference_projection(public_projection, reference.event().price());
        self.owner
            .service_reference_evidence(reference)
            .map_err(invariant)?;
        self.drain_effects(effects);
        input_mix.okx_reference_observations =
            input_mix.okx_reference_observations.saturating_add(1);

        let quote_started = Instant::now();
        let service_ns = self.cursor.next_time();
        let quote_service_count = self.service_one_undrained(service_ns)?;
        let acknowledgement_ns = self.cursor.next_time();
        self.validate_pending_acknowledgement(false)?;
        let quote_acknowledgement_count = self
            .acknowledge_one_undrained(acknowledgement_ns, durability)
            .await?;
        let quote_action_ns = elapsed_ns(quote_started);
        Self::validate_service_count(quote_service_count)?;
        Self::validate_service_count(quote_acknowledgement_count)?;
        self.drain_effects(effects);
        input_mix.quote_evaluation_timers = input_mix.quote_evaluation_timers.saturating_add(1);
        input_mix.quote_intent_acknowledgements =
            input_mix.quote_intent_acknowledgements.saturating_add(1);
        if let Some(samples) = action_latencies.as_deref_mut() {
            samples.push(quote_action_ns);
        }

        self.owner
            .execute_prepared_quote_fixture(
                self.cursor.next_completion(None),
                PmFakePlaceScript::acknowledged(venue, Box::new([])).map_err(invariant)?,
                self.cursor.next_time(),
            )
            .map_err(invariant)?;
        let service_ns = self.cursor.next_time();
        self.service_one(service_ns, effects)?;
        input_mix.fake_place_acceptances = input_mix.fake_place_acceptances.saturating_add(1);
        let acknowledgement_ns = self.cursor.next_time();
        self.acknowledge_one(acknowledgement_ns, durability, true, effects)
            .await?;

        if is_cancel_cycle(cycle) {
            self.run_cancel_cycle(
                venue,
                durability,
                allocation.as_deref_mut(),
                action_latencies,
                effects,
                input_mix,
                excluded_elapsed_ns,
            )
            .await?;
        } else {
            self.run_fill_cycle(
                venue,
                durability,
                allocation,
                effects,
                input_mix,
                excluded_elapsed_ns,
            )
            .await?;
        }
        self.service_freshness(effects)?;
        if is_cancel_cycle(cycle) {
            input_mix.cancel_freshness_timers = input_mix.cancel_freshness_timers.saturating_add(1);
        } else {
            input_mix.fill_freshness_timers = input_mix.fill_freshness_timers.saturating_add(1);
        }
        Ok(())
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "the evidence cycle keeps allocation, latency, effects, counters, and excluded-time accounting explicit"
    )]
    async fn run_cancel_cycle(
        &mut self,
        venue: PmVenueOrderKey,
        durability: DurabilityMode,
        allocation: Option<&mut MeasurementWindow>,
        action_latencies: Option<&mut Vec<u64>>,
        effects: &mut EffectProjection,
        input_mix: &mut InputMixReport,
        excluded_elapsed_ns: &mut u128,
    ) -> Result<(), PmEvidenceError> {
        let mutation_before = self.owner.mutation_counters();
        let deadline = self.cursor.next_time();
        self.owner
            .schedule(
                PmOrderSide::Buy,
                PmScheduledActionKind::QuoteEvaluation,
                deadline,
                deadline,
                wall(deadline) / 1_000_000,
            )
            .map_err(invariant)?;
        let replace_started = Instant::now();
        let cancel_service_count = self.service_one_undrained(deadline)?;
        let acknowledgement_ns = self.cursor.next_time();
        self.validate_pending_acknowledgement(false)?;
        let cancel_acknowledgement_count = self
            .acknowledge_one_undrained(acknowledgement_ns, durability)
            .await?;
        let cancel_action_ns = elapsed_ns(replace_started);
        Self::validate_service_count(cancel_service_count)?;
        Self::validate_service_count(cancel_acknowledgement_count)?;
        self.drain_effects(effects);
        let mutation_after = self.owner.mutation_counters();
        let cancel_before_replace = delta(
            mutation_after.cancel_before_replace(),
            mutation_before.cancel_before_replace(),
        );
        let cancel_intents = delta(
            mutation_after.cancel_intents(),
            mutation_before.cancel_intents(),
        );
        if cancel_before_replace != 1 || cancel_intents != 1 {
            return Err(PmEvidenceError::invariant(format!(
                "cycle {} replacement evaluation emitted cancel-before-replace={cancel_before_replace} and cancel-intents={cancel_intents}, expected exactly one each",
                self.cursor.absolute_cycle,
            )));
        }
        input_mix.replace_timers = input_mix.replace_timers.saturating_add(1);
        input_mix.cancel_intent_acknowledgements =
            input_mix.cancel_intent_acknowledgements.saturating_add(1);
        if let Some(samples) = action_latencies {
            samples.push(cancel_action_ns);
        }

        let occurrence = self.cursor.next_completion(None);
        let effect_ns = self.cursor.next_time();
        self.owner
            .execute_prepared_cancel_fixture(occurrence, PmFakeCancelScript::accepted(), effect_ns)
            .map_err(invariant)?;
        let service_ns = self.cursor.next_time();
        self.service_one(service_ns, effects)?;
        input_mix.fake_cancel_acceptances = input_mix.fake_cancel_acceptances.saturating_add(1);
        let acknowledgement_ns = self.cursor.next_time();
        self.acknowledge_one(acknowledgement_ns, durability, true, effects)
            .await?;

        self.apply_order_detail_absence(venue, allocation, effects, excluded_elapsed_ns)?;
        input_mix.cancel_order_detail_absences =
            input_mix.cancel_order_detail_absences.saturating_add(1);
        effects.cancelled_orders = effects.cancelled_orders.saturating_add(1);
        Ok(())
    }

    async fn run_fill_cycle(
        &mut self,
        venue: PmVenueOrderKey,
        durability: DurabilityMode,
        allocation: Option<&mut MeasurementWindow>,
        effects: &mut EffectProjection,
        input_mix: &mut InputMixReport,
        excluded_elapsed_ns: &mut u128,
    ) -> Result<(), PmEvidenceError> {
        let mut allocation = allocation;
        self.ingest_private_fill(allocation.as_deref_mut(), effects, excluded_elapsed_ns)?;
        input_mix.private_unique_fills = input_mix.private_unique_fills.saturating_add(1);
        let acknowledgement_ns = self.cursor.next_time();
        self.acknowledge_one(acknowledgement_ns, durability, true, effects)
            .await?;

        self.ingest_private_fill(allocation.as_deref_mut(), effects, excluded_elapsed_ns)?;
        input_mix.private_duplicate_fills = input_mix.private_duplicate_fills.saturating_add(1);
        self.apply_order_detail_absence(
            venue,
            allocation.as_deref_mut(),
            effects,
            excluded_elapsed_ns,
        )?;
        input_mix.fill_order_detail_absences =
            input_mix.fill_order_detail_absences.saturating_add(1);
        let watermark_advanced =
            self.apply_paired_fill(allocation, effects, excluded_elapsed_ns)?;
        input_mix.paired_reconciliations = input_mix.paired_reconciliations.saturating_add(1);
        if watermark_advanced {
            let acknowledgement_ns = self.cursor.next_time();
            self.acknowledge_one(acknowledgement_ns, durability, true, effects)
                .await?;
        }
        effects.filled_orders = effects.filled_orders.saturating_add(1);
        Ok(())
    }

    fn ingest_private_fill(
        &mut self,
        allocation: Option<&mut MeasurementWindow>,
        effects: &mut EffectProjection,
        excluded_elapsed_ns: &mut u128,
    ) -> Result<(), PmEvidenceError> {
        let occurrence = self.cursor.next_completion(None);
        with_paused(allocation, excluded_elapsed_ns, || {
            self.owner
                .ingest_private_fixture(
                    occurrence,
                    self.raw_fill.as_bytes(),
                    self.account.nominal_fill_fee,
                )
                .map_err(invariant)
        })?;
        let service_ns = self.cursor.next_time();
        self.service_one(service_ns, effects)
    }

    fn apply_order_detail_absence(
        &mut self,
        venue: PmVenueOrderKey,
        allocation: Option<&mut MeasurementWindow>,
        effects: &mut EffectProjection,
        excluded_elapsed_ns: &mut u128,
    ) -> Result<(), PmEvidenceError> {
        let occurrence = self.cursor.next_query()?;
        with_paused(allocation, excluded_elapsed_ns, || {
            self.owner
                .ingest_order_detail_fixture(PmOrderDetailFixtureInput::new(
                    occurrence, venue, None,
                ))
                .map_err(invariant)
        })?;
        let service_ns = self.cursor.next_time();
        self.service_one(service_ns, effects)
    }

    fn apply_paired_fill(
        &mut self,
        allocation: Option<&mut MeasurementWindow>,
        effects: &mut EffectProjection,
        excluded_elapsed_ns: &mut u128,
    ) -> Result<bool, PmEvidenceError> {
        let fill = [self.raw_fill.as_bytes()];
        let requested = self.cursor.fill_cursor(self.account.scope);
        let advances = self.cursor.advance_fill_cursor_if_cut();
        let resulting = self.cursor.fill_cursor(self.account.scope);
        let occurrence = self.cursor.next_query()?;
        let input = PmReconciliationFixtureInput::new(
            occurrence,
            &self.account.balances,
            &self.account.allowances,
            &self.account.positions,
            Some(requested),
            resulting,
            &fill,
            self.account.nominal_fill_fee,
        );
        with_paused(allocation, excluded_elapsed_ns, || {
            self.owner
                .ingest_reconciliation_fixture(input)
                .map_err(invariant)
        })?;
        let service_ns = self.cursor.next_time();
        self.service_one(service_ns, effects)?;
        effects.paired_reconciliations = effects.paired_reconciliations.saturating_add(1);
        Ok(advances)
    }

    fn service_freshness(&mut self, effects: &mut EffectProjection) -> Result<(), PmEvidenceError> {
        let deadline = self.cursor.next_time();
        self.owner
            .schedule(
                PmOrderSide::Buy,
                PmScheduledActionKind::Freshness,
                deadline,
                deadline,
                wall(deadline) / 1_000_000,
            )
            .map_err(invariant)?;
        self.service_one(deadline, effects)
    }

    fn build_cycle_fixtures(
        &mut self,
        absolute_cycle: u64,
        allocation: Option<&mut MeasurementWindow>,
        excluded_elapsed_ns: &mut u128,
    ) -> Result<PmVenueOrderKey, PmEvidenceError> {
        with_paused(allocation, excluded_elapsed_ns, || {
            self.venue_id.clear();
            write!(&mut self.venue_id, "phase6-venue-{absolute_cycle:06}").map_err(invariant)?;
            let venue = PmVenueOrderKey::new(
                self.account.scope.handle(),
                PmVenueOrderId::new(&self.venue_id).map_err(invariant)?,
            );
            self.raw_fill.clear();
            write!(
                &mut self.raw_fill,
                r#"{{"event_type":"trade","id":"phase6-fill-{absolute_cycle:06}","market":"{MARKET}","asset_id":"{}","side":"BUY","size":"5","price":"0.40","status":"MATCHED","maker_address":"{PM_FUNDER}","transaction_hash":"0xfeed","order_id":"{}","trader_side":"MAKER"}}"#,
                super::fixture::TOKEN,
                venue.id().as_str(),
            )
            .map_err(invariant)?;
            Ok(venue)
        })
    }

    fn service_one(
        &mut self,
        monotonic_ns: u64,
        effects: &mut EffectProjection,
    ) -> Result<(), PmEvidenceError> {
        self.service_one_count(monotonic_ns, effects).map(|_| ())
    }

    fn service_one_count(
        &mut self,
        monotonic_ns: u64,
        effects: &mut EffectProjection,
    ) -> Result<u64, PmEvidenceError> {
        let serviced = self.service_one_undrained(monotonic_ns)?;
        Self::validate_service_count(serviced)?;
        self.drain_effects(effects);
        Ok(serviced)
    }

    fn service_one_undrained(&mut self, monotonic_ns: u64) -> Result<u64, PmEvidenceError> {
        let serviced = self.owner.service_turn(monotonic_ns).map_err(invariant)?;
        u64::try_from(serviced.total()).map_err(invariant)
    }

    fn validate_service_count(serviced: u64) -> Result<(), PmEvidenceError> {
        if serviced == 0 {
            return Err(PmEvidenceError::invariant(
                "fixed workload expected one owner reduction",
            ));
        }
        Ok(())
    }

    fn settle(
        &mut self,
        monotonic_ns: u64,
        effects: &mut EffectProjection,
    ) -> Result<(), PmEvidenceError> {
        self.settle_count(monotonic_ns, effects).map(|_| ())
    }

    fn settle_count(
        &mut self,
        monotonic_ns: u64,
        effects: &mut EffectProjection,
    ) -> Result<u64, PmEvidenceError> {
        let mut reductions = 0_u64;
        for _ in 0..16 {
            let serviced = self.owner.service_turn(monotonic_ns).map_err(invariant)?;
            self.drain_effects(effects);
            reductions =
                reductions.saturating_add(u64::try_from(serviced.total()).map_err(invariant)?);
            if serviced.total() == 0 {
                return Ok(reductions);
            }
        }
        Err(PmEvidenceError::invariant(
            "fixed owner did not settle within sixteen turns",
        ))
    }

    fn acknowledge_one_sealed(&mut self, monotonic_ns: u64) -> Result<u64, PmEvidenceError> {
        let occurrence = self.cursor.completion_at(None, monotonic_ns);
        if !self
            .owner
            .poll_persistence_fixture(occurrence, monotonic_ns)
            .map_err(invariant)?
        {
            return Err(PmEvidenceError::invariant(
                "sealed persistence acknowledgement was not immediate",
            ));
        }
        self.service_one_count(monotonic_ns + 1, &mut EffectProjection::new())
    }

    async fn acknowledge_setup_real(
        &mut self,
        monotonic_ns: u64,
        effects: &mut EffectProjection,
    ) -> Result<u64, PmEvidenceError> {
        let started = Instant::now();
        let occurrence = self.cursor.completion_at(None, monotonic_ns);
        let received_clock = occurrence.received_clock();
        let ordering = occurrence.ordering();
        loop {
            let occurrence = PmFixtureCompletionOccurrence::new(received_clock, ordering);
            if self
                .owner
                .poll_persistence_fixture(occurrence, monotonic_ns)
                .map_err(invariant)?
            {
                return self.service_one_count(monotonic_ns + 1, effects);
            }
            if started.elapsed() >= DURABILITY_TIMEOUT {
                return Err(PmEvidenceError::invariant(
                    "real setup journal acknowledgement timed out",
                ));
            }
            tokio::task::yield_now().await;
        }
    }

    fn observe_setup_ack(&mut self, reductions: u64) -> Result<(), PmEvidenceError> {
        self.setup.w0_internal_fact_acknowledgements = self
            .setup
            .w0_internal_fact_acknowledgements
            .saturating_add(1);
        self.setup.w0_owner_reductions = self.setup.w0_owner_reductions.saturating_add(reductions);
        self.setup.w0_journal_records = self.setup.w0_journal_records.saturating_add(1);
        self.setup.w0_watermark_advances = self.setup.w0_watermark_advances.saturating_add(1);
        if reductions != 1 {
            return Err(PmEvidenceError::invariant(format!(
                "setup fact acknowledgement reduced {reductions} inputs, expected 1"
            )));
        }
        Ok(())
    }

    fn validate_sealed_setup(&self) -> Result<(), PmEvidenceError> {
        let projection = self.owner.sealed_journal_projection().ok_or_else(|| {
            PmEvidenceError::invariant("sealed setup omitted its journal projection")
        })?;
        let expected = PmSealedJournalRecordCounts {
            headers: 1,
            fill_watermark_advances: 1,
            ..PmSealedJournalRecordCounts::default()
        };
        if projection.record_count() != 2
            || projection.last_sequence() != 1
            || projection.records_by_kind() != expected
            || projection.segment_record_count() != 2
            || projection.segment_records_by_kind() != expected
            || !projection.segment_valid()
        {
            return Err(PmEvidenceError::invariant(format!(
                "sealed header/W0 setup differs: projection={projection:?}, expected kinds={expected:?}"
            )));
        }
        Ok(())
    }

    async fn acknowledge_one(
        &mut self,
        monotonic_ns: u64,
        durability: DurabilityMode,
        fact: bool,
        effects: &mut EffectProjection,
    ) -> Result<(), PmEvidenceError> {
        self.validate_pending_acknowledgement(fact)?;
        let serviced = self
            .acknowledge_one_undrained(monotonic_ns, durability)
            .await?;
        Self::validate_service_count(serviced)?;
        self.drain_effects(effects);
        if fact {
            self.cursor.internal_fact_acks = self.cursor.internal_fact_acks.saturating_add(1);
        }
        Ok(())
    }

    fn validate_pending_acknowledgement(&self, fact: bool) -> Result<(), PmEvidenceError> {
        let persistence = self.owner.persistence_metrics();
        if persistence.depth() == 0 {
            return Err(PmEvidenceError::invariant(format!(
                "cycle {} expected a pending {} record before acknowledgement polling; persistence={persistence:?}",
                self.cursor.absolute_cycle,
                if fact { "fact" } else { "intent" },
            )));
        }
        Ok(())
    }

    async fn acknowledge_one_undrained(
        &mut self,
        monotonic_ns: u64,
        durability: DurabilityMode,
    ) -> Result<u64, PmEvidenceError> {
        let started = Instant::now();
        let occurrence = self.cursor.completion_at(None, monotonic_ns);
        let received_clock = occurrence.received_clock();
        let ordering = occurrence.ordering();
        loop {
            let occurrence = PmFixtureCompletionOccurrence::new(received_clock, ordering);
            let admitted = self
                .owner
                .poll_persistence_fixture(occurrence, monotonic_ns)
                .map_err(invariant)?;
            if admitted {
                return self.service_one_undrained(monotonic_ns + 1);
            }
            if durability == DurabilityMode::Sealed {
                return Err(PmEvidenceError::invariant(
                    "sealed acknowledgement unexpectedly remained pending",
                ));
            }
            if started.elapsed() >= DURABILITY_TIMEOUT {
                return Err(PmEvidenceError::invariant(
                    "real journal acknowledgement timed out",
                ));
            }
            tokio::task::yield_now().await;
        }
    }

    fn drain_effects(&mut self, projection: &mut EffectProjection) {
        while let Some(effect) = self.owner.pop_effect() {
            projection.observe(effect);
        }
    }
}

#[derive(Clone, Copy)]
struct CounterCut {
    mutation: PmMutationCounters,
    coordinator: PmCoordinatorCounters,
    refresh: PmRefreshObligationMetrics,
    persistence_saturations: u64,
    fake_saturations: u64,
    output_saturations: u64,
    internal_fact_acks: u64,
}

impl CounterCut {
    fn capture(owner: &PmCoordinator<Phase6Model>, internal_fact_acks: u64) -> Self {
        Self {
            mutation: owner.mutation_counters(),
            coordinator: owner.counters(),
            refresh: owner.refresh_obligation_metrics(),
            persistence_saturations: owner.persistence_metrics().saturations(),
            fake_saturations: owner.fake_effect_metrics().saturations(),
            output_saturations: owner.product_effect_metrics().rejected_full(),
            internal_fact_acks,
        }
    }
}

fn nominal_delta(
    before: CounterCut,
    after: CounterCut,
    effects: &EffectProjection,
    input_mix: InputMixReport,
) -> NominalCounters {
    let quote_intents = delta(
        after.mutation.quote_intents(),
        before.mutation.quote_intents(),
    );
    let place_results = delta(
        after.mutation.place_results(),
        before.mutation.place_results(),
    );
    let cancel_intents = delta(
        after.mutation.cancel_intents(),
        before.mutation.cancel_intents(),
    );
    let cancel_results = delta(
        after.mutation.cancel_results(),
        before.mutation.cancel_results(),
    );
    let unique_fills = delta(
        after.mutation.unique_fills(),
        before.mutation.unique_fills(),
    );
    let watermark_advances = delta(
        after.mutation.fill_watermark_compactions(),
        before.mutation.fill_watermark_compactions(),
    );
    let internal_fact_acknowledgements = delta(after.internal_fact_acks, before.internal_fact_acks);
    let journal_records = quote_intents
        .saturating_add(place_results)
        .saturating_add(cancel_intents)
        .saturating_add(cancel_results)
        .saturating_add(unique_fills)
        .saturating_add(watermark_advances);
    NominalCounters {
        external_observations: input_mix.total(),
        internal_fact_acknowledgements,
        owner_reductions: delta(after.coordinator.inputs(), before.coordinator.inputs()),
        journal_records,
        quote_evaluations: delta(
            after.coordinator.quote_evaluations(),
            before.coordinator.quote_evaluations(),
        ),
        quote_candidates_evaluated: delta(
            after.coordinator.quote_candidates(),
            before.coordinator.quote_candidates(),
        ),
        quote_intents,
        place_results,
        prepared_quote_projections: effects.prepared_quotes,
        executed_quote_projections: effects.executed_quotes,
        cancel_decisions: cancel_intents,
        cancel_intents,
        cancel_results,
        prepared_cancel_projections: effects.prepared_cancels,
        executed_cancel_projections: effects.executed_cancels,
        unique_fills,
        duplicate_fills: delta(
            after.mutation.duplicate_fills(),
            before.mutation.duplicate_fills(),
        ),
        filled_orders: effects.filled_orders,
        cancelled_orders: effects.cancelled_orders,
        paired_reconciliations: effects.paired_reconciliations,
        watermark_advances,
        owned_lifecycle_rows_compacted: delta(
            after.mutation.owned_lifecycle_rows_compacted(),
            before.mutation.owned_lifecycle_rows_compacted(),
        ),
        canonical_order_rows_compacted: delta(
            after.mutation.canonical_order_rows_compacted(),
            before.mutation.canonical_order_rows_compacted(),
        ),
        owned_fill_keys_compacted: delta(
            after.mutation.owned_fill_keys_compacted(),
            before.mutation.owned_fill_keys_compacted(),
        ),
        canonical_fill_rows_compacted: delta(
            after.mutation.canonical_fill_rows_compacted(),
            before.mutation.canonical_fill_rows_compacted(),
        ),
        refresh_tickets_inserted: delta(
            after.refresh.canonical_insertions(),
            before.refresh.canonical_insertions(),
        ),
        refresh_tickets_admitted: delta(
            after.refresh.fill_observed_admissions(),
            before.refresh.fill_observed_admissions(),
        ),
        refresh_effects: delta(
            after.refresh.fill_observed_effects(),
            before.refresh.fill_observed_effects(),
        ),
        refresh_tickets_completed: delta(
            after.refresh.fill_observed_completions(),
            before.refresh.fill_observed_completions(),
        ),
        refresh_ticket_high_water: after.refresh.fill_observed_high_water(),
        refresh_duplicate_or_superseded: delta(
            after.refresh.duplicate_or_superseded_admissions(),
            before.refresh.duplicate_or_superseded_admissions(),
        ),
        queue_saturations: delta(
            after
                .persistence_saturations
                .saturating_add(after.fake_saturations)
                .saturating_add(after.output_saturations),
            before
                .persistence_saturations
                .saturating_add(before.fake_saturations)
                .saturating_add(before.output_saturations),
        ),
        state_bearing_drops: 0,
    }
}

fn journal_delta(
    before: Option<PmSealedJournalProjection>,
    after: Option<PmSealedJournalProjection>,
    cycles: usize,
    expected: u64,
) -> Result<(u64, [u8; 32]), PmEvidenceError> {
    match (before, after) {
        (Some(before), Some(after)) => {
            let records = delta(after.record_count(), before.record_count());
            if records != expected {
                return Err(PmEvidenceError::invariant(format!(
                    "sealed journal delta is {records}, expected {expected}"
                )));
            }
            let sequences = delta(after.last_sequence(), before.last_sequence());
            if sequences != records {
                return Err(PmEvidenceError::invariant(format!(
                    "sealed journal advanced {sequences} sequences for {records} records"
                )));
            }
            if before.segment_record_count() != 0 {
                return Err(PmEvidenceError::invariant(format!(
                    "sealed journal segment began with {} retained records",
                    before.segment_record_count()
                )));
            }
            if after.segment_record_count() != records {
                return Err(PmEvidenceError::invariant(format!(
                    "sealed journal segment contains {} records, expected {records}",
                    after.segment_record_count()
                )));
            }
            let cycles = u64::try_from(cycles)
                .map_err(|_| PmEvidenceError::invariant("cycle count exceeds u64"))?;
            let expected_by_kind = PmSealedJournalRecordCounts {
                headers: 0,
                quote_intents: cycles,
                place_results: cycles,
                cancel_intents: cycles / 2,
                cancel_results: cycles / 2,
                fills_applied: cycles / 2,
                order_terminals: 0,
                safety_halts: 0,
                fill_watermark_advances: cycles / 1_000,
            };
            let actual_by_kind = after.segment_records_by_kind();
            if actual_by_kind != expected_by_kind {
                return Err(PmEvidenceError::invariant(format!(
                    "sealed journal record-kind projection differs: actual={actual_by_kind:?}, expected={expected_by_kind:?}"
                )));
            }
            if !after.segment_valid() {
                return Err(PmEvidenceError::invariant(
                    "sealed journal segment failed monotonic normalization",
                ));
            }
            Ok((records, after.segment_hash()))
        }
        (None, None) => Ok((expected, [0; 32])),
        _ => Err(PmEvidenceError::invariant(
            "journal backend changed during one nominal pass",
        )),
    }
}

fn validate_sealed_header(projection: PmSealedJournalProjection) -> Result<(), PmEvidenceError> {
    let expected = PmSealedJournalRecordCounts {
        headers: 1,
        ..PmSealedJournalRecordCounts::default()
    };
    if projection.record_count() != 1
        || projection.last_sequence() != 0
        || projection.records_by_kind() != expected
        || projection.segment_record_count() != 1
        || projection.segment_records_by_kind() != expected
        || !projection.segment_valid()
    {
        return Err(PmEvidenceError::invariant(format!(
            "sealed journal did not start from one real sequence-0 header: {projection:?}"
        )));
    }
    Ok(())
}

fn hash_book_projection(
    projection: &mut Sha256,
    book: PmBookDecisionProjection,
) -> Result<(), PmEvidenceError> {
    let top = book
        .top()
        .ok_or_else(|| PmEvidenceError::invariant("owner PM book projection omitted its top"))?;
    projection.update(b"pm");
    projection.update(
        top.bid()
            .ok_or_else(|| PmEvidenceError::invariant("owner PM top omitted bid"))?
            .price()
            .units()
            .to_be_bytes(),
    );
    projection.update(
        top.ask()
            .ok_or_else(|| PmEvidenceError::invariant("owner PM top omitted ask"))?
            .price()
            .units()
            .to_be_bytes(),
    );
    Ok(())
}

fn hash_reference_projection(projection: &mut Sha256, price: OkxReferencePrice) {
    projection.update(b"okx");
    projection.update(price.coefficient().to_be_bytes());
    projection.update([price.decimal_scale()]);
}

fn with_paused<T>(
    window: Option<&mut MeasurementWindow>,
    excluded_elapsed_ns: &mut u128,
    operation: impl FnOnce() -> Result<T, PmEvidenceError>,
) -> Result<T, PmEvidenceError> {
    let started = Instant::now();
    let Some(window) = window else {
        let result = operation();
        *excluded_elapsed_ns = excluded_elapsed_ns.saturating_add(started.elapsed().as_nanos());
        return result;
    };
    window.pause().map_err(invariant)?;
    let result = operation();
    let resumed = window.resume().map_err(invariant);
    *excluded_elapsed_ns = excluded_elapsed_ns.saturating_add(started.elapsed().as_nanos());
    match (result, resumed) {
        (Ok(value), Ok(())) => Ok(value),
        (Err(error), _) | (Ok(_), Err(error)) => Err(error),
    }
}

fn ordering(
    snapshot_revision: Option<u64>,
    ingress: u64,
    hash: Option<VenueEventHash>,
) -> Result<EventOrdering, PmEvidenceError> {
    EventOrdering::new(
        ConnectionEpoch::new(1),
        snapshot_revision.map(SnapshotRevision::new),
        None,
        hash,
        IngressSequence::new(ingress),
    )
    .map_err(invariant)
}

fn event_clock(monotonic_ns: u64) -> Result<EventClock, PmEvidenceError> {
    EventClock::new(
        None,
        wall(monotonic_ns),
        monotonic_ns,
        monotonic_ns.saturating_add(1),
    )
    .map_err(invariant)
}

const fn wall(monotonic_ns: u64) -> u64 {
    WALL_BASE.saturating_add(monotonic_ns)
}

fn elapsed_ns(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_nanos()).unwrap_or(u64::MAX)
}

fn invariant(error: impl std::fmt::Display) -> PmEvidenceError {
    PmEvidenceError::invariant(error.to_string())
}

const fn delta(after: u64, before: u64) -> u64 {
    after.saturating_sub(before)
}
