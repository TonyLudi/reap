use std::fmt;

use reap_pm_core::{EvmAddress, MAX_PM_RECONCILIATION_FILLS, MAX_PM_RECONCILIATION_ORDERS};
use reap_polymarket_auth::{FixedOrderId, L2Timestamp};
use reap_polymarket_wire::{
    PmLiveOpenOrderPage, PmLiveOrder, PmLiveTradePage, PmWireScope, parse_live_open_order_page,
    parse_live_order_detail, parse_live_trade_page,
};
use zeroize::Zeroizing;

use crate::{
    PmLiveAdapterError, PmReadServerTime,
    private_credentials::PmHttpCredentialRole,
    private_http::{
        PmPrivateHttpObservation, PmPrivateHttpTransport, PmPrivateRoute, first_page_cursor,
    },
};

/// A deliberate fail-closed ceiling for one account-wide paginated cut.
pub const MAX_PM_AUTHENTICATED_CUT_PAGES: usize = 1_024;
pub const MAX_PM_AUTHENTICATED_ORDER_ROWS: usize = MAX_PM_RECONCILIATION_ORDERS;
pub const MAX_PM_AUTHENTICATED_TRADE_ROWS: usize = MAX_PM_RECONCILIATION_FILLS;

pub enum PmOpenOrdersCutProgress {
    Incomplete(PmOpenOrdersAssembly),
    Complete(PmCompleteOpenOrdersCut),
}

impl fmt::Debug for PmOpenOrdersCutProgress {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Incomplete(value) => value.fmt(formatter),
            Self::Complete(value) => value.fmt(formatter),
        }
    }
}

/// Move-only partial open-order evidence. It deliberately exposes counts, not
/// rows, so a nonterminal cut cannot be mistaken for complete reconciliation.
pub struct PmOpenOrdersAssembly {
    pages: Vec<PmLiveOpenOrderPage>,
    requested_cursors: Vec<String>,
    next_cursor: String,
    row_count: usize,
}

impl PmOpenOrdersAssembly {
    #[must_use]
    pub const fn pages_received(&self) -> usize {
        self.pages.len()
    }

    #[must_use]
    pub const fn rows_received(&self) -> usize {
        self.row_count
    }
}

impl fmt::Debug for PmOpenOrdersAssembly {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PmOpenOrdersAssembly(Incomplete)")
            .field("pages_received", &self.pages.len())
            .field("rows_received", &self.row_count)
            .finish()
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct PmCompleteOpenOrdersCut {
    pages: Box<[PmLiveOpenOrderPage]>,
    row_count: usize,
}

impl PmCompleteOpenOrdersCut {
    #[must_use]
    pub fn pages(&self) -> &[PmLiveOpenOrderPage] {
        &self.pages
    }

    #[must_use]
    pub const fn row_count(&self) -> usize {
        self.row_count
    }

    /// Construct complete typed evidence for downstream contract tests.
    /// Production transport authority never enables this feature.
    #[cfg(feature = "test-support")]
    pub fn test_support_from_pages(pages: Box<[PmLiveOpenOrderPage]>) -> Option<Self> {
        complete_open_orders_for_test(pages)
    }
}

pub enum PmTradesCutProgress {
    Incomplete(PmTradesAssembly),
    Complete(PmCompleteTradesCut),
}

impl fmt::Debug for PmTradesCutProgress {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Incomplete(value) => value.fmt(formatter),
            Self::Complete(value) => value.fmt(formatter),
        }
    }
}

/// Move-only partial trade evidence. Rows remain sealed until the exact
/// terminal cursor is observed.
pub struct PmTradesAssembly {
    pages: Vec<PmLiveTradePage>,
    requested_cursors: Vec<String>,
    next_cursor: String,
    row_count: usize,
}

impl PmTradesAssembly {
    #[must_use]
    pub const fn pages_received(&self) -> usize {
        self.pages.len()
    }

    #[must_use]
    pub const fn rows_received(&self) -> usize {
        self.row_count
    }
}

impl fmt::Debug for PmTradesAssembly {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PmTradesAssembly(Incomplete)")
            .field("pages_received", &self.pages.len())
            .field("rows_received", &self.row_count)
            .finish()
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct PmCompleteTradesCut {
    pages: Box<[PmLiveTradePage]>,
    row_count: usize,
}

impl PmCompleteTradesCut {
    #[must_use]
    pub fn pages(&self) -> &[PmLiveTradePage] {
        &self.pages
    }

    #[must_use]
    pub const fn row_count(&self) -> usize {
        self.row_count
    }

    /// Construct complete typed evidence for downstream contract tests.
    /// Production transport authority never enables this feature.
    #[cfg(feature = "test-support")]
    pub fn test_support_from_pages(pages: Box<[PmLiveTradePage]>) -> Option<Self> {
        complete_trades_for_test(pages)
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum PmExactOrderObservation {
    Present(Box<PmLiveOrder>),
    Absent,
}

/// Borrowed authenticated capability for complete account cuts and one strict
/// journal-known exact-order lookup.
pub struct PmReconciliationHttpRole<'a> {
    authority: &'a mut PmHttpCredentialRole,
    transport: &'a PmPrivateHttpTransport,
    exact_order_scope: PmWireScope,
    expected_order_maker: EvmAddress,
}

impl<'a> PmReconciliationHttpRole<'a> {
    pub(crate) const fn new(
        authority: &'a mut PmHttpCredentialRole,
        transport: &'a PmPrivateHttpTransport,
        exact_order_scope: PmWireScope,
        expected_order_maker: EvmAddress,
    ) -> Self {
        Self {
            authority,
            transport,
            exact_order_scope,
            expected_order_maker,
        }
    }

    /// Start an unfiltered account-wide cut at exact cursor `MA==`.
    pub async fn begin_open_orders(
        &mut self,
        server_time: PmReadServerTime,
    ) -> Result<PmOpenOrdersCutProgress, PmLiveAdapterError> {
        let page = self
            .open_orders_page(
                server_time
                    .into_l2_timestamp()
                    .map_err(|_| PmLiveAdapterError::ProductClock)?,
                first_page_cursor(),
            )
            .await?;
        advance_open_orders(vec![page], vec![first_page_cursor().to_owned()], 0)
    }

    /// Consume the only authority to continue this open-order cut.
    pub async fn continue_open_orders(
        &mut self,
        server_time: PmReadServerTime,
        mut assembly: PmOpenOrdersAssembly,
    ) -> Result<PmOpenOrdersCutProgress, PmLiveAdapterError> {
        if assembly.pages.len() >= MAX_PM_AUTHENTICATED_CUT_PAGES {
            return Err(PmLiveAdapterError::PaginationPageLimit);
        }
        let cursor = std::mem::take(&mut assembly.next_cursor);
        assembly.requested_cursors.push(cursor.clone());
        let page = self
            .open_orders_page(
                server_time
                    .into_l2_timestamp()
                    .map_err(|_| PmLiveAdapterError::ProductClock)?,
                &cursor,
            )
            .await?;
        assembly.pages.push(page);
        advance_open_orders(
            assembly.pages,
            assembly.requested_cursors,
            assembly.row_count,
        )
    }

    /// Start an unfiltered full-account trade cut.
    pub async fn begin_trades(
        &mut self,
        server_time: PmReadServerTime,
    ) -> Result<PmTradesCutProgress, PmLiveAdapterError> {
        let page = self
            .trades_page(
                server_time
                    .into_l2_timestamp()
                    .map_err(|_| PmLiveAdapterError::ProductClock)?,
                first_page_cursor(),
            )
            .await?;
        advance_trades(vec![page], vec![first_page_cursor().to_owned()], 0)
    }

    /// Consume the only authority to continue this trade cut.
    pub async fn continue_trades(
        &mut self,
        server_time: PmReadServerTime,
        mut assembly: PmTradesAssembly,
    ) -> Result<PmTradesCutProgress, PmLiveAdapterError> {
        if assembly.pages.len() >= MAX_PM_AUTHENTICATED_CUT_PAGES {
            return Err(PmLiveAdapterError::PaginationPageLimit);
        }
        let cursor = std::mem::take(&mut assembly.next_cursor);
        assembly.requested_cursors.push(cursor.clone());
        let page = self
            .trades_page(
                server_time
                    .into_l2_timestamp()
                    .map_err(|_| PmLiveAdapterError::ProductClock)?,
                &cursor,
            )
            .await?;
        assembly.pages.push(page);
        advance_trades(
            assembly.pages,
            assembly.requested_cursors,
            assembly.row_count,
        )
    }

    pub async fn exact_local_order_detail(
        &mut self,
        server_time: PmReadServerTime,
        order_id: FixedOrderId,
    ) -> Result<PmExactOrderObservation, PmLiveAdapterError> {
        let headers = self
            .authority
            .authenticate_exact_order(
                server_time
                    .into_l2_timestamp()
                    .map_err(|_| PmLiveAdapterError::ProductClock)?,
                order_id,
            )
            .await?;
        match self
            .transport
            .get(PmPrivateRoute::ExactOrder(order_id), headers)
            .await?
        {
            PmPrivateHttpObservation::NotFound => Ok(PmExactOrderObservation::Absent),
            PmPrivateHttpObservation::Found(body) => {
                let order = self
                    .authority
                    .bind_exact_order(parse_live_order_detail(&body)?)
                    .await?;
                if order.id().as_str() != order_id.to_string() {
                    return Err(PmLiveAdapterError::ExactOrderIdentityMismatch);
                }
                if order.maker() != self.expected_order_maker {
                    return Err(PmLiveAdapterError::ExactOrderMakerMismatch);
                }
                if order.condition() != self.exact_order_scope.condition()
                    || order.token() != self.exact_order_scope.token()
                {
                    return Err(PmLiveAdapterError::ExactOrderScopeMismatch);
                }
                Ok(PmExactOrderObservation::Present(Box::new(order)))
            }
        }
    }

    async fn open_orders_page(
        &mut self,
        timestamp: L2Timestamp,
        cursor: &str,
    ) -> Result<PmLiveOpenOrderPage, PmLiveAdapterError> {
        let headers = self.authority.authenticate_open_orders(timestamp).await?;
        let body = found(
            self.transport
                .get(PmPrivateRoute::OpenOrders { cursor }, headers)
                .await?,
        )?;
        self.authority
            .bind_open_orders(parse_live_open_order_page(&body)?)
            .await
    }

    async fn trades_page(
        &mut self,
        timestamp: L2Timestamp,
        cursor: &str,
    ) -> Result<PmLiveTradePage, PmLiveAdapterError> {
        let headers = self.authority.authenticate_trades(timestamp).await?;
        let body = found(
            self.transport
                .get(PmPrivateRoute::Trades { cursor }, headers)
                .await?,
        )?;
        self.authority
            .bind_trades(parse_live_trade_page(&body)?)
            .await
    }
}

fn advance_open_orders(
    pages: Vec<PmLiveOpenOrderPage>,
    requested_cursors: Vec<String>,
    prior_rows: usize,
) -> Result<PmOpenOrdersCutProgress, PmLiveAdapterError> {
    let page = pages.last().expect("advance always has a page");
    let row_count = checked_rows(
        prior_rows,
        page.orders().len(),
        MAX_PM_AUTHENTICATED_ORDER_ROWS,
    )?;
    let next_cursor = page.next_cursor().map(|cursor| cursor.as_str().to_owned());
    match next_cursor {
        None => Ok(PmOpenOrdersCutProgress::Complete(PmCompleteOpenOrdersCut {
            pages: pages.into_boxed_slice(),
            row_count,
        })),
        Some(next_cursor) => {
            enforce_continuation(&pages, &requested_cursors, &next_cursor)?;
            Ok(PmOpenOrdersCutProgress::Incomplete(PmOpenOrdersAssembly {
                pages,
                requested_cursors,
                next_cursor,
                row_count,
            }))
        }
    }
}

fn advance_trades(
    pages: Vec<PmLiveTradePage>,
    requested_cursors: Vec<String>,
    prior_rows: usize,
) -> Result<PmTradesCutProgress, PmLiveAdapterError> {
    let page = pages.last().expect("advance always has a page");
    let row_count = checked_rows(
        prior_rows,
        page.trades().len(),
        MAX_PM_AUTHENTICATED_TRADE_ROWS,
    )?;
    let next_cursor = page.next_cursor().map(|cursor| cursor.as_str().to_owned());
    match next_cursor {
        None => Ok(PmTradesCutProgress::Complete(PmCompleteTradesCut {
            pages: pages.into_boxed_slice(),
            row_count,
        })),
        Some(next_cursor) => {
            enforce_continuation(&pages, &requested_cursors, &next_cursor)?;
            Ok(PmTradesCutProgress::Incomplete(PmTradesAssembly {
                pages,
                requested_cursors,
                next_cursor,
                row_count,
            }))
        }
    }
}

fn enforce_continuation<T>(
    pages: &[T],
    requested_cursors: &[String],
    next_cursor: &str,
) -> Result<(), PmLiveAdapterError> {
    if pages.len() >= MAX_PM_AUTHENTICATED_CUT_PAGES {
        return Err(PmLiveAdapterError::PaginationPageLimit);
    }
    if requested_cursors.iter().any(|cursor| cursor == next_cursor) {
        return Err(PmLiveAdapterError::PaginationCursorCycle);
    }
    Ok(())
}

fn checked_rows(
    prior: usize,
    additional: usize,
    maximum: usize,
) -> Result<usize, PmLiveAdapterError> {
    prior
        .checked_add(additional)
        .filter(|total| *total <= maximum)
        .ok_or(PmLiveAdapterError::PaginationRowLimit)
}

#[cfg(feature = "test-support")]
fn complete_open_orders_for_test(
    pages: Box<[PmLiveOpenOrderPage]>,
) -> Option<PmCompleteOpenOrdersCut> {
    if pages.is_empty()
        || pages.len() > MAX_PM_AUTHENTICATED_CUT_PAGES
        || !pages.last().is_some_and(PmLiveOpenOrderPage::terminal)
        || pages[..pages.len() - 1]
            .iter()
            .any(PmLiveOpenOrderPage::terminal)
    {
        return None;
    }
    let row_count = pages.iter().try_fold(0_usize, |total, page| {
        checked_rows(total, page.orders().len(), MAX_PM_AUTHENTICATED_ORDER_ROWS).ok()
    })?;
    Some(PmCompleteOpenOrdersCut { pages, row_count })
}

#[cfg(feature = "test-support")]
fn complete_trades_for_test(pages: Box<[PmLiveTradePage]>) -> Option<PmCompleteTradesCut> {
    if pages.is_empty()
        || pages.len() > MAX_PM_AUTHENTICATED_CUT_PAGES
        || !pages.last().is_some_and(PmLiveTradePage::terminal)
        || pages[..pages.len() - 1]
            .iter()
            .any(PmLiveTradePage::terminal)
    {
        return None;
    }
    let row_count = pages.iter().try_fold(0_usize, |total, page| {
        checked_rows(total, page.trades().len(), MAX_PM_AUTHENTICATED_TRADE_ROWS).ok()
    })?;
    Some(PmCompleteTradesCut { pages, row_count })
}

fn found(observation: PmPrivateHttpObservation) -> Result<Zeroizing<Vec<u8>>, PmLiveAdapterError> {
    match observation {
        PmPrivateHttpObservation::Found(body) => Ok(body),
        PmPrivateHttpObservation::NotFound => {
            Err(PmLiveAdapterError::UnexpectedStatus { status: 404 })
        }
    }
}

#[cfg(test)]
mod tests {
    use reap_polymarket_wire::{parse_live_open_order_page, parse_live_trade_page};

    use super::*;

    fn open_page(cursor: &str) -> PmLiveOpenOrderPage {
        parse_live_open_order_page(
            format!(r#"{{"data":[],"next_cursor":"{cursor}","limit":128,"count":0}}"#).as_bytes(),
        )
        .unwrap()
    }

    fn trade_page(cursor: &str) -> PmLiveTradePage {
        parse_live_trade_page(
            format!(r#"{{"data":[],"next_cursor":"{cursor}","limit":128,"count":0}}"#).as_bytes(),
        )
        .unwrap()
    }

    #[test]
    fn cursor_cycles_and_page_caps_cannot_produce_incomplete_authority() {
        assert!(matches!(
            advance_open_orders(vec![open_page("MA==")], vec!["MA==".into()], 0),
            Err(PmLiveAdapterError::PaginationCursorCycle)
        ));
        assert!(matches!(
            advance_trades(
                vec![trade_page("cursor-1")],
                vec!["MA==".into(), "cursor-1".into()],
                0
            ),
            Err(PmLiveAdapterError::PaginationCursorCycle)
        ));

        let pages = (0..MAX_PM_AUTHENTICATED_CUT_PAGES)
            .map(|_| open_page("next"))
            .collect();
        assert!(matches!(
            advance_open_orders(pages, vec!["MA==".into()], 0),
            Err(PmLiveAdapterError::PaginationPageLimit)
        ));
    }

    #[test]
    fn only_exact_terminal_cursor_constructs_a_complete_cut() {
        let progress = advance_open_orders(vec![open_page("LTE=")], vec!["MA==".into()], 0)
            .expect("terminal cut");
        let PmOpenOrdersCutProgress::Complete(complete) = progress else {
            panic!("terminal cursor must complete")
        };
        assert_eq!(complete.pages().len(), 1);
        assert_eq!(complete.row_count(), 0);

        let progress = advance_trades(vec![trade_page("next")], vec!["MA==".into()], 0)
            .expect("nonterminal cut");
        let PmTradesCutProgress::Incomplete(incomplete) = progress else {
            panic!("nonterminal cursor must remain incomplete")
        };
        assert_eq!(incomplete.pages_received(), 1);
        assert_eq!(incomplete.rows_received(), 0);

        assert_eq!(
            checked_rows(
                MAX_PM_AUTHENTICATED_ORDER_ROWS,
                1,
                MAX_PM_AUTHENTICATED_ORDER_ROWS
            ),
            Err(PmLiveAdapterError::PaginationRowLimit)
        );
        assert_eq!(
            checked_rows(
                MAX_PM_AUTHENTICATED_TRADE_ROWS,
                1,
                MAX_PM_AUTHENTICATED_TRADE_ROWS
            ),
            Err(PmLiveAdapterError::PaginationRowLimit)
        );
    }
}
