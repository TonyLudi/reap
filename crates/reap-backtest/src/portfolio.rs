use std::collections::HashMap;

use reap_core::{FillLiquidity, OrderUpdate, Side, Symbol};
use reap_strategy::InstrumentConfig;

#[derive(Debug, Clone)]
pub struct Portfolio {
    instruments: HashMap<Symbol, InstrumentConfig>,
    positions: HashMap<Symbol, f64>,
    inverse_cash_coin: HashMap<Symbol, f64>,
    cash_usd: f64,
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
        self.cash_usd -= notional.abs() * fee_rate;
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

    pub fn cash_usd(&self) -> f64 {
        self.cash_usd
    }

    pub fn positions(&self) -> &HashMap<Symbol, f64> {
        &self.positions
    }
}
