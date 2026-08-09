//! Static authenticated restart gate and append-once Goal-F bridge repair.
//!
//! This path is absent from the fake constructor. It validates both journals
//! and repairs only an auth-result-durable/Goal-F-bridge-missing crash point
//! before canonical owner bootstrap or mutation-worker activation.

use std::time::Duration;

use super::*;
use crate::authenticated_journal::PmAuthenticatedJournalRecovery;
use crate::coordinator::authenticated_recovery::PmAuthenticatedRecoveryGate;
use crate::journal::{PmJournalReceiptPoll, PmPendingJournalRecord, recover_pm_mutation_journal};

const MAX_BRIDGE_DURABILITY_TIMEOUT: Duration = Duration::from_secs(60);

impl PmMutationOwner {
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn start_authenticated(
        config: &PmConnectivityConfig,
        mut private: Box<PmPrivateMonitorRuntime>,
        preparation: PmMutationPreparationRole,
        journal_path: PathBuf,
        authenticated: &PmAuthenticatedJournalRecovery,
        durability_timeout: Duration,
    ) -> Result<(Self, PmJournalRecovery), PmMutationError> {
        if durability_timeout.is_zero() || durability_timeout > MAX_BRIDGE_DURABILITY_TIMEOUT {
            return Err(PmMutationError::InvalidAuthenticatedBridgeDurabilityTimeout);
        }
        let scope = PmJournalScopeV1::from_config(config)?;
        let instrument_scope = PmFixtureInstrumentScope::from_metadata(
            config.account().instrument(),
            config.account().expected_metadata(),
        )?;
        let instrument_id = config.account().instrument_id();
        let account_signature_profile = config.account().signature_profile();
        if private.account_scope() != scope.account_scope()
            || private.instrument() != config.account().instrument()
            || preparation.account_scope() != scope.account_scope()
            || preparation.instrument() != config.account().instrument()
            || preparation.instrument_id() != instrument_id
            || preparation.account_signature_profile() != account_signature_profile
        {
            return Err(PmMutationError::CompositionScopeMismatch);
        }
        let effects = PmMutationDispatchQueue::new()?;

        let (mut journal, initial_recovery) =
            PmMutationJournal::start(journal_path.clone(), scope.clone()).await?;
        let (recovery, gate) = match validate_and_repair(
            &mut journal,
            initial_recovery,
            journal_path,
            scope.clone(),
            authenticated,
            durability_timeout,
        )
        .await
        {
            Ok(recovered) => recovered,
            Err(primary) => {
                let _ = journal.shutdown().await;
                return Err(primary);
            }
        };

        if let Err(error) =
            super::super::mutation_recovery::recover_private_owner(private.as_mut(), &recovery)
        {
            let _ = journal.shutdown().await;
            return Err(error);
        }
        let Some(next_intent_id) = recovery
            .last_intent_id()
            .max(recovery.compacted_intent_id())
            .checked_add(1)
        else {
            let _ = journal.shutdown().await;
            return Err(PmMutationError::IntentIdentityExhausted);
        };
        let halt = if let Some(reason) = recovery.safety_reason() {
            Some(PmMutationHalt::RecoveredSafetyHalt(reason))
        } else if gate.requires_reconciliation() || recovery.requires_reconciliation() {
            Some(PmMutationHalt::RecoveryReconciliationRequired)
        } else {
            None
        };
        Ok((
            Self {
                scope,
                account_signature_profile,
                instrument_scope,
                instrument_id,
                private,
                preparation,
                journal,
                persistence: PmPersistenceQueue::new(),
                effects,
                durable_consequences: VecDeque::with_capacity(PM_PENDING_PERSISTENCE_CAPACITY),
                quarantined_live_bridges: VecDeque::with_capacity(2),
                applied_live_bridges: VecDeque::with_capacity(2),
                failed_live_bridges: VecDeque::with_capacity(2),
                failed_goal_f_write: None,
                reconciliation_reductions: PmReconciliationReductions::new(),
                next_intent_id,
                current_revisions: None,
                halt,
                counters: PmMutationCounters::default(),
            },
            recovery,
        ))
    }
}

async fn validate_and_repair(
    journal: &mut PmMutationJournal,
    initial_recovery: PmJournalRecovery,
    journal_path: PathBuf,
    scope: PmJournalScopeV1,
    authenticated: &PmAuthenticatedJournalRecovery,
    durability_timeout: Duration,
) -> Result<(PmJournalRecovery, PmAuthenticatedRecoveryGate), PmMutationError> {
    let initial_gate = PmAuthenticatedRecoveryGate::validate(&initial_recovery, authenticated)?;
    if initial_gate.missing_bridges().is_empty() {
        return Ok((initial_recovery, initial_gate));
    }
    for bridge in initial_gate.missing_bridge_records()? {
        let pending = journal.try_record(PmJournalRecordV1::AuthenticatedResult(bridge))?;
        await_bridge_acknowledgement(pending, durability_timeout).await?;
    }

    // Keep the original writer and lease live while reading the exact durable
    // cut. This prevents a competing runtime from entering between repair and
    // revalidation; durable acknowledgement already guarantees flushed bytes.
    let recovery =
        tokio::task::spawn_blocking(move || recover_pm_mutation_journal(journal_path, &scope))
            .await
            .map_err(PmJournalError::RecoveryTask)?
            .map_err(PmJournalError::Recovery)?;
    let gate = PmAuthenticatedRecoveryGate::validate(&recovery, authenticated)?;
    if !gate.missing_bridges().is_empty() {
        return Err(PmMutationError::AuthenticatedBridgeRepairIncomplete);
    }
    Ok((recovery, gate))
}

async fn await_bridge_acknowledgement(
    pending: PmPendingJournalRecord,
    durability_timeout: Duration,
) -> Result<(), PmMutationError> {
    tokio::time::timeout(durability_timeout, async move {
        let mut pending = pending;
        loop {
            match pending.poll() {
                PmJournalReceiptPoll::Pending(next) => {
                    pending = next;
                    tokio::task::yield_now().await;
                }
                PmJournalReceiptPoll::Acknowledged(acknowledged) => {
                    if acknowledged.consume() == 0 {
                        return Err(PmMutationError::InvalidDurableConsequence);
                    }
                    return Ok(());
                }
                PmJournalReceiptPoll::Failed(message) => {
                    return Err(PmMutationError::AuthenticatedBridgeDurabilityFailed(
                        message,
                    ));
                }
                PmJournalReceiptPoll::Closed => {
                    return Err(PmMutationError::AuthenticatedBridgeWriterClosed);
                }
            }
        }
    })
    .await
    .map_err(|_| PmMutationError::AuthenticatedBridgeDurabilityTimeout)?
}
