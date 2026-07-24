use reap_pm_core::{
    PmAccountScope, PmClientOrderKey, PmInstrumentHandle, PmOrderSide, PmVenueOrderKey,
};
use sha2::{Digest, Sha256};

use crate::coordinator::{
    PmCancelIntentReason, PmControlReason, PmDurableRecordKind, PmFakeEffectStage,
    PmHealthMetricKind, PmProductEffect, PmRefreshEffectKind,
};
use crate::evidence::PmEvidenceError;
use crate::journal::{PmJournalFingerprintV1, derive_pm_journal_client_order_from_fingerprint};

const VENUE_PREFIX: &str = "phase6-venue-";

pub(super) struct EffectProjection {
    pub(super) prepared_quotes: u64,
    pub(super) executed_quotes: u64,
    pub(super) prepared_cancels: u64,
    pub(super) executed_cancels: u64,
    pub(super) filled_orders: u64,
    pub(super) cancelled_orders: u64,
    pub(super) paired_reconciliations: u64,
    logical: Sha256,
    scope_fingerprint: PmJournalFingerprintV1,
    account: reap_pm_core::PmAccountHandle,
    first_absolute_cycle: u64,
    first_intent_id: u64,
    quote_ordinal: u64,
    current_client: Option<PmClientOrderKey>,
    correlation_bases: [Option<u64>; 8],
    fact_ack_sequence_base: Option<u64>,
    valid: bool,
}

impl EffectProjection {
    pub(super) fn new() -> Self {
        Self::for_pass(
            PmJournalFingerprintV1::from_bytes([0; 32]),
            reap_pm_core::PmAccountHandle::from_ordinal(0),
            1,
            1,
        )
    }

    pub(super) fn for_pass(
        scope_fingerprint: PmJournalFingerprintV1,
        account: reap_pm_core::PmAccountHandle,
        first_absolute_cycle: u64,
        first_intent_id: u64,
    ) -> Self {
        Self {
            prepared_quotes: 0,
            executed_quotes: 0,
            prepared_cancels: 0,
            executed_cancels: 0,
            filled_orders: 0,
            cancelled_orders: 0,
            paired_reconciliations: 0,
            logical: Sha256::new(),
            scope_fingerprint,
            account,
            first_absolute_cycle,
            first_intent_id,
            quote_ordinal: 0,
            current_client: None,
            correlation_bases: [None; 8],
            fact_ack_sequence_base: None,
            valid: true,
        }
    }

    pub(super) fn observe(&mut self, effect: PmProductEffect) {
        match effect {
            PmProductEffect::FakePassiveQuote(effect) => {
                self.logical.update([0]);
                self.hash_scope(effect.account_scope());
                self.hash_instrument(effect.instrument());
                self.hash_current_client(Some(effect.client_order()));
                self.logical.update([side_tag(effect.side())]);
                self.logical.update(effect.price().units().to_be_bytes());
                self.logical
                    .update(effect.quantity().protocol_units().to_be_bytes());
                self.logical.update([stage_tag(effect.stage())]);
                match effect.stage() {
                    PmFakeEffectStage::PreparedAfterDurability => {
                        self.prepared_quotes = self.prepared_quotes.saturating_add(1);
                    }
                    PmFakeEffectStage::ExecutedByFixture => {
                        self.executed_quotes = self.executed_quotes.saturating_add(1);
                    }
                }
            }
            PmProductEffect::FakeCancelOwned(effect) => {
                self.logical.update([1]);
                self.hash_scope(effect.account_scope());
                self.hash_instrument(effect.instrument());
                self.hash_current_client(Some(effect.client_order()));
                self.hash_venue(effect.venue_order());
                self.logical.update([stage_tag(effect.stage())]);
                match effect.stage() {
                    PmFakeEffectStage::PreparedAfterDurability => {
                        self.prepared_cancels = self.prepared_cancels.saturating_add(1);
                    }
                    PmFakeEffectStage::ExecutedByFixture => {
                        self.executed_cancels = self.executed_cancels.saturating_add(1);
                    }
                }
            }
            PmProductEffect::ReconciliationRefresh(effect) => {
                self.logical.update([2]);
                self.hash_scope(effect.account_scope());
                self.hash_instrument(effect.instrument());
                match effect.kind() {
                    PmRefreshEffectKind::CompleteReconciliation => self.logical.update([0]),
                    PmRefreshEffectKind::OpenOrders => self.logical.update([1]),
                    PmRefreshEffectKind::OrderDetail(venue) => {
                        self.logical.update([2]);
                        self.hash_venue(venue);
                    }
                    PmRefreshEffectKind::Account => self.logical.update([3]),
                }
            }
            PmProductEffect::DurableRecord(effect) => {
                self.logical.update([3]);
                let kind = effect.kind();
                self.logical.update([durable_tag(kind)]);
                if kind == PmDurableRecordKind::QuoteIntent {
                    self.establish_quote(effect.client_order(), effect.correlation());
                } else {
                    self.hash_current_client(effect.client_order());
                }
                self.hash_correlation(kind, effect.correlation());
            }
            PmProductEffect::HealthMetricAudit(effect) => {
                self.logical.update([4, metric_tag(effect.kind())]);
                self.hash_metric_value(effect.kind(), effect.value());
            }
            PmProductEffect::FailClosedHaltOrCancel(effect) => {
                self.logical.update([5]);
                self.hash_scope(effect.account_scope());
                self.hash_instrument(effect.instrument());
                self.logical.update([control_reason_tag(effect.reason())]);
                match effect.cancel_intent() {
                    Some((client, reason)) => {
                        self.logical.update([1]);
                        self.hash_current_client(Some(client));
                        self.logical.update([cancel_reason_tag(reason)]);
                    }
                    None => self.logical.update([0]),
                }
            }
        }
    }

    pub(super) fn finish_hash(&self) -> Result<[u8; 32], PmEvidenceError> {
        if !self.valid {
            return Err(PmEvidenceError::invariant(
                "effect projection contained a mismatched normalized identity or sequence",
            ));
        }
        let digest = self.logical.clone().finalize();
        let mut output = [0; 32];
        output.copy_from_slice(&digest);
        Ok(output)
    }

    fn establish_quote(&mut self, client: Option<PmClientOrderKey>, correlation: u64) {
        self.quote_ordinal = self.quote_ordinal.saturating_add(1);
        let expected_intent = self
            .first_intent_id
            .saturating_add(self.quote_ordinal.saturating_sub(1));
        self.valid &= correlation == expected_intent;
        let expected = derive_pm_journal_client_order_from_fingerprint(
            self.account,
            self.scope_fingerprint,
            expected_intent,
        )
        .ok();
        self.valid &= client == expected;
        self.current_client = expected;
        self.hash_current_client(client);
    }

    fn hash_current_client(&mut self, client: Option<PmClientOrderKey>) {
        match client {
            Some(client) => {
                self.logical.update([1]);
                self.valid &= Some(client) == self.current_client;
                self.valid &= client.account() == self.account;
                self.logical.update(self.quote_ordinal.to_be_bytes());
            }
            None => self.logical.update([0]),
        }
    }

    fn hash_venue(&mut self, venue: PmVenueOrderKey) {
        self.valid &= venue.account() == self.account;
        let venue_id = venue.id();
        let suffix = venue_id.as_str().strip_prefix(VENUE_PREFIX);
        let absolute = suffix.and_then(|value| value.parse::<u64>().ok());
        let expected = self
            .first_absolute_cycle
            .saturating_add(self.quote_ordinal.saturating_sub(1));
        self.valid &= suffix.is_some_and(|value| value.len() == 6);
        self.valid &= absolute == Some(expected);
        self.logical.update(self.quote_ordinal.to_be_bytes());
    }

    fn hash_scope(&mut self, scope: PmAccountScope) {
        let environment = scope.environment();
        self.logical
            .update([u8::try_from(environment.as_str().len()).unwrap_or(u8::MAX)]);
        self.logical.update(environment.as_str().as_bytes());
        self.logical.update(scope.chain().value().to_be_bytes());
        self.logical.update(scope.signer().address().bytes());
        self.logical.update(scope.funder().address().bytes());
        self.logical.update(scope.handle().ordinal().to_be_bytes());
    }

    fn hash_instrument(&mut self, instrument: PmInstrumentHandle) {
        self.logical
            .update(instrument.market().ordinal().to_be_bytes());
        self.logical
            .update(instrument.token().ordinal().to_be_bytes());
    }

    fn hash_correlation(&mut self, kind: PmDurableRecordKind, correlation: u64) {
        let index = usize::from(durable_tag(kind));
        let base = self.correlation_bases[index].get_or_insert(correlation);
        self.valid &= correlation >= *base;
        self.logical.update(
            correlation
                .saturating_sub(*base)
                .saturating_add(1)
                .to_be_bytes(),
        );
    }

    fn hash_metric_value(&mut self, kind: PmHealthMetricKind, value: u64) {
        if kind == PmHealthMetricKind::PersistenceAcknowledged && value > 1 {
            self.logical.update([1]);
            let base = self.fact_ack_sequence_base.get_or_insert(value);
            self.valid &= value >= *base;
            self.logical
                .update(value.saturating_sub(*base).saturating_add(1).to_be_bytes());
        } else {
            self.logical.update([0]);
            self.logical.update(value.to_be_bytes());
        }
    }
}

const fn side_tag(side: PmOrderSide) -> u8 {
    match side {
        PmOrderSide::Buy => 0,
        PmOrderSide::Sell => 1,
    }
}

const fn stage_tag(stage: PmFakeEffectStage) -> u8 {
    match stage {
        PmFakeEffectStage::PreparedAfterDurability => 0,
        PmFakeEffectStage::ExecutedByFixture => 1,
    }
}

const fn durable_tag(kind: PmDurableRecordKind) -> u8 {
    match kind {
        PmDurableRecordKind::QuoteIntent => 0,
        PmDurableRecordKind::PlaceResult => 1,
        PmDurableRecordKind::FillApplied => 2,
        PmDurableRecordKind::OrderTerminal => 3,
        PmDurableRecordKind::CancelIntent => 4,
        PmDurableRecordKind::CancelResult => 5,
        PmDurableRecordKind::SafetyHalt => 6,
        PmDurableRecordKind::FillWatermarkAdvanced => 7,
    }
}

const fn metric_tag(kind: PmHealthMetricKind) -> u8 {
    match kind {
        PmHealthMetricKind::InputObserved => 0,
        PmHealthMetricKind::InputIgnoredStale => 1,
        PmHealthMetricKind::QuoteDecision => 2,
        PmHealthMetricKind::QuoteSuppressed => 3,
        PmHealthMetricKind::DuplicateQuote => 4,
        PmHealthMetricKind::PersistencePending => 5,
        PmHealthMetricKind::PersistenceAcknowledged => 6,
        PmHealthMetricKind::FakeEffectExecuted => 7,
        PmHealthMetricKind::RefreshRequested => 8,
    }
}

const fn control_reason_tag(reason: PmControlReason) -> u8 {
    match reason {
        PmControlReason::RequestedShutdown => 0,
        PmControlReason::RecoveredSafetyHalt => 1,
        PmControlReason::PublicUnavailable => 2,
        PmControlReason::PrivateUnavailable => 3,
        PmControlReason::RiskLimit => 4,
        PmControlReason::PersistenceUnavailable => 5,
        PmControlReason::SchedulerOverload => 6,
        PmControlReason::ContractViolation => 7,
    }
}

const fn cancel_reason_tag(reason: PmCancelIntentReason) -> u8 {
    match reason {
        PmCancelIntentReason::Replacement => 0,
        PmCancelIntentReason::StaleReference => 1,
        PmCancelIntentReason::StaleBook => 2,
        PmCancelIntentReason::SafetyHalt => 3,
    }
}
