//! Product-owned fixture normalization and owner-bound delivery opening.
//!
//! These seams are deliberately crate-private. The public product Run accepts
//! only the narrow raw fixture inputs, and this module keeps each normalized
//! aggregate bound to the exact adapter role until the complete scheduler
//! services it.

use reap_pm_core::{
    ConnectionEpoch, EventEnvelope, EventOrdering, PmCompleteAccountSnapshot, PmCompleteFillQuery,
    PmCompleteOpenOrdersSnapshot, PmConnectionId, PmExactOrderDetail, PmProductSource,
    ReceivedEventClock,
};
use reap_polymarket_adapter::{
    PmCompleteAccountSnapshotDelivery, PmCompleteFillQueryDelivery, PmCompleteOpenOrdersDelivery,
    PmExactOrderDetailDelivery, PmFixtureCompletionOccurrence, PmFixtureFeeEvidence,
    PmFixturePrivateBatch, PmFixturePrivateDelivery,
};

use super::{
    PmAccountFixtureInput, PmOpenOrdersFixtureInput, PmOrderDetailFixtureInput,
    PmPrivateMonitorError, PmPrivateMonitorRuntime, PmReconciliationFixtureInput,
    validate_private_role_reconnect, validate_scope,
};

/// The two exact owner-bound deliveries that make one complete account/fill
/// reconciliation cut.
#[derive(Debug)]
pub(crate) struct PmFixturePairedReconciliationDelivery {
    account: PmCompleteAccountSnapshotDelivery,
    fills: PmCompleteFillQueryDelivery,
}

impl PmFixturePairedReconciliationDelivery {
    fn new(
        account: PmCompleteAccountSnapshotDelivery,
        fills: PmCompleteFillQueryDelivery,
    ) -> Result<Self, PmPrivateMonitorError> {
        if account.account_scope() != fills.account_scope()
            || account.instrument_scope() != fills.instrument_scope()
            || account.source() != fills.source()
            || account.connection() != fills.connection()
            || account.ordering() != fills.ordering()
            || account.received_clock() != fills.received_clock()
        {
            return Err(PmPrivateMonitorError::DeliveryScopeMismatch);
        }
        Ok(Self { account, fills })
    }

    pub(crate) const fn source(&self) -> PmProductSource {
        self.account.source()
    }

    pub(crate) const fn connection(&self) -> PmConnectionId {
        self.account.connection()
    }

    pub(crate) const fn ordering(&self) -> EventOrdering {
        self.account.ordering()
    }

    pub(crate) const fn received_clock(&self) -> ReceivedEventClock {
        self.account.received_clock()
    }
}

impl PmPrivateMonitorRuntime {
    /// Advances only the exact fixture normalization role. Canonical private
    /// readiness advances later, in scheduler order, when the connection
    /// occurrence is serviced.
    pub(crate) fn prepare_product_private_reconnect(
        &mut self,
        connection_epoch: ConnectionEpoch,
    ) -> Result<(), PmPrivateMonitorError> {
        validate_private_role_reconnect(self.private.active_epoch(), connection_epoch)?;
        self.private.reconnect(connection_epoch)?;
        Ok(())
    }

    pub(crate) fn receive_product_private_fixture(
        &mut self,
        occurrence: PmFixtureCompletionOccurrence,
        raw: &[u8],
        fee: PmFixtureFeeEvidence,
    ) -> Result<PmFixturePrivateDelivery, PmPrivateMonitorError> {
        Ok(self.private.receive_user_fixture(occurrence, raw, fee)?)
    }

    pub(crate) fn complete_product_account_fixture(
        &mut self,
        input: PmAccountFixtureInput<'_>,
    ) -> Result<PmCompleteAccountSnapshotDelivery, PmPrivateMonitorError> {
        let query = input.occurrence;
        self.validate_private_epoch(query.connection_epoch)?;
        let request = self
            .account
            .request_snapshot(query.connection_epoch, query.request_sequence)?;
        Ok(request.complete(
            query.completion(),
            query.snapshot,
            self.account.account_scope(),
            input.balances,
            input.allowances,
            input.positions,
        )?)
    }

    pub(crate) fn complete_product_open_orders_fixture(
        &mut self,
        input: PmOpenOrdersFixtureInput<'_>,
    ) -> Result<PmCompleteOpenOrdersDelivery, PmPrivateMonitorError> {
        let query = input.occurrence;
        self.validate_private_epoch(query.connection_epoch)?;
        let request = self
            .reconciliation
            .request_open_orders(query.connection_epoch, query.request_sequence)?;
        Ok(request.complete_json_objects(query.completion(), query.snapshot, input.raw_orders)?)
    }

    pub(crate) fn complete_product_order_detail_fixture(
        &mut self,
        input: PmOrderDetailFixtureInput<'_>,
    ) -> Result<PmExactOrderDetailDelivery, PmPrivateMonitorError> {
        let query = input.occurrence;
        self.validate_private_epoch(query.connection_epoch)?;
        let request = self.reconciliation.request_order_detail(
            query.connection_epoch,
            query.request_sequence,
            input.requested_order,
        )?;
        Ok(request.complete_json_object(query.completion(), query.snapshot, input.raw_order)?)
    }

    pub(crate) fn complete_product_reconciliation_fixture(
        &mut self,
        input: PmReconciliationFixtureInput<'_>,
    ) -> Result<PmFixturePairedReconciliationDelivery, PmPrivateMonitorError> {
        let query = input.occurrence;
        self.validate_private_epoch(query.connection_epoch)?;
        let account_request = self
            .account
            .request_snapshot(query.connection_epoch, query.request_sequence)?;
        let fill_request = self.reconciliation.request_fills(
            query.connection_epoch,
            query.request_sequence,
            input.requested_after,
        )?;
        let account = account_request.complete(
            query.completion(),
            query.snapshot,
            self.account.account_scope(),
            input.balances,
            input.allowances,
            input.positions,
        )?;
        let fills = fill_request.complete_user_frames(
            query.completion(),
            query.snapshot,
            input.resulting_watermark,
            input.raw_fill_frames,
            input.fee,
        )?;
        PmFixturePairedReconciliationDelivery::new(account, fills)
    }

    pub(crate) fn open_product_private_fixture(
        &self,
        delivery: PmFixturePrivateDelivery,
        monotonic_service_ns: u64,
    ) -> Result<EventEnvelope<PmFixturePrivateBatch>, PmPrivateMonitorError> {
        let serviced = delivery.service_at(monotonic_service_ns)?;
        let expected_account = self.private.account_scope();
        let expected_instrument = self.private.instrument_scope();
        let mut opened = None;
        match self
            .private
            .reduce_private_delivery(serviced, |scope, envelope| {
                validate_scope(scope, expected_account, expected_instrument)?;
                opened = Some(envelope);
                Ok::<(), PmPrivateMonitorError>(())
            }) {
            Ok(result) => result?,
            Err(_) => return Err(PmPrivateMonitorError::PrivateDeliveryOwnerMismatch),
        }
        Ok(opened.expect("owner-opened private fixture delivery"))
    }

    pub(crate) fn open_product_account_fixture(
        &self,
        delivery: PmCompleteAccountSnapshotDelivery,
        monotonic_service_ns: u64,
    ) -> Result<EventEnvelope<PmCompleteAccountSnapshot>, PmPrivateMonitorError> {
        let serviced = delivery.service_at(monotonic_service_ns)?;
        let expected_account = self.account.account_scope();
        let expected_instrument = self.account.instrument_scope();
        let mut opened = None;
        match self
            .account
            .reduce_snapshot_delivery(serviced, |scope, envelope| {
                validate_scope(scope, expected_account, expected_instrument)?;
                opened = Some(envelope);
                Ok::<(), PmPrivateMonitorError>(())
            }) {
            Ok(result) => result?,
            Err(_) => return Err(PmPrivateMonitorError::AccountDeliveryOwnerMismatch),
        }
        Ok(opened.expect("owner-opened account fixture delivery"))
    }

    pub(crate) fn open_product_open_orders_fixture(
        &self,
        delivery: PmCompleteOpenOrdersDelivery,
        monotonic_service_ns: u64,
    ) -> Result<EventEnvelope<PmCompleteOpenOrdersSnapshot>, PmPrivateMonitorError> {
        let serviced = delivery.service_at(monotonic_service_ns)?;
        let expected_account = self.reconciliation.account_scope();
        let expected_instrument = self.reconciliation.instrument_scope();
        let mut opened = None;
        match self
            .reconciliation
            .reduce_open_orders_delivery(serviced, |scope, envelope| {
                validate_scope(scope, expected_account, expected_instrument)?;
                opened = Some(envelope);
                Ok::<(), PmPrivateMonitorError>(())
            }) {
            Ok(result) => result?,
            Err(_) => {
                return Err(PmPrivateMonitorError::ReconciliationDeliveryOwnerMismatch);
            }
        }
        Ok(opened.expect("owner-opened open-orders fixture delivery"))
    }

    pub(crate) fn open_product_order_detail_fixture(
        &self,
        delivery: PmExactOrderDetailDelivery,
        monotonic_service_ns: u64,
    ) -> Result<EventEnvelope<PmExactOrderDetail>, PmPrivateMonitorError> {
        let serviced = delivery.service_at(monotonic_service_ns)?;
        let expected_account = self.reconciliation.account_scope();
        let expected_instrument = self.reconciliation.instrument_scope();
        let mut opened = None;
        match self
            .reconciliation
            .reduce_order_detail_delivery(serviced, |scope, envelope| {
                validate_scope(scope, expected_account, expected_instrument)?;
                opened = Some(envelope);
                Ok::<(), PmPrivateMonitorError>(())
            }) {
            Ok(result) => result?,
            Err(_) => {
                return Err(PmPrivateMonitorError::ReconciliationDeliveryOwnerMismatch);
            }
        }
        Ok(opened.expect("owner-opened order-detail fixture delivery"))
    }

    pub(crate) fn open_product_reconciliation_fixture(
        &self,
        delivery: PmFixturePairedReconciliationDelivery,
        monotonic_service_ns: u64,
    ) -> Result<
        (
            EventEnvelope<PmCompleteAccountSnapshot>,
            EventEnvelope<PmCompleteFillQuery>,
        ),
        PmPrivateMonitorError,
    > {
        let PmFixturePairedReconciliationDelivery { account, fills } = delivery;
        let account = self.open_product_account_fixture(account, monotonic_service_ns)?;
        let serviced = fills.service_at(monotonic_service_ns)?;
        let expected_account = self.reconciliation.account_scope();
        let expected_instrument = self.reconciliation.instrument_scope();
        let mut opened = None;
        match self
            .reconciliation
            .reduce_fill_query_delivery(serviced, |scope, envelope| {
                validate_scope(scope, expected_account, expected_instrument)?;
                opened = Some(envelope);
                Ok::<(), PmPrivateMonitorError>(())
            }) {
            Ok(result) => result?,
            Err(_) => {
                return Err(PmPrivateMonitorError::ReconciliationDeliveryOwnerMismatch);
            }
        }
        Ok((
            account,
            opened.expect("owner-opened fill-query fixture delivery"),
        ))
    }
}
