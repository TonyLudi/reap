mod chaos;

pub use chaos::{
    ChaosConfig, ChaosStrategy, InstrumentConfig, InstrumentKindConfig, RiskGroupConfig,
};

use crate::types::{MarketEvent, OrderUpdate, StrategyCommand};

pub trait Strategy {
    fn on_market_event(&mut self, event: &MarketEvent) -> Vec<StrategyCommand>;

    fn on_order_update(&mut self, update: &OrderUpdate) -> Vec<StrategyCommand>;
}
