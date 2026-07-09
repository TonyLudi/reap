mod chaos;

pub use chaos::{
    ChaosConfig, ChaosStrategy, InstrumentConfig, InstrumentKindConfig, RiskGroupConfig,
};

use reap_core::{OrderIntent, StrategyEvent};

pub trait Strategy {
    fn on_event(&mut self, event: &StrategyEvent) -> Vec<OrderIntent>;
}
