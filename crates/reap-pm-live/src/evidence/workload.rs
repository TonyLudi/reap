use std::fmt::Write as _;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use reap_benchmark_allocator::{AllocationSnapshot, MeasurementWindow};
use reap_pm_core::{
    ConnectionEpoch, EventClock, EventOrdering, IngressSequence, OkxReferencePrice, PmAccountScope,
    PmFillQueryCursor, PmInstrumentHandle, PmMarketEvent, PmOrderSide, PmVenueOrderId,
    PmVenueOrderKey, SnapshotRevision, VenueEventHash,
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
    PmBookDecisionProjection, PmCoordinator, PmCoordinatorCounters, PmDurableRecordEffect,
    PmDurableRecordKind as RecordKind, PmEffectDispatchStage, PmFakeEffectMetrics,
    PmHealthMetricKind, PmMarketInput, PmMutationCounters, PmPersistenceMetrics, PmProductEffect,
    PmRefreshObligationMetrics,
};
use crate::fake_effect::PmFixtureEffectExecutor;
use crate::journal::{PmJournalScopeV1, PmSealedJournalProjection, PmSealedJournalRecordCounts};
use crate::lanes::{PmCompleteServiceCounts, PmLaneKind};
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

#[rustfmt::skip]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExpectedAck { Quote, Cancel { venue_order: PmVenueOrderKey },
    Fact { kind: RecordKind, record: Option<PmDurableRecordEffect> } }

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
    fake_executor: PmFixtureEffectExecutor,
    public: PublicFixture,
    cursor: WorkloadCursor,
    account: AccountRows,
    journal_fingerprint: crate::journal::PmJournalFingerprintV1,
    raw_fill: String,
    venue_id: String,
    setup: SetupCounters,
    pending_fact: Option<PmDurableRecordEffect>,
    fact_conflict: bool,
}

pub(crate) fn run_benchmark_warmup() -> Result<BenchmarkWarmup, PmEvidenceError> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| PmEvidenceError::invariant(error.to_string()))?;
    runtime.block_on(Box::pin(run_benchmark_warmup_async()))
}

async fn run_benchmark_warmup_async() -> Result<BenchmarkWarmup, PmEvidenceError> {
    let mut warmup = EvidenceRun::start_sealed().await?;
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
    let mut measured = EvidenceRun::start_sealed().await?;
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
    async fn start_sealed() -> Result<Self, PmEvidenceError> {
        let config = connectivity_config();
        let (owner, fake_executor) = PmCoordinator::start_sealed_evidence(
            &config,
            model(),
            risk_limits(),
            coordinator_policy(),
        )
        .map_err(PmEvidenceError::invariant)?;
        let mut run = Self::new(owner, fake_executor, &config)?;
        run.prepare_private(DurabilityMode::Sealed).await?;
        run.prepare_public()?;
        validate_setup(run.setup, None)?;
        Ok(run)
    }

    async fn start_real(journal_path: PathBuf) -> Result<Self, PmEvidenceError> {
        let config = connectivity_config();
        let (owner, recovery, fake_executor) = PmCoordinator::start_real_writer_evidence(
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
        let mut run = Self::new(owner, fake_executor, &config)?;
        run.prepare_private(DurabilityMode::RealWriter).await?;
        run.prepare_public()?;
        validate_setup(run.setup, Some(2))?;
        Ok(run)
    }

    fn new(
        owner: PmCoordinator<Phase6Model>,
        fake_executor: PmFixtureEffectExecutor,
        config: &reap_pm_live_contracts::PmConnectivityConfig,
    ) -> Result<Self, PmEvidenceError> {
        let sealed = owner.sealed_journal_projection();
        if let Some(projection) = sealed {
            validate_sealed_header(projection)?;
        }
        Ok(Self {
            owner,
            fake_executor,
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
            pending_fact: None,
            fact_conflict: false,
        })
    }

    #[rustfmt::skip]
    async fn prepare_private(&mut self, durability: DurabilityMode) -> Result<(), PmEvidenceError> {
        self.prepare_private_inputs()?;
        let mut effects = EffectProjection::new();
        let expected = self.take_fact(RecordKind::FillWatermarkAdvanced, durability)?;
        let (reductions, _) = self.ack_expected(150, durability, expected, &mut effects).await?;
        self.observe_setup_ack(reductions)?;
        if durability == DurabilityMode::Sealed {
            self.validate_sealed_setup()?;
        }
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
        Self::validate_service_count(quote_service_count)?;
        let ack_ns = self.cursor.next_time();
        let (_, completed_at) = self
            .ack_expected(ack_ns, durability, ExpectedAck::Quote, effects)
            .await?;
        let quote_action_ns = elapsed_between(quote_started, completed_at);
        input_mix.quote_evaluation_timers = input_mix.quote_evaluation_timers.saturating_add(1);
        input_mix.quote_intent_acknowledgements =
            input_mix.quote_intent_acknowledgements.saturating_add(1);
        if let Some(samples) = action_latencies.as_deref_mut() {
            samples.push(quote_action_ns);
        }

        self.owner
            .execute_prepared_quote_fixture(
                &self.fake_executor,
                self.cursor.next_completion(None),
                PmFakePlaceScript::acknowledged(venue, Box::new([])).map_err(invariant)?,
                self.cursor.next_time(),
            )
            .map_err(invariant)?;
        let service_ns = self.cursor.next_time();
        self.service_one(service_ns, effects)?;
        input_mix.fake_place_acceptances = input_mix.fake_place_acceptances.saturating_add(1);
        self.ack_fact(durability, RecordKind::PlaceResult, effects)
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
        Self::validate_service_count(cancel_service_count)?;
        let ack_ns = self.cursor.next_time();
        let (_, completed_at) = self
            .ack_expected(
                ack_ns,
                durability,
                ExpectedAck::Cancel { venue_order: venue },
                effects,
            )
            .await?;
        let cancel_action_ns = elapsed_between(replace_started, completed_at);
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
            .execute_prepared_cancel_fixture(
                &self.fake_executor,
                occurrence,
                PmFakeCancelScript::accepted(),
                effect_ns,
            )
            .map_err(invariant)?;
        let service_ns = self.cursor.next_time();
        self.service_one(service_ns, effects)?;
        input_mix.fake_cancel_acceptances = input_mix.fake_cancel_acceptances.saturating_add(1);
        self.ack_fact(durability, RecordKind::CancelResult, effects)
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
        self.ack_fact(durability, RecordKind::FillApplied, effects)
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
            self.ack_fact(durability, RecordKind::FillWatermarkAdvanced, effects)
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
        let serviced = self.service_one_undrained(monotonic_ns)?;
        Self::validate_service_count(serviced)?;
        self.drain_effects(effects);
        Ok(())
    }

    fn service_one_undrained(&mut self, monotonic_ns: u64) -> Result<u64, PmEvidenceError> {
        let serviced = self.owner.service_turn(monotonic_ns).map_err(invariant)?;
        u64::try_from(serviced.total()).map_err(invariant)
    }

    #[rustfmt::skip]
    fn validate_service_count(serviced: u64) -> Result<(), PmEvidenceError> {
        if serviced == 0 {
            Err(PmEvidenceError::invariant("fixed workload expected one owner reduction"))
        } else {
            Ok(())
        }
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

    #[rustfmt::skip]
    fn observe_setup_ack(&mut self, reductions: u64) -> Result<(), PmEvidenceError> {
        self.setup.w0_internal_fact_acknowledgements =
            self.setup.w0_internal_fact_acknowledgements.saturating_add(1);
        self.setup.w0_owner_reductions = self.setup.w0_owner_reductions.saturating_add(reductions);
        self.setup.w0_journal_records = self.setup.w0_journal_records.saturating_add(1);
        self.setup.w0_watermark_advances = self.setup.w0_watermark_advances.saturating_add(1);
        if reductions != 1 {
            return Err(PmEvidenceError::invariant(format!("setup fact acknowledgement reduced {reductions} inputs, expected 1")));
        }
        Ok(())
    }

    #[rustfmt::skip]
    fn validate_sealed_setup(&self) -> Result<(), PmEvidenceError> {
        let projection = self.owner.sealed_journal_projection()
            .ok_or_else(|| PmEvidenceError::invariant("sealed setup omitted its journal projection"))?;
        let expected = PmSealedJournalRecordCounts {
            headers: 1, fill_watermark_advances: 1, ..PmSealedJournalRecordCounts::default()
        };
        if projection.record_count() != 2
            || projection.last_sequence() != 1
            || projection.records_by_kind() != expected
            || projection.segment_record_count() != 2
            || projection.segment_records_by_kind() != expected
            || !projection.segment_valid()
        {
            return Err(PmEvidenceError::invariant(format!("sealed header/W0 setup differs: projection={projection:?}, expected kinds={expected:?}")));
        }
        Ok(())
    }

    #[rustfmt::skip]
    async fn ack_fact(&mut self, durability: DurabilityMode, kind: RecordKind, effects: &mut EffectProjection) -> Result<(), PmEvidenceError> {
        let expected = self.take_fact(kind, durability)?;
        let monotonic_ns = self.cursor.next_time();
        self.ack_expected(monotonic_ns, durability, expected, effects)
            .await?;
        self.cursor.internal_fact_acks = self.cursor.internal_fact_acks.saturating_add(1);
        Ok(())
    }

    #[rustfmt::skip]
    fn take_fact(&mut self, kind: RecordKind, durability: DurabilityMode) -> Result<ExpectedAck, PmEvidenceError> {
        let record = self.pending_fact.take();
        let conflict = std::mem::take(&mut self.fact_conflict);
        let expected = ExpectedAck::Fact { kind, record };
        if conflict || record.is_none_or(|record| record.kind() != kind
            || record.client_order().is_some_and(|key| key.account() != self.account.scope.handle())) {
            let cut = CounterCut::capture(&self.owner, self.cursor.internal_fact_acks);
            return Err(ack_error(self.cursor.absolute_cycle, durability, expected, "fact_record_identity_mismatch",
                AckDelta::between(cut, cut), cut, cut, AckEffects::default(), format!("record={record:?} conflict={conflict}")));
        }
        Ok(expected)
    }

    #[rustfmt::skip]
    fn ack_cut(&self, durability: DurabilityMode, expected: ExpectedAck) -> Result<CounterCut, PmEvidenceError> {
        let before = CounterCut::capture(&self.owner, self.cursor.internal_fact_acks);
        if before.persistence.depth() != 1 {
            return Err(self.ack_failure(durability, expected, "pending_record_identity_mismatch",
                before, AckEffects::default(), "expected exactly one pending record"));
        }
        Ok(before)
    }

    #[rustfmt::skip]
    async fn ack_expected(&mut self, monotonic_ns: u64, durability: DurabilityMode, expected: ExpectedAck, effects: &mut EffectProjection) -> Result<(u64, Instant), PmEvidenceError> {
        let before = self.ack_cut(durability, expected)?;
        let started = Instant::now();
        let occurrence = self.cursor.completion_at(None, monotonic_ns);
        let received_clock = occurrence.received_clock();
        let ordering = occurrence.ordering();
        loop {
            let occurrence = PmFixtureCompletionOccurrence::new(received_clock, ordering);
            let admitted = self.owner
                .poll_persistence_fixture(occurrence, monotonic_ns)
                .map_err(|error| self.ack_failure(durability, expected, "persistence_poll_error",
                    before, AckEffects::default(), error))?;
            if admitted {
                return self.finish_ack(monotonic_ns + 1, durability, expected, before, effects);
            }
            if durability == DurabilityMode::Sealed {
                return Err(self.ack_failure(durability, expected, "sealed_acknowledgement_pending",
                    before, AckEffects::default(), "record remained pending"));
            }
            if started.elapsed() >= DURABILITY_TIMEOUT {
                return Err(self.ack_failure(durability, expected, "real_writer_acknowledgement_timeout",
                    before, AckEffects::default(), "durability timeout elapsed"));
            }
            tokio::task::yield_now().await;
        }
    }

    #[rustfmt::skip]
    fn finish_ack(&mut self, monotonic_ns: u64, durability: DurabilityMode, expected: ExpectedAck,
        before: CounterCut, effects: &mut EffectProjection) -> Result<(u64, Instant), PmEvidenceError> {
        let service = self.owner.service_turn(monotonic_ns);
        let completed_at = Instant::now();
        let after = CounterCut::capture(&self.owner, self.cursor.internal_fact_acks);
        let observed = self.drain_effects(effects);
        let delta = AckDelta::between(before, after);
        let counts = match service {
            Ok(counts) => counts,
            Err(error) => {
                let class = delta.primary_failure().unwrap_or("owner_service_error");
                return Err(ack_error(self.cursor.absolute_cycle, durability, expected, class,
                    delta, before, after, observed, error));
            }
        };
        validate_ack(self.cursor.absolute_cycle, durability, expected, counts, delta, before, after,
            observed, self.account.scope, self.public.instrument)?;
        Ok((u64::try_from(counts.total()).map_err(invariant)?, completed_at))
    }

    #[rustfmt::skip]
    fn ack_failure(&self, durability: DurabilityMode, expected: ExpectedAck, class: &'static str,
        before: CounterCut, observed: AckEffects, detail: impl std::fmt::Display) -> PmEvidenceError {
        let after = CounterCut::capture(&self.owner, self.cursor.internal_fact_acks);
        let delta = AckDelta::between(before, after);
        ack_error(self.cursor.absolute_cycle, durability, expected, delta.primary_failure().unwrap_or(class),
            delta, before, after, observed, detail)
    }

    #[rustfmt::skip]
    fn drain_effects(&mut self, projection: &mut EffectProjection) -> AckEffects {
        let mut observed = AckEffects::default();
        while let Some(effect) = self.owner.pop_effect() {
            if let PmProductEffect::DurableRecord(record) = effect
                && !matches!(record.kind(), RecordKind::QuoteIntent | RecordKind::CancelIntent)
            {
                self.fact_conflict |= self.pending_fact.replace(record).is_some();
            }
            observed.observe(effect);
            projection.observe(effect);
        }
        observed
    }
}

#[rustfmt::skip]
#[derive(Debug, Clone, Copy, Default)]
struct AckEffects { values: [Option<PmProductEffect>; 3], len: u8 }

#[rustfmt::skip]
impl AckEffects {
    fn observe(&mut self, effect: PmProductEffect) {
        let relevant = matches!(effect, PmProductEffect::DurableRecord(_) | PmProductEffect::PlaceGtcPostOnly(_) | PmProductEffect::CancelOwned(_))
            || matches!(effect, PmProductEffect::HealthMetricAudit(metric) if metric.kind() == PmHealthMetricKind::PersistenceAcknowledged);
        if !relevant { return; }
        if let Some(slot) = self.values.get_mut(usize::from(self.len)) { *slot = Some(effect); }
        self.len = self.len.saturating_add(1);
    }
}

#[derive(Debug, Clone, Copy)]
struct AckDelta(Option<[u64; 11]>);

#[rustfmt::skip]
impl AckDelta {
    fn between(before: CounterCut, after: CounterCut) -> Self { Self(checked_deltas(ack_values(after), ack_values(before))) }
    fn primary_failure(self) -> Option<&'static str> {
        let Some(v) = self.0 else { return Some("counter_regression"); };
        [(v[2], "persistence_durability_failure"), (v[3], "persistence_closed_failure"),
         (v[4], "persistence_age_fault"), (v[7], "mutation_durable_failure"),
         (v[8], "mutation_preparation_failure")].into_iter()
            .find_map(|(count, class)| (count != 0).then_some(class))
    }
}

#[rustfmt::skip]
fn ack_values(c: CounterCut) -> [u64; 11] {
    [c.persistence.admitted(), c.persistence.acknowledged(), c.persistence.durability_failures(),
     c.persistence.closed_failures(), c.persistence.age_faults(), c.mutation.prepared_quotes(),
     c.mutation.prepared_cancels(), c.mutation.durable_failures(), c.mutation.preparation_failures(),
     c.coordinator.inputs(), c.coordinator.durable_record_effects()]
}

fn checked_deltas(after: [u64; 11], before: [u64; 11]) -> Option<[u64; 11]> {
    let mut values = [0; 11];
    for (index, (after, before)) in after.into_iter().zip(before).enumerate() {
        values[index] = after.checked_sub(before)?;
    }
    Some(values)
}

#[rustfmt::skip]
#[allow(clippy::too_many_arguments, reason = "the immutable before/after acknowledgement proof stays explicit")]
fn validate_ack(cycle: u64, durability: DurabilityMode, expected: ExpectedAck, counts: PmCompleteServiceCounts, delta: AckDelta, before: CounterCut, after: CounterCut, effects: AckEffects, scope: PmAccountScope, instrument: PmInstrumentHandle) -> Result<(), PmEvidenceError> {
    let prepared = match expected { ExpectedAck::Quote => [1, 0], ExpectedAck::Cancel { .. } => [0, 1], ExpectedAck::Fact { .. } => [0, 0] };
    let policy = AckPolicy {
        primary: delta.primary_failure(),
        service: counts.total() == 1 && counts.for_lane(PmLaneKind::Persistence) == Some(1),
        durable: before.persistence.depth() == 1 && after.persistence.depth() == 0
            && delta.0.is_some_and(|v| v[..5] == [0, 1, 0, 0, 0] && v[9..] == [1, 0]),
        prepared: delta.0.is_some_and(|v| v[5..7] == prepared),
        identity: effects_match(expected, effects, scope, instrument, before.persistence.admitted()),
        queue: fake_ack_matches(expected, before.fake, after.fake),
    };
    let class = ack_class(policy);
    if let Some(class) = class {
        return Err(ack_error(cycle, durability, expected, class, delta, before, after, effects, "acknowledgement contract mismatch"));
    }
    Ok(())
}

#[rustfmt::skip]
fn effects_match(expected: ExpectedAck, effects: AckEffects, scope: PmAccountScope, instrument: PmInstrumentHandle, fact_sequence: u64) -> bool {
    match (expected, effects.len, effects.values) {
        (ExpectedAck::Quote, 3, [Some(PmProductEffect::DurableRecord(r)), Some(PmProductEffect::PlaceGtcPostOnly(q)), Some(PmProductEffect::HealthMetricAudit(h))]) =>
            r.kind() == RecordKind::QuoteIntent && r.client_order() == Some(q.client_order()) && q.account_scope() == scope && q.instrument() == instrument
                && q.stage() == PmEffectDispatchStage::PreparedAfterDurability && h.kind() == PmHealthMetricKind::PersistenceAcknowledged && h.value() == 1,
        (ExpectedAck::Cancel { venue_order }, 3, [Some(PmProductEffect::DurableRecord(r)), Some(PmProductEffect::CancelOwned(c)), Some(PmProductEffect::HealthMetricAudit(h))]) =>
            r.kind() == RecordKind::CancelIntent && r.client_order() == Some(c.client_order()) && c.account_scope() == scope && c.instrument() == instrument
                && c.venue_order() == venue_order && c.stage() == PmEffectDispatchStage::PreparedAfterDurability && h.kind() == PmHealthMetricKind::PersistenceAcknowledged && h.value() == 1,
        (ExpectedAck::Fact { kind, record: Some(record) }, 1, [Some(PmProductEffect::HealthMetricAudit(h)), None, None]) =>
            record.kind() == kind && !matches!(kind, RecordKind::QuoteIntent | RecordKind::CancelIntent)
                && record.client_order().is_none_or(|key| key.account() == scope.handle())
                && h.kind() == PmHealthMetricKind::PersistenceAcknowledged && h.value() == fact_sequence,
        _ => false,
    }
}

#[rustfmt::skip]
#[derive(Debug, Clone, Copy)]
struct AckPolicy { primary: Option<&'static str>, service: bool, durable: bool, prepared: bool, identity: bool, queue: bool }

#[rustfmt::skip]
fn ack_class(policy: AckPolicy) -> Option<&'static str> {
    policy.primary
        .or((!policy.service).then_some("owner_service_identity_mismatch"))
        .or((!policy.durable).then_some("durable_acknowledgement_mismatch"))
        .or((!policy.prepared || !policy.identity).then_some("prepared_effect_identity_mismatch"))
        .or((!policy.queue).then_some("fake_queue_identity_mismatch"))
}

#[rustfmt::skip]
fn fake_faults(m: PmFakeEffectMetrics) -> [u64; 11] {
    [m.released_before_journal(), m.retained_after_durable_failure(), m.invalidated_after_durability(), m.retained_after_commit_failure(),
     m.retained_after_age(), m.retained_after_suppression(), m.retained_after_revision_change(), m.aged_safety_services(),
     m.saturations(), m.age_faults(), m.clock_regressions()]
}

#[rustfmt::skip]
fn fake_ack_matches(expected: ExpectedAck, before: PmFakeEffectMetrics, after: PmFakeEffectMetrics) -> bool {
    match expected {
        ExpectedAck::Quote | ExpectedAck::Cancel { .. } =>
            [before.depth(), before.queued(), before.blocked(), before.retained()] == [1, 0, 0, 1]
                && [after.depth(), after.queued(), after.blocked(), after.retained()] == [1, 1, 0, 0]
                && after.committed_after_durability().checked_sub(before.committed_after_durability()) == Some(1)
                && after.serviced() == before.serviced() && fake_faults(after) == fake_faults(before),
        ExpectedAck::Fact { .. } => before.depth() == 0 && before == after,
    }
}

#[rustfmt::skip]
#[allow(clippy::too_many_arguments, reason = "the failure retains every immutable diagnostic cut")]
fn ack_error(cycle: u64, durability: DurabilityMode, expected: ExpectedAck, class: &'static str, delta: AckDelta, before: CounterCut, after: CounterCut, effects: AckEffects, detail: impl std::fmt::Display) -> PmEvidenceError {
    PmEvidenceError::invariant(format!("acknowledgement boundary failed: cycle={cycle} durability={durability:?} expected={expected:?} primary_failure_class={class} delta_order=[admitted,acknowledged,persistence_durability,persistence_closed,persistence_age,prepared_quote,prepared_cancel,mutation_durable,mutation_preparation,coordinator_inputs,durable_record_effects] delta={delta:?} before={before:?} after={after:?} effects={effects:?} detail={detail}"))
}

#[rustfmt::skip]
#[derive(Debug, Clone, Copy)]
struct CounterCut { mutation: PmMutationCounters, coordinator: PmCoordinatorCounters,
    refresh: PmRefreshObligationMetrics, persistence: PmPersistenceMetrics, fake: PmFakeEffectMetrics,
    output_saturations: u64, internal_fact_acks: u64 }

#[rustfmt::skip]
impl CounterCut {
    fn capture(owner: &PmCoordinator<Phase6Model>, internal_fact_acks: u64) -> Self {
        Self {
            mutation: owner.mutation_counters(), coordinator: owner.counters(),
            refresh: owner.refresh_obligation_metrics(), persistence: owner.persistence_metrics(),
            fake: owner.fake_effect_metrics(),
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
                .persistence
                .saturations()
                .saturating_add(after.fake.saturations())
                .saturating_add(after.output_saturations),
            before
                .persistence
                .saturations()
                .saturating_add(before.fake.saturations())
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
                authenticated_results: 0,
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

#[rustfmt::skip]
fn validate_sealed_header(projection: PmSealedJournalProjection) -> Result<(), PmEvidenceError> {
    let expected = PmSealedJournalRecordCounts { headers: 1, ..PmSealedJournalRecordCounts::default() };
    if projection.record_count() != 1 || projection.last_sequence() != 0
        || projection.records_by_kind() != expected || projection.segment_record_count() != 1
        || projection.segment_records_by_kind() != expected || !projection.segment_valid()
    {
        return Err(PmEvidenceError::invariant(format!("sealed journal did not start from one real sequence-0 header: {projection:?}")));
    }
    Ok(())
}

#[rustfmt::skip]
fn hash_book_projection(projection: &mut Sha256, book: PmBookDecisionProjection) -> Result<(), PmEvidenceError> {
    let top = book
        .top()
        .ok_or_else(|| PmEvidenceError::invariant("owner PM book projection omitted its top"))?;
    projection.update(b"pm");
    let bid = top.bid().ok_or_else(|| PmEvidenceError::invariant("owner PM top omitted bid"))?;
    let ask = top.ask().ok_or_else(|| PmEvidenceError::invariant("owner PM top omitted ask"))?;
    projection.update(bid.price().units().to_be_bytes());
    projection.update(ask.price().units().to_be_bytes());
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

#[rustfmt::skip]
fn ordering(snapshot_revision: Option<u64>, ingress: u64, hash: Option<VenueEventHash>) -> Result<EventOrdering, PmEvidenceError> {
    EventOrdering::new(
        ConnectionEpoch::new(1), snapshot_revision.map(SnapshotRevision::new),
        None, hash, IngressSequence::new(ingress),
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

fn elapsed_between(started: Instant, completed: Instant) -> u64 {
    u64::try_from(completed.duration_since(started).as_nanos()).unwrap_or(u64::MAX)
}

fn invariant(error: impl std::fmt::Display) -> PmEvidenceError {
    PmEvidenceError::invariant(error.to_string())
}

const fn delta(after: u64, before: u64) -> u64 {
    after.saturating_sub(before)
}

#[rustfmt::skip]
#[cfg(test)]
mod tests {
    use super::*;
    const WORKLOAD_TEST_STACK_BYTES: usize = 16 * 1024 * 1024;

    #[test]
    fn real_writer_acknowledgement_is_bound_to_expected_prepared_effect() {
        run_workload_test("pm-evidence-real-writer-ack", || async {
            let directory = tempfile::tempdir_in(std::env::current_exe().unwrap().parent().unwrap()).unwrap();
            let mut run = EvidenceRun::start_real(directory.path().join("ack.jsonl")).await.unwrap();
            run.cursor.align_pass_start().unwrap();
            let mut effects = EffectProjection::new(); let mut input_mix = InputMixReport::default();
            let mut public = Sha256::new(); let mut excluded = 0;
            for cycle in 1..=2 {
                run.run_cycle(cycle, DurabilityMode::RealWriter, None, None, &mut effects,
                    &mut input_mix, &mut public, &mut excluded).await.unwrap();
            }
            let nominal = AckPolicy { primary: None, service: true, durable: true, prepared: true, identity: true, queue: true };
            assert_eq!(ack_class(nominal), None);
            assert_eq!(ack_class(AckPolicy { prepared: false, ..nominal }), Some("prepared_effect_identity_mismatch"));
            assert_eq!(ack_class(AckPolicy { identity: false, ..nominal }), Some("prepared_effect_identity_mismatch"));
            assert_eq!(ack_class(AckPolicy { primary: Some("persistence_durability_failure"), identity: false, ..nominal }), Some("persistence_durability_failure"));
            let cut = CounterCut::capture(&run.owner, run.cursor.internal_fact_acks); let error = ack_error(73, DurabilityMode::RealWriter, ExpectedAck::Quote, "prepared_effect_identity_mismatch",
                AckDelta(Some([0; 11])), cut, cut, AckEffects::default(), "injected classifier seam").to_string();
            assert!(error.contains("cycle=73") && error.contains("expected=Quote") && error.contains("delta_order=") && error.contains("primary_failure_class=prepared_effect_identity_mismatch"));
            run.owner.shutdown_evidence().await.unwrap();
        });
    }

    fn run_workload_test<F, Fut>(name: &str, test: F)
    where
        F: FnOnce() -> Fut + Send + 'static,
        Fut: std::future::Future<Output = ()> + 'static,
    {
        let handle = std::thread::Builder::new()
            .name(name.to_owned())
            .stack_size(WORKLOAD_TEST_STACK_BYTES)
            .spawn(move || {
                tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("workload test runtime")
                    .block_on(test());
            })
            .expect("spawn bounded workload test thread");
        if let Err(panic) = handle.join() {
            std::panic::resume_unwind(panic);
        }
    }
}
