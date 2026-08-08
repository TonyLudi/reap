//! Purpose-specific reconciliation-lane reduction for the sole coordinator.

use super::*;

impl<M: PmQuoteModel> PmCoordinator<M> {
    pub(super) fn service_reconciliation(
        &mut self,
        item: PmCompleteServiced<PmReconciliationInput>,
        effects: &mut PmProductEffectBatch,
    ) -> Result<(), PmCoordinatorError> {
        let monotonic_service_ns = item.clock().monotonic_service_ns();
        #[cfg(any(test, feature = "loopback-evidence"))]
        let source = product_source(item.source())?;
        #[cfg(any(test, feature = "loopback-evidence"))]
        let connection = item.connection();
        let clock = item.clock();
        self.invalidate_quote_authority_at(clock.monotonic_service_ns())?;
        match item.into_value() {
            #[cfg(any(test, feature = "loopback-evidence"))]
            PmReconciliationInput::DependencyUnavailable(failure) => {
                let _request_sequence = failure.request_sequence();
                self.mutation
                    .private_mut()
                    .reduce_serviced_http_dependency_failure(
                        source,
                        connection,
                        failure.dependency(),
                        failure.fault(),
                    )?;
                let _refresh = self.admit_refresh_reason(
                    PmRefreshReason::ExternalIngressFault,
                    monotonic_service_ns,
                    effects,
                )?;
                effects.push(PmProductEffect::FailClosedHaltOrCancel(
                    PmFailClosedEffect::halt(
                        self.account_scope,
                        self.instrument,
                        PmControlReason::PrivateUnavailable,
                    ),
                ))?;
                self.schedule_both_sides(
                    PmScheduledActionKind::CancelOwnedQuote,
                    monotonic_service_ns,
                    captured_wall_timestamp_ms(clock.local_wall_receive_ns())?,
                )?;
            }
            PmReconciliationInput::OpenOrders(delivery) => {
                let envelope = self
                    .mutation
                    .private_mut()
                    .open_product_open_orders(delivery, monotonic_service_ns)?;
                let source = envelope.source();
                let connection = envelope.connection_id();
                let clock = envelope.clock();
                let ordering = envelope.ordering();
                let apply = self.mutation.private_mut().reduce_serviced_open_orders(
                    source,
                    connection,
                    clock,
                    ordering,
                    envelope.into_payload(),
                )?;
                if matches!(apply, PmOpenOrdersApply::Applied { .. }) {
                    let _completed = self.complete_reconciled_refresh_reason(
                        PmRefreshReason::AmbiguousOrder,
                        monotonic_service_ns,
                        effects,
                    )?;
                    if self.mutation.has_missing_order_detail() {
                        let _detail = self.admit_refresh_reason(
                            PmRefreshReason::MissingOrderDetail,
                            monotonic_service_ns,
                            effects,
                        )?;
                    }
                    while effects.len() < MAX_PM_EFFECTS_PER_INPUT.saturating_sub(1) {
                        if !self.admit_next_refresh(monotonic_service_ns, effects)? {
                            break;
                        }
                    }
                    if self.mutation.next_pending_refresh().is_some() {
                        return Err(PmCoordinatorError::EffectProjectionSaturated);
                    }
                }
            }
            PmReconciliationInput::OrderDetail(delivery) => {
                let envelope = self
                    .mutation
                    .private_mut()
                    .open_product_order_detail(delivery, monotonic_service_ns)?;
                let source = envelope.source();
                let connection = envelope.connection_id();
                let clock = envelope.clock();
                let ordering = envelope.ordering();
                self.mutation.private_mut().reduce_serviced_order_detail(
                    source,
                    connection,
                    clock,
                    ordering,
                    envelope.into_payload(),
                )?;
                if self
                    .refresh_obligations
                    .has_reason(PmRefreshReason::MissingOrderDetail)
                {
                    if self.mutation.has_missing_order_detail() {
                        let _newer = self
                            .mutation
                            .require_refresh(PmRefreshReason::MissingOrderDetail)?;
                    }
                    let _completed = self.complete_reconciled_refresh_reason(
                        PmRefreshReason::MissingOrderDetail,
                        monotonic_service_ns,
                        effects,
                    )?;
                }
            }
            PmReconciliationInput::StandaloneAccount(delivery) => {
                let envelope = self
                    .mutation
                    .private_mut()
                    .open_product_account(delivery, monotonic_service_ns)?;
                let source = envelope.source();
                let connection = envelope.connection_id();
                let clock = envelope.clock();
                let ordering = envelope.ordering();
                self.mutation
                    .private_mut()
                    .reduce_serviced_account_snapshot(
                        source,
                        connection,
                        clock,
                        ordering,
                        envelope.into_payload(),
                    )?;
            }
            PmReconciliationInput::Paired(delivery) => {
                let (account, fills) = self
                    .mutation
                    .private_mut()
                    .open_product_reconciliation(delivery, monotonic_service_ns)?;
                let unique_before = self.mutation.counters().unique_fills();
                let apply = self
                    .mutation
                    .reduce_serviced_reconciliation(account, fills)?;
                if matches!(apply, PmReconciliationApply::Applied { .. }) {
                    self.note_complete_reconciliation();
                    for reason in [
                        PmRefreshReason::PrivateReconnect,
                        PmRefreshReason::FillObserved,
                        PmRefreshReason::ExternalIngressFault,
                    ] {
                        let _completed = self.complete_reconciled_refresh_reason(
                            reason,
                            monotonic_service_ns,
                            effects,
                        )?;
                    }
                }
                if self.mutation.counters().unique_fills() != unique_before {
                    for tracked in self.tracked_quotes.into_iter().flatten() {
                        self.refresh_tracked_quote(tracked.client_order);
                    }
                }
            }
        }
        self.advance_private_readiness_revision()?;
        push_metric(effects, PmHealthMetricKind::InputObserved, 1)
    }
}
