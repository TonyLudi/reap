use std::collections::BTreeMap;

use reap_pm_core::{
    MAX_PM_RECONCILIATION_FILLS, MAX_PM_RECONCILIATION_ORDERS, PmFillEvent, PmFillQueryCursor,
    PmOrderEvent,
};
use reap_polymarket_wire::{PmLiveOpenOrderPage, PmLiveTradePage};

use crate::live_diagnostics::ForeignDiagnosticsBuilder;
use crate::live_normalization::{
    LiveNormalizationScope, PmLiveNormalizationError, fill_event_from_leg, normalize_rest_order,
    normalize_trade,
};
use crate::{
    PmCanonicalFullAccountFillSnapshot, PmCompleteFillCutEvidence, PmCompleteFillQueryDelivery,
    PmCompleteOpenOrdersDelivery, PmForeignRowDiagnostics, PmFullAccountFillCutScope,
    PmFullAccountFillSnapshotDigest,
};

const OPEN_ORDER_FOREIGN_DOMAIN: &[u8] = b"reap.pm.live.open-orders-foreign.v1\0";
const FILL_FOREIGN_DOMAIN: &[u8] = b"reap.pm.live.fills-foreign.v1\0";

/// Owner-bound live open-order completion plus explicit foreign-row evidence.
pub struct PmLiveOpenOrdersCompletion {
    pub(crate) delivery: PmCompleteOpenOrdersDelivery,
    pub(crate) foreign_diagnostics: PmForeignRowDiagnostics,
}

pub(crate) struct NormalizedLiveOpenOrders {
    pub(crate) orders: Box<[PmOrderEvent]>,
    pub(crate) foreign_diagnostics: PmForeignRowDiagnostics,
}

pub(crate) fn normalize_live_open_order_pages(
    scope: LiveNormalizationScope,
    pages: &[PmLiveOpenOrderPage],
) -> Result<NormalizedLiveOpenOrders, PmLiveNormalizationError> {
    validate_open_order_pages(pages)?;
    let mut diagnostics =
        ForeignDiagnosticsBuilder::new(OPEN_ORDER_FOREIGN_DOMAIN, MAX_PM_RECONCILIATION_ORDERS);
    let mut seen = BTreeMap::new();
    let mut configured = Vec::new();
    let mut row_count = 0_usize;
    for page in pages {
        row_count = row_count
            .checked_add(page.orders().len())
            .filter(|count| *count <= MAX_PM_RECONCILIATION_ORDERS)
            .ok_or(PmLiveNormalizationError::TooManyRows)?;
        for order in page.orders() {
            let row = normalize_rest_order(scope, order, true)?;
            match seen.get(&row.venue_order) {
                Some(facts) if *facts != row.facts_digest => {
                    return Err(PmLiveNormalizationError::ConflictingOrder);
                }
                Some(_) => continue,
                None => {
                    seen.insert(row.venue_order, row.facts_digest);
                }
            }
            match row.configured {
                Some(event) => configured.push((row.venue_order, event)),
                None => diagnostics.push(row.key_digest, row.facts_digest)?,
            }
        }
    }
    configured.sort_unstable_by_key(|(venue_order, _)| *venue_order);
    Ok(NormalizedLiveOpenOrders {
        orders: configured
            .into_iter()
            .map(|(_, event)| event)
            .collect::<Vec<_>>()
            .into_boxed_slice(),
        foreign_diagnostics: diagnostics.finish()?,
    })
}

pub(crate) struct NormalizedLiveFillCut {
    pub(crate) fills: Box<[PmFillEvent]>,
    pub(crate) resulting_watermark: PmFillQueryCursor,
    pub(crate) foreign_diagnostics: PmForeignRowDiagnostics,
    pub(crate) full_account_digest: PmFullAccountFillSnapshotDigest,
}

pub(crate) fn normalize_live_trade_pages(
    scope: LiveNormalizationScope,
    pages: &[PmLiveTradePage],
    requested_after: Option<PmFillQueryCursor>,
    evidence: PmCompleteFillCutEvidence,
) -> Result<NormalizedLiveFillCut, PmLiveNormalizationError> {
    validate_trade_pages(pages)?;
    let mut diagnostics =
        ForeignDiagnosticsBuilder::new(FILL_FOREIGN_DOMAIN, MAX_PM_RECONCILIATION_FILLS);
    let mut seen = BTreeMap::new();
    let mut local_legs = Vec::new();
    let mut row_count = 0_usize;
    for page in pages {
        row_count = row_count
            .checked_add(page.trades().len())
            .filter(|count| *count <= MAX_PM_RECONCILIATION_FILLS)
            .ok_or(PmLiveNormalizationError::TooManyRows)?;
        for trade in page.trades() {
            let normalized = normalize_trade(scope, trade)?;
            if normalized.unresolved.is_some() {
                return Err(PmLiveNormalizationError::UnresolvedCompleteTrade);
            }
            for candidate in normalized.candidates {
                match seen.get(&candidate.key) {
                    Some(facts) if *facts != candidate.facts_digest => {
                        return Err(PmLiveNormalizationError::ConflictingTradeLeg);
                    }
                    Some(_) => continue,
                    None => {
                        if seen.len() == MAX_PM_RECONCILIATION_FILLS {
                            return Err(PmLiveNormalizationError::TooManyRows);
                        }
                        seen.insert(candidate.key, candidate.facts_digest);
                    }
                }
                match candidate.leg {
                    Some(leg) => {
                        if !scope.is_configured(leg.condition(), leg.token()) {
                            diagnostics.push(candidate.key_digest, candidate.facts_digest)?;
                        }
                        if local_legs.len() == MAX_PM_RECONCILIATION_FILLS {
                            return Err(PmLiveNormalizationError::TooManyRows);
                        }
                        local_legs.push(leg);
                    }
                    None => diagnostics.push(candidate.key_digest, candidate.facts_digest)?,
                }
            }
        }
    }

    let metadata = scope.instrument.metadata();
    let full_account = PmCanonicalFullAccountFillSnapshot::new(
        PmFullAccountFillCutScope::new(
            scope.account,
            metadata.condition(),
            metadata.market(),
            metadata.outcome().token(),
        ),
        local_legs.into_boxed_slice(),
    )?;
    let resulting_watermark = full_account.derive_cursor(requested_after, evidence)?;
    let fills = full_account
        .legs()
        .iter()
        .copied()
        .filter(|leg| scope.is_configured(leg.condition(), leg.token()))
        .map(|leg| fill_event_from_leg(scope, leg))
        .collect::<Result<Vec<_>, _>>()?
        .into_boxed_slice();
    Ok(NormalizedLiveFillCut {
        fills,
        resulting_watermark,
        foreign_diagnostics: diagnostics.finish()?,
        full_account_digest: full_account.digest(),
    })
}

fn validate_open_order_pages(
    pages: &[PmLiveOpenOrderPage],
) -> Result<(), PmLiveNormalizationError> {
    if pages.is_empty() || !pages.last().is_some_and(PmLiveOpenOrderPage::terminal) {
        return Err(PmLiveNormalizationError::IncompleteCut);
    }
    if pages[..pages.len() - 1]
        .iter()
        .any(PmLiveOpenOrderPage::terminal)
    {
        return Err(PmLiveNormalizationError::PageAfterTerminal);
    }
    Ok(())
}

fn validate_trade_pages(pages: &[PmLiveTradePage]) -> Result<(), PmLiveNormalizationError> {
    if pages.is_empty() || !pages.last().is_some_and(PmLiveTradePage::terminal) {
        return Err(PmLiveNormalizationError::IncompleteCut);
    }
    if pages[..pages.len() - 1]
        .iter()
        .any(PmLiveTradePage::terminal)
    {
        return Err(PmLiveNormalizationError::PageAfterTerminal);
    }
    Ok(())
}

impl PmLiveOpenOrdersCompletion {
    #[must_use]
    pub const fn foreign_diagnostics(&self) -> PmForeignRowDiagnostics {
        self.foreign_diagnostics
    }

    #[must_use]
    pub fn into_delivery(self) -> PmCompleteOpenOrdersDelivery {
        self.delivery
    }

    #[must_use]
    pub fn into_parts(self) -> (PmCompleteOpenOrdersDelivery, PmForeignRowDiagnostics) {
        (self.delivery, self.foreign_diagnostics)
    }
}

/// Owner-bound live complete-fill delivery plus full-account diagnostic proof.
pub struct PmLiveFillQueryCompletion {
    pub(crate) delivery: PmCompleteFillQueryDelivery,
    pub(crate) foreign_diagnostics: PmForeignRowDiagnostics,
    pub(crate) full_account_digest: PmFullAccountFillSnapshotDigest,
}

impl PmLiveFillQueryCompletion {
    #[must_use]
    pub const fn foreign_diagnostics(&self) -> PmForeignRowDiagnostics {
        self.foreign_diagnostics
    }

    #[must_use]
    pub const fn full_account_digest(&self) -> PmFullAccountFillSnapshotDigest {
        self.full_account_digest
    }

    #[must_use]
    pub fn into_delivery(self) -> PmCompleteFillQueryDelivery {
        self.delivery
    }

    #[must_use]
    pub fn into_parts(
        self,
    ) -> (
        PmCompleteFillQueryDelivery,
        PmForeignRowDiagnostics,
        PmFullAccountFillSnapshotDigest,
    ) {
        (
            self.delivery,
            self.foreign_diagnostics,
            self.full_account_digest,
        )
    }
}
