mod chaos;

pub use chaos::{
    ChaosConfig, ChaosStrategy, CoinConfig, ConfigValidation, HaltIntervalConfig, InstrumentConfig,
    InstrumentKindConfig, MissedHedge, RiskGroupConfig, RiskGroupKindConfig, SkewTypeConfig,
};

use reap_core::{OrderIntent, StrategyEvent};

pub trait Strategy {
    fn on_event(&mut self, event: &StrategyEvent) -> Vec<OrderIntent>;

    /// A terminal strategy safety stop that the engine must promote to global risk.
    fn safety_halt_reason(&self) -> Option<&str> {
        None
    }

    fn on_owned_event(&mut self, event: StrategyEvent) -> Vec<OrderIntent> {
        self.on_event(&event)
    }
}
