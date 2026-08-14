//! Live-only coordinator ingress and authenticated persistence bridge seams.
//!
//! Fixture construction and lane servicing remain in the parent module. This
//! child contains only sealed live carriers that delegate into the same sole
//! coordinator and canonical reducers.

use super::*;

impl<M: PmQuoteModel> PmCoordinator<M> {
    pub(crate) fn connect_private_live(
        &mut self,
        input: PmLiveConnectionInput,
    ) -> Result<(), PmCoordinatorError> {
        self.connect_private(input.into_occurrence())
    }

    pub(crate) fn mark_private_live_unavailable(
        &mut self,
        input: PmLiveRetirementInput,
    ) -> Result<(), PmCoordinatorError> {
        let (occurrence, fault) = input.into_parts();
        self.mark_private_unavailable(occurrence, fault)
    }

    pub(crate) fn ingest_private_live(
        &mut self,
        input: PmLivePrivateInput,
    ) -> Result<PmLiveIngressReport, PmCoordinatorError> {
        let (delivery, report) = self
            .mutation
            .private_mut()
            .receive_product_private_live(input)?;
        let input = PmPrivateInput::Batch(delivery);
        let ingress = input
            .ingress()
            .expect("private batches retain exact product ingress");
        self.enqueue_private(ingress, input)?;
        Ok(report)
    }

    pub(crate) fn ingest_account_live(
        &mut self,
        input: PmLiveAccountInput,
    ) -> Result<PmLiveIngressReport, PmCoordinatorError> {
        let (delivery, report) = self
            .mutation
            .private_mut()
            .complete_product_account_live(input)?;
        let input = PmReconciliationInput::StandaloneAccount(delivery);
        let ingress = input
            .ingress()
            .expect("complete account delivery carries product ingress");
        self.enqueue_reconciliation(ingress, input)?;
        Ok(report)
    }

    pub(crate) fn ingest_open_orders_live(
        &mut self,
        input: PmLiveOpenOrdersInput,
    ) -> Result<PmLiveIngressReport, PmCoordinatorError> {
        let (delivery, report) = self
            .mutation
            .private_mut()
            .complete_product_open_orders_live(input)?;
        let input = PmReconciliationInput::OpenOrders(delivery);
        let ingress = input
            .ingress()
            .expect("complete open-orders delivery carries product ingress");
        self.enqueue_reconciliation(ingress, input)?;
        Ok(report)
    }

    pub(crate) fn ingest_order_detail_live(
        &mut self,
        input: PmLiveOrderDetailInput,
    ) -> Result<PmLiveIngressReport, PmCoordinatorError> {
        let (delivery, report) = self
            .mutation
            .private_mut()
            .complete_product_order_detail_live(input)?;
        let input = PmReconciliationInput::OrderDetail(delivery);
        let ingress = input
            .ingress()
            .expect("complete order-detail delivery carries product ingress");
        self.enqueue_reconciliation(ingress, input)?;
        Ok(report)
    }

    pub(crate) fn ingest_reconciliation_live(
        &mut self,
        input: PmLiveReconciliationInput,
    ) -> Result<PmLiveIngressReport, PmCoordinatorError> {
        let (delivery, report) = self
            .mutation
            .private_mut()
            .complete_product_reconciliation_live(input)?;
        let input = PmReconciliationInput::Paired(delivery);
        let ingress = input
            .ingress()
            .expect("complete reconciliation delivery carries product ingress");
        self.enqueue_reconciliation(ingress, input)?;
        Ok(report)
    }

    #[cfg(any(test, feature = "loopback-evidence"))]
    pub(crate) fn ingest_account_live_failure(
        &mut self,
        input: PmLiveAccountFailureInput,
    ) -> Result<(), PmCoordinatorError> {
        self.ingest_live_http_dependency_failure(input.into_dependency_failure())
    }

    #[cfg(any(test, feature = "loopback-evidence"))]
    pub(crate) fn ingest_open_orders_live_failure(
        &mut self,
        input: PmLiveOpenOrdersFailureInput,
    ) -> Result<(), PmCoordinatorError> {
        self.ingest_live_http_dependency_failure(input.into_dependency_failure())
    }

    #[cfg(any(test, feature = "loopback-evidence"))]
    pub(crate) fn ingest_order_detail_live_failure(
        &mut self,
        input: PmLiveOrderDetailFailureInput,
    ) -> Result<(), PmCoordinatorError> {
        self.ingest_live_http_dependency_failure(input.into_dependency_failure())
    }

    #[cfg(any(test, feature = "loopback-evidence"))]
    pub(crate) fn ingest_reconciliation_live_failure(
        &mut self,
        input: PmLiveReconciliationFailureInput,
    ) -> Result<(), PmCoordinatorError> {
        self.ingest_live_http_dependency_failure(input.into_dependency_failure())
    }

    #[cfg(any(test, feature = "loopback-evidence"))]
    fn ingest_live_http_dependency_failure(
        &mut self,
        failure: PmLiveHttpDependencyFailure,
    ) -> Result<(), PmCoordinatorError> {
        let ingress = self.account_ingress(failure.occurrence());
        self.mutation.invalidate_revisions();
        self.reconciliation_gate = true;
        self.reconciliation_recovered = false;
        self.enqueue_reconciliation(
            ingress,
            PmReconciliationInput::DependencyUnavailable(failure),
        )
    }

    pub(crate) fn request_live_shutdown(
        &mut self,
        occurrence: PmLiveInternalControlOccurrence,
    ) -> Result<(), PmCoordinatorError> {
        self.request_shutdown(occurrence.into_occurrence())
    }

    /// Polls Goal-F persistence using an occurrence minted by the sole live
    /// issuer. Authenticated composition cannot substitute fixture ordering.
    #[cfg(any(test, feature = "loopback-evidence"))]
    pub(crate) fn poll_persistence_live(
        &mut self,
        occurrence: PmLivePersistencePollOccurrence,
        monotonic_poll_ns: u64,
    ) -> Result<bool, PmCoordinatorError> {
        let ingress = self.internal_ingress(&occurrence.into_occurrence());
        self.poll_persistence_into_lane(ingress, monotonic_poll_ns)
    }

    /// Takes the move-only proof emitted only after the Goal-F bridge is
    /// durable and its canonical reduction completed.
    #[cfg(any(test, feature = "loopback-evidence"))]
    pub(crate) fn take_authenticated_bridge_applied(
        &mut self,
    ) -> Option<crate::coordinator::live_completion::PmAuthenticatedBridgeApplied> {
        self.mutation.take_applied_live_bridge()
    }

    /// Takes the bounded failure identity emitted when the authenticated
    /// result writer failed and the move-only completion was quarantined.
    #[cfg(any(test, feature = "loopback-evidence"))]
    pub(crate) fn take_authenticated_bridge_failure(
        &mut self,
    ) -> Option<crate::coordinator::PmAuthenticatedBridgeFailure> {
        self.mutation.take_failed_live_bridge()
    }

    /// Takes the first bounded, secret-free non-bridge Goal-F writer failure.
    #[cfg(any(test, feature = "loopback-evidence"))]
    pub(crate) fn take_goal_f_writer_failure(
        &mut self,
    ) -> Option<crate::coordinator::PmGoalFWriterFailure> {
        self.mutation.take_failed_goal_f_write()
    }

    /// Proves that every coordinator-owned stage capable of carrying Goal-F
    /// durability work is empty after live producers have been stopped.
    ///
    /// A writer poll is not complete merely because it left the mutation
    /// queue: it can still be retained for admission, queued in the
    /// persistence rank, or waiting for its copied durable consequence to be
    /// published. Controlled shutdown therefore checks the whole bounded
    /// chain before releasing either journal owner.
    #[cfg(any(test, feature = "loopback-evidence"))]
    pub(crate) fn goal_f_shutdown_quiescent(&self) -> bool {
        self.goal_f_shutdown_unresolved_counts()
            .into_iter()
            .all(|count| count == 0)
    }

    /// Fixed-order, secret-free evidence for a Goal-F shutdown that could not
    /// reach quiescence. The ranks are mutation queue, critical lane,
    /// persistence lane, retained critical admission, retained persistence
    /// admission, durable consequences, and copied outputs.
    #[cfg(any(test, feature = "loopback-evidence"))]
    pub(crate) fn goal_f_shutdown_unresolved_counts(&self) -> [usize; 7] {
        let (critical_lane, persistence_lane) = self
            .lanes
            .as_ref()
            .map_or((1, 1), |lanes| lanes.goal_f_input_lane_depths());
        [
            self.mutation.pending_persistence(),
            critical_lane,
            persistence_lane,
            usize::from(self.retained_critical.is_some()),
            usize::from(self.retained_persistence.is_some()),
            self.mutation.pending_durable_consequences(),
            self.outputs.len(),
        ]
    }

    /// Fixed-order, secret-free evidence that accepted authenticated read
    /// input still belongs to the sole coordinator. The ranks are public,
    /// private, and reconciliation lanes followed by retained private and
    /// retained reconciliation admissions. Public admission errors return
    /// synchronously to their socket/book acknowledgement and therefore have
    /// no separate retained-admission slot.
    #[cfg(any(test, feature = "loopback-evidence"))]
    pub(crate) fn read_shutdown_unresolved_counts(&self) -> [usize; 5] {
        let (public_lane, private_lane, reconciliation_lane) = self
            .lanes
            .as_ref()
            .map_or((1, 1, 1), |lanes| lanes.read_input_lane_depths());
        [
            public_lane,
            private_lane,
            reconciliation_lane,
            usize::from(self.retained_private_admission.is_some()),
            usize::from(self.retained_reconciliation_admission.is_some()),
        ]
    }
}
