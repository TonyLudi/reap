use serde::{Deserialize, Serialize};

use reap_core::{Level, OrderBook, Price, Quantity, Side, Symbol, TimeMs};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BookStatus {
    Empty,
    Recovering,
    Ready,
    Stale,
    Gapped,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct LiquidityFill {
    pub px: Price,
    pub qty: Quantity,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BookReducer {
    symbol: Symbol,
    status: BookStatus,
    book: Option<OrderBook>,
    last_update_ms: Option<TimeMs>,
}

impl BookReducer {
    pub fn new(symbol: impl Into<Symbol>) -> Self {
        Self {
            symbol: symbol.into(),
            status: BookStatus::Empty,
            book: None,
            last_update_ms: None,
        }
    }

    pub fn symbol(&self) -> &str {
        &self.symbol
    }

    pub fn status(&self) -> BookStatus {
        self.status
    }

    pub fn is_ready(&self) -> bool {
        self.status == BookStatus::Ready
    }

    pub fn book(&self) -> Option<&OrderBook> {
        self.book.as_ref()
    }

    pub fn last_update_ms(&self) -> Option<TimeMs> {
        self.last_update_ms
    }

    pub fn apply_snapshot(&mut self, book: OrderBook) -> BookStatus {
        self.symbol = book.symbol.clone();
        self.last_update_ms = Some(book.ts_ms);
        self.status = if valid_book(&book) {
            BookStatus::Ready
        } else {
            BookStatus::Empty
        };
        self.book = Some(book);
        self.status
    }

    pub fn mark_recovering(&mut self) {
        self.status = BookStatus::Recovering;
    }

    pub fn mark_gapped(&mut self) {
        self.status = BookStatus::Gapped;
    }

    pub fn mark_stale_if_older_than(&mut self, now_ms: TimeMs, max_age_ms: TimeMs) -> BookStatus {
        if matches!(self.status, BookStatus::Ready)
            && self
                .last_update_ms
                .is_some_and(|last| now_ms.saturating_sub(last) > max_age_ms)
        {
            self.status = BookStatus::Stale;
        }
        self.status
    }

    pub fn levels(&self, side: Side) -> &[Level] {
        self.book
            .as_ref()
            .map(|book| book.levels(side))
            .unwrap_or(&[])
    }

    pub fn best(&self, side: Side) -> Option<Level> {
        self.book.as_ref().and_then(|book| match side {
            Side::Buy => book.best_bid(),
            Side::Sell => book.best_ask(),
        })
    }

    pub fn mid(&self) -> Option<Price> {
        self.book.as_ref()?.mid()
    }

    pub fn take_liquidity(
        &mut self,
        taker_side: Side,
        limit_px: Price,
        mut qty: Quantity,
    ) -> Vec<LiquidityFill> {
        let Some(book) = self.book.as_mut() else {
            return Vec::new();
        };

        let levels = book.levels_mut(taker_side.reverse());
        let mut fills = Vec::new();
        for level in levels.iter_mut() {
            if qty <= 0.0 {
                break;
            }
            if level.qty <= 0.0 || !taker_side.crosses(limit_px, level.px) {
                break;
            }
            let fill_qty = qty.min(level.qty);
            level.qty -= fill_qty;
            qty -= fill_qty;
            fills.push(LiquidityFill {
                px: level.px,
                qty: fill_qty,
            });
        }
        levels.retain(|level| level.qty > 0.0);
        self.status = if valid_book(book) {
            BookStatus::Ready
        } else {
            BookStatus::Empty
        };
        fills
    }
}

fn valid_book(book: &OrderBook) -> bool {
    !book.bids.is_empty()
        && !book.asks.is_empty()
        && book
            .bids
            .iter()
            .chain(book.asks.iter())
            .all(|level| level.px.is_finite() && level.qty.is_finite() && level.qty > 0.0)
}

#[cfg(test)]
mod tests {
    use reap_core::Level;

    use super::*;

    #[test]
    fn snapshot_sets_ready_and_exposes_mid() {
        let mut reducer = BookReducer::new("BTC-USDT");
        let status = reducer.apply_snapshot(OrderBook::one_level(
            "BTC-USDT",
            10,
            Level::new(100.0, 1.0),
            Level::new(102.0, 2.0),
        ));

        assert_eq!(status, BookStatus::Ready);
        assert_eq!(reducer.mid(), Some(101.0));
    }

    #[test]
    fn status_transitions_cover_recovery_gap_and_stale() {
        let mut reducer = BookReducer::new("BTC-USDT");
        reducer.mark_recovering();
        assert_eq!(reducer.status(), BookStatus::Recovering);
        reducer.mark_gapped();
        assert_eq!(reducer.status(), BookStatus::Gapped);
        reducer.apply_snapshot(OrderBook::one_level(
            "BTC-USDT",
            10,
            Level::new(100.0, 1.0),
            Level::new(101.0, 1.0),
        ));
        assert_eq!(reducer.mark_stale_if_older_than(20, 5), BookStatus::Stale);
    }

    #[test]
    fn take_liquidity_consumes_crossing_depth() {
        let mut reducer = BookReducer::new("BTC-USDT");
        reducer.apply_snapshot(OrderBook {
            symbol: "BTC-USDT".to_string(),
            ts_ms: 1,
            bids: vec![Level::new(100.0, 1.0)],
            asks: vec![Level::new(101.0, 0.4), Level::new(102.0, 0.8)],
        });

        let fills = reducer.take_liquidity(Side::Buy, 102.0, 1.0);

        assert_eq!(fills.len(), 2);
        assert_eq!(
            fills[0],
            LiquidityFill {
                px: 101.0,
                qty: 0.4
            }
        );
        assert_eq!(
            fills[1],
            LiquidityFill {
                px: 102.0,
                qty: 0.6
            }
        );
        let best_ask = reducer.best(Side::Sell).unwrap();
        assert_eq!(best_ask.px, 102.0);
        assert!((best_ask.qty - 0.2).abs() < f64::EPSILON);
    }
}
