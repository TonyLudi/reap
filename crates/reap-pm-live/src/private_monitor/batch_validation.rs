use reap_pm_core::{PmClientOrderKey, PmFillKey, PmVenueOrderKey};
use reap_pm_state::PmUnresolvedFillKey;
use reap_polymarket_adapter::{
    MAX_PM_PRIVATE_NORMALIZED_OBSERVATIONS, PmPrivateLifecycleObservation,
};

use super::PmPrivateMonitorError;

pub(super) struct PmPrivateBatchIdentityScratch {
    client_orders: Vec<PmClientOrderKey>,
    venue_orders: Vec<PmVenueOrderKey>,
    fills: Vec<PmFillKey>,
    unresolved: Vec<PmUnresolvedFillKey>,
}

impl PmPrivateBatchIdentityScratch {
    pub(super) fn new() -> Self {
        Self {
            client_orders: Vec::with_capacity(MAX_PM_PRIVATE_NORMALIZED_OBSERVATIONS),
            venue_orders: Vec::with_capacity(MAX_PM_PRIVATE_NORMALIZED_OBSERVATIONS),
            fills: Vec::with_capacity(MAX_PM_PRIVATE_NORMALIZED_OBSERVATIONS),
            unresolved: Vec::with_capacity(MAX_PM_PRIVATE_NORMALIZED_OBSERVATIONS),
        }
    }

    pub(super) fn reserved_capacity_bytes(&self) -> usize {
        self.client_orders
            .capacity()
            .saturating_mul(std::mem::size_of::<PmClientOrderKey>())
            .saturating_add(
                self.venue_orders
                    .capacity()
                    .saturating_mul(std::mem::size_of::<PmVenueOrderKey>()),
            )
            .saturating_add(
                self.fills
                    .capacity()
                    .saturating_mul(std::mem::size_of::<PmFillKey>()),
            )
            .saturating_add(
                self.unresolved
                    .capacity()
                    .saturating_mul(std::mem::size_of::<PmUnresolvedFillKey>()),
            )
    }

    pub(super) fn validate(
        &mut self,
        observations: &[PmPrivateLifecycleObservation],
    ) -> Result<(), PmPrivateMonitorError> {
        if observations.len() > MAX_PM_PRIVATE_NORMALIZED_OBSERVATIONS {
            return Err(PmPrivateMonitorError::BatchCounterOverflow);
        }
        self.client_orders.clear();
        self.venue_orders.clear();
        self.fills.clear();
        self.unresolved.clear();
        for observation in observations {
            match *observation {
                PmPrivateLifecycleObservation::Order(order) => {
                    self.client_orders.extend(order.order().client_order_key());
                    self.venue_orders.extend(order.order().venue_order_key());
                }
                PmPrivateLifecycleObservation::Fill(fill) => {
                    self.fills.push(fill.fill_key());
                }
                PmPrivateLifecycleObservation::UnresolvedTrade(trade) => {
                    self.unresolved.push(PmUnresolvedFillKey::new(
                        trade.fill_id(),
                        trade.order(),
                        trade.candidate_order(),
                    ));
                }
            }
        }
        self.client_orders.sort_unstable();
        self.venue_orders.sort_unstable();
        self.fills.sort_unstable();
        self.unresolved.sort_unstable();
        if has_adjacent_duplicate(&self.client_orders)
            || has_adjacent_duplicate(&self.venue_orders)
            || has_adjacent_duplicate(&self.fills)
            || has_adjacent_duplicate(&self.unresolved)
            || self.unresolved.iter().any(|unresolved| {
                unresolved.exact_order().is_some_and(|order| {
                    self.fills
                        .binary_search(&PmFillKey::new(order, unresolved.fill_id()))
                        .is_ok()
                })
            })
        {
            Err(PmPrivateMonitorError::DuplicateBatchIdentity)
        } else {
            Ok(())
        }
    }
}

fn has_adjacent_duplicate<T: Eq>(values: &[T]) -> bool {
    values.windows(2).any(|pair| pair[0] == pair[1])
}

#[cfg(test)]
pub(crate) struct PmPrivateBatchValidationProbe {
    scratch: PmPrivateBatchIdentityScratch,
    observation: PmPrivateLifecycleObservation,
}

#[cfg(test)]
impl PmPrivateBatchValidationProbe {
    pub(crate) fn new(observation: PmPrivateLifecycleObservation) -> Self {
        Self {
            scratch: PmPrivateBatchIdentityScratch::new(),
            observation,
        }
    }

    pub(crate) fn exercise(&mut self) -> bool {
        self.scratch.validate(&[self.observation]).is_ok()
            && matches!(
                self.scratch.validate(&[self.observation, self.observation]),
                Err(PmPrivateMonitorError::DuplicateBatchIdentity)
            )
    }
}

#[cfg(test)]
#[path = "batch_validation_tests.rs"]
mod tests;
