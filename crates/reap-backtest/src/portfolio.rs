use std::collections::{BTreeMap, BTreeSet, HashMap};

use reap_core::{FillLiquidity, OrderUpdate, Symbol};
use reap_strategy::InstrumentConfig;

#[derive(Debug, Clone)]
pub struct Portfolio {
    instruments: HashMap<Symbol, InstrumentConfig>,
    positions: HashMap<Symbol, f64>,
    inverse_cash_coin: HashMap<Symbol, f64>,
    cash_by_currency: HashMap<String, f64>,
    fee_cost_usd: f64,
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
            positions: HashMap::new(),
            inverse_cash_coin: HashMap::new(),
            cash_by_currency: HashMap::new(),
            fee_cost_usd: 0.0,
            funding_pnl_usd: 0.0,
            turnover_usd: 0.0,
            currency_conversion_failures: 0,
            invalid_accounting_events: 0,
        }
    }

    pub fn apply_fill(&mut self, update: &OrderUpdate, currency_rates_usd: &HashMap<String, f64>) {
        if !update.has_fill() {
            return;
        }
        let Some(inst) = self.instruments.get(&update.symbol) else {
            self.invalid_accounting_events = self.invalid_accounting_events.saturating_add(1);
            return;
        };
        let kind = inst.kind;
        let contract_value = inst.contract_value;
        let maker_fee = inst.maker_fee;
        let taker_fee = inst.taker_fee;
        let accounting_currency = instrument_accounting_currency(inst);
        if !update.last_fill_qty.is_finite()
            || !update.last_fill_price.is_finite()
            || update.last_fill_price <= 0.0
        {
            self.invalid_accounting_events = self.invalid_accounting_events.saturating_add(1);
            return;
        }
        let signed_qty = update.side.factor() * update.last_fill_qty;
        *self.positions.entry(update.symbol.clone()).or_default() += signed_qty;

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

        let fee_rate = match update.last_fill_liquidity {
            Some(FillLiquidity::Maker) => maker_fee,
            Some(FillLiquidity::Taker) => taker_fee,
            None => 0.0,
        };
        let absolute_notional = notional.abs();
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

        let (currency_rate, complete) =
            currency_rate_or_par(&accounting_currency, currency_rates_usd);
        if !complete {
            self.currency_conversion_failures = self.currency_conversion_failures.saturating_add(1);
        }
        self.fee_cost_usd += fee_cost * currency_rate;
        self.turnover_usd += absolute_notional * currency_rate;
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

    pub fn positions(&self) -> &HashMap<Symbol, f64> {
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

        portfolio.apply_fill(&fill("BTC-USDT", Side::Buy, 2.0, 100.0), &HashMap::new());

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
