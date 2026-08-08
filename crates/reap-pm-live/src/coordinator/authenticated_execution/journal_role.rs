use std::{path::PathBuf, sync::Arc, time::Duration};

use tokio::sync::Mutex;

use super::{PmAuthenticatedDurabilityStage, PmAuthenticatedExecutionError};
#[cfg(test)]
use crate::authenticated_journal::PmAuthenticatedJournalWriteLatch;
use crate::authenticated_journal::{
    PmAuthenticatedCancelResultAcknowledged, PmAuthenticatedCancelResultAdmissionError,
    PmAuthenticatedCancelResultUnresolved, PmAuthenticatedCancelResultV1,
    PmAuthenticatedCancelSendGrant, PmAuthenticatedJournalRecordV1, PmAuthenticatedJournalRecovery,
    PmAuthenticatedJournalScopeV1, PmAuthenticatedMutationJournal, PmAuthenticatedPendingRecord,
    PmAuthenticatedPlaceResultAcknowledged, PmAuthenticatedPlaceResultAdmissionError,
    PmAuthenticatedPlaceResultUnresolved, PmAuthenticatedPlaceResultV1,
    PmAuthenticatedPlaceSendGrant, PmAuthenticatedPreparedAcknowledged, PmAuthenticatedReceiptPoll,
    PmAuthenticatedSendGrant,
};

pub(super) struct PmAuthenticatedJournalRuntime {
    shared: Arc<Mutex<PmAuthenticatedMutationJournal>>,
    durability_timeout: Duration,
}

const MAX_AUTHENTICATED_DURABILITY_TIMEOUT: Duration = Duration::from_secs(60);

impl PmAuthenticatedJournalRuntime {
    pub(super) async fn start(
        path: PathBuf,
        scope: PmAuthenticatedJournalScopeV1,
        durability_timeout: Duration,
    ) -> Result<(Self, PmAuthenticatedJournalRecovery), PmAuthenticatedExecutionError> {
        if durability_timeout.is_zero() || durability_timeout > MAX_AUTHENTICATED_DURABILITY_TIMEOUT
        {
            return Err(PmAuthenticatedExecutionError::InvalidDurabilityTimeout);
        }
        let (journal, recovery) = PmAuthenticatedMutationJournal::start(path, scope).await?;
        Ok((
            Self {
                shared: Arc::new(Mutex::new(journal)),
                durability_timeout,
            },
            recovery,
        ))
    }

    pub(super) fn role(&self) -> PmAuthenticatedJournalRole {
        PmAuthenticatedJournalRole {
            shared: Arc::clone(&self.shared),
            durability_timeout: self.durability_timeout,
        }
    }

    #[cfg(test)]
    async fn delay_next_write_for_test(&self) {
        self.shared.lock().await.delay_next_write_for_test();
    }

    #[cfg(test)]
    pub(super) async fn latch_next_write_for_test(&self) -> PmAuthenticatedJournalWriteLatch {
        self.shared.lock().await.latch_next_write_for_test()
    }

    pub(super) async fn shutdown(self) -> Result<(), PmAuthenticatedExecutionError> {
        let journal = Arc::try_unwrap(self.shared)
            .map_err(|_| PmAuthenticatedExecutionError::JournalRoleStillLive)?
            .into_inner();
        tokio::time::timeout(self.durability_timeout, journal.shutdown())
            .await
            .map_err(|_| PmAuthenticatedExecutionError::JournalShutdownTimeout)??;
        Ok(())
    }
}

pub(super) struct PmAuthenticatedJournalRole {
    shared: Arc<Mutex<PmAuthenticatedMutationJournal>>,
    durability_timeout: Duration,
}

impl PmAuthenticatedJournalRole {
    pub(super) async fn record_prepared(
        &self,
        record: PmAuthenticatedJournalRecordV1,
    ) -> Result<PmAuthenticatedPreparedAcknowledged, PmAuthenticatedExecutionError> {
        let pending = self.shared.lock().await.try_record(record)?;
        match await_receipt(
            pending,
            self.durability_timeout,
            PmAuthenticatedDurabilityStage::Prepared,
        )
        .await?
        {
            PmAuthenticatedReceiptPoll::PreparedAcknowledged(acknowledged) => Ok(acknowledged),
            _ => Err(PmAuthenticatedExecutionError::JournalOperationMismatch),
        }
    }

    pub(super) async fn authorize_dispatch(
        &self,
        prepared: PmAuthenticatedPreparedAcknowledged,
    ) -> Result<PmAuthenticatedSendGrant, PmAuthenticatedExecutionError> {
        let pending = self.shared.lock().await.try_authorize_dispatch(prepared)?;
        match await_receipt(
            pending,
            self.durability_timeout,
            PmAuthenticatedDurabilityStage::DispatchAuthorized,
        )
        .await?
        {
            PmAuthenticatedReceiptPoll::SendGranted(grant) => Ok(grant),
            _ => Err(PmAuthenticatedExecutionError::JournalOperationMismatch),
        }
    }

    pub(super) async fn record_place_result(
        &self,
        grant: PmAuthenticatedPlaceSendGrant,
        result: PmAuthenticatedPlaceResultV1,
    ) -> Result<PmAuthenticatedPlaceResultAcknowledged, PmPlaceResultRecordError> {
        let pending = self
            .shared
            .lock()
            .await
            .try_record_place_result(grant, result)
            .map_err(PmPlaceResultRecordError::Admission)?;
        await_place_result(pending, self.durability_timeout)
            .await
            .map_err(PmPlaceResultRecordError::AfterAdmission)
    }

    pub(super) async fn record_cancel_result(
        &self,
        grant: PmAuthenticatedCancelSendGrant,
        result: PmAuthenticatedCancelResultV1,
    ) -> Result<PmAuthenticatedCancelResultAcknowledged, PmCancelResultRecordError> {
        let pending = self
            .shared
            .lock()
            .await
            .try_record_cancel_result(grant, result)
            .map_err(PmCancelResultRecordError::Admission)?;
        await_cancel_result(pending, self.durability_timeout)
            .await
            .map_err(PmCancelResultRecordError::AfterAdmission)
    }
}

#[allow(
    clippy::large_enum_variant,
    reason = "each variant retains the exact bounded pre- or post-admission result authority inline for quarantine"
)]
pub(super) enum PmPlaceResultRecordError {
    Admission(PmAuthenticatedPlaceResultAdmissionError),
    AfterAdmission(PmPlaceResultAfterAdmissionError),
}

#[allow(
    clippy::large_enum_variant,
    reason = "each variant retains the exact bounded pre- or post-admission result authority inline for quarantine"
)]
pub(super) enum PmCancelResultRecordError {
    Admission(PmAuthenticatedCancelResultAdmissionError),
    AfterAdmission(PmCancelResultAfterAdmissionError),
}

pub(super) struct PmPlaceResultAfterAdmissionError {
    error: PmAuthenticatedExecutionError,
    unresolved: PmAuthenticatedPlaceResultUnresolved,
}

impl PmPlaceResultAfterAdmissionError {
    pub(super) fn into_parts(
        self,
    ) -> (
        PmAuthenticatedExecutionError,
        PmAuthenticatedPlaceResultUnresolved,
    ) {
        (self.error, self.unresolved)
    }
}

pub(super) struct PmCancelResultAfterAdmissionError {
    error: PmAuthenticatedExecutionError,
    unresolved: PmAuthenticatedCancelResultUnresolved,
}

impl PmCancelResultAfterAdmissionError {
    pub(super) fn into_parts(
        self,
    ) -> (
        PmAuthenticatedExecutionError,
        PmAuthenticatedCancelResultUnresolved,
    ) {
        (self.error, self.unresolved)
    }
}

async fn await_place_result(
    mut pending: PmAuthenticatedPendingRecord,
    timeout: Duration,
) -> Result<PmAuthenticatedPlaceResultAcknowledged, PmPlaceResultAfterAdmissionError> {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        pending = match pending.poll() {
            PmAuthenticatedReceiptPoll::Pending(next) => next,
            PmAuthenticatedReceiptPoll::PlaceResultAcknowledged(acknowledged) => {
                return Ok(acknowledged);
            }
            PmAuthenticatedReceiptPoll::PlaceResultFailed {
                message,
                unresolved,
            } => {
                return Err(PmPlaceResultAfterAdmissionError {
                    error: crate::authenticated_journal::PmAuthenticatedJournalError::Durability(
                        message,
                    )
                    .into(),
                    unresolved,
                });
            }
            PmAuthenticatedReceiptPoll::PlaceResultClosed(unresolved) => {
                return Err(PmPlaceResultAfterAdmissionError {
                    error: crate::authenticated_journal::PmAuthenticatedJournalError::WriterClosed
                        .into(),
                    unresolved,
                });
            }
            _ => unreachable!("place-result pending record changed operation kind"),
        };
        if tokio::time::timeout_at(deadline, tokio::task::yield_now())
            .await
            .is_err()
        {
            let unresolved = match pending.into_unresolved_place_result() {
                Ok(unresolved) => unresolved,
                Err(_) => unreachable!("place-result timeout changed operation kind"),
            };
            return Err(PmPlaceResultAfterAdmissionError {
                error: PmAuthenticatedExecutionError::JournalAcknowledgementTimeout(
                    PmAuthenticatedDurabilityStage::Result,
                ),
                unresolved,
            });
        }
    }
}

async fn await_cancel_result(
    mut pending: PmAuthenticatedPendingRecord,
    timeout: Duration,
) -> Result<PmAuthenticatedCancelResultAcknowledged, PmCancelResultAfterAdmissionError> {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        pending = match pending.poll() {
            PmAuthenticatedReceiptPoll::Pending(next) => next,
            PmAuthenticatedReceiptPoll::CancelResultAcknowledged(acknowledged) => {
                return Ok(acknowledged);
            }
            PmAuthenticatedReceiptPoll::CancelResultFailed {
                message,
                unresolved,
            } => {
                return Err(PmCancelResultAfterAdmissionError {
                    error: crate::authenticated_journal::PmAuthenticatedJournalError::Durability(
                        message,
                    )
                    .into(),
                    unresolved,
                });
            }
            PmAuthenticatedReceiptPoll::CancelResultClosed(unresolved) => {
                return Err(PmCancelResultAfterAdmissionError {
                    error: crate::authenticated_journal::PmAuthenticatedJournalError::WriterClosed
                        .into(),
                    unresolved,
                });
            }
            _ => unreachable!("cancel-result pending record changed operation kind"),
        };
        if tokio::time::timeout_at(deadline, tokio::task::yield_now())
            .await
            .is_err()
        {
            let unresolved = match pending.into_unresolved_cancel_result() {
                Ok(unresolved) => unresolved,
                Err(_) => unreachable!("cancel-result timeout changed operation kind"),
            };
            return Err(PmCancelResultAfterAdmissionError {
                error: PmAuthenticatedExecutionError::JournalAcknowledgementTimeout(
                    PmAuthenticatedDurabilityStage::Result,
                ),
                unresolved,
            });
        }
    }
}

async fn await_receipt(
    pending: PmAuthenticatedPendingRecord,
    timeout: Duration,
    stage: PmAuthenticatedDurabilityStage,
) -> Result<PmAuthenticatedReceiptPoll, PmAuthenticatedExecutionError> {
    tokio::time::timeout(timeout, async move {
        let mut pending = pending;
        loop {
            match pending.poll() {
                PmAuthenticatedReceiptPoll::Pending(next) => {
                    pending = next;
                    tokio::task::yield_now().await;
                }
                PmAuthenticatedReceiptPoll::Failed(message) => {
                    return Err(
                        crate::authenticated_journal::PmAuthenticatedJournalError::Durability(
                            message,
                        )
                        .into(),
                    );
                }
                PmAuthenticatedReceiptPoll::Closed => {
                    return Err(
                        crate::authenticated_journal::PmAuthenticatedJournalError::WriterClosed
                            .into(),
                    );
                }
                completed => return Ok(completed),
            }
        }
    })
    .await
    .map_err(|_| PmAuthenticatedExecutionError::JournalAcknowledgementTimeout(stage))?
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use reap_pm_core::{PmClientOrderId, PmClientOrderKey, PmVenueOrderId, PmVenueOrderKey};

    use super::*;
    use crate::authenticated_journal::{
        PmAuthenticatedCancelPreparedV1, PmAuthenticatedCancelResultKindV1,
        PmAuthenticatedCoordinatorIdentityV1, PmAuthenticatedPlacePreparedV1,
        PmAuthenticatedPlaceResultKindV1,
    };

    const EXPECTED_ORDER_ID: [u8; 32] = [0x55; 32];

    fn test_scope() -> PmAuthenticatedJournalScopeV1 {
        PmAuthenticatedJournalScopeV1::from_config(
            &crate::evidence::connectivity_config(),
            [0x44; 32],
        )
        .expect("authenticated journal test scope")
    }

    fn coordinator(scope: &PmAuthenticatedJournalScopeV1) -> PmAuthenticatedCoordinatorIdentityV1 {
        PmAuthenticatedCoordinatorIdentityV1::new(
            PmClientOrderKey::new(
                scope.account(),
                PmClientOrderId::from_bytes([0x33; 16]).expect("client order"),
            ),
            scope.instrument(),
        )
    }

    fn venue_order(scope: &PmAuthenticatedJournalScopeV1) -> PmVenueOrderKey {
        PmVenueOrderKey::new(
            scope.account(),
            PmVenueOrderId::new(
                "0x5555555555555555555555555555555555555555555555555555555555555555",
            )
            .expect("venue order"),
        )
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn place_result_timeout_returns_sealed_exact_evidence() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let scope = test_scope();
        let (runtime, _) = PmAuthenticatedJournalRuntime::start(
            directory.path().join("place-timeout.jsonl"),
            scope.clone(),
            Duration::from_secs(2),
        )
        .await
        .expect("start authenticated journal runtime");
        let mut role = runtime.role();
        let prepared = PmAuthenticatedPlacePreparedV1::new(
            &scope,
            coordinator(&scope),
            7,
            [0x22; 32],
            EXPECTED_ORDER_ID,
            1_760_000_000,
        )
        .expect("place Prepared");
        let prepared = role
            .record_prepared(PmAuthenticatedJournalRecordV1::PlacePrepared(prepared))
            .await
            .expect("durable place Prepared");
        let grant = match role
            .authorize_dispatch(prepared)
            .await
            .expect("durable place grant")
        {
            PmAuthenticatedSendGrant::Place(grant) => grant,
            PmAuthenticatedSendGrant::Cancel(_) => panic!("place grant changed operation kind"),
        };
        let grant_sequence = grant.grant_sequence();
        let client_order = grant.client_order();
        let observed = [0x66; 32];
        let result = PmAuthenticatedPlaceResultV1::acknowledgement_unknown(
            coordinator(&scope),
            grant_sequence,
            Some(observed),
        );
        runtime.delay_next_write_for_test().await;
        role.durability_timeout = Duration::from_millis(10);
        let failure = match role.record_place_result(grant, result).await {
            Err(PmPlaceResultRecordError::AfterAdmission(failure)) => failure,
            Err(PmPlaceResultRecordError::Admission(_)) => {
                panic!("delayed write failed before result admission")
            }
            Ok(_) => panic!("delayed result unexpectedly beat the timeout"),
        };
        let (error, unresolved) = failure.into_parts();
        assert!(matches!(
            error,
            PmAuthenticatedExecutionError::JournalAcknowledgementTimeout(
                PmAuthenticatedDurabilityStage::Result
            )
        ));
        assert_eq!(unresolved.sequence(), 3);
        assert_eq!(unresolved.grant_sequence(), grant_sequence);
        assert_eq!(unresolved.client_order(), client_order);
        assert_eq!(
            unresolved.outcome(),
            PmAuthenticatedPlaceResultKindV1::AcknowledgementUnknown
        );
        assert_eq!(unresolved.observed_order_id(), Some(observed));

        tokio::time::sleep(Duration::from_millis(300)).await;
        drop(role);
        runtime.shutdown().await.expect("clean delayed shutdown");
    }

    #[tokio::test]
    async fn shutdown_fails_closed_while_a_purpose_role_is_live() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let (runtime, _) = PmAuthenticatedJournalRuntime::start(
            directory.path().join("live-role-shutdown.jsonl"),
            test_scope(),
            Duration::from_secs(2),
        )
        .await
        .expect("start authenticated journal runtime");
        let live_role = runtime.role();

        assert!(matches!(
            runtime.shutdown().await,
            Err(PmAuthenticatedExecutionError::JournalRoleStillLive)
        ));
        drop(live_role);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn cancel_result_timeout_returns_sealed_exact_owned_evidence() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let scope = test_scope();
        let (runtime, _) = PmAuthenticatedJournalRuntime::start(
            directory.path().join("cancel-timeout.jsonl"),
            scope.clone(),
            Duration::from_secs(2),
        )
        .await
        .expect("start authenticated journal runtime");
        let mut role = runtime.role();
        let venue_order = venue_order(&scope);
        let prepared = PmAuthenticatedCancelPreparedV1::new(
            &scope,
            coordinator(&scope),
            venue_order,
            8,
            [0x22; 32],
            EXPECTED_ORDER_ID,
            1_760_000_000,
        )
        .expect("cancel Prepared");
        let prepared = role
            .record_prepared(PmAuthenticatedJournalRecordV1::CancelPrepared(prepared))
            .await
            .expect("durable cancel Prepared");
        let grant = match role
            .authorize_dispatch(prepared)
            .await
            .expect("durable cancel grant")
        {
            PmAuthenticatedSendGrant::Cancel(grant) => grant,
            PmAuthenticatedSendGrant::Place(_) => panic!("cancel grant changed operation kind"),
        };
        let grant_sequence = grant.grant_sequence();
        let client_order = grant.client_order();
        let observed = [0x66; 32];
        let result = PmAuthenticatedCancelResultV1::out_of_profile(
            coordinator(&scope),
            venue_order,
            grant_sequence,
            Some(observed),
        );
        runtime.delay_next_write_for_test().await;
        role.durability_timeout = Duration::from_millis(10);
        let failure = match role.record_cancel_result(grant, result).await {
            Err(PmCancelResultRecordError::AfterAdmission(failure)) => failure,
            Err(PmCancelResultRecordError::Admission(_)) => {
                panic!("delayed write failed before result admission")
            }
            Ok(_) => panic!("delayed result unexpectedly beat the timeout"),
        };
        let (error, unresolved) = failure.into_parts();
        assert!(matches!(
            error,
            PmAuthenticatedExecutionError::JournalAcknowledgementTimeout(
                PmAuthenticatedDurabilityStage::Result
            )
        ));
        assert_eq!(unresolved.sequence(), 3);
        assert_eq!(unresolved.grant_sequence(), grant_sequence);
        assert_eq!(unresolved.client_order(), client_order);
        assert_eq!(
            unresolved.outcome(),
            PmAuthenticatedCancelResultKindV1::OutOfProfile
        );
        assert_eq!(unresolved.observed_order_id(), Some(observed));

        tokio::time::sleep(Duration::from_millis(300)).await;
        drop(role);
        runtime.shutdown().await.expect("clean delayed shutdown");
    }
}
