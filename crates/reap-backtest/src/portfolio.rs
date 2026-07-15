use std::collections::{BTreeMap, BTreeSet, HashMap};

use reap_core::{FillFee, FillLiquidity, OrderUpdate, Symbol};
use reap_strategy::InstrumentConfig;

use crate::execution::BacktestInitialPortfolioConfig;

#[derive(Debug, Clone)]
pub struct Portfolio {
    instruments: BTreeMap<Symbol, InstrumentConfig>,
    positions: BTreeMap<Symbol, f64>,
    position_avg_prices: BTreeMap<Symbol, f64>,
    inverse_cash_coin: BTreeMap<Symbol, f64>,
    cash_by_currency: BTreeMap<String, f64>,
    fee_cost_usd: f64,
    exact_fee_fills: u64,
    estimated_fee_fills: u64,
    funding_pnl_usd: f64,
    turnover_usd: f64,
    currency_conversion_failures: u64,
    invalid_accounting_events: u64,
}

impl Portfolio {
    pub fn new(instruments: &[InstrumentConfig]) -> Self {
        Self {
            instruments: instruments
                .iter()
                .map(|inst| (inst.symbol.clone(), inst.clone()))
                .collect(),
            positions: BTreeMap::new(),
            position_avg_prices: BTreeMap::new(),
            inverse_cash_coin: BTreeMap::new(),
            cash_by_currency: BTreeMap::new(),
            fee_cost_usd: 0.0,
            exact_fee_fills: 0,
            estimated_fee_fills: 0,
            funding_pnl_usd: 0.0,
            turnover_usd: 0.0,
            currency_conversion_failures: 0,
            invalid_accounting_events: 0,
        }
    }

    pub fn with_initial(
        instruments: &[InstrumentConfig],
        initial: &BacktestInitialPortfolioConfig,
    ) -> Self {
        let mut portfolio = Self::new(instruments);
        for balance in &initial.balances {
            if let Some(symbol) = &balance.valuation_symbol {
                portfolio.positions.insert(symbol.clone(), balance.total);
                portfolio.position_avg_prices.insert(symbol.clone(), 0.0);
            } else {
                *portfolio
                    .cash_by_currency
                    .entry(balance.currency.clone())
                    .or_default() += balance.total;
            }
        }
        for position in &initial.positions {
            let Some(instrument) = portfolio.instruments.get(&position.symbol) else {
                continue;
            };
            portfolio
                .positions
                .insert(position.symbol.clone(), position.qty);
            portfolio
                .position_avg_prices
                .insert(position.symbol.clone(), position.avg_price);
            if position.qty == 0.0 {
                continue;
            }
            if instrument.kind.is_inverse() {
                portfolio.inverse_cash_coin.insert(
                    position.symbol.clone(),
                    position.qty * instrument.contract_value / position.avg_price,
                );
            } else {
                *portfolio
                    .cash_by_currency
                    .entry(instrument_accounting_currency(instrument))
                    .or_default() -= position.qty * instrument.contract_value * position.avg_price;
            }
        }
        portfolio
    }

    pub fn apply_fill(&mut self, update: &OrderUpdate, currency_rates_usd: &HashMap<String, f64>) {
        if !update.has_fill() {
            return;
        }
        let Some(inst) = self.instruments.get(&update.symbol).cloned() else {
            self.invalid_accounting_events = self.invalid_accounting_events.saturating_add(1);
            return;
        };
        let kind = inst.kind;
        let contract_value = inst.contract_value;
        let maker_fee = inst.maker_fee;
        let taker_fee = inst.taker_fee;
        let accounting_currency = instrument_accounting_currency(&inst);
        if !update.last_fill_qty.is_finite()
            || !update.last_fill_price.is_finite()
            || update.last_fill_price <= 0.0
        {
            self.invalid_accounting_events = self.invalid_accounting_events.saturating_add(1);
            return;
        }
        if update
            .last_fill_fee
            .as_ref()
            .is_some_and(|fee| !fee.amount.is_finite() || fee.currency.trim().is_empty())
        {
            self.invalid_accounting_events = self.invalid_accounting_events.saturating_add(1);
            return;
        }
        let signed_qty = update.side.factor() * update.last_fill_qty;
        self.apply_position_fill(&update.symbol, signed_qty, update.last_fill_price);

        let notional = if kind.is_spot() {
            update.last_fill_qty * update.last_fill_price
        } else if kind.is_inverse() {
            update.last_fill_qty * contract_value
        } else {
            update.last_fill_qty * contract_value * update.last_fill_price
        };
        if kind.is_inverse() {
            *self
                .inverse_cash_coin
                .entry(update.symbol.clone())
                .or_default() += signed_qty * contract_value / update.last_fill_price;
        } else {
            let cash = self
                .cash_by_currency
                .entry(accounting_currency.clone())
                .or_default();
            *cash -= update.side.factor() * notional;
        }

        let absolute_notional = notional.abs();
        let (currency_rate, accounting_conversion_complete) =
            currency_rate_or_par(&accounting_currency, currency_rates_usd);
        let mut conversion_failed = !accounting_conversion_complete;
        if let Some(fee) = &update.last_fill_fee {
            self.exact_fee_fills = self.exact_fee_fills.saturating_add(1);
            self.apply_exact_fee(&inst, &update.symbol, fee);
            let (fee_rate_usd, complete) = if fee.amount == 0.0 {
                (1.0, true)
            } else {
                fee_currency_rate_usd(
                    &inst,
                    &fee.currency,
                    update.last_fill_price,
                    currency_rates_usd,
                )
            };
            if !complete {
                conversion_failed = true;
            }
            self.fee_cost_usd += -fee.amount * fee_rate_usd;
        } else {
            self.estimated_fee_fills = self.estimated_fee_fills.saturating_add(1);
            let fee_rate = match update.last_fill_liquidity {
                Some(FillLiquidity::Maker) => maker_fee,
                Some(FillLiquidity::Taker) => taker_fee,
                None => 0.0,
            };
            let fee_cost = absolute_notional * fee_rate;
            if kind.is_inverse() {
                let fee_coin = fee_cost / update.last_fill_price;
                *self
                    .inverse_cash_coin
                    .entry(update.symbol.clone())
                    .or_default() -= fee_coin;
            } else {
                *self
                    .cash_by_currency
                    .entry(accounting_currency.clone())
                    .or_default() -= fee_cost;
            }

            self.fee_cost_usd += fee_cost * currency_rate;
        }

        if conversion_failed {
            self.currency_conversion_failures = self.currency_conversion_failures.saturating_add(1);
        }
        self.turnover_usd += absolute_notional * currency_rate;
    }

    fn apply_position_fill(&mut self, symbol: &str, signed_qty: f64, price: f64) {
        let inverse = self
            .instruments
            .get(symbol)
            .is_some_and(|instrument| instrument.kind.is_inverse());
        let old_qty = self.positions.get(symbol).copied().unwrap_or(0.0);
        let old_avg = self.position_avg_prices.get(symbol).copied().unwrap_or(0.0);
        let new_qty = old_qty + signed_qty;
        let new_avg = if new_qty.abs() <= f64::EPSILON {
            0.0
        } else if old_qty == 0.0 || old_qty.signum() == signed_qty.signum() {
            if inverse && old_qty != 0.0 {
                new_qty.abs() / (old_qty.abs() / old_avg + signed_qty.abs() / price)
            } else {
                let old_notional = old_qty.abs() * old_avg;
                let added_notional = signed_qty.abs() * price;
                (old_notional + added_notional) / new_qty.abs()
            }
        } else if signed_qty.abs() < old_qty.abs() {
            old_avg
        } else {
            price
        };
        self.positions.insert(symbol.to_string(), new_qty);
        self.position_avg_prices.insert(symbol.to_string(), new_avg);
    }

    fn apply_exact_fee(&mut self, instrument: &InstrumentConfig, symbol: &str, fee: &FillFee) {
        let currency = normalize_currency(&fee.currency);
        let base_currency = normalize_currency(&instrument.base_currency);
        let settle_currency = normalize_currency(&instrument.settle_currency);
        if instrument.kind.is_spot()
            && !instrument.base_currency.trim().is_empty()
            && currency == base_currency
        {
            *self.positions.entry(symbol.to_string()).or_default() += fee.amount;
        } else if instrument.kind.is_inverse()
            && ((!instrument.settle_currency.trim().is_empty() && currency == settle_currency)
                || (!instrument.base_currency.trim().is_empty() && currency == base_currency))
        {
            *self
                .inverse_cash_coin
                .entry(symbol.to_string())
                .or_default() += fee.amount;
        } else {
            *self.cash_by_currency.entry(currency).or_default() += fee.amount;
        }
    }

    /// Applies one swap funding settlement and returns USD PnL (positive means received).
    pub fn apply_funding(
        &mut self,
        symbol: &str,
        rate: f64,
        mark: f64,
        currency_rates_usd: &HashMap<String, f64>,
    ) -> Option<f64> {
        let inst = self.instruments.get(symbol)?;
        let kind = inst.kind;
        let contract_value = inst.contract_value;
        let accounting_currency = instrument_accounting_currency(inst);
        if !kind.is_swap() || !rate.is_finite() {
            return None;
        }
        let position = self.positions.get(symbol).copied().unwrap_or(0.0);
        if position == 0.0 {
            return Some(0.0);
        }
        if !mark.is_finite() || mark <= 0.0 {
            return None;
        }

        let funding_cost = if kind.is_inverse() {
            let funding_cost_coin = position * contract_value * rate / mark;
            *self
                .inverse_cash_coin
                .entry(symbol.to_string())
                .or_default() -= funding_cost_coin;
            funding_cost_coin * mark
        } else {
            let funding_cost = position * contract_value * rate * mark;
            *self
                .cash_by_currency
                .entry(accounting_currency.clone())
                .or_default() -= funding_cost;
            funding_cost
        };
        let (currency_rate, complete) =
            currency_rate_or_par(&accounting_currency, currency_rates_usd);
        if !complete {
            self.currency_conversion_failures = self.currency_conversion_failures.saturating_add(1);
        }
        let funding_pnl = -funding_cost * currency_rate;
        self.funding_pnl_usd += funding_pnl;
        Some(funding_pnl)
    }

    pub fn supports_funding(&self, symbol: &str) -> bool {
        self.instruments
            .get(symbol)
            .is_some_and(|instrument| instrument.kind.is_swap())
    }

    pub fn equity_usd(
        &self,
        marks: &HashMap<Symbol, f64>,
        currency_rates_usd: &HashMap<String, f64>,
    ) -> f64 {
        let mut equity = self.cash_usd(currency_rates_usd);
        for (symbol, qty) in &self.positions {
            let Some(inst) = self.instruments.get(symbol) else {
                continue;
            };
            let Some(mark) = marks.get(symbol) else {
                continue;
            };
            let contribution = if inst.kind.is_spot() {
                qty * mark
            } else if inst.kind.is_inverse() {
                let cash_coin = self.inverse_cash_coin.get(symbol).copied().unwrap_or(0.0);
                (cash_coin - qty * inst.contract_value / mark) * mark
            } else {
                qty * inst.contract_value * mark
            };
            equity += contribution
                * currency_rate_or_par(&instrument_accounting_currency(inst), currency_rates_usd).0;
        }
        equity
    }

    pub fn equity_usd_checked(
        &self,
        marks: &HashMap<Symbol, f64>,
        currency_rates_usd: &HashMap<String, f64>,
    ) -> Option<f64> {
        let mut equity = self.cash_usd_checked(currency_rates_usd)?;
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
            equity += contribution
                * currency_rate_checked(&instrument_accounting_currency(inst), currency_rates_usd)?;
        }
        equity.is_finite().then_some(equity)
    }

    pub fn gross_exposure_usd_checked(
        &self,
        marks: &HashMap<Symbol, f64>,
        currency_rates_usd: &HashMap<String, f64>,
    ) -> Option<f64> {
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
            gross_exposure += notional.abs()
                * currency_rate_checked(&instrument_accounting_currency(inst), currency_rates_usd)?;
        }
        gross_exposure.is_finite().then_some(gross_exposure)
    }

    pub fn cash_usd(&self, currency_rates_usd: &HashMap<String, f64>) -> f64 {
        self.cash_by_currency
            .iter()
            .map(|(currency, amount)| amount * currency_rate_or_par(currency, currency_rates_usd).0)
            .sum()
    }

    pub fn cash_usd_checked(&self, currency_rates_usd: &HashMap<String, f64>) -> Option<f64> {
        self.cash_by_currency
            .iter()
            .try_fold(0.0, |total, (currency, amount)| {
                if *amount == 0.0 {
                    return Some(total);
                }
                let value = amount * currency_rate_checked(currency, currency_rates_usd)?;
                value.is_finite().then_some(total + value)
            })
            .filter(|cash| cash.is_finite())
    }

    pub fn fee_cost_usd(&self) -> f64 {
        self.fee_cost_usd
    }

    pub fn exact_fee_fills(&self) -> u64 {
        self.exact_fee_fills
    }

    pub fn estimated_fee_fills(&self) -> u64 {
        self.estimated_fee_fills
    }

    pub fn funding_pnl_usd(&self) -> f64 {
        self.funding_pnl_usd
    }

    pub fn turnover_usd(&self) -> f64 {
        self.turnover_usd
    }

    pub fn currency_conversion_failures(&self) -> u64 {
        self.currency_conversion_failures
    }

    pub fn invalid_accounting_events(&self) -> u64 {
        self.invalid_accounting_events
    }

    pub fn cash_by_currency(&self) -> BTreeMap<String, f64> {
        self.cash_by_currency
            .iter()
            .map(|(currency, amount)| (currency.clone(), *amount))
            .collect()
    }

    pub fn inverse_cash_coin_by_symbol(&self) -> BTreeMap<Symbol, f64> {
        self.inverse_cash_coin
            .iter()
            .map(|(symbol, amount)| (symbol.clone(), *amount))
            .collect()
    }

    pub fn position_avg_price(&self, symbol: &str) -> f64 {
        self.position_avg_prices.get(symbol).copied().unwrap_or(0.0)
    }

    /// Reconstructs the exchange-style cash balance from the internal cost-basis ledger.
    pub fn account_balance(&self, currency: &str) -> f64 {
        let currency = normalize_currency(currency);
        let mut balance = self.cash_by_currency.get(&currency).copied().unwrap_or(0.0);
        for (symbol, qty) in &self.positions {
            let Some(instrument) = self.instruments.get(symbol) else {
                continue;
            };
            let avg_price = self.position_avg_price(symbol);
            if instrument.kind.is_spot() {
                if normalize_currency(&instrument.base_currency) == currency {
                    balance += qty;
                }
            } else if instrument.kind.is_inverse() {
                let settle_currency = if instrument.settle_currency.is_empty() {
                    normalize_currency(&instrument.base_currency)
                } else {
                    normalize_currency(&instrument.settle_currency)
                };
                if settle_currency == currency {
                    let inverse_cash = self.inverse_cash_coin.get(symbol).copied().unwrap_or(0.0);
                    let open_basis = if *qty == 0.0 || avg_price <= 0.0 {
                        0.0
                    } else {
                        qty * instrument.contract_value / avg_price
                    };
                    balance += inverse_cash - open_basis;
                }
            } else if instrument_accounting_currency(instrument) == currency {
                balance += qty * instrument.contract_value * avg_price;
            }
        }
        balance
    }

    #[cfg(test)]
    pub fn missing_currency_rates(&self, currency_rates_usd: &HashMap<String, f64>) -> Vec<String> {
        let mut required = BTreeSet::new();
        for (currency, amount) in &self.cash_by_currency {
            if *amount != 0.0 {
                required.insert(currency.clone());
            }
        }
        for (symbol, quantity) in &self.positions {
            let inverse_cash = self.inverse_cash_coin.get(symbol).copied().unwrap_or(0.0);
            if *quantity == 0.0 && inverse_cash == 0.0 {
                continue;
            }
            if let Some(instrument) = self.instruments.get(symbol) {
                required.insert(instrument_accounting_currency(instrument));
            }
        }
        required
            .into_iter()
            .filter(|currency| currency_rate_checked(currency, currency_rates_usd).is_none())
            .collect()
    }

    pub fn notional_currency_rate_usd_checked(
        &self,
        symbol: &str,
        currency_rates_usd: &HashMap<String, f64>,
    ) -> Option<f64> {
        let instrument = self.instruments.get(symbol)?;
        currency_rate_checked(
            &instrument_accounting_currency(instrument),
            currency_rates_usd,
        )
    }

    pub fn notional_currency_rate_usd(
        &self,
        symbol: &str,
        currency_rates_usd: &HashMap<String, f64>,
    ) -> f64 {
        self.instruments
            .get(symbol)
            .map(|instrument| {
                currency_rate_or_par(
                    &instrument_accounting_currency(instrument),
                    currency_rates_usd,
                )
                .0
            })
            .unwrap_or(1.0)
    }

    pub fn positions(&self) -> &BTreeMap<Symbol, f64> {
        &self.positions
    }
}

pub(crate) fn required_accounting_currencies(instruments: &[InstrumentConfig]) -> BTreeSet<String> {
    instruments
        .iter()
        .map(instrument_accounting_currency)
        .filter(|currency| currency != "USD")
        .collect()
}

fn instrument_accounting_currency(instrument: &InstrumentConfig) -> String {
    let configured = if instrument.kind.is_derivative()
        && !instrument.kind.is_inverse()
        && !instrument.settle_currency.is_empty()
    {
        &instrument.settle_currency
    } else {
        &instrument.quote_currency
    };
    normalize_currency(configured)
}

fn normalize_currency(currency: &str) -> String {
    let currency = currency.trim();
    if currency.is_empty() {
        "USD".to_string()
    } else {
        currency.to_ascii_uppercase()
    }
}

fn currency_rate_checked(currency: &str, currency_rates_usd: &HashMap<String, f64>) -> Option<f64> {
    if currency == "USD" {
        return Some(1.0);
    }
    currency_rates_usd
        .get(currency)
        .copied()
        .filter(|rate| rate.is_finite() && *rate > 0.0)
}

fn fee_currency_rate_usd(
    instrument: &InstrumentConfig,
    fee_currency: &str,
    fill_price: f64,
    currency_rates_usd: &HashMap<String, f64>,
) -> (f64, bool) {
    let fee_currency = normalize_currency(fee_currency);
    if !instrument.base_currency.trim().is_empty()
        && fee_currency == normalize_currency(&instrument.base_currency)
    {
        let quote_currency = normalize_currency(&instrument.quote_currency);
        let (quote_rate, complete) = currency_rate_or_par(&quote_currency, currency_rates_usd);
        return (fill_price * quote_rate, complete);
    }
    currency_rate_or_par(&fee_currency, currency_rates_usd)
}

fn currency_rate_or_par(currency: &str, currency_rates_usd: &HashMap<String, f64>) -> (f64, bool) {
    currency_rate_checked(currency, currency_rates_usd)
        .map(|rate| (rate, true))
        .unwrap_or((1.0, currency == "USD"))
}

#[cfg(test)]
mod tests {
    use reap_core::{OrderEvent, OrderStatus, Side, TimeInForce};
    use reap_strategy::InstrumentKindConfig;

    use super::*;
    use crate::{BacktestInitialBalanceConfig, BacktestInitialPositionConfig};

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
            last_fill_fee: None,
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

        portfolio.apply_fill(&fill("BTC-USDT", Side::Buy, 2.0, 100.0), &HashMap::new());

        assert_eq!(portfolio.turnover_usd(), 200.0);
        assert_eq!(portfolio.fee_cost_usd(), 0.2);
        assert_eq!(portfolio.exact_fee_fills(), 0);
        assert_eq!(portfolio.estimated_fee_fills(), 1);
    }

    #[test]
    fn initial_account_snapshot_separates_capital_from_linear_and_spot_pnl() {
        let instruments = vec![
            InstrumentConfig {
                symbol: "BTC-USDT".to_string(),
                kind: InstrumentKindConfig::Spot,
                base_currency: "BTC".to_string(),
                quote_currency: "USDT".to_string(),
                taker_fee: 0.0,
                ..InstrumentConfig::default()
            },
            InstrumentConfig {
                symbol: "BTC-USDT-SWAP".to_string(),
                kind: InstrumentKindConfig::LinearSwap,
                base_currency: "BTC".to_string(),
                quote_currency: "USDT".to_string(),
                settle_currency: "USDT".to_string(),
                contract_value: 0.1,
                taker_fee: 0.0,
                ..InstrumentConfig::default()
            },
        ];
        let initial = BacktestInitialPortfolioConfig {
            balances: vec![
                BacktestInitialBalanceConfig {
                    currency: "BTC".to_string(),
                    total: 2.0,
                    valuation_symbol: Some("BTC-USDT".to_string()),
                    ..Default::default()
                },
                BacktestInitialBalanceConfig {
                    currency: "USDT".to_string(),
                    total: 10_000.0,
                    valuation_symbol: None,
                    ..Default::default()
                },
            ],
            positions: vec![BacktestInitialPositionConfig {
                symbol: "BTC-USDT-SWAP".to_string(),
                qty: 10.0,
                avg_price: 100.0,
                margin_mode: Some(reap_core::PositionMarginMode::Cross),
            }],
            ..Default::default()
        };
        let rates = HashMap::from([("USDT".to_string(), 1.0)]);
        let opening_marks = HashMap::from([
            ("BTC-USDT".to_string(), 100.0),
            ("BTC-USDT-SWAP".to_string(), 100.0),
        ]);
        let final_marks = HashMap::from([
            ("BTC-USDT".to_string(), 110.0),
            ("BTC-USDT-SWAP".to_string(), 110.0),
        ]);
        let mut portfolio = Portfolio::with_initial(&instruments, &initial);

        assert_eq!(portfolio.account_balance("BTC"), 2.0);
        assert_eq!(portfolio.account_balance("USDT"), 10_000.0);
        assert_eq!(
            portfolio.equity_usd_checked(&opening_marks, &rates),
            Some(10_200.0)
        );
        assert_eq!(
            portfolio.equity_usd_checked(&final_marks, &rates),
            Some(10_230.0)
        );

        portfolio.apply_fill(&fill("BTC-USDT-SWAP", Side::Sell, 10.0, 110.0), &rates);
        assert_eq!(portfolio.positions()["BTC-USDT-SWAP"], 0.0);
        assert_eq!(portfolio.position_avg_price("BTC-USDT-SWAP"), 0.0);
        assert_eq!(portfolio.account_balance("USDT"), 10_010.0);
    }

    #[test]
    fn derivative_position_average_survives_reductions_and_resets_on_flip() {
        let instrument = InstrumentConfig {
            symbol: "BTC-SWAP".to_string(),
            kind: InstrumentKindConfig::LinearSwap,
            contract_value: 1.0,
            taker_fee: 0.0,
            ..InstrumentConfig::default()
        };
        let mut portfolio = Portfolio::new(&[instrument]);

        portfolio.apply_fill(&fill("BTC-SWAP", Side::Buy, 2.0, 100.0), &HashMap::new());
        portfolio.apply_fill(&fill("BTC-SWAP", Side::Buy, 1.0, 130.0), &HashMap::new());
        assert_eq!(portfolio.position_avg_price("BTC-SWAP"), 110.0);

        portfolio.apply_fill(&fill("BTC-SWAP", Side::Sell, 1.0, 140.0), &HashMap::new());
        assert_eq!(portfolio.position_avg_price("BTC-SWAP"), 110.0);

        portfolio.apply_fill(&fill("BTC-SWAP", Side::Sell, 3.0, 150.0), &HashMap::new());
        assert_eq!(portfolio.positions()["BTC-SWAP"], -1.0);
        assert_eq!(portfolio.position_avg_price("BTC-SWAP"), 150.0);
    }

    #[test]
    fn inverse_position_additions_use_harmonic_average_cost() {
        let instrument = InstrumentConfig {
            symbol: "BTC-USD-SWAP".to_string(),
            kind: InstrumentKindConfig::InverseSwap,
            contract_value: 100.0,
            taker_fee: 0.0,
            ..InstrumentConfig::default()
        };
        let mut portfolio = Portfolio::new(&[instrument]);

        portfolio.apply_fill(
            &fill("BTC-USD-SWAP", Side::Buy, 1.0, 40_000.0),
            &HashMap::new(),
        );
        portfolio.apply_fill(
            &fill("BTC-USD-SWAP", Side::Buy, 1.0, 60_000.0),
            &HashMap::new(),
        );

        assert!((portfolio.position_avg_price("BTC-USD-SWAP") - 48_000.0).abs() < 1e-9);
        assert!(portfolio.account_balance("USD").abs() < 1e-12);
    }

    #[test]
    fn exact_spot_base_fee_overrides_rate_and_reduces_inventory() {
        let instrument = InstrumentConfig {
            symbol: "BTC-USDT".to_string(),
            kind: InstrumentKindConfig::Spot,
            base_currency: "BTC".to_string(),
            quote_currency: "USDT".to_string(),
            taker_fee: 0.5,
            ..InstrumentConfig::default()
        };
        let rates = HashMap::from([("USDT".to_string(), 0.95)]);
        let marks = HashMap::from([("BTC-USDT".to_string(), 100.0)]);
        let mut update = fill("BTC-USDT", Side::Buy, 2.0, 100.0);
        update.last_fill_fee = Some(FillFee {
            amount: -0.002,
            currency: "BTC".to_string(),
        });
        let mut portfolio = Portfolio::new(&[instrument]);

        portfolio.apply_fill(&update, &rates);

        assert!((portfolio.positions()["BTC-USDT"] - 1.998).abs() < 1e-12);
        assert_eq!(portfolio.cash_by_currency().get("USDT"), Some(&-200.0));
        assert!((portfolio.fee_cost_usd() - 0.19).abs() < 1e-12);
        assert!((portfolio.equity_usd_checked(&marks, &rates).unwrap() + 0.19).abs() < 1e-12);
        assert_eq!(portfolio.exact_fee_fills(), 1);
        assert_eq!(portfolio.estimated_fee_fills(), 0);
        assert_eq!(portfolio.currency_conversion_failures(), 0);
    }

    #[test]
    fn exact_inverse_fee_is_booked_in_settlement_coin() {
        let instrument = InstrumentConfig {
            symbol: "BTC-USD-SWAP".to_string(),
            kind: InstrumentKindConfig::InverseSwap,
            base_currency: "BTC".to_string(),
            quote_currency: "USD".to_string(),
            settle_currency: "BTC".to_string(),
            contract_value: 100.0,
            taker_fee: 0.5,
            ..InstrumentConfig::default()
        };
        let marks = HashMap::from([("BTC-USD-SWAP".to_string(), 50_000.0)]);
        let mut update = fill("BTC-USD-SWAP", Side::Buy, 10.0, 50_000.0);
        update.last_fill_fee = Some(FillFee {
            amount: -0.00001,
            currency: "BTC".to_string(),
        });
        let mut portfolio = Portfolio::new(&[instrument]);

        portfolio.apply_fill(&update, &HashMap::new());

        assert!((portfolio.inverse_cash_coin_by_symbol()["BTC-USD-SWAP"] - 0.01999).abs() < 1e-12);
        assert!((portfolio.fee_cost_usd() - 0.5).abs() < 1e-12);
        assert!(
            (portfolio
                .equity_usd_checked(&marks, &HashMap::new())
                .unwrap()
                + 0.5)
                .abs()
                < 1e-9
        );
        assert_eq!(portfolio.exact_fee_fills(), 1);
        assert_eq!(portfolio.estimated_fee_fills(), 0);
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
        portfolio.apply_fill(
            &fill("BTC-SWAP", Side::Buy, 10.0, 50_000.0),
            &HashMap::new(),
        );

        let funding_pnl = portfolio
            .apply_funding("BTC-SWAP", 0.001, 50_000.0, &HashMap::new())
            .unwrap();
        let equity = portfolio.equity_usd(
            &HashMap::from([("BTC-SWAP".to_string(), 50_000.0)]),
            &HashMap::new(),
        );

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
        portfolio.apply_fill(
            &fill("BTC-USD-SWAP", Side::Buy, 10.0, 50_000.0),
            &HashMap::new(),
        );

        let funding_pnl = portfolio
            .apply_funding("BTC-USD-SWAP", 0.001, 50_000.0, &HashMap::new())
            .unwrap();
        let equity = portfolio.equity_usd(
            &HashMap::from([("BTC-USD-SWAP".to_string(), 50_000.0)]),
            &HashMap::new(),
        );

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
        portfolio.apply_fill(
            &fill("BTC-SWAP", Side::Sell, 10.0, 50_000.0),
            &HashMap::new(),
        );

        let funding_pnl = portfolio
            .apply_funding("BTC-SWAP", 0.001, 50_000.0, &HashMap::new())
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
        portfolio.apply_fill(&fill("BTC-USDT", Side::Buy, 1.0, 100.0), &HashMap::new());

        assert_eq!(
            portfolio.equity_usd_checked(&HashMap::new(), &HashMap::new()),
            None
        );
        assert_eq!(
            portfolio.equity_usd_checked(
                &HashMap::from([("BTC-USDT".to_string(), 110.0)]),
                &HashMap::new(),
            ),
            Some(10.0)
        );
        assert_eq!(
            portfolio.gross_exposure_usd_checked(
                &HashMap::from([("BTC-USDT".to_string(), 110.0)]),
                &HashMap::new(),
            ),
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
        portfolio.apply_fill(&fill("BTC-USDT", Side::Buy, 1.0, 100.0), &HashMap::new());
        portfolio.apply_fill(&fill("BTC-USDT", Side::Sell, 1.0, 110.0), &HashMap::new());

        assert_eq!(
            portfolio.equity_usd_checked(&HashMap::new(), &HashMap::new()),
            Some(10.0)
        );
        assert_eq!(
            portfolio.gross_exposure_usd_checked(&HashMap::new(), &HashMap::new()),
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
        portfolio.apply_fill(
            &fill("BTC-USD-SWAP", Side::Buy, 1.0, 50_000.0),
            &HashMap::new(),
        );
        portfolio.apply_fill(
            &fill("BTC-USD-SWAP", Side::Sell, 1.0, 55_000.0),
            &HashMap::new(),
        );

        assert_eq!(
            portfolio.equity_usd_checked(&HashMap::new(), &HashMap::new()),
            None
        );
        let equity = portfolio
            .equity_usd_checked(
                &HashMap::from([("BTC-USD-SWAP".to_string(), 55_000.0)]),
                &HashMap::new(),
            )
            .unwrap();
        assert!((equity - 10.0).abs() < 1e-12);
        assert_eq!(
            portfolio.gross_exposure_usd_checked(&HashMap::new(), &HashMap::new()),
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
        portfolio.apply_fill(
            &fill("BTC-USD-SWAP", Side::Buy, 2.0, 50_000.0),
            &HashMap::new(),
        );

        assert_eq!(
            portfolio.gross_exposure_usd_checked(&HashMap::new(), &HashMap::new()),
            Some(200.0)
        );
        assert_eq!(
            portfolio.equity_usd_checked(&HashMap::new(), &HashMap::new()),
            None
        );
    }

    #[test]
    fn spot_cash_equity_and_exposure_follow_quote_currency_rate() {
        let instrument = InstrumentConfig {
            symbol: "BTC-USDT".to_string(),
            kind: InstrumentKindConfig::Spot,
            base_currency: "BTC".to_string(),
            quote_currency: "USDT".to_string(),
            taker_fee: 0.0,
            ..InstrumentConfig::default()
        };
        let rates = HashMap::from([("USDT".to_string(), 0.95)]);
        let marks = HashMap::from([("BTC-USDT".to_string(), 110.0)]);
        let mut portfolio = Portfolio::new(&[instrument]);

        portfolio.apply_fill(&fill("BTC-USDT", Side::Buy, 1.0, 100.0), &rates);

        assert_eq!(portfolio.cash_by_currency().get("USDT"), Some(&-100.0));
        assert_eq!(portfolio.cash_usd_checked(&rates), Some(-95.0));
        assert_eq!(portfolio.equity_usd_checked(&marks, &rates), Some(9.5));
        assert_eq!(
            portfolio.gross_exposure_usd_checked(&marks, &rates),
            Some(104.5)
        );
        assert_eq!(portfolio.turnover_usd(), 95.0);
        assert_eq!(portfolio.currency_conversion_failures(), 0);
    }

    #[test]
    fn missing_quote_rate_uses_report_fallback_but_fails_checked_accounting() {
        let instrument = InstrumentConfig {
            symbol: "BTC-USDT".to_string(),
            kind: InstrumentKindConfig::Spot,
            base_currency: "BTC".to_string(),
            quote_currency: "USDT".to_string(),
            taker_fee: 0.0,
            ..InstrumentConfig::default()
        };
        let marks = HashMap::from([("BTC-USDT".to_string(), 110.0)]);
        let mut portfolio = Portfolio::new(&[instrument]);

        portfolio.apply_fill(&fill("BTC-USDT", Side::Buy, 1.0, 100.0), &HashMap::new());

        assert_eq!(portfolio.equity_usd(&marks, &HashMap::new()), 10.0);
        assert_eq!(portfolio.equity_usd_checked(&marks, &HashMap::new()), None);
        assert_eq!(
            portfolio.missing_currency_rates(&HashMap::new()),
            vec!["USDT".to_string()]
        );
        assert_eq!(portfolio.currency_conversion_failures(), 1);
    }

    #[test]
    fn linear_funding_settles_and_converts_in_settlement_currency() {
        let instrument = InstrumentConfig {
            symbol: "BTC-USDT-SWAP".to_string(),
            kind: InstrumentKindConfig::LinearSwap,
            base_currency: "BTC".to_string(),
            quote_currency: "USDT".to_string(),
            settle_currency: "USDT".to_string(),
            contract_value: 0.01,
            taker_fee: 0.0,
            ..InstrumentConfig::default()
        };
        let rates = HashMap::from([("USDT".to_string(), 0.95)]);
        let marks = HashMap::from([("BTC-USDT-SWAP".to_string(), 50_000.0)]);
        let mut portfolio = Portfolio::new(&[instrument]);
        portfolio.apply_fill(&fill("BTC-USDT-SWAP", Side::Buy, 10.0, 50_000.0), &rates);

        let funding_pnl = portfolio
            .apply_funding("BTC-USDT-SWAP", 0.001, 50_000.0, &rates)
            .unwrap();

        assert_eq!(funding_pnl, -4.75);
        assert_eq!(portfolio.funding_pnl_usd(), -4.75);
        assert_eq!(portfolio.equity_usd_checked(&marks, &rates), Some(-4.75));
    }

    #[test]
    fn inverse_fee_is_booked_in_coin_and_valued_through_quote_currency() {
        let instrument = InstrumentConfig {
            symbol: "BTC-USDT-SWAP".to_string(),
            kind: InstrumentKindConfig::InverseSwap,
            base_currency: "BTC".to_string(),
            quote_currency: "USDT".to_string(),
            settle_currency: "BTC".to_string(),
            contract_value: 100.0,
            taker_fee: 0.001,
            ..InstrumentConfig::default()
        };
        let rates = HashMap::from([("USDT".to_string(), 0.95)]);
        let marks = HashMap::from([("BTC-USDT-SWAP".to_string(), 50_000.0)]);
        let mut portfolio = Portfolio::new(&[instrument]);

        portfolio.apply_fill(&fill("BTC-USDT-SWAP", Side::Buy, 10.0, 50_000.0), &rates);

        assert!((portfolio.fee_cost_usd() - 0.95).abs() < 1e-12);
        assert!((portfolio.turnover_usd() - 950.0).abs() < 1e-12);
        assert!((portfolio.equity_usd_checked(&marks, &rates).unwrap() + 0.95).abs() < 1e-12);
    }
}
