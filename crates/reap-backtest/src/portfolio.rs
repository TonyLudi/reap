use std::collections::HashMap;

use reap_core::{FillLiquidity, OrderUpdate, Side, Symbol};
use reap_strategy::{InstrumentConfig, InstrumentKindConfig};

#[derive(Debug, Clone)]
pub struct Portfolio {
    instruments: HashMap<Symbol, InstrumentConfig>,
    positions: HashMap<Symbol, f64>,
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

        let notional = match inst.kind {
            InstrumentKindConfig::Spot => update.last_fill_qty * update.last_fill_price,
            InstrumentKindConfig::Future => {
                update.last_fill_qty * inst.contract_value * update.last_fill_price
            }
        };
        match update.side {
            Side::Buy => self.cash_usd -= notional,
            Side::Sell => self.cash_usd += notional,
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
            equity += match inst.kind {
                InstrumentKindConfig::Spot => qty * mark,
                InstrumentKindConfig::Future => qty * inst.contract_value * mark,
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
