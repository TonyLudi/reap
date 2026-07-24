//! Copied product-effect projections.
//!
//! Mutation authorities, fake commands, journal receipts, and adapter roles
//! are deliberately absent. The coordinator retains and consumes those exact
//! values internally; callers observe only these bounded projections.

use reap_pm_core::{
    PmAccountScope, PmClientOrderKey, PmInstrumentHandle, PmOrderSide, PmPrice, PmQuantity,
    PmVenueOrderKey,
};

use super::input::PmControlReason;

/// Maximum copied projections produced by one serviced product input.
pub const MAX_PM_EFFECTS_PER_INPUT: usize = 16;
/// Bounded copied-output capacity between complete scheduler turns.
pub const MAX_PM_PRODUCT_EFFECT_OUTPUTS: usize = 4_096;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PmFakeEffectStage {
    PreparedAfterDurability,
    ExecutedByFixture,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PmFakeQuoteEffect {
    account_scope: PmAccountScope,
    instrument: PmInstrumentHandle,
    client_order: PmClientOrderKey,
    side: PmOrderSide,
    price: PmPrice,
    quantity: PmQuantity,
    stage: PmFakeEffectStage,
}

impl PmFakeQuoteEffect {
    pub(super) const fn new(
        account_scope: PmAccountScope,
        instrument: PmInstrumentHandle,
        client_order: PmClientOrderKey,
        side: PmOrderSide,
        price: PmPrice,
        quantity: PmQuantity,
        stage: PmFakeEffectStage,
    ) -> Self {
        Self {
            account_scope,
            instrument,
            client_order,
            side,
            price,
            quantity,
            stage,
        }
    }

    #[must_use]
    pub const fn account_scope(self) -> PmAccountScope {
        self.account_scope
    }

    #[must_use]
    pub const fn instrument(self) -> PmInstrumentHandle {
        self.instrument
    }

    #[must_use]
    pub const fn client_order(self) -> PmClientOrderKey {
        self.client_order
    }

    #[must_use]
    pub const fn side(self) -> PmOrderSide {
        self.side
    }

    #[must_use]
    pub const fn price(self) -> PmPrice {
        self.price
    }

    #[must_use]
    pub const fn quantity(self) -> PmQuantity {
        self.quantity
    }

    #[must_use]
    pub const fn stage(self) -> PmFakeEffectStage {
        self.stage
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PmFakeCancelEffect {
    account_scope: PmAccountScope,
    instrument: PmInstrumentHandle,
    client_order: PmClientOrderKey,
    venue_order: PmVenueOrderKey,
    stage: PmFakeEffectStage,
}

impl PmFakeCancelEffect {
    pub(super) const fn new(
        account_scope: PmAccountScope,
        instrument: PmInstrumentHandle,
        client_order: PmClientOrderKey,
        venue_order: PmVenueOrderKey,
        stage: PmFakeEffectStage,
    ) -> Self {
        Self {
            account_scope,
            instrument,
            client_order,
            venue_order,
            stage,
        }
    }

    #[must_use]
    pub const fn account_scope(self) -> PmAccountScope {
        self.account_scope
    }

    #[must_use]
    pub const fn instrument(self) -> PmInstrumentHandle {
        self.instrument
    }

    #[must_use]
    pub const fn client_order(self) -> PmClientOrderKey {
        self.client_order
    }

    #[must_use]
    pub const fn venue_order(self) -> PmVenueOrderKey {
        self.venue_order
    }

    #[must_use]
    pub const fn stage(self) -> PmFakeEffectStage {
        self.stage
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PmRefreshEffectKind {
    CompleteReconciliation,
    OpenOrders,
    OrderDetail(PmVenueOrderKey),
    Account,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PmRefreshEffect {
    account_scope: PmAccountScope,
    instrument: PmInstrumentHandle,
    kind: PmRefreshEffectKind,
}

impl PmRefreshEffect {
    pub(super) const fn new(
        account_scope: PmAccountScope,
        instrument: PmInstrumentHandle,
        kind: PmRefreshEffectKind,
    ) -> Self {
        Self {
            account_scope,
            instrument,
            kind,
        }
    }

    #[must_use]
    pub const fn account_scope(self) -> PmAccountScope {
        self.account_scope
    }

    #[must_use]
    pub const fn instrument(self) -> PmInstrumentHandle {
        self.instrument
    }

    #[must_use]
    pub const fn kind(self) -> PmRefreshEffectKind {
        self.kind
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PmDurableRecordKind {
    QuoteIntent,
    PlaceResult,
    FillApplied,
    OrderTerminal,
    CancelIntent,
    CancelResult,
    SafetyHalt,
    FillWatermarkAdvanced,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PmDurableRecordEffect {
    kind: PmDurableRecordKind,
    client_order: Option<PmClientOrderKey>,
    correlation: u64,
}

impl PmDurableRecordEffect {
    pub(super) const fn new(
        kind: PmDurableRecordKind,
        client_order: Option<PmClientOrderKey>,
        correlation: u64,
    ) -> Self {
        Self {
            kind,
            client_order,
            correlation,
        }
    }

    #[must_use]
    pub const fn kind(self) -> PmDurableRecordKind {
        self.kind
    }

    #[must_use]
    pub const fn client_order(self) -> Option<PmClientOrderKey> {
        self.client_order
    }

    /// Stable local correlation, never a durable writer receipt or authority.
    #[must_use]
    pub const fn correlation(self) -> u64 {
        self.correlation
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PmHealthMetricKind {
    InputObserved,
    InputIgnoredStale,
    QuoteDecision,
    QuoteSuppressed,
    DuplicateQuote,
    PersistencePending,
    PersistenceAcknowledged,
    FakeEffectExecuted,
    RefreshRequested,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PmHealthMetricEffect {
    kind: PmHealthMetricKind,
    value: u64,
}

impl PmHealthMetricEffect {
    pub(super) const fn new(kind: PmHealthMetricKind, value: u64) -> Self {
        Self { kind, value }
    }

    #[must_use]
    pub const fn kind(self) -> PmHealthMetricKind {
        self.kind
    }

    #[must_use]
    pub const fn value(self) -> u64 {
        self.value
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PmCancelIntentReason {
    Replacement,
    StaleReference,
    StaleBook,
    SafetyHalt,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PmFailClosedEffect {
    account_scope: PmAccountScope,
    instrument: PmInstrumentHandle,
    reason: PmControlReason,
    cancel: Option<(PmClientOrderKey, PmCancelIntentReason)>,
}

impl PmFailClosedEffect {
    pub(super) const fn halt(
        account_scope: PmAccountScope,
        instrument: PmInstrumentHandle,
        reason: PmControlReason,
    ) -> Self {
        Self {
            account_scope,
            instrument,
            reason,
            cancel: None,
        }
    }

    pub(super) const fn cancel(
        account_scope: PmAccountScope,
        instrument: PmInstrumentHandle,
        reason: PmControlReason,
        client_order: PmClientOrderKey,
        cancel_reason: PmCancelIntentReason,
    ) -> Self {
        Self {
            account_scope,
            instrument,
            reason,
            cancel: Some((client_order, cancel_reason)),
        }
    }

    #[must_use]
    pub const fn account_scope(self) -> PmAccountScope {
        self.account_scope
    }

    #[must_use]
    pub const fn instrument(self) -> PmInstrumentHandle {
        self.instrument
    }

    #[must_use]
    pub const fn reason(self) -> PmControlReason {
        self.reason
    }

    #[must_use]
    pub const fn cancel_intent(self) -> Option<(PmClientOrderKey, PmCancelIntentReason)> {
        self.cancel
    }
}

/// Closed, copied effect union. It cannot be converted into a fake command,
/// journal receipt, prepared quote, or prepared cancel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PmProductEffect {
    FakePassiveQuote(PmFakeQuoteEffect),
    FakeCancelOwned(PmFakeCancelEffect),
    ReconciliationRefresh(PmRefreshEffect),
    DurableRecord(PmDurableRecordEffect),
    HealthMetricAudit(PmHealthMetricEffect),
    FailClosedHaltOrCancel(PmFailClosedEffect),
}

/// Allocation-free effects from one owner reduction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PmProductEffectBatch {
    values: [Option<PmProductEffect>; MAX_PM_EFFECTS_PER_INPUT],
    len: u8,
}

impl PmProductEffectBatch {
    pub(super) const fn new() -> Self {
        Self {
            values: [None; MAX_PM_EFFECTS_PER_INPUT],
            len: 0,
        }
    }

    pub(super) fn push(&mut self, effect: PmProductEffect) -> Result<(), PmEffectCapacityError> {
        let index = usize::from(self.len);
        let Some(slot) = self.values.get_mut(index) else {
            return Err(PmEffectCapacityError);
        };
        *slot = Some(effect);
        self.len = self
            .len
            .checked_add(1)
            .expect("fixed effect capacity fits in u8");
        Ok(())
    }

    #[must_use]
    pub const fn len(&self) -> usize {
        self.len as usize
    }

    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    #[must_use]
    pub fn get(&self, index: usize) -> Option<PmProductEffect> {
        self.values.get(index).copied().flatten()
    }

    pub fn iter(&self) -> impl Iterator<Item = PmProductEffect> + '_ {
        self.values[..self.len()].iter().map(|effect| {
            effect.expect("every slot below the fixed effect-batch length is populated")
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct PmEffectCapacityError;

/// Copied-output queue pressure without exposing an internal queue slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PmProductEffectMetrics {
    capacity: usize,
    depth: usize,
    high_water: usize,
    rejected_full: u64,
}

impl PmProductEffectMetrics {
    #[must_use]
    pub const fn capacity(self) -> usize {
        self.capacity
    }

    #[must_use]
    pub const fn depth(self) -> usize {
        self.depth
    }

    #[must_use]
    pub const fn high_water(self) -> usize {
        self.high_water
    }

    #[must_use]
    pub const fn rejected_full(self) -> u64 {
        self.rejected_full
    }
}

/// Preallocated copied-effect output owned by the coordinator.
///
/// This queue carries observations of completed owner transitions. It never
/// contains prepared authorities or replaces the bounded fake/reconciliation
/// effect lanes.
pub(crate) struct PmProductEffectOutput {
    values: Box<[Option<PmProductEffect>]>,
    head: u16,
    len: u16,
    high_water: u16,
    rejected_full: u64,
}

impl PmProductEffectOutput {
    pub(crate) fn new() -> Self {
        Self {
            values: vec![None; MAX_PM_PRODUCT_EFFECT_OUTPUTS].into_boxed_slice(),
            head: 0,
            len: 0,
            high_water: 0,
            rejected_full: 0,
        }
    }

    pub(crate) fn push_batch(
        &mut self,
        batch: PmProductEffectBatch,
    ) -> Result<(), PmEffectCapacityError> {
        self.ensure_capacity(batch.len())?;
        for effect in batch.iter() {
            let tail =
                (usize::from(self.head) + usize::from(self.len)) % MAX_PM_PRODUCT_EFFECT_OUTPUTS;
            self.values[tail] = Some(effect);
            self.len += 1;
        }
        self.high_water = self.high_water.max(self.len);
        Ok(())
    }

    pub(crate) fn ensure_capacity(
        &mut self,
        additional: usize,
    ) -> Result<(), PmEffectCapacityError> {
        if !self.can_accept(additional) {
            self.rejected_full = self.rejected_full.saturating_add(1);
            Err(PmEffectCapacityError)
        } else {
            Ok(())
        }
    }

    pub(crate) const fn can_accept(&self, additional: usize) -> bool {
        match self.len().checked_add(additional) {
            Some(required) => required <= MAX_PM_PRODUCT_EFFECT_OUTPUTS,
            None => false,
        }
    }

    pub(crate) fn pop(&mut self) -> Option<PmProductEffect> {
        if self.len == 0 {
            return None;
        }
        let index = usize::from(self.head);
        let effect = self.values[index]
            .take()
            .expect("every occupied copied-effect slot is populated");
        self.head = ((index + 1) % MAX_PM_PRODUCT_EFFECT_OUTPUTS) as u16;
        self.len -= 1;
        Some(effect)
    }

    pub(crate) const fn len(&self) -> usize {
        self.len as usize
    }

    pub(crate) const fn high_water(&self) -> usize {
        self.high_water as usize
    }

    pub(crate) const fn rejected_full(&self) -> u64 {
        self.rejected_full
    }

    pub(crate) const fn metrics(&self) -> PmProductEffectMetrics {
        PmProductEffectMetrics {
            capacity: MAX_PM_PRODUCT_EFFECT_OUTPUTS,
            depth: self.len(),
            high_water: self.high_water(),
            rejected_full: self.rejected_full(),
        }
    }

    pub(crate) fn reserved_capacity_bytes(&self) -> usize {
        std::mem::size_of_val(self.values.as_ref())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixed_batch_never_grows_and_preserves_order() {
        let mut batch = PmProductEffectBatch::new();
        for value in 1..=MAX_PM_EFFECTS_PER_INPUT {
            batch
                .push(PmProductEffect::HealthMetricAudit(
                    PmHealthMetricEffect::new(PmHealthMetricKind::InputObserved, value as u64),
                ))
                .expect("within fixed capacity");
        }
        assert_eq!(batch.len(), MAX_PM_EFFECTS_PER_INPUT);
        assert_eq!(
            batch
                .iter()
                .map(|effect| match effect {
                    PmProductEffect::HealthMetricAudit(metric) => metric.value(),
                    _ => unreachable!("test inserts only metrics"),
                })
                .collect::<Vec<_>>(),
            (1_u64..=MAX_PM_EFFECTS_PER_INPUT as u64).collect::<Vec<_>>()
        );
        assert_eq!(
            batch.push(PmProductEffect::HealthMetricAudit(
                PmHealthMetricEffect::new(PmHealthMetricKind::InputObserved, 17)
            )),
            Err(PmEffectCapacityError)
        );
    }

    #[test]
    fn copied_output_storage_is_fixed_on_the_heap() {
        let mut output = PmProductEffectOutput::new();
        let storage = output.values.as_ptr();
        let reserved_capacity_bytes = output.reserved_capacity_bytes();
        let expected_payload_bytes = MAX_PM_PRODUCT_EFFECT_OUTPUTS
            .checked_mul(std::mem::size_of::<Option<PmProductEffect>>())
            .expect("fixed copied-output payload fits usize");

        assert_eq!(output.values.len(), MAX_PM_PRODUCT_EFFECT_OUTPUTS);
        assert_eq!(reserved_capacity_bytes, expected_payload_bytes);
        assert!(
            std::mem::size_of::<PmProductEffectOutput>() <= 32,
            "the copied-output payload must not be carried inline by the coordinator"
        );

        for batch_ordinal in 0..(MAX_PM_PRODUCT_EFFECT_OUTPUTS / MAX_PM_EFFECTS_PER_INPUT) {
            let mut batch = PmProductEffectBatch::new();
            for offset in 0..MAX_PM_EFFECTS_PER_INPUT {
                let value = batch_ordinal
                    .checked_mul(MAX_PM_EFFECTS_PER_INPUT)
                    .and_then(|value| value.checked_add(offset))
                    .expect("fixed output ordinal fits usize");
                batch
                    .push(PmProductEffect::HealthMetricAudit(
                        PmHealthMetricEffect::new(PmHealthMetricKind::InputObserved, value as u64),
                    ))
                    .expect("batch is within its fixed capacity");
            }
            output
                .push_batch(batch)
                .expect("copied-output queue has fixed remaining capacity");
        }

        assert_eq!(output.values.as_ptr(), storage);
        assert_eq!(output.len(), MAX_PM_PRODUCT_EFFECT_OUTPUTS);
        assert_eq!(output.metrics().capacity(), MAX_PM_PRODUCT_EFFECT_OUTPUTS);
        assert_eq!(output.high_water(), MAX_PM_PRODUCT_EFFECT_OUTPUTS);
        assert_eq!(output.ensure_capacity(1), Err(PmEffectCapacityError));
        assert_eq!(output.rejected_full(), 1);
        assert_eq!(output.values.as_ptr(), storage);
        assert_eq!(output.reserved_capacity_bytes(), reserved_capacity_bytes);

        for expected in 0..MAX_PM_PRODUCT_EFFECT_OUTPUTS {
            let PmProductEffect::HealthMetricAudit(metric) =
                output.pop().expect("every fixed output slot was populated")
            else {
                panic!("test inserts only health metrics");
            };
            assert_eq!(metric.value(), expected as u64);
        }
        assert!(output.pop().is_none());
        assert_eq!(output.values.as_ptr(), storage);
        assert_eq!(output.reserved_capacity_bytes(), reserved_capacity_bytes);
    }
}
