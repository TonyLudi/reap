use std::collections::{HashMap, HashSet};
use std::fmt;

use reap_core::{AccountUpdate, FillKey};
use reap_risk::{InstrumentOrderLimits, InstrumentRiskModel};
use reap_strategy::{InstrumentConfig, InstrumentKindConfig};
use reap_venue::okx::{
    OkxAccountBalanceSnapshot, OkxAccountConfig, OkxAccountPositionsSnapshot, OkxContractType,
    OkxInstrument, OkxInstrumentType,
};
use reap_venue::{RemoteFill, RemoteOrder};
use serde::{Deserialize, Serialize};

use crate::{LiveConfig, OkxTradeModeConfig};

#[derive(Debug, Clone)]
pub struct AccountBootstrapSnapshot {
    pub account_config: OkxAccountConfig,
    pub instruments: HashMap<String, OkxInstrument>,
    pub balance_economics: OkxAccountBalanceSnapshot,
    pub position_risks: OkxAccountPositionsSnapshot,
    pub balance: AccountUpdate,
    pub positions: AccountUpdate,
    pub open_orders: Vec<RemoteOrder>,
    pub recent_fills: Vec<RemoteFill>,
}

impl AccountBootstrapSnapshot {
    pub fn scoped_account_update(&self, account_id: &str) -> AccountUpdate {
        let mut balances = self.balance.balances.clone();
        for balance in &mut balances {
            balance.account_id = Some(account_id.to_string());
        }
        let mut margins = self.balance.margins.clone();
        for margin in &mut margins {
            margin.account_id = Some(account_id.to_string());
        }
        let mut positions = self.balance.positions.clone();
        positions.extend(self.positions.positions.clone());
        AccountUpdate {
            ts_ms: self.balance.ts_ms.max(self.positions.ts_ms),
            balances,
            positions,
            margins,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VerifiedInstrument {
    pub account_id: String,
    pub symbol: String,
    pub instrument_type: OkxInstrumentType,
    pub trade_mode: OkxTradeModeConfig,
    pub risk_model: InstrumentRiskModel,
    pub order_limits: InstrumentOrderLimits,
    pub tick_size: f64,
    pub lot_size: f64,
    pub min_size: f64,
    pub contract_value: Option<f64>,
}

#[derive(Debug, Clone)]
pub struct VerifiedBootstrap {
    pub instruments: HashMap<String, VerifiedInstrument>,
    pub account_updates: HashMap<String, AccountUpdate>,
    pub baseline_fill_ids: HashMap<String, HashSet<FillKey>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BootstrapValidation {
    pub errors: Vec<String>,
}

impl fmt::Display for BootstrapValidation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.errors.join("; "))
    }
}

impl std::error::Error for BootstrapValidation {}

pub fn verify_bootstrap(
    config: &LiveConfig,
    snapshots: &HashMap<String, AccountBootstrapSnapshot>,
) -> Result<VerifiedBootstrap, BootstrapValidation> {
    let mut errors = Vec::new();
    let mut verified = HashMap::new();
    let mut account_updates = HashMap::new();
    let mut baseline_fill_ids = HashMap::new();

    for account in &config.accounts {
        let Some(snapshot) = snapshots.get(&account.id) else {
            errors.push(format!(
                "missing bootstrap snapshot for account {}",
                account.id
            ));
            continue;
        };
        if snapshot.account_config.account_level != account.expected_account_level {
            errors.push(format!(
                "account {} level mismatch: configured {:?}, exchange {:?}",
                account.id, account.expected_account_level, snapshot.account_config.account_level
            ));
        }
        if snapshot.account_config.position_mode != account.expected_position_mode {
            errors.push(format!(
                "account {} position mode mismatch: configured {:?}, exchange {:?}",
                account.id, account.expected_position_mode, snapshot.account_config.position_mode
            ));
        }
        if snapshot.account_config.user_id.trim().is_empty()
            || snapshot.account_config.main_user_id.trim().is_empty()
        {
            errors.push(format!(
                "account {} returned no stable OKX user identity during bootstrap",
                account.id
            ));
        }
        if snapshot.balance.balances.is_empty() {
            errors.push(format!(
                "account {} returned no balances during bootstrap",
                account.id
            ));
        }
        let cash_policy = crate::evaluate_account_cash_policy(
            config,
            &account.id,
            &snapshot.account_config,
            &snapshot.balance_economics,
            &snapshot.position_risks,
        );
        errors.extend(
            cash_policy
                .violations
                .into_iter()
                .map(|error| format!("account {} cash policy violation: {error}", account.id)),
        );
        let account_update = snapshot.scoped_account_update(&account.id);
        errors.extend(
            config
                .account_state_policy_errors(&account.id, &account_update)
                .into_iter()
                .map(|error| format!("account {} state policy violation: {error}", account.id)),
        );
        account_updates.insert(account.id.clone(), account_update);
        baseline_fill_ids.insert(
            account.id.clone(),
            snapshot
                .recent_fills
                .iter()
                .filter(|fill| !fill.fill_id.is_empty())
                .map(|fill| FillKey::new(fill.symbol.clone(), fill.fill_id.clone()))
                .collect(),
        );

        for instrument in config.instruments_for_account(&account.id) {
            let Some(metadata) = snapshot.instruments.get(&instrument.symbol) else {
                errors.push(format!(
                    "account {} is missing exchange metadata for {}",
                    account.id, instrument.symbol
                ));
                continue;
            };
            let Some(trade_mode) = account.trade_modes.get(&instrument.symbol).copied() else {
                continue;
            };
            verify_instrument(instrument, metadata, trade_mode, &account.id, &mut errors);
            if instrument_errors(instrument, metadata, trade_mode).is_empty() {
                verified.insert(
                    instrument.symbol.clone(),
                    VerifiedInstrument {
                        account_id: account.id.clone(),
                        symbol: instrument.symbol.clone(),
                        instrument_type: metadata.instrument_type,
                        trade_mode,
                        risk_model: risk_model(instrument),
                        order_limits: InstrumentOrderLimits {
                            max_limit_quantity: metadata.max_limit_size,
                            max_limit_notional_usd: metadata.max_limit_amount_usd,
                        },
                        tick_size: metadata.tick_size,
                        lot_size: metadata.lot_size,
                        min_size: metadata.min_size,
                        contract_value: metadata.contract_value,
                    },
                );
            }
        }
    }

    for account_id in snapshots.keys() {
        if config.account(account_id).is_none() {
            errors.push(format!(
                "received bootstrap snapshot for unknown account {account_id}"
            ));
        }
    }
    errors.sort();
    errors.dedup();
    if errors.is_empty() {
        Ok(VerifiedBootstrap {
            instruments: verified,
            account_updates,
            baseline_fill_ids,
        })
    } else {
        Err(BootstrapValidation { errors })
    }
}

fn verify_instrument(
    configured: &InstrumentConfig,
    exchange: &OkxInstrument,
    trade_mode: OkxTradeModeConfig,
    account_id: &str,
    errors: &mut Vec<String>,
) {
    errors.extend(
        instrument_errors(configured, exchange, trade_mode)
            .into_iter()
            .map(|error| format!("account {account_id} {}: {error}", configured.symbol)),
    );
}

fn instrument_errors(
    configured: &InstrumentConfig,
    exchange: &OkxInstrument,
    trade_mode: OkxTradeModeConfig,
) -> Vec<String> {
    let mut errors = Vec::new();
    if exchange.symbol != configured.symbol {
        errors.push(format!(
            "symbol mismatch: exchange returned {}",
            exchange.symbol
        ));
    }
    let expected_type = okx_instrument_type(configured.kind);
    if exchange.instrument_type != expected_type {
        errors.push(format!(
            "instrument type mismatch: configured {:?}, exchange {:?}",
            expected_type, exchange.instrument_type
        ));
    }
    let expected_contract = expected_contract_type(configured.kind);
    if exchange.contract_type != expected_contract {
        errors.push(format!(
            "contract type mismatch: configured {:?}, exchange {:?}",
            expected_contract, exchange.contract_type
        ));
    }
    if exchange.state != "live" {
        errors.push(format!(
            "instrument state is {} instead of live",
            exchange.state
        ));
    }
    if exchange.trade_fee_group_id.trim().is_empty() {
        errors.push("exchange returned no trade-fee groupId".to_string());
    }
    if configured.kind.is_derivative() && exchange.instrument_family.trim().is_empty() {
        errors.push("exchange returned no derivative instFamily".to_string());
    }
    compare_number(
        "tick_size",
        configured.tick_size,
        exchange.tick_size,
        &mut errors,
    );
    compare_number(
        "lot_size",
        configured.lot_size,
        exchange.lot_size,
        &mut errors,
    );
    if configured.min_trade_size + exchange.lot_size * 1e-9 < exchange.min_size {
        errors.push(format!(
            "min_trade_size {} is below exchange minimum {}",
            configured.min_trade_size, exchange.min_size
        ));
    }
    let lot_units = configured.min_trade_size / exchange.lot_size;
    if (lot_units - lot_units.round()).abs() > 1e-9 {
        errors.push(format!(
            "min_trade_size {} is not aligned to exchange lot size {}",
            configured.min_trade_size, exchange.lot_size
        ));
    }
    if configured.max_order_size > exchange.max_limit_size + exchange.lot_size * 1e-9 {
        errors.push(format!(
            "max_order_size {} exceeds exchange limit-order maximum {}",
            configured.max_order_size, exchange.max_limit_size
        ));
    }
    if configured.kind.is_spot() {
        match exchange.max_limit_amount_usd {
            Some(limit) if configured.max_order_size_usd > limit + limit.abs().max(1.0) * 1e-12 => {
                errors.push(format!(
                    "max_order_size_usd {} exceeds exchange limit-order amount maximum {}",
                    configured.max_order_size_usd, limit
                ));
            }
            Some(_) => {}
            None => errors.push("exchange omitted spot maxLmtAmt".to_string()),
        }
    }
    if configured.kind.is_derivative() {
        match exchange.contract_value {
            Some(value) => compare_number(
                "contract_value",
                configured.contract_value,
                value,
                &mut errors,
            ),
            None => errors.push("exchange omitted derivative contract value".to_string()),
        }
        if trade_mode == OkxTradeModeConfig::Cash {
            errors.push("derivative instrument cannot use cash trade mode".to_string());
        }
    }
    let derived_base = exchange
        .underlying
        .split_once('-')
        .map(|(base, _)| base)
        .unwrap_or("");
    let derived_quote = exchange
        .underlying
        .split_once('-')
        .map(|(_, quote)| quote)
        .unwrap_or("");
    compare_text(
        "base_currency",
        &configured.base_currency,
        if exchange.base_currency.is_empty() {
            derived_base
        } else {
            &exchange.base_currency
        },
        &mut errors,
    );
    compare_text(
        "quote_currency",
        &configured.quote_currency,
        if exchange.quote_currency.is_empty() {
            derived_quote
        } else {
            &exchange.quote_currency
        },
        &mut errors,
    );
    compare_text(
        "settle_currency",
        &configured.settle_currency,
        &exchange.settle_currency,
        &mut errors,
    );
    errors
}

pub fn okx_instrument_type(kind: InstrumentKindConfig) -> OkxInstrumentType {
    match kind {
        InstrumentKindConfig::Spot => OkxInstrumentType::Spot,
        InstrumentKindConfig::Future
        | InstrumentKindConfig::LinearFuture
        | InstrumentKindConfig::InverseFuture => OkxInstrumentType::Futures,
        InstrumentKindConfig::LinearSwap | InstrumentKindConfig::InverseSwap => {
            OkxInstrumentType::Swap
        }
    }
}

fn expected_contract_type(kind: InstrumentKindConfig) -> Option<OkxContractType> {
    match kind {
        InstrumentKindConfig::Spot => None,
        InstrumentKindConfig::Future
        | InstrumentKindConfig::LinearFuture
        | InstrumentKindConfig::LinearSwap => Some(OkxContractType::Linear),
        InstrumentKindConfig::InverseFuture | InstrumentKindConfig::InverseSwap => {
            Some(OkxContractType::Inverse)
        }
    }
}

fn risk_model(instrument: &InstrumentConfig) -> InstrumentRiskModel {
    if instrument.kind.is_spot() {
        InstrumentRiskModel::Spot
    } else if instrument.kind.is_inverse() {
        InstrumentRiskModel::InverseDerivative {
            contract_value: instrument.contract_value,
        }
    } else {
        InstrumentRiskModel::LinearDerivative {
            contract_value: instrument.contract_value,
        }
    }
}

fn compare_number(name: &str, configured: f64, exchange: f64, errors: &mut Vec<String>) {
    let tolerance = configured.abs().max(exchange.abs()).max(1.0) * 1e-12;
    if (configured - exchange).abs() > tolerance {
        errors.push(format!(
            "{name} mismatch: configured {configured}, exchange {exchange}"
        ));
    }
}

fn compare_text(name: &str, configured: &str, exchange: &str, errors: &mut Vec<String>) {
    if !configured.is_empty() && configured != exchange {
        errors.push(format!(
            "{name} mismatch: configured {configured}, exchange {exchange}"
        ));
    }
}

#[cfg(test)]
mod tests {
    use reap_core::{Balance, MarginSnapshot, Position, PositionMarginMode};
    use reap_risk::RiskLimits;
    use reap_strategy::ChaosConfig;
    use reap_venue::okx::{
        OkxAccountBalanceSnapshot, OkxAccountLevel, OkxAccountPositionsSnapshot, OkxBalanceDetail,
        OkxPositionMode,
    };

    use crate::{
        LiveAccountConfig, LiveStorageConfig, OkxTradeModeConfig, OkxVenueConfig, RuntimeConfig,
    };

    use super::*;

    fn config() -> LiveConfig {
        let mut strategy: ChaosConfig =
            toml::from_str(include_str!("../../../examples/iarb2-basic.toml")).unwrap();
        strategy.risk_groups[0].account_id = Some("main".to_string());
        strategy.instruments[1].kind = InstrumentKindConfig::LinearSwap;
        strategy.instruments[1].symbol = "BTC-USDT-SWAP".to_string();
        strategy.instruments[1].risk_group = "main".to_string();
        strategy.risk_groups[0].symbols[1] = "BTC-USDT-SWAP".to_string();
        strategy.instruments[1].base_currency = "BTC".to_string();
        strategy.instruments[1].quote_currency = "USDT".to_string();
        strategy.instruments[1].settle_currency = "USDT".to_string();
        LiveConfig {
            strategy,
            risk: RiskLimits::default(),
            venue: OkxVenueConfig::default(),
            runtime: RuntimeConfig::default(),
            storage: LiveStorageConfig::default(),
            operator: crate::OperatorConfig::default(),
            alerts: crate::AlertConfig::default(),
            host_guard: crate::HostGuardConfig::default(),
            accounts: vec![LiveAccountConfig {
                id: "main".to_string(),
                api_key_env: "KEY".to_string(),
                secret_key_env: "SECRET".to_string(),
                passphrase_env: "PASS".to_string(),
                expected_account_level: OkxAccountLevel::SingleCurrencyMargin,
                expected_position_mode: OkxPositionMode::NetMode,
                id_prefix: "reap".to_string(),
                node_id: 1,
                trade_modes: HashMap::from([
                    ("BTC-USDT".to_string(), OkxTradeModeConfig::Cash),
                    ("BTC-USDT-SWAP".to_string(), OkxTradeModeConfig::Cross),
                ]),
            }],
        }
    }

    fn snapshot() -> AccountBootstrapSnapshot {
        AccountBootstrapSnapshot {
            account_config: OkxAccountConfig {
                account_level: reap_venue::okx::OkxAccountLevel::SingleCurrencyMargin,
                position_mode: reap_venue::okx::OkxPositionMode::NetMode,
                account_stp_mode: "cancel_maker".to_string(),
                user_id: "7".to_string(),
                main_user_id: "6".to_string(),
                enable_spot_borrow: Some(false),
                auto_loan: Some(false),
                spot_borrow_auto_repay: Some(false),
            },
            instruments: HashMap::from([
                (
                    "BTC-USDT".to_string(),
                    OkxInstrument {
                        symbol: "BTC-USDT".to_string(),
                        instrument_type: OkxInstrumentType::Spot,
                        instrument_family: "".to_string(),
                        trade_fee_group_id: "1".to_string(),
                        underlying: "".to_string(),
                        base_currency: "BTC".to_string(),
                        quote_currency: "USDT".to_string(),
                        settle_currency: "".to_string(),
                        contract_type: None,
                        contract_value: None,
                        contract_value_currency: "".to_string(),
                        tick_size: 0.1,
                        lot_size: 0.0001,
                        min_size: 0.0001,
                        max_limit_size: 100.0,
                        max_market_size: 1_000_000.0,
                        max_limit_amount_usd: Some(1_000_000.0),
                        max_market_amount_usd: Some(1_000_000.0),
                        state: "live".to_string(),
                        upcoming_changes: Vec::new(),
                    },
                ),
                (
                    "BTC-USDT-SWAP".to_string(),
                    OkxInstrument {
                        symbol: "BTC-USDT-SWAP".to_string(),
                        instrument_type: OkxInstrumentType::Swap,
                        instrument_family: "BTC-USDT".to_string(),
                        trade_fee_group_id: "2".to_string(),
                        underlying: "BTC-USDT".to_string(),
                        base_currency: "BTC".to_string(),
                        quote_currency: "USDT".to_string(),
                        settle_currency: "USDT".to_string(),
                        contract_type: Some(OkxContractType::Linear),
                        contract_value: Some(0.001),
                        contract_value_currency: "BTC".to_string(),
                        tick_size: 0.1,
                        lot_size: 1.0,
                        min_size: 1.0,
                        max_limit_size: 1_000_000.0,
                        max_market_size: 1_000_000.0,
                        max_limit_amount_usd: None,
                        max_market_amount_usd: None,
                        state: "live".to_string(),
                        upcoming_changes: Vec::new(),
                    },
                ),
            ]),
            balance_economics: OkxAccountBalanceSnapshot {
                update_time_ms: 1,
                total_equity_usd: Some(10_000.0),
                adjusted_equity_usd: Some(10_000.0),
                borrow_frozen_usd: None,
                notional_usd_for_borrow: None,
                margin_ratio: Some(10.0),
                notional_usd: Some(0.0),
                details: vec![OkxBalanceDetail {
                    currency: "USDT".to_string(),
                    update_time_ms: 1,
                    cash_balance: Some(10_000.0),
                    available_balance: Some(9_000.0),
                    equity: Some(10_000.0),
                    equity_usd: Some(10_000.0),
                    discounted_equity_usd: Some(10_000.0),
                    unrealized_pnl: Some(0.0),
                    liability: None,
                    cross_liability: None,
                    isolated_liability: None,
                    unrealized_loss_liability: None,
                    accrued_interest: None,
                    borrow_frozen_usd: None,
                    max_loan: None,
                    forced_repayment_indicator: None,
                }],
            },
            position_risks: OkxAccountPositionsSnapshot {
                update_time_ms: 1,
                positions: Vec::new(),
            },
            balance: AccountUpdate {
                ts_ms: 1,
                balances: vec![Balance {
                    account_id: None,
                    currency: "USDT".to_string(),
                    total: 10_000.0,
                    available: 9_000.0,
                    equity: 10_000.0,
                    liability: 0.0,
                    max_loan: 0.0,
                    forced_repayment_indicator: None,
                }],
                positions: Vec::new(),
                margins: vec![MarginSnapshot {
                    account_id: None,
                    ratio: None,
                    exchange_ratio: Some(10.0),
                    adjusted_equity_usd: Some(10_000.0),
                    notional_usd: Some(0.0),
                }],
            },
            positions: AccountUpdate {
                ts_ms: 1,
                balances: Vec::new(),
                positions: Vec::new(),
                margins: Vec::new(),
            },
            open_orders: Vec::new(),
            recent_fills: Vec::new(),
        }
    }

    #[test]
    fn verifies_account_metadata_and_builds_risk_models() {
        let config = config();
        assert!(config.validate().valid, "{:?}", config.validate().errors);
        let verified =
            verify_bootstrap(&config, &HashMap::from([("main".to_string(), snapshot())])).unwrap();

        assert_eq!(verified.instruments.len(), 2);
        assert!(matches!(
            verified.instruments["BTC-USDT-SWAP"].risk_model,
            InstrumentRiskModel::LinearDerivative {
                contract_value: 0.001
            }
        ));
        assert_eq!(
            verified.instruments["BTC-USDT"].order_limits,
            InstrumentOrderLimits {
                max_limit_quantity: 100.0,
                max_limit_notional_usd: Some(1_000_000.0),
            }
        );
        assert_eq!(
            verified.account_updates["main"].balances[0]
                .account_id
                .as_deref(),
            Some("main")
        );
    }

    #[test]
    fn rejects_missing_exchange_account_identity() {
        let config = config();
        let mut snapshot = snapshot();
        snapshot.account_config.user_id.clear();

        let error = verify_bootstrap(&config, &HashMap::from([("main".to_string(), snapshot)]))
            .unwrap_err();

        assert!(error.to_string().contains("stable OKX user identity"));
    }

    #[test]
    fn rejects_nonzero_economic_liability_before_live_startup() {
        let config = config();
        let mut snapshot = snapshot();
        snapshot.balance_economics.details[0].liability = Some(0.01);

        let error = verify_bootstrap(&config, &HashMap::from([("main".to_string(), snapshot)]))
            .unwrap_err();

        assert!(error.to_string().contains("cash policy violation"));
        assert!(error.to_string().contains("liab is nonzero"));
    }

    #[test]
    fn rejects_metadata_drift() {
        let config = config();
        let mut snapshot = snapshot();
        snapshot.instruments.get_mut("BTC-USDT").unwrap().tick_size = 0.01;
        let error = verify_bootstrap(&config, &HashMap::from([("main".to_string(), snapshot)]))
            .unwrap_err();
        assert!(error.to_string().contains("tick_size mismatch"));
    }

    #[test]
    fn rejects_strategy_order_sizes_above_exchange_maxima() {
        let config = config();
        let mut snapshot = snapshot();
        let spot = snapshot.instruments.get_mut("BTC-USDT").unwrap();
        spot.max_limit_size = 0.5;
        spot.max_limit_amount_usd = Some(4_000.0);

        let error = verify_bootstrap(&config, &HashMap::from([("main".to_string(), snapshot)]))
            .unwrap_err()
            .to_string();

        assert!(error.contains("max_order_size 1 exceeds exchange limit-order maximum 0.5"));
        assert!(
            error.contains(
                "max_order_size_usd 5000 exceeds exchange limit-order amount maximum 4000"
            )
        );
    }

    #[test]
    fn rejects_missing_current_fee_group_metadata() {
        let config = config();
        let mut snapshot = snapshot();
        snapshot
            .instruments
            .get_mut("BTC-USDT-SWAP")
            .unwrap()
            .trade_fee_group_id
            .clear();

        let error = verify_bootstrap(&config, &HashMap::from([("main".to_string(), snapshot)]))
            .unwrap_err();

        assert!(error.to_string().contains("trade-fee groupId"));
    }

    #[test]
    fn rejects_nonzero_position_with_wrong_margin_mode() {
        let config = config();
        let mut snapshot = snapshot();
        snapshot.positions.positions.push(Position {
            symbol: "BTC-USDT-SWAP".to_string(),
            qty: 2.0,
            avg_price: 50_000.0,
            margin_mode: Some(PositionMarginMode::Isolated),
        });

        let error = verify_bootstrap(&config, &HashMap::from([("main".to_string(), snapshot)]))
            .unwrap_err();

        assert!(
            error
                .to_string()
                .contains("BTC-USDT-SWAP expected Cross, received Isolated")
        );
    }

    #[test]
    fn rejects_unmanaged_nonzero_position() {
        let config = config();
        let mut snapshot = snapshot();
        snapshot.positions.positions.push(Position {
            symbol: "ETH-USDT-SWAP".to_string(),
            qty: 1.0,
            avg_price: 3_000.0,
            margin_mode: Some(PositionMarginMode::Cross),
        });

        let error = verify_bootstrap(&config, &HashMap::from([("main".to_string(), snapshot)]))
            .unwrap_err();

        assert!(
            error
                .to_string()
                .contains("unmanaged nonzero position ETH-USDT-SWAP qty=1")
        );
    }

    #[test]
    fn rejects_forced_repayment_indicator_at_limit() {
        let config = config();
        let mut snapshot = snapshot();
        snapshot.balance.balances[0].forced_repayment_indicator = Some(1);

        let error = verify_bootstrap(&config, &HashMap::from([("main".to_string(), snapshot)]))
            .unwrap_err();

        assert!(
            error
                .to_string()
                .contains("currency USDT forced repayment indicator 1 reached limit 1")
        );
    }
}
