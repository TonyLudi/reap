mod chaos;

pub use chaos::{
    ChaosConfig, ChaosStrategy, CoinConfig, ConfigValidation, HaltIntervalConfig, InstrumentConfig,
    InstrumentKindConfig, MissedHedge, RiskGroupConfig, RiskGroupKindConfig, SkewTypeConfig,
};

use reap_core::{OrderIntent, StrategyEvent};

pub trait Strategy {
    fn on_event(&mut self, event: &StrategyEvent) -> Vec<OrderIntent>;

    fn on_owned_event(&mut self, event: StrategyEvent) -> Vec<OrderIntent> {
        self.on_event(&event)
    }
}
