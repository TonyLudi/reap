use std::collections::HashMap;

use reap_core::{FillLiquidity, OrderUpdate, Side, Symbol};
use reap_strategy::InstrumentConfig;

#[derive(Debug, Clone)]
pub struct Portfolio {
    instruments: HashMap<Symbol, InstrumentConfig>,
    positions: HashMap<Symbol, f64>,
    inverse_cash_coin: HashMap<Symbol, f64>,
    cash_usd: f64,
    fee_cost_usd: f64,
    funding_pnl_usd: f64,
    turnover_usd: f64,
}

impl Portfolio {
    pub fn new(instruments: &[InstrumentConfig]) -> Self {
        Self {
            instruments: instruments
                .iter()
                .map(|inst| (inst.symbol.clone(), inst.clone()))
                .collect(),
            positions: HashMap::new(),
            inverse_cash_coin: HashMap::new(),
            cash_usd: 0.0,
            fee_cost_usd: 0.0,
            funding_pnl_usd: 0.0,
            turnover_usd: 0.0,
        }
    }

    pub fn apply_fill(&mut self, update: &OrderUpdate) {
        if !update.has_fill() {
            return;
        }
        let Some(inst) = self.instruments.get(&update.symbol) else {
            return;
        };
        let signed_qty = update.side.factor() * update.last_fill_qty;
        *self.positions.entry(update.symbol.clone()).or_default() += signed_qty;

        let notional = if inst.kind.is_spot() {
            update.last_fill_qty * update.last_fill_price
        } else if inst.kind.is_inverse() {
            update.last_fill_qty * inst.contract_value
        } else {
            update.last_fill_qty * inst.contract_value * update.last_fill_price
        };
        if inst.kind.is_inverse() {
            *self
                .inverse_cash_coin
                .entry(update.symbol.clone())
                .or_default() += signed_qty * inst.contract_value / update.last_fill_price;
        } else {
            match update.side {
                Side::Buy => self.cash_usd -= notional,
                Side::Sell => self.cash_usd += notional,
            }
        }

        let fee_rate = match update.last_fill_liquidity {
            Some(FillLiquidity::Maker) => inst.maker_fee,
            Some(FillLiquidity::Taker) => inst.taker_fee,
            None => 0.0,
        };
        let absolute_notional = notional.abs();
        let fee_cost = absolute_notional * fee_rate;
        self.cash_usd -= fee_cost;
        self.fee_cost_usd += fee_cost;
        self.turnover_usd += absolute_notional;
    }

    /// Applies one swap funding settlement and returns USD PnL (positive means received).
    pub fn apply_funding(&mut self, symbol: &str, rate: f64, mark: f64) -> Option<f64> {
        let inst = self.instruments.get(symbol)?;
        if !inst.kind.is_swap() || !rate.is_finite() {
            return None;
        }
        let position = self.positions.get(symbol).copied().unwrap_or(0.0);
        if position == 0.0 {
            return Some(0.0);
        }
        if !mark.is_finite() || mark <= 0.0 {
            return None;
        }

        let funding_cost_usd = if inst.kind.is_inverse() {
            let funding_cost_coin = position * inst.contract_value * rate / mark;
            *self
                .inverse_cash_coin
                .entry(symbol.to_string())
                .or_default() -= funding_cost_coin;
            funding_cost_coin * mark
        } else {
            let funding_cost = position * inst.contract_value * rate * mark;
            self.cash_usd -= funding_cost;
            funding_cost
        };
        let funding_pnl = -funding_cost_usd;
        self.funding_pnl_usd += funding_pnl;
        Some(funding_pnl)
    }

    pub fn supports_funding(&self, symbol: &str) -> bool {
        self.instruments
            .get(symbol)
            .is_some_and(|instrument| instrument.kind.is_swap())
    }

    pub fn equity_usd(&self, marks: &HashMap<Symbol, f64>) -> f64 {
        let mut equity = self.cash_usd;
        for (symbol, qty) in &self.positions {
            let Some(inst) = self.instruments.get(symbol) else {
                continue;
            };
            let Some(mark) = marks.get(symbol) else {
                continue;
            };
            equity += if inst.kind.is_spot() {
                qty * mark
            } else if inst.kind.is_inverse() {
                let cash_coin = self.inverse_cash_coin.get(symbol).copied().unwrap_or(0.0);
                (cash_coin - qty * inst.contract_value / mark) * mark
            } else {
                qty * inst.contract_value * mark
            };
        }
        equity
    }

    pub fn equity_usd_checked(&self, marks: &HashMap<Symbol, f64>) -> Option<f64> {
        let mut equity = self.cash_usd;
        if !equity.is_finite() {
            return None;
        }
        for (symbol, qty) in &self.positions {
            let inst = self.instruments.get(symbol)?;
            let inverse_cash_coin = self.inverse_cash_coin.get(symbol).copied().unwrap_or(0.0);
            if *qty == 0.0 && (!inst.kind.is_inverse() || inverse_cash_coin == 0.0) {
                continue;
            }
            let mark = marks
                .get(symbol)
                .copied()
                .filter(|mark| mark.is_finite() && *mark > 0.0)?;
            let contribution = if inst.kind.is_spot() {
                qty * mark
            } else if inst.kind.is_inverse() {
                (inverse_cash_coin - qty * inst.contract_value / mark) * mark
            } else {
                qty * inst.contract_value * mark
            };
            if !contribution.is_finite() {
                return None;
            }
            equity += contribution;
        }
        equity.is_finite().then_some(equity)
    }

    pub fn gross_exposure_usd_checked(&self, marks: &HashMap<Symbol, f64>) -> Option<f64> {
        let mut gross_exposure = 0.0;
        for (symbol, qty) in &self.positions {
            let inst = self.instruments.get(symbol)?;
            if *qty == 0.0 {
                continue;
            }
            let notional = if inst.kind.is_inverse() {
                qty * inst.contract_value
            } else {
                let mark = marks
                    .get(symbol)
                    .copied()
                    .filter(|mark| mark.is_finite() && *mark > 0.0)?;
                if inst.kind.is_spot() {
                    qty * mark
                } else {
                    qty * inst.contract_value * mark
                }
            };
            if !notional.is_finite() {
                return None;
            }
            gross_exposure += notional.abs();
        }
        gross_exposure.is_finite().then_some(gross_exposure)
    }

    pub fn cash_usd(&self) -> f64 {
        self.cash_usd
    }

    pub fn fee_cost_usd(&self) -> f64 {
        self.fee_cost_usd
    }

    pub fn funding_pnl_usd(&self) -> f64 {
        self.funding_pnl_usd
    }

    pub fn turnover_usd(&self) -> f64 {
        self.turnover_usd
    }

    pub fn positions(&self) -> &HashMap<Symbol, f64> {
        &self.positions
    }
}

#[cfg(test)]
mod tests {
    use reap_core::{OrderEvent, OrderStatus, TimeInForce};
    use reap_strategy::InstrumentKindConfig;

    use super::*;

    fn fill(symbol: &str, side: Side, qty: f64, price: f64) -> OrderUpdate {
        OrderUpdate {
            ts_ms: 1,
            order_id: "fill-1".to_string(),
            symbol: symbol.to_string(),
            side,
            event: OrderEvent::FullyFilled,
            status: OrderStatus::Filled,
            price,
            time_in_force: Some(TimeInForce::Ioc),
            qty,
            open_qty: 0.0,
            filled_qty: qty,
            avg_fill_price: price,
            last_fill_qty: qty,
            last_fill_price: price,
            last_fill_liquidity: Some(FillLiquidity::Taker),
            reason: "test".to_string(),
        }
    }

    #[test]
    fn attributes_fee_cost_and_turnover() {
        let instrument = InstrumentConfig {
            symbol: "BTC-USDT".to_string(),
            kind: InstrumentKindConfig::Spot,
            taker_fee: 0.001,
            ..InstrumentConfig::default()
        };
        let mut portfolio = Portfolio::new(&[instrument]);

        portfolio.apply_fill(&fill("BTC-USDT", Side::Buy, 2.0, 100.0));

        assert_eq!(portfolio.turnover_usd(), 200.0);
        assert_eq!(portfolio.fee_cost_usd(), 0.2);
    }

    #[test]
    fn settles_linear_swap_funding_from_signed_notional() {
        let instrument = InstrumentConfig {
            symbol: "BTC-SWAP".to_string(),
            kind: InstrumentKindConfig::LinearSwap,
            contract_value: 0.01,
            taker_fee: 0.0,
            ..InstrumentConfig::default()
        };
        let mut portfolio = Portfolio::new(&[instrument]);
        portfolio.apply_fill(&fill("BTC-SWAP", Side::Buy, 10.0, 50_000.0));

        let funding_pnl = portfolio
            .apply_funding("BTC-SWAP", 0.001, 50_000.0)
            .unwrap();
        let equity = portfolio.equity_usd(&HashMap::from([("BTC-SWAP".to_string(), 50_000.0)]));

        assert_eq!(funding_pnl, -5.0);
        assert_eq!(portfolio.funding_pnl_usd(), -5.0);
        assert_eq!(equity, -5.0);
    }

    #[test]
    fn settles_inverse_swap_funding_in_coin() {
        let instrument = InstrumentConfig {
            symbol: "BTC-USD-SWAP".to_string(),
            kind: InstrumentKindConfig::InverseSwap,
            contract_value: 100.0,
            taker_fee: 0.0,
            ..InstrumentConfig::default()
        };
        let mut portfolio = Portfolio::new(&[instrument]);
        portfolio.apply_fill(&fill("BTC-USD-SWAP", Side::Buy, 10.0, 50_000.0));

        let funding_pnl = portfolio
            .apply_funding("BTC-USD-SWAP", 0.001, 50_000.0)
            .unwrap();
        let equity = portfolio.equity_usd(&HashMap::from([("BTC-USD-SWAP".to_string(), 50_000.0)]));

        assert_eq!(funding_pnl, -1.0);
        assert_eq!(portfolio.funding_pnl_usd(), -1.0);
        assert!((equity + 1.0).abs() < 1e-9, "{equity}");
    }

    #[test]
    fn short_swap_receives_positive_rate_funding() {
        let instrument = InstrumentConfig {
            symbol: "BTC-SWAP".to_string(),
            kind: InstrumentKindConfig::LinearSwap,
            contract_value: 0.01,
            taker_fee: 0.0,
            ..InstrumentConfig::default()
        };
        let mut portfolio = Portfolio::new(&[instrument]);
        portfolio.apply_fill(&fill("BTC-SWAP", Side::Sell, 10.0, 50_000.0));

        let funding_pnl = portfolio
            .apply_funding("BTC-SWAP", 0.001, 50_000.0)
            .unwrap();

        assert_eq!(funding_pnl, 5.0);
        assert_eq!(portfolio.funding_pnl_usd(), 5.0);
    }

    #[test]
    fn checked_equity_requires_a_valid_mark_for_every_position() {
        let instrument = InstrumentConfig {
            symbol: "BTC-USDT".to_string(),
            kind: InstrumentKindConfig::Spot,
            taker_fee: 0.0,
            ..InstrumentConfig::default()
        };
        let mut portfolio = Portfolio::new(&[instrument]);
        portfolio.apply_fill(&fill("BTC-USDT", Side::Buy, 1.0, 100.0));

        assert_eq!(portfolio.equity_usd_checked(&HashMap::new()), None);
        assert_eq!(
            portfolio.equity_usd_checked(&HashMap::from([("BTC-USDT".to_string(), 110.0)])),
            Some(10.0)
        );
        assert_eq!(
            portfolio.gross_exposure_usd_checked(&HashMap::from([("BTC-USDT".to_string(), 110.0)])),
            Some(110.0)
        );
    }

    #[test]
    fn checked_equity_does_not_require_a_mark_for_a_flat_spot_position() {
        let instrument = InstrumentConfig {
            symbol: "BTC-USDT".to_string(),
            kind: InstrumentKindConfig::Spot,
            taker_fee: 0.0,
            ..InstrumentConfig::default()
        };
        let mut portfolio = Portfolio::new(&[instrument]);
        portfolio.apply_fill(&fill("BTC-USDT", Side::Buy, 1.0, 100.0));
        portfolio.apply_fill(&fill("BTC-USDT", Side::Sell, 1.0, 110.0));

        assert_eq!(portfolio.equity_usd_checked(&HashMap::new()), Some(10.0));
        assert_eq!(
            portfolio.gross_exposure_usd_checked(&HashMap::new()),
            Some(0.0)
        );
    }

    #[test]
    fn checked_equity_requires_a_mark_for_flat_inverse_coin_cash() {
        let instrument = InstrumentConfig {
            symbol: "BTC-USD-SWAP".to_string(),
            kind: InstrumentKindConfig::InverseSwap,
            contract_value: 100.0,
            taker_fee: 0.0,
            ..InstrumentConfig::default()
        };
        let mut portfolio = Portfolio::new(&[instrument]);
        portfolio.apply_fill(&fill("BTC-USD-SWAP", Side::Buy, 1.0, 50_000.0));
        portfolio.apply_fill(&fill("BTC-USD-SWAP", Side::Sell, 1.0, 55_000.0));

        assert_eq!(portfolio.equity_usd_checked(&HashMap::new()), None);
        let equity = portfolio
            .equity_usd_checked(&HashMap::from([("BTC-USD-SWAP".to_string(), 55_000.0)]))
            .unwrap();
        assert!((equity - 10.0).abs() < 1e-12);
        assert_eq!(
            portfolio.gross_exposure_usd_checked(&HashMap::new()),
            Some(0.0)
        );
    }

    #[test]
    fn checked_gross_exposure_does_not_require_an_inverse_mark() {
        let instrument = InstrumentConfig {
            symbol: "BTC-USD-SWAP".to_string(),
            kind: InstrumentKindConfig::InverseSwap,
            contract_value: 100.0,
            taker_fee: 0.0,
            ..InstrumentConfig::default()
        };
        let mut portfolio = Portfolio::new(&[instrument]);
        portfolio.apply_fill(&fill("BTC-USD-SWAP", Side::Buy, 2.0, 50_000.0));

        assert_eq!(
            portfolio.gross_exposure_usd_checked(&HashMap::new()),
            Some(200.0)
        );
        assert_eq!(portfolio.equity_usd_checked(&HashMap::new()), None);
    }
}
