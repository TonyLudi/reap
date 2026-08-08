//! Typed live account/private ingress over the existing owner-bound roles.
//!
//! Transport assembly and authentication finish before these values exist.
//! This module performs only synchronous normalization, owner-bound delivery
//! opening, and admission into the monitor's existing canonical reducers.

#![allow(
    dead_code,
    reason = "the sealed live ingress seam is consumed by the Phase 5 supervisor"
)]

use reap_pm_core::{PmFillQueryCursor, PmVenueOrderKey};
use reap_pm_state::{
    PmAccountSnapshotApply, PmOpenOrdersApply, PmOrderApply, PmReconciliationApply,
};
use reap_polymarket_adapter::{PmForeignRowDiagnostics, PmFullAccountFillSnapshotDigest};
use reap_polymarket_auth::CredentialOwnedUserFrame;
use reap_polymarket_live_adapter::{
    PmAccountAsset, PmAccountBalanceAllowance, PmCompleteOpenOrdersCut, PmCompleteTradesCut,
    PmExactOrderObservation,
};

use super::{
    PmFixtureQueryOccurrence, PmPrivateBatchApply, PmPrivateMonitorError,
    PmPrivateMonitorInputError, PmPrivateMonitorRuntime, PmReadOnlyMonitor,
    open_order_reservations_into, reduce_private_batch, remote_reservation,
};

mod occurrence;

#[allow(
    unused_imports,
    reason = "the sealed issuer is consumed by the Phase 5 static live supervisor"
)]
pub(crate) use occurrence::{
    PmLiveAccountFailureInput, PmLiveAccountQueryTicket, PmLiveConnectionInput,
    PmLiveHttpDependencyFailure, PmLiveHttpQueryFailure, PmLiveInternalControlOccurrence,
    PmLiveMutationCompletionOccurrence, PmLiveOccurrenceError, PmLiveOccurrenceIssuer,
    PmLiveOpenOrdersFailureInput, PmLiveOpenOrdersQueryTicket, PmLiveOrderDetailFailureInput,
    PmLiveOrderDetailQueryTicket, PmLivePersistencePollOccurrence, PmLivePreOpenRetirement,
    PmLiveReconciliationFailureInput, PmLiveReconciliationQueryTicket, PmLiveRetirementInput,
    PmLiveRetirementOutcome,
};

/// One complete, credential-owner-bound private-stream occurrence.
#[derive(Debug)]
pub(crate) struct PmLivePrivateInput {
    occurrence: reap_polymarket_adapter::PmCompletionOccurrence,
    frame: CredentialOwnedUserFrame,
}

impl PmLivePrivateInput {
    #[must_use]
    pub(crate) const fn new(
        occurrence: reap_polymarket_adapter::PmCompletionOccurrence,
        frame: CredentialOwnedUserFrame,
    ) -> Self {
        Self { occurrence, frame }
    }
}

/// One terminally paginated account-wide open-order cut.
#[derive(Debug)]
pub(crate) struct PmLiveOpenOrdersInput {
    occurrence: PmFixtureQueryOccurrence,
    cut: PmCompleteOpenOrdersCut,
}

impl PmLiveOpenOrdersInput {
    #[must_use]
    pub(crate) const fn new(
        occurrence: PmFixtureQueryOccurrence,
        cut: PmCompleteOpenOrdersCut,
    ) -> Self {
        Self { occurrence, cut }
    }
}

/// One authenticated exact-order result for a journal-known order identity.
#[derive(Debug)]
pub(crate) struct PmLiveOrderDetailInput {
    occurrence: PmFixtureQueryOccurrence,
    requested_order: PmVenueOrderKey,
    observation: PmExactOrderObservation,
}

impl PmLiveOrderDetailInput {
    #[must_use]
    pub(crate) const fn new(
        occurrence: PmFixtureQueryOccurrence,
        requested_order: PmVenueOrderKey,
        observation: PmExactOrderObservation,
    ) -> Self {
        Self {
            occurrence,
            requested_order,
            observation,
        }
    }
}

/// One atomic collateral plus configured-conditional account cut.
#[derive(Debug)]
pub(crate) struct PmLiveAccountInput {
    occurrence: PmFixtureQueryOccurrence,
    collateral: PmAccountBalanceAllowance,
    conditional: PmAccountBalanceAllowance,
}

impl PmLiveAccountInput {
    pub(crate) fn new(
        occurrence: PmFixtureQueryOccurrence,
        collateral: PmAccountBalanceAllowance,
        conditional: PmAccountBalanceAllowance,
    ) -> Result<Self, PmPrivateMonitorInputError> {
        validate_asset_kinds(&collateral, &conditional)?;
        Ok(Self {
            occurrence,
            collateral,
            conditional,
        })
    }
}

/// One inseparable account plus terminal full-account fill reconciliation.
#[derive(Debug)]
pub(crate) struct PmLiveReconciliationInput {
    occurrence: PmFixtureQueryOccurrence,
    collateral: PmAccountBalanceAllowance,
    conditional: PmAccountBalanceAllowance,
    requested_after: Option<PmFillQueryCursor>,
    trades: PmCompleteTradesCut,
}

impl PmLiveReconciliationInput {
    #[allow(
        clippy::too_many_arguments,
        reason = "the complete account/fill cut keeps every causal authority explicit"
    )]
    pub(crate) fn new(
        occurrence: PmFixtureQueryOccurrence,
        collateral: PmAccountBalanceAllowance,
        conditional: PmAccountBalanceAllowance,
        requested_after: Option<PmFillQueryCursor>,
        trades: PmCompleteTradesCut,
    ) -> Result<Self, PmPrivateMonitorInputError> {
        validate_asset_kinds(&collateral, &conditional)?;
        Ok(Self {
            occurrence,
            collateral,
            conditional,
            requested_after,
            trades,
        })
    }
}

/// Explicit, secret-free evidence retained when live input is accepted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PmLiveIngressReport {
    Private {
        foreign: PmForeignRowDiagnostics,
    },
    OpenOrders {
        foreign: PmForeignRowDiagnostics,
    },
    OrderDetail,
    Account {
        foreign: PmForeignRowDiagnostics,
    },
    Reconciliation {
        account_foreign: PmForeignRowDiagnostics,
        fill_foreign: PmForeignRowDiagnostics,
        full_account_fill_digest: PmFullAccountFillSnapshotDigest,
    },
}

impl PmLiveIngressReport {
    #[must_use]
    pub(crate) const fn private_foreign(self) -> Option<PmForeignRowDiagnostics> {
        match self {
            Self::Private { foreign } => Some(foreign),
            _ => None,
        }
    }

    #[must_use]
    pub(crate) const fn open_orders_foreign(self) -> Option<PmForeignRowDiagnostics> {
        match self {
            Self::OpenOrders { foreign } => Some(foreign),
            _ => None,
        }
    }

    #[must_use]
    pub(crate) const fn account_foreign(self) -> Option<PmForeignRowDiagnostics> {
        match self {
            Self::Account { foreign }
            | Self::Reconciliation {
                account_foreign: foreign,
                ..
            } => Some(foreign),
            _ => None,
        }
    }

    #[must_use]
    pub(crate) const fn fill_foreign(self) -> Option<PmForeignRowDiagnostics> {
        match self {
            Self::Reconciliation { fill_foreign, .. } => Some(fill_foreign),
            _ => None,
        }
    }

    #[must_use]
    pub(crate) const fn full_account_fill_digest(self) -> Option<PmFullAccountFillSnapshotDigest> {
        match self {
            Self::Reconciliation {
                full_account_fill_digest,
                ..
            } => Some(full_account_fill_digest),
            _ => None,
        }
    }
}

/// Canonical monitor apply result paired with its retained live diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PmLiveMonitorApply<T> {
    apply: T,
    report: PmLiveIngressReport,
}

impl<T> PmLiveMonitorApply<T> {
    const fn new(apply: T, report: PmLiveIngressReport) -> Self {
        Self { apply, report }
    }

    #[must_use]
    pub(crate) const fn apply(&self) -> &T {
        &self.apply
    }

    #[must_use]
    pub(crate) const fn report(&self) -> PmLiveIngressReport {
        self.report
    }

    #[must_use]
    pub(crate) fn into_parts(self) -> (T, PmLiveIngressReport) {
        (self.apply, self.report)
    }
}

impl PmPrivateMonitorRuntime {
    pub(crate) fn receive_product_private_live(
        &mut self,
        input: PmLivePrivateInput,
    ) -> Result<
        (
            reap_polymarket_adapter::PmPrivateDelivery,
            PmLiveIngressReport,
        ),
        PmPrivateMonitorError,
    > {
        let completion = self
            .private
            .receive_live_user_frame(input.occurrence, input.frame)?;
        let (delivery, foreign) = completion.into_parts();
        Ok((delivery, PmLiveIngressReport::Private { foreign }))
    }

    pub(crate) fn complete_product_account_live(
        &mut self,
        input: PmLiveAccountInput,
    ) -> Result<
        (
            reap_polymarket_adapter::PmCompleteAccountSnapshotDelivery,
            PmLiveIngressReport,
        ),
        PmPrivateMonitorError,
    > {
        let PmLiveAccountInput {
            occurrence,
            collateral,
            conditional,
        } = input;
        self.validate_private_epoch(occurrence.connection_epoch())?;
        self.validate_live_account_assets(&collateral, &conditional)?;
        let request = self
            .account
            .request_snapshot(occurrence.connection_epoch(), occurrence.request_sequence())?;
        let completion = request.complete_live(
            occurrence.completion(),
            occurrence.snapshot(),
            collateral.value(),
            conditional.value(),
        )?;
        let (delivery, foreign) = completion.into_parts();
        Ok((delivery, PmLiveIngressReport::Account { foreign }))
    }

    pub(crate) fn complete_product_open_orders_live(
        &mut self,
        input: PmLiveOpenOrdersInput,
    ) -> Result<
        (
            reap_polymarket_adapter::PmCompleteOpenOrdersDelivery,
            PmLiveIngressReport,
        ),
        PmPrivateMonitorError,
    > {
        let PmLiveOpenOrdersInput { occurrence, cut } = input;
        self.validate_private_epoch(occurrence.connection_epoch())?;
        let request = self
            .reconciliation
            .request_open_orders(occurrence.connection_epoch(), occurrence.request_sequence())?;
        let completion = request.complete_live_pages(
            occurrence.completion(),
            occurrence.snapshot(),
            cut.pages(),
        )?;
        let (delivery, foreign) = completion.into_parts();
        Ok((delivery, PmLiveIngressReport::OpenOrders { foreign }))
    }

    pub(crate) fn complete_product_order_detail_live(
        &mut self,
        input: PmLiveOrderDetailInput,
    ) -> Result<
        (
            reap_polymarket_adapter::PmExactOrderDetailDelivery,
            PmLiveIngressReport,
        ),
        PmPrivateMonitorError,
    > {
        let PmLiveOrderDetailInput {
            occurrence,
            requested_order,
            observation,
        } = input;
        self.validate_private_epoch(occurrence.connection_epoch())?;
        let request = self.reconciliation.request_order_detail(
            occurrence.connection_epoch(),
            occurrence.request_sequence(),
            requested_order,
        )?;
        let present = match observation {
            PmExactOrderObservation::Present(order) => Some(order),
            PmExactOrderObservation::Absent => None,
        };
        let delivery = request.complete_live(
            occurrence.completion(),
            occurrence.snapshot(),
            present.as_deref(),
        )?;
        Ok((delivery, PmLiveIngressReport::OrderDetail))
    }

    pub(crate) fn complete_product_reconciliation_live(
        &mut self,
        input: PmLiveReconciliationInput,
    ) -> Result<(super::PmPairedReconciliationDelivery, PmLiveIngressReport), PmPrivateMonitorError>
    {
        let PmLiveReconciliationInput {
            occurrence,
            collateral,
            conditional,
            requested_after,
            trades,
        } = input;
        self.validate_private_epoch(occurrence.connection_epoch())?;
        self.validate_live_account_assets(&collateral, &conditional)?;

        let account_request = self
            .account
            .request_snapshot(occurrence.connection_epoch(), occurrence.request_sequence())?;
        let fill_request = self.reconciliation.request_fills(
            occurrence.connection_epoch(),
            occurrence.request_sequence(),
            requested_after,
        )?;
        let account = account_request.complete_live(
            occurrence.completion(),
            occurrence.snapshot(),
            collateral.value(),
            conditional.value(),
        )?;
        let fills = fill_request.complete_live_trade_pages(
            occurrence.completion(),
            occurrence.snapshot(),
            trades.pages(),
        )?;
        let (account_delivery, account_foreign) = account.into_parts();
        let (fill_delivery, fill_foreign, full_account_fill_digest) = fills.into_parts();
        let paired = super::PmPairedReconciliationDelivery::new(account_delivery, fill_delivery)?;
        Ok((
            paired,
            PmLiveIngressReport::Reconciliation {
                account_foreign,
                fill_foreign,
                full_account_fill_digest,
            },
        ))
    }

    fn validate_live_account_assets(
        &self,
        collateral: &PmAccountBalanceAllowance,
        conditional: &PmAccountBalanceAllowance,
    ) -> Result<(), PmPrivateMonitorError> {
        validate_asset_kinds(collateral, conditional).map_err(PmPrivateMonitorError::Input)?;
        let expected = self.account.instrument_scope().metadata().outcome().token();
        if conditional.asset() != PmAccountAsset::Conditional(expected) {
            return Err(PmPrivateMonitorError::ConditionalTokenMismatch);
        }
        Ok(())
    }

    fn ingest_private_live(
        &mut self,
        input: PmLivePrivateInput,
        monotonic_service_ns: u64,
    ) -> Result<PmLiveMonitorApply<PmPrivateBatchApply>, PmPrivateMonitorError> {
        input
            .occurrence
            .received_clock()
            .service_at(monotonic_service_ns)?;
        let (delivery, report) = self.receive_product_private_live(input)?;
        let envelope = self.open_product_private(delivery, monotonic_service_ns)?;
        let apply = reduce_private_batch(
            &mut self.state,
            &mut self.private_batch_identity_scratch,
            envelope,
        )?;
        Ok(PmLiveMonitorApply::new(apply, report))
    }

    fn ingest_account_live(
        &mut self,
        input: PmLiveAccountInput,
    ) -> Result<PmLiveMonitorApply<PmAccountSnapshotApply>, PmPrivateMonitorError> {
        let monotonic_service_ns = input.occurrence.monotonic_service_ns();
        let (delivery, report) = self.complete_product_account_live(input)?;
        let envelope = self.open_product_account(delivery, monotonic_service_ns)?;
        let apply = self.state.apply_account_snapshot(envelope)?;
        Ok(PmLiveMonitorApply::new(apply, report))
    }

    fn ingest_open_orders_live(
        &mut self,
        input: PmLiveOpenOrdersInput,
    ) -> Result<PmLiveMonitorApply<PmOpenOrdersApply>, PmPrivateMonitorError> {
        let monotonic_service_ns = input.occurrence.monotonic_service_ns();
        let (delivery, report) = self.complete_product_open_orders_live(input)?;
        let envelope = self.open_product_open_orders(delivery, monotonic_service_ns)?;
        open_order_reservations_into(
            envelope.payload().orders(),
            &mut self.open_order_reservation_scratch,
        )?;
        let apply = self
            .state
            .apply_open_orders_snapshot(envelope, &self.open_order_reservation_scratch)?;
        Ok(PmLiveMonitorApply::new(apply, report))
    }

    fn ingest_order_detail_live(
        &mut self,
        input: PmLiveOrderDetailInput,
    ) -> Result<PmLiveMonitorApply<PmOrderApply>, PmPrivateMonitorError> {
        let monotonic_service_ns = input.occurrence.monotonic_service_ns();
        let (delivery, report) = self.complete_product_order_detail_live(input)?;
        let envelope = self.open_product_order_detail(delivery, monotonic_service_ns)?;
        let reservation = envelope
            .payload()
            .order()
            .map(remote_reservation)
            .transpose()?
            .unwrap_or(reap_pm_state::PmReservationKnowledge::Unknown);
        let apply = self.state.apply_order_detail(envelope, reservation)?;
        Ok(PmLiveMonitorApply::new(apply, report))
    }

    fn ingest_reconciliation_live(
        &mut self,
        input: PmLiveReconciliationInput,
    ) -> Result<PmLiveMonitorApply<PmReconciliationApply>, PmPrivateMonitorError> {
        let monotonic_service_ns = input.occurrence.monotonic_service_ns();
        let (delivery, report) = self.complete_product_reconciliation_live(input)?;
        let (account, fills) = self.open_product_reconciliation(delivery, monotonic_service_ns)?;
        let apply = self.state.apply_reconciliation(account, fills)?;
        Ok(PmLiveMonitorApply::new(apply, report))
    }
}

impl PmReadOnlyMonitor {
    /// Neutral connection name; the fixture spelling remains as a compatible
    /// alias on the product Run.
    pub(crate) fn connect_private(
        &mut self,
        connection_epoch: reap_pm_core::ConnectionEpoch,
        monotonic_observed_ns: u64,
    ) -> Result<(), PmPrivateMonitorError> {
        let result = self
            .runtime
            .reconnect_private(connection_epoch, monotonic_observed_ns);
        self.record_result(
            reap_pm_state::PmPrivateExternalIngressLane::Reconnect,
            result,
        )
    }

    pub(crate) fn ingest_private_live(
        &mut self,
        input: PmLivePrivateInput,
        monotonic_service_ns: u64,
    ) -> Result<PmLiveMonitorApply<PmPrivateBatchApply>, PmPrivateMonitorError> {
        let result = self
            .runtime
            .ingest_private_live(input, monotonic_service_ns);
        self.record_result(
            reap_pm_state::PmPrivateExternalIngressLane::PrivateLifecycle,
            result,
        )
    }

    pub(crate) fn ingest_account_live(
        &mut self,
        input: PmLiveAccountInput,
    ) -> Result<PmLiveMonitorApply<PmAccountSnapshotApply>, PmPrivateMonitorError> {
        let result = self.runtime.ingest_account_live(input);
        self.record_result(
            reap_pm_state::PmPrivateExternalIngressLane::AccountSnapshot,
            result,
        )
    }

    pub(crate) fn ingest_open_orders_live(
        &mut self,
        input: PmLiveOpenOrdersInput,
    ) -> Result<PmLiveMonitorApply<PmOpenOrdersApply>, PmPrivateMonitorError> {
        let result = self.runtime.ingest_open_orders_live(input);
        self.record_result(
            reap_pm_state::PmPrivateExternalIngressLane::OpenOrders,
            result,
        )
    }

    pub(crate) fn ingest_order_detail_live(
        &mut self,
        input: PmLiveOrderDetailInput,
    ) -> Result<PmLiveMonitorApply<PmOrderApply>, PmPrivateMonitorError> {
        let result = self.runtime.ingest_order_detail_live(input);
        self.record_result(
            reap_pm_state::PmPrivateExternalIngressLane::OrderDetail,
            result,
        )
    }

    pub(crate) fn ingest_reconciliation_live(
        &mut self,
        input: PmLiveReconciliationInput,
    ) -> Result<PmLiveMonitorApply<PmReconciliationApply>, PmPrivateMonitorError> {
        let result = self.runtime.ingest_reconciliation_live(input);
        self.record_result(
            reap_pm_state::PmPrivateExternalIngressLane::Reconciliation,
            result,
        )
    }
}

fn validate_asset_kinds(
    collateral: &PmAccountBalanceAllowance,
    conditional: &PmAccountBalanceAllowance,
) -> Result<(), PmPrivateMonitorInputError> {
    if collateral.asset() != PmAccountAsset::Collateral
        || !matches!(conditional.asset(), PmAccountAsset::Conditional(_))
    {
        return Err(PmPrivateMonitorInputError::AccountAssetKindMismatch);
    }
    Ok(())
}

#[cfg(test)]
mod tests;
