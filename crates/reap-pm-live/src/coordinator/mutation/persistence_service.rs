//! Scheduler-facing reduction of exact durable-writer receipts.
//!
//! Receipt polling stays separate from mutation admission and backend effect
//! dispatch so the second authenticated barrier cannot be hidden inside the
//! core lifecycle owner.

use super::*;

impl PmMutationOwner {
    #[cfg(test)]
    pub(crate) fn service_persistence(
        &mut self,
        monotonic_now_ns: u64,
    ) -> Result<PmPersistenceService, PmMutationError> {
        let poll = self.poll_persistence(monotonic_now_ns)?;
        self.reduce_persistence_poll(poll, monotonic_now_ns)
    }

    /// Polls the bounded durable-writer edge without consuming the exact
    /// acknowledgement authority.
    pub(crate) fn poll_persistence(
        &mut self,
        monotonic_now_ns: u64,
    ) -> Result<PmPersistencePoll, PmMutationError> {
        match self.persistence.poll_one(monotonic_now_ns) {
            Ok(poll) => Ok(poll),
            Err(PmPersistenceError::ClockRegression) => {
                self.halt = Some(PmMutationHalt::PersistenceClockRegression);
                Err(PmPersistenceError::ClockRegression.into())
            }
            Err(error) => {
                self.halt = Some(PmMutationHalt::PersistenceSaturated);
                Err(error.into())
            }
        }
    }

    /// Reduces one scheduler-serviced durable result exactly once.
    pub(crate) fn reduce_persistence_poll(
        &mut self,
        poll: PmPersistencePoll,
        monotonic_service_ns: u64,
    ) -> Result<PmPersistenceService, PmMutationError> {
        Self::validate_persistence_time(monotonic_service_ns)?;
        match poll {
            PmPersistencePoll::Empty => Ok(PmPersistenceService::Empty),
            PmPersistencePoll::Pending => Ok(PmPersistenceService::Pending),
            PmPersistencePoll::QuoteAcknowledged {
                reserved,
                effect_permit,
                acknowledgement,
            } => {
                let identity = PmPersistenceIntentIdentity::Quote {
                    intent: reserved.intent(),
                    client_order: reserved.client_order(),
                };
                let Some(revisions) = self.current_revisions else {
                    return self.invalidate_durable_quote(
                        identity,
                        effect_permit,
                        monotonic_service_ns,
                    );
                };
                match self.preparation.prepare_quote(
                    reserved,
                    self.instrument_scope,
                    revisions,
                    monotonic_service_ns,
                    acknowledgement,
                ) {
                    Ok(authority) => {
                        self.effects.commit(
                            effect_permit,
                            PmPreparedMutation::Quote { authority },
                            monotonic_service_ns,
                        )?;
                        self.counters.prepared_quotes =
                            self.counters.prepared_quotes.saturating_add(1);
                        Ok(PmPersistenceService::PreparedQuote { identity })
                    }
                    Err(
                        error @ (PmAuthorityError::RevisionChanged
                        | PmAuthorityError::ApprovalExpired),
                    ) => {
                        let _expected_invalidation = error;
                        self.invalidate_durable_quote(identity, effect_permit, monotonic_service_ns)
                    }
                    Err(error) => {
                        self.retain_failed_effect(
                            effect_permit,
                            PmMutationHalt::PreparationFailed,
                        )?;
                        self.counters.preparation_failures =
                            self.counters.preparation_failures.saturating_add(1);
                        Err(error.into())
                    }
                }
            }
            PmPersistencePoll::CancelAcknowledged {
                reserved,
                owned_intent,
                effect_permit,
                acknowledgement,
            } => {
                let identity = PmPersistenceIntentIdentity::Cancel {
                    client_order: owned_intent.client_order(),
                    venue_order: owned_intent.venue_order(),
                };
                match self.preparation.prepare_cancel(
                    reserved,
                    self.instrument_scope,
                    monotonic_service_ns,
                    acknowledgement,
                ) {
                    Ok(authority) => {
                        self.effects.commit(
                            effect_permit,
                            PmPreparedMutation::Cancel {
                                authority,
                                owned_intent,
                            },
                            monotonic_service_ns,
                        )?;
                        self.counters.prepared_cancels =
                            self.counters.prepared_cancels.saturating_add(1);
                        Ok(PmPersistenceService::PreparedCancel { identity })
                    }
                    Err(error) => {
                        self.retain_failed_effect(
                            effect_permit,
                            PmMutationHalt::PreparationFailed,
                        )?;
                        self.counters.preparation_failures =
                            self.counters.preparation_failures.saturating_add(1);
                        Err(error.into())
                    }
                }
            }
            PmPersistencePoll::FactAcknowledged {
                acknowledgement,
                compaction,
            } => {
                let sequence = acknowledgement.consume();
                if let Some(ticket) = compaction {
                    let compacted = self
                        .private
                        .commit_fill_watermark_compaction(ticket)
                        .map_err(|error| {
                            self.halt = Some(PmMutationHalt::InternalInvariant);
                            PmMutationError::State(error)
                        })?;
                    self.count_compaction(compacted);
                }
                Ok(PmPersistenceService::FactAcknowledged { sequence })
            }
            PmPersistencePoll::IntentFailed {
                identity,
                effect_permit,
                reason,
            } => {
                self.retain_goal_f_writer_failure(PmGoalFWriterFailure::new(
                    PmGoalFWriterFailureIdentity::Intent(identity),
                    &reason,
                ));
                let halt = Self::halt_for_persistence_failure(&reason);
                self.retain_failed_effect(effect_permit, halt)?;
                self.record_persistence_failure(&reason);
                Ok(PmPersistenceService::IntentFailed { identity })
            }
            PmPersistencePoll::FactFailed {
                identity,
                reason,
                compaction,
            } => {
                self.retain_goal_f_writer_failure(PmGoalFWriterFailure::new(identity, &reason));
                if let Some(ticket) = compaction {
                    self.private
                        .abort_fill_watermark_compaction(ticket)
                        .map_err(|error| {
                            self.halt = Some(PmMutationHalt::InternalInvariant);
                            PmMutationError::State(error)
                        })?;
                }
                self.halt = Some(Self::halt_for_persistence_failure(&reason));
                self.record_persistence_failure(&reason);
                Ok(PmPersistenceService::FactFailed)
            }
            PmPersistencePoll::LiveBridgeAcknowledged {
                acknowledgement,
                completion,
                correlation,
            } => {
                if self.applied_live_bridges.len() >= 2 {
                    self.halt = Some(PmMutationHalt::InternalInvariant);
                    self.quarantine_live_bridge(completion);
                    return Err(PmMutationError::AuthenticatedBridgeCompletionSaturated);
                }
                let sequence = acknowledgement.consume();
                if sequence == 0 {
                    self.halt = Some(PmMutationHalt::InternalInvariant);
                    self.quarantine_live_bridge(completion);
                    return Err(PmMutationError::InvalidDurableConsequence);
                }
                let (kind, client_order, service, applied) = match completion {
                    PmPendingLiveBridge::Place(completion) => {
                        let client_order = completion.client_order();
                        let applied = PmAuthenticatedBridgeApplied::place(&completion);
                        super::super::authenticated_reduction::apply_live_place(
                            self,
                            *completion,
                            monotonic_service_ns,
                        )?;
                        (
                            PmDurableRecordKind::PlaceResult,
                            client_order,
                            PmPersistenceService::LivePlaceApplied { client_order },
                            applied,
                        )
                    }
                    PmPendingLiveBridge::Cancel(completion) => {
                        let client_order = completion.client_order();
                        let venue_order = completion.venue_order();
                        let applied = PmAuthenticatedBridgeApplied::cancel(&completion);
                        super::super::authenticated_reduction::apply_live_cancel(
                            self,
                            completion,
                            monotonic_service_ns,
                        )?;
                        (
                            PmDurableRecordKind::CancelResult,
                            client_order,
                            PmPersistenceService::LiveCancelApplied {
                                client_order,
                                venue_order,
                            },
                            applied,
                        )
                    }
                };
                self.durable_consequences.push_back(PmDurableConsequence {
                    kind,
                    client_order: Some(client_order),
                    correlation,
                });
                self.applied_live_bridges.push_back(applied);
                Ok(service)
            }
            PmPersistencePoll::LiveBridgeFailed { reason, completion } => {
                let client_order = completion.client_order();
                self.halt = Some(Self::halt_for_persistence_failure(&reason));
                self.record_persistence_failure(&reason);
                if self.failed_live_bridges.len() < 2 {
                    self.failed_live_bridges
                        .push_back(PmAuthenticatedBridgeFailure::from_pending(
                            &completion,
                            &reason,
                        ));
                } else {
                    self.halt = Some(PmMutationHalt::InternalInvariant);
                }
                self.quarantine_live_bridge(completion);
                Ok(PmPersistenceService::LiveBridgeFailed { client_order })
            }
        }
    }

    pub(super) fn retain_goal_f_writer_failure(&mut self, failure: PmGoalFWriterFailure) {
        if self.failed_goal_f_write.is_none() {
            self.failed_goal_f_write = Some(failure);
        }
    }
}
