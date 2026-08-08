use reap_polymarket_adapter::{PmExactOwnedCancelRequest, PmGtcPostOnlyPlaceRequest};
use reap_polymarket_live_adapter::{
    PmAuthorizedMutationServerTime, PmExactOwnedCancelLoopbackRole, PmFixedPlaceLoopbackRole,
    PmLoopbackCancelAuthenticationFailure, PmLoopbackCancelAuthenticationRole,
    PmLoopbackPlaceAuthenticationFailure, PmLoopbackPlaceAuthenticationRole,
    PmRetainedOwnedCancelRequest, PmRetainedPlaceRequest,
};

use super::journal_role::{
    PmAuthenticatedJournalRole, PmCancelResultRecordError, PmPlaceResultRecordError,
};
use super::{PmAuthenticatedExecutionError, outcome};
use crate::authenticated_journal::{
    PmAuthenticatedCancelPreparedV1, PmAuthenticatedCancelResultUnresolved,
    PmAuthenticatedCancelResultV1, PmAuthenticatedCancelSendGrant, PmAuthenticatedJournalRecordV1,
    PmAuthenticatedJournalScopeV1, PmAuthenticatedPlacePreparedV1,
    PmAuthenticatedPlaceResultUnresolved, PmAuthenticatedPlaceResultV1,
    PmAuthenticatedPlaceSendGrant, PmAuthenticatedSendGrant,
};
use crate::coordinator::dispatch::{PmPreparedCancelDispatch, PmPreparedPlaceDispatch};
use crate::coordinator::live_completion::{
    PmAuthenticatedBridgeApplied, PmLiveCancelCompletion, PmLivePlaceCompletion,
};
use reap_pm_state::PmOwnedCancelIntent;

/// One bounded place-only authenticated execution role.
pub(crate) struct PmAuthenticatedPlaceWorker {
    scope: PmAuthenticatedJournalScopeV1,
    journal: PmAuthenticatedJournalRole,
    authentication: PmLoopbackPlaceAuthenticationRole,
    transport: PmFixedPlaceLoopbackRole,
    quarantined_pre_send: Option<PmQuarantinedPlaceAuthentication>,
    quarantined_authenticated_pre_send: Option<PmQuarantinedAuthenticatedPlace>,
    quarantined_post_send: Option<PmQuarantinedPlaceResult>,
    pending_goal_f_bridge: Option<PmPendingGoalFBridge>,
    halted: bool,
}

impl PmAuthenticatedPlaceWorker {
    pub(super) fn new(
        scope: PmAuthenticatedJournalScopeV1,
        journal: PmAuthenticatedJournalRole,
        authentication: PmLoopbackPlaceAuthenticationRole,
        transport: PmFixedPlaceLoopbackRole,
    ) -> Self {
        Self {
            scope,
            journal,
            authentication,
            transport,
            quarantined_pre_send: None,
            quarantined_authenticated_pre_send: None,
            quarantined_post_send: None,
            pending_goal_f_bridge: None,
            halted: false,
        }
    }

    pub(super) fn is_available(&self) -> bool {
        !(self.halted
            || self.quarantined_pre_send.is_some()
            || self.quarantined_authenticated_pre_send.is_some()
            || self.quarantined_post_send.is_some()
            || self.pending_goal_f_bridge.is_some())
    }

    /// Executes the exact Prepared -> DispatchAuthorized -> Result durable
    /// choreography. The sole network write consumes the retained request only
    /// after both preceding authenticated-journal acknowledgements.
    async fn execute(
        &mut self,
        dispatch: PmPreparedPlaceDispatch,
        timestamp: PmAuthorizedMutationServerTime,
    ) -> Result<PmLivePlaceCompletion, PmAuthenticatedExecutionError> {
        let prior_goal_f_sequence = dispatch.journal_sequence();
        let client_order = dispatch.client_order();
        let instrument = dispatch.instrument_id();
        let request = dispatch.into_request();
        // Await this retaining handoff to completion. Selecting it against
        // cancellation could drop the moved request before an error returns it.
        let retained = match self
            .authentication
            .authenticate_place(request, timestamp)
            .await
        {
            Ok(retained) => retained,
            Err(failure) => {
                let reason = failure.reason();
                self.retain_authentication_failure(prior_goal_f_sequence, failure)?;
                return Err(PmAuthenticatedExecutionError::Authentication(reason));
            }
        };
        // Keep the secret-derived exact-body identity stack-local across the
        // one send. Only the secret-free semantic identity crosses a durable
        // journal constructor or grant match.
        let runtime_exact_body_commitment = retained.runtime_exact_body_commitment();
        let semantic_request_commitment = retained.semantic_request_commitment().bytes();
        let expected_order_id = retained.expected_order_id().bytes();
        let l2_timestamp_seconds = retained.l2_timestamp_seconds();
        let coordinator = crate::authenticated_journal::PmAuthenticatedCoordinatorIdentityV1::new(
            client_order,
            instrument,
        );
        let prepared = match PmAuthenticatedPlacePreparedV1::new(
            &self.scope,
            coordinator,
            prior_goal_f_sequence,
            semantic_request_commitment,
            expected_order_id,
            l2_timestamp_seconds,
        ) {
            Ok(prepared) => prepared,
            Err(error) => {
                self.retain_authenticated_pre_send(
                    prior_goal_f_sequence,
                    client_order,
                    retained,
                    None,
                )?;
                return Err(error.into());
            }
        };
        let prepared = match self
            .journal
            .record_prepared(PmAuthenticatedJournalRecordV1::PlacePrepared(prepared))
            .await
        {
            Ok(prepared) => prepared,
            Err(error) => {
                self.retain_authenticated_pre_send(
                    prior_goal_f_sequence,
                    client_order,
                    retained,
                    None,
                )?;
                return Err(error);
            }
        };
        let grant = match self.journal.authorize_dispatch(prepared).await {
            Ok(PmAuthenticatedSendGrant::Place(grant)) => grant,
            Ok(unexpected @ PmAuthenticatedSendGrant::Cancel(_)) => {
                self.retain_authenticated_pre_send(
                    prior_goal_f_sequence,
                    client_order,
                    retained,
                    Some(unexpected),
                )?;
                return Err(PmAuthenticatedExecutionError::JournalOperationMismatch);
            }
            Err(error) => {
                self.retain_authenticated_pre_send(
                    prior_goal_f_sequence,
                    client_order,
                    retained,
                    None,
                )?;
                return Err(error);
            }
        };
        if !grant.matches_retained_request(
            semantic_request_commitment,
            expected_order_id,
            l2_timestamp_seconds,
        ) {
            self.retain_authenticated_pre_send(
                prior_goal_f_sequence,
                client_order,
                retained,
                Some(PmAuthenticatedSendGrant::Place(grant)),
            )?;
            return Err(PmAuthenticatedExecutionError::RetainedRequestMismatch);
        }

        let observed = self.transport.send(retained).await;
        let result = match outcome::place_result(&grant, runtime_exact_body_commitment, &observed) {
            Ok(result) => result,
            Err(error) => {
                self.retain_unclassified_post_send(grant);
                return Err(error);
            }
        };
        let acknowledged = match self.journal.record_place_result(grant, result).await {
            Ok(acknowledged) => acknowledged,
            Err(PmPlaceResultRecordError::Admission(failure)) => {
                let (error, grant, result) = failure.into_parts();
                self.retain_result_admission_failure(grant, result);
                return Err(error.into());
            }
            Err(PmPlaceResultRecordError::AfterAdmission(failure)) => {
                let (error, unresolved) = failure.into_parts();
                self.retain_result_durability_failure(unresolved);
                return Err(error);
            }
        };
        Ok(PmLivePlaceCompletion::after_durable_result(acknowledged)?)
    }

    /// Runs after the owned task has already taken the sole place capacity.
    /// This future must be awaited to completion by its supervisor; it is
    /// never selected against cancellation after the Goal-F dispatch moves.
    pub(super) async fn run_task(
        &mut self,
        dispatch: PmPreparedPlaceDispatch,
        timestamp: PmAuthorizedMutationServerTime,
    ) -> Result<PmLivePlaceCompletion, PmAuthenticatedExecutionError> {
        let result = self.execute(dispatch, timestamp).await;
        match &result {
            Ok(completion) => {
                self.pending_goal_f_bridge = Some(PmPendingGoalFBridge {
                    client_order: completion.client_order(),
                    auth_result_sequence: completion.auth_result_sequence(),
                });
            }
            Err(_) => self.halted = true,
        }
        result
    }

    #[cfg(test)]
    pub(crate) fn quarantined_pre_send(&self) -> Option<PmPreSendQuarantineProjection> {
        self.quarantined_pre_send
            .as_ref()
            .map(|quarantined| PmPreSendQuarantineProjection {
                prior_goal_f_sequence: quarantined.prior_goal_f_sequence,
                client_order: quarantined.request.client_order(),
            })
    }

    #[cfg(test)]
    pub(super) fn quarantined_place_request_for_test(&self) -> Option<&PmGtcPostOnlyPlaceRequest> {
        self.quarantined_pre_send
            .as_ref()
            .map(|quarantined| &quarantined.request)
    }

    #[allow(
        clippy::result_large_err,
        reason = "pre-send quarantine reports the bounded execution failure inline while retaining the exact request in the worker"
    )]
    fn retain_authentication_failure(
        &mut self,
        prior_goal_f_sequence: u64,
        failure: PmLoopbackPlaceAuthenticationFailure,
    ) -> Result<(), PmAuthenticatedExecutionError> {
        if self.quarantined_pre_send.is_some() {
            return Err(PmAuthenticatedExecutionError::PreSendQuarantineOccupied);
        }
        self.quarantined_pre_send = Some(PmQuarantinedPlaceAuthentication {
            prior_goal_f_sequence,
            request: failure.into_request(),
        });
        Ok(())
    }

    #[allow(
        clippy::result_large_err,
        reason = "pre-send quarantine reports the bounded execution failure inline while retaining the exact grant and request"
    )]
    fn retain_authenticated_pre_send(
        &mut self,
        prior_goal_f_sequence: u64,
        client_order: reap_pm_core::PmClientOrderKey,
        retained: PmRetainedPlaceRequest,
        grant: Option<PmAuthenticatedSendGrant>,
    ) -> Result<(), PmAuthenticatedExecutionError> {
        if self.quarantined_authenticated_pre_send.is_some() {
            return Err(PmAuthenticatedExecutionError::PreSendQuarantineOccupied);
        }
        self.quarantined_authenticated_pre_send = Some(PmQuarantinedAuthenticatedPlace {
            prior_goal_f_sequence,
            client_order,
            retained,
            grant,
        });
        Ok(())
    }

    fn retain_result_admission_failure(
        &mut self,
        grant: PmAuthenticatedPlaceSendGrant,
        result: PmAuthenticatedPlaceResultV1,
    ) {
        debug_assert!(
            self.quarantined_post_send.is_none(),
            "exclusive place-worker reservation forbids a second post-send quarantine",
        );
        self.quarantined_post_send = Some(PmQuarantinedPlaceResult::Classified { grant, result });
    }

    fn retain_unclassified_post_send(&mut self, grant: PmAuthenticatedPlaceSendGrant) {
        debug_assert!(
            self.quarantined_post_send.is_none(),
            "exclusive place-worker reservation forbids a second post-send quarantine",
        );
        self.quarantined_post_send = Some(PmQuarantinedPlaceResult::Unclassified { grant });
    }

    fn retain_result_durability_failure(
        &mut self,
        unresolved: PmAuthenticatedPlaceResultUnresolved,
    ) {
        debug_assert!(
            self.quarantined_post_send.is_none(),
            "exclusive place-worker reservation forbids a second post-send quarantine",
        );
        self.quarantined_post_send =
            Some(PmQuarantinedPlaceResult::DurabilityUnresolved { unresolved });
    }

    #[allow(
        clippy::result_large_err,
        reason = "bridge confirmation keeps the bounded execution failure inline without weakening the exact proof handshake"
    )]
    pub(super) fn confirm_goal_f_bridge(
        &mut self,
        applied: PmAuthenticatedBridgeApplied,
    ) -> Result<(), PmAuthenticatedExecutionError> {
        let (is_place, client_order, auth_result_sequence) = applied.into_identity();
        if !is_place {
            return Err(PmAuthenticatedExecutionError::GoalFBridgeAcknowledgementMismatch);
        }
        if self.pending_goal_f_bridge
            != Some(PmPendingGoalFBridge {
                client_order,
                auth_result_sequence,
            })
        {
            return Err(PmAuthenticatedExecutionError::GoalFBridgeAcknowledgementMismatch);
        }
        self.pending_goal_f_bridge = None;
        Ok(())
    }
}

/// One bounded exact-owned-cancel authenticated execution role. It owns a
/// distinct transport so a slow place request never monopolizes safe cancel.
pub(crate) struct PmAuthenticatedCancelWorker {
    scope: PmAuthenticatedJournalScopeV1,
    journal: PmAuthenticatedJournalRole,
    authentication: PmLoopbackCancelAuthenticationRole,
    transport: PmExactOwnedCancelLoopbackRole,
    quarantined_pre_send: Option<PmQuarantinedCancelAuthentication>,
    quarantined_authenticated_pre_send: Option<PmQuarantinedAuthenticatedCancel>,
    quarantined_post_send: Option<PmQuarantinedCancelResult>,
    pending_goal_f_bridge: Option<PmPendingGoalFBridge>,
    halted: bool,
}

impl PmAuthenticatedCancelWorker {
    pub(super) fn new(
        scope: PmAuthenticatedJournalScopeV1,
        journal: PmAuthenticatedJournalRole,
        authentication: PmLoopbackCancelAuthenticationRole,
        transport: PmExactOwnedCancelLoopbackRole,
    ) -> Self {
        Self {
            scope,
            journal,
            authentication,
            transport,
            quarantined_pre_send: None,
            quarantined_authenticated_pre_send: None,
            quarantined_post_send: None,
            pending_goal_f_bridge: None,
            halted: false,
        }
    }

    pub(super) fn is_available(&self) -> bool {
        !(self.halted
            || self.quarantined_pre_send.is_some()
            || self.quarantined_authenticated_pre_send.is_some()
            || self.quarantined_post_send.is_some()
            || self.pending_goal_f_bridge.is_some())
    }

    async fn execute(
        &mut self,
        dispatch: PmPreparedCancelDispatch,
        intent: PmOwnedCancelIntent,
        timestamp: PmAuthorizedMutationServerTime,
    ) -> Result<PmLiveCancelCompletion, PmAuthenticatedExecutionError> {
        if dispatch.client_order() != intent.client_order()
            || dispatch.venue_order() != intent.venue_order()
        {
            return Err(PmAuthenticatedExecutionError::CancelOwnershipMismatch);
        }
        let prior_goal_f_sequence = dispatch.journal_sequence();
        let client_order = dispatch.client_order();
        let instrument = dispatch.instrument_id();
        let venue_order = dispatch.venue_order();
        let request = dispatch.into_request();
        // Await this retaining handoff to completion; cancellation would lose
        // the only in-process return path for the moved owned-cancel request.
        let retained = match self
            .authentication
            .authenticate_cancel(request, timestamp)
            .await
        {
            Ok(retained) => retained,
            Err(failure) => {
                let reason = failure.reason();
                self.retain_authentication_failure(prior_goal_f_sequence, intent, failure)?;
                return Err(PmAuthenticatedExecutionError::Authentication(reason));
            }
        };
        // Keep the secret-derived exact-body identity stack-local across the
        // one send. Only the secret-free semantic identity crosses a durable
        // journal constructor or grant match.
        let runtime_exact_body_commitment = retained.runtime_exact_body_commitment();
        let semantic_request_commitment = retained.semantic_request_commitment().bytes();
        let fixed_order_id = retained.order_id().bytes();
        let l2_timestamp_seconds = retained.l2_timestamp_seconds();
        let coordinator = crate::authenticated_journal::PmAuthenticatedCoordinatorIdentityV1::new(
            client_order,
            instrument,
        );
        let prepared = match PmAuthenticatedCancelPreparedV1::new(
            &self.scope,
            coordinator,
            venue_order,
            prior_goal_f_sequence,
            semantic_request_commitment,
            fixed_order_id,
            l2_timestamp_seconds,
        ) {
            Ok(prepared) => prepared,
            Err(error) => {
                self.retain_authenticated_pre_send(
                    prior_goal_f_sequence,
                    client_order,
                    intent,
                    retained,
                    None,
                )?;
                return Err(error.into());
            }
        };
        let prepared = match self
            .journal
            .record_prepared(PmAuthenticatedJournalRecordV1::CancelPrepared(prepared))
            .await
        {
            Ok(prepared) => prepared,
            Err(error) => {
                self.retain_authenticated_pre_send(
                    prior_goal_f_sequence,
                    client_order,
                    intent,
                    retained,
                    None,
                )?;
                return Err(error);
            }
        };
        let grant = match self.journal.authorize_dispatch(prepared).await {
            Ok(PmAuthenticatedSendGrant::Cancel(grant)) => grant,
            Ok(unexpected @ PmAuthenticatedSendGrant::Place(_)) => {
                self.retain_authenticated_pre_send(
                    prior_goal_f_sequence,
                    client_order,
                    intent,
                    retained,
                    Some(unexpected),
                )?;
                return Err(PmAuthenticatedExecutionError::JournalOperationMismatch);
            }
            Err(error) => {
                self.retain_authenticated_pre_send(
                    prior_goal_f_sequence,
                    client_order,
                    intent,
                    retained,
                    None,
                )?;
                return Err(error);
            }
        };
        if !grant.matches_retained_request(
            semantic_request_commitment,
            fixed_order_id,
            l2_timestamp_seconds,
        ) {
            self.retain_authenticated_pre_send(
                prior_goal_f_sequence,
                client_order,
                intent,
                retained,
                Some(PmAuthenticatedSendGrant::Cancel(grant)),
            )?;
            return Err(PmAuthenticatedExecutionError::RetainedRequestMismatch);
        }

        let observed = self.transport.send(retained).await;
        let result = match outcome::cancel_result(&grant, runtime_exact_body_commitment, &observed)
        {
            Ok(result) => result,
            Err(error) => {
                self.retain_unclassified_post_send(intent, grant);
                return Err(error);
            }
        };
        let acknowledged = match self.journal.record_cancel_result(grant, result).await {
            Ok(acknowledged) => acknowledged,
            Err(PmCancelResultRecordError::Admission(failure)) => {
                let (error, grant, result) = failure.into_parts();
                self.retain_result_admission_failure(intent, grant, result);
                return Err(error.into());
            }
            Err(PmCancelResultRecordError::AfterAdmission(failure)) => {
                let (error, unresolved) = failure.into_parts();
                self.retain_result_durability_failure(intent, unresolved);
                return Err(error);
            }
        };
        Ok(PmLiveCancelCompletion::after_durable_result(
            intent,
            acknowledged,
        )?)
    }

    pub(super) async fn run_task(
        &mut self,
        dispatch: PmPreparedCancelDispatch,
        intent: PmOwnedCancelIntent,
        timestamp: PmAuthorizedMutationServerTime,
    ) -> Result<PmLiveCancelCompletion, PmAuthenticatedExecutionError> {
        let result = self.execute(dispatch, intent, timestamp).await;
        match &result {
            Ok(completion) => {
                self.pending_goal_f_bridge = Some(PmPendingGoalFBridge {
                    client_order: completion.client_order(),
                    auth_result_sequence: completion.auth_result_sequence(),
                });
            }
            Err(_) => self.halted = true,
        }
        result
    }

    #[allow(
        clippy::result_large_err,
        reason = "pre-send quarantine reports the bounded execution failure inline while retaining the exact owned-cancel request"
    )]
    fn retain_authentication_failure(
        &mut self,
        prior_goal_f_sequence: u64,
        intent: PmOwnedCancelIntent,
        failure: PmLoopbackCancelAuthenticationFailure,
    ) -> Result<(), PmAuthenticatedExecutionError> {
        if self.quarantined_pre_send.is_some() {
            return Err(PmAuthenticatedExecutionError::PreSendQuarantineOccupied);
        }
        self.quarantined_pre_send = Some(PmQuarantinedCancelAuthentication {
            prior_goal_f_sequence,
            request: failure.into_request(),
            intent,
        });
        Ok(())
    }

    #[allow(
        clippy::result_large_err,
        reason = "pre-send quarantine reports the bounded execution failure inline while retaining exact cancel authority"
    )]
    fn retain_authenticated_pre_send(
        &mut self,
        prior_goal_f_sequence: u64,
        client_order: reap_pm_core::PmClientOrderKey,
        intent: PmOwnedCancelIntent,
        retained: PmRetainedOwnedCancelRequest,
        grant: Option<PmAuthenticatedSendGrant>,
    ) -> Result<(), PmAuthenticatedExecutionError> {
        if self.quarantined_authenticated_pre_send.is_some() {
            return Err(PmAuthenticatedExecutionError::PreSendQuarantineOccupied);
        }
        self.quarantined_authenticated_pre_send = Some(PmQuarantinedAuthenticatedCancel {
            prior_goal_f_sequence,
            client_order,
            intent,
            retained,
            grant,
        });
        Ok(())
    }

    fn retain_result_admission_failure(
        &mut self,
        intent: PmOwnedCancelIntent,
        grant: PmAuthenticatedCancelSendGrant,
        result: PmAuthenticatedCancelResultV1,
    ) {
        debug_assert!(
            self.quarantined_post_send.is_none(),
            "exclusive cancel-worker reservation forbids a second post-send quarantine",
        );
        self.quarantined_post_send = Some(PmQuarantinedCancelResult::Classified {
            intent,
            grant,
            result,
        });
    }

    fn retain_unclassified_post_send(
        &mut self,
        intent: PmOwnedCancelIntent,
        grant: PmAuthenticatedCancelSendGrant,
    ) {
        debug_assert!(
            self.quarantined_post_send.is_none(),
            "exclusive cancel-worker reservation forbids a second post-send quarantine",
        );
        self.quarantined_post_send =
            Some(PmQuarantinedCancelResult::Unclassified { intent, grant });
    }

    fn retain_result_durability_failure(
        &mut self,
        intent: PmOwnedCancelIntent,
        unresolved: PmAuthenticatedCancelResultUnresolved,
    ) {
        debug_assert!(
            self.quarantined_post_send.is_none(),
            "exclusive cancel-worker reservation forbids a second post-send quarantine",
        );
        self.quarantined_post_send =
            Some(PmQuarantinedCancelResult::DurabilityUnresolved { intent, unresolved });
    }

    #[allow(
        clippy::result_large_err,
        reason = "bridge confirmation keeps the bounded execution failure inline without weakening the exact proof handshake"
    )]
    pub(super) fn confirm_goal_f_bridge(
        &mut self,
        applied: PmAuthenticatedBridgeApplied,
    ) -> Result<(), PmAuthenticatedExecutionError> {
        let (is_place, client_order, auth_result_sequence) = applied.into_identity();
        if is_place {
            return Err(PmAuthenticatedExecutionError::GoalFBridgeAcknowledgementMismatch);
        }
        if self.pending_goal_f_bridge
            != Some(PmPendingGoalFBridge {
                client_order,
                auth_result_sequence,
            })
        {
            return Err(PmAuthenticatedExecutionError::GoalFBridgeAcknowledgementMismatch);
        }
        self.pending_goal_f_bridge = None;
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PmPendingGoalFBridge {
    client_order: reap_pm_core::PmClientOrderKey,
    auth_result_sequence: u64,
}

#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "opaque quarantine owns the move-only neutral request until worker shutdown"
    )
)]
struct PmQuarantinedPlaceAuthentication {
    prior_goal_f_sequence: u64,
    request: PmGtcPostOnlyPlaceRequest,
}

#[allow(
    dead_code,
    reason = "opaque quarantine owns authenticated request authority until worker shutdown"
)]
struct PmQuarantinedAuthenticatedPlace {
    prior_goal_f_sequence: u64,
    client_order: reap_pm_core::PmClientOrderKey,
    retained: PmRetainedPlaceRequest,
    grant: Option<PmAuthenticatedSendGrant>,
}

#[allow(
    dead_code,
    reason = "opaque quarantine owns post-send place evidence until worker shutdown"
)]
enum PmQuarantinedPlaceResult {
    Unclassified {
        grant: PmAuthenticatedPlaceSendGrant,
    },
    Classified {
        grant: PmAuthenticatedPlaceSendGrant,
        result: PmAuthenticatedPlaceResultV1,
    },
    DurabilityUnresolved {
        unresolved: PmAuthenticatedPlaceResultUnresolved,
    },
}

#[allow(
    dead_code,
    reason = "opaque quarantine owns the exact cancel request and ownership until worker shutdown"
)]
struct PmQuarantinedCancelAuthentication {
    prior_goal_f_sequence: u64,
    request: PmExactOwnedCancelRequest,
    intent: PmOwnedCancelIntent,
}

#[allow(
    dead_code,
    reason = "opaque quarantine owns authenticated cancel authority until worker shutdown"
)]
struct PmQuarantinedAuthenticatedCancel {
    prior_goal_f_sequence: u64,
    client_order: reap_pm_core::PmClientOrderKey,
    intent: PmOwnedCancelIntent,
    retained: PmRetainedOwnedCancelRequest,
    grant: Option<PmAuthenticatedSendGrant>,
}

#[allow(
    dead_code,
    reason = "opaque quarantine owns post-send cancel evidence until worker shutdown"
)]
enum PmQuarantinedCancelResult {
    Unclassified {
        intent: PmOwnedCancelIntent,
        grant: PmAuthenticatedCancelSendGrant,
    },
    Classified {
        intent: PmOwnedCancelIntent,
        grant: PmAuthenticatedCancelSendGrant,
        result: PmAuthenticatedCancelResultV1,
    },
    DurabilityUnresolved {
        intent: PmOwnedCancelIntent,
        unresolved: PmAuthenticatedCancelResultUnresolved,
    },
}

#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PmPreSendQuarantineProjection {
    prior_goal_f_sequence: u64,
    client_order: reap_pm_core::PmClientOrderKey,
}

#[cfg(test)]
impl PmPreSendQuarantineProjection {
    pub(crate) const fn prior_goal_f_sequence(self) -> u64 {
        self.prior_goal_f_sequence
    }

    pub(crate) const fn client_order(self) -> reap_pm_core::PmClientOrderKey {
        self.client_order
    }
}
