use std::collections::{BTreeMap, HashSet};

use reap_core::{MarketEvent, TimeMs};
use reap_strategy::ReferenceDataKind;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::LiveConfig;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LivePhase {
    Configured,
    Reconciling,
    AwaitingStreams,
    Ready,
    Degraded,
    Stopping,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadinessSnapshot {
    pub phase: LivePhase,
    pub metadata_verified: bool,
    pub storage_ready: bool,
    pub public_connectivity_ready: bool,
    pub missing_reconciliation: Vec<String>,
    pub missing_account_snapshots: Vec<String>,
    pub missing_books: Vec<String>,
    pub missing_private_streams: Vec<String>,
    #[serde(default)]
    pub missing_order_transports: Vec<String>,
    #[serde(default)]
    pub missing_stablecoin_rates: Vec<String>,
    #[serde(default)]
    pub missing_strategy_references: Vec<String>,
    pub faults: BTreeMap<String, String>,
}

impl ReadinessSnapshot {
    pub fn is_ready(&self) -> bool {
        self.phase == LivePhase::Ready
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum StartupError {
    #[error("unknown required account {0}")]
    UnknownAccount(String),
    #[error("unknown required symbol {0}")]
    UnknownSymbol(String),
    #[error("unknown required stablecoin reference {0}")]
    UnknownStablecoinReference(String),
}

#[derive(Debug, Clone)]
pub struct StartupGate {
    phase: LivePhase,
    required_symbols: HashSet<String>,
    required_accounts: HashSet<String>,
    required_order_transports: HashSet<String>,
    required_stablecoin_rates: HashSet<String>,
    required_strategy_references: BTreeMap<String, StrategyReferenceReadiness>,
    metadata_verified: bool,
    storage_ready: bool,
    public_connectivity_ready: bool,
    reconciled_accounts: HashSet<String>,
    account_snapshots: HashSet<String>,
    ready_books: HashSet<String>,
    ready_private_streams: HashSet<String>,
    ready_order_transports: HashSet<String>,
    ready_stablecoin_rates: HashSet<String>,
    ready_strategy_references: HashSet<String>,
    faults: BTreeMap<String, String>,
    was_ready: bool,
}

#[derive(Debug, Clone)]
struct StrategyReferenceReadiness {
    max_age_ms: TimeMs,
    source_ts_ms: Option<TimeMs>,
}

impl StartupGate {
    pub fn new(config: &LiveConfig) -> Self {
        Self::new_with_order_transport(config, false)
    }

    pub fn new_with_order_transport(config: &LiveConfig, required: bool) -> Self {
        let required_accounts = config.required_accounts();
        let required_strategy_references = config
            .strategy
            .reference_data_requirements()
            .into_iter()
            .map(|requirement| {
                (
                    strategy_reference_key(requirement.kind, &requirement.symbol),
                    StrategyReferenceReadiness {
                        max_age_ms: requirement.max_age_ms,
                        source_ts_ms: None,
                    },
                )
            })
            .collect();
        Self {
            phase: LivePhase::Configured,
            required_symbols: config.required_symbols(),
            required_accounts: required_accounts.clone(),
            required_order_transports: if required {
                required_accounts
            } else {
                HashSet::new()
            },
            required_stablecoin_rates: config
                .risk
                .stablecoin_guards
                .iter()
                .map(|guard| guard.symbol.clone())
                .collect(),
            required_strategy_references,
            metadata_verified: false,
            storage_ready: false,
            public_connectivity_ready: false,
            reconciled_accounts: HashSet::new(),
            account_snapshots: HashSet::new(),
            ready_books: HashSet::new(),
            ready_private_streams: HashSet::new(),
            ready_order_transports: HashSet::new(),
            ready_stablecoin_rates: HashSet::new(),
            ready_strategy_references: HashSet::new(),
            faults: BTreeMap::new(),
            was_ready: false,
        }
    }

    pub fn phase(&self) -> LivePhase {
        self.phase
    }

    pub fn can_submit_new(&self, order_entry_enabled: bool) -> bool {
        order_entry_enabled && self.phase == LivePhase::Ready
    }

    pub fn can_cancel(&self) -> bool {
        self.phase != LivePhase::Stopping
    }

    pub fn strategy_references_ready(&self) -> bool {
        self.required_strategy_references
            .keys()
            .all(|key| self.ready_strategy_references.contains(key))
    }

    pub fn mark_metadata_verified(&mut self) {
        self.metadata_verified = true;
        self.faults.remove("metadata");
        self.refresh();
    }

    pub fn mark_metadata_failed(&mut self, reason: impl Into<String>) {
        self.metadata_verified = false;
        self.faults.insert("metadata".to_string(), reason.into());
        self.refresh();
    }

    pub fn mark_storage(&mut self, ready: bool, reason: impl Into<String>) {
        self.storage_ready = ready;
        set_fault(&mut self.faults, "storage", ready, reason);
        self.refresh();
    }

    pub fn mark_public_connectivity(&mut self, ready: bool, reason: impl Into<String>) {
        self.public_connectivity_ready = ready;
        if ready {
            self.faults.remove("public_connectivity");
        } else if self.was_ready {
            self.faults
                .insert("public_connectivity".to_string(), reason.into());
        }
        self.refresh();
    }

    pub fn mark_reconciled(
        &mut self,
        account_id: &str,
        clean: bool,
        reason: impl Into<String>,
    ) -> Result<(), StartupError> {
        self.require_account(account_id)?;
        if clean {
            self.reconciled_accounts.insert(account_id.to_string());
        } else {
            self.reconciled_accounts.remove(account_id);
        }
        set_fault(
            &mut self.faults,
            &format!("reconcile:{account_id}"),
            clean,
            reason,
        );
        self.refresh();
        Ok(())
    }

    pub fn mark_account_snapshot(
        &mut self,
        account_id: &str,
        ready: bool,
        reason: impl Into<String>,
    ) -> Result<(), StartupError> {
        self.require_account(account_id)?;
        set_membership(&mut self.account_snapshots, account_id, ready);
        set_fault(
            &mut self.faults,
            &format!("account_snapshot:{account_id}"),
            ready,
            reason,
        );
        self.refresh();
        Ok(())
    }

    pub fn mark_book(
        &mut self,
        symbol: &str,
        ready: bool,
        reason: impl Into<String>,
    ) -> Result<(), StartupError> {
        self.require_symbol(symbol)?;
        set_membership(&mut self.ready_books, symbol, ready);
        set_fault(&mut self.faults, &format!("book:{symbol}"), ready, reason);
        self.refresh();
        Ok(())
    }

    pub fn mark_private_stream(
        &mut self,
        account_id: &str,
        ready: bool,
        reason: impl Into<String>,
    ) -> Result<(), StartupError> {
        self.require_account(account_id)?;
        set_membership(&mut self.ready_private_streams, account_id, ready);
        set_fault(
            &mut self.faults,
            &format!("private:{account_id}"),
            ready,
            reason,
        );
        self.refresh();
        Ok(())
    }

    pub fn mark_order_transport(
        &mut self,
        account_id: &str,
        ready: bool,
        reason: impl Into<String>,
    ) -> Result<(), StartupError> {
        self.require_account(account_id)?;
        if !self.required_order_transports.contains(account_id) {
            return Ok(());
        }
        set_membership(&mut self.ready_order_transports, account_id, ready);
        set_fault(
            &mut self.faults,
            &format!("order_transport:{account_id}"),
            ready,
            reason,
        );
        self.refresh();
        Ok(())
    }

    pub fn mark_stablecoin_rate(
        &mut self,
        symbol: &str,
        healthy: bool,
        reason: impl Into<String>,
    ) -> Result<(), StartupError> {
        self.require_stablecoin_reference(symbol)?;
        set_membership(&mut self.ready_stablecoin_rates, symbol, healthy);
        let key = format!("stablecoin:{symbol}");
        if healthy {
            self.faults.remove(&key);
        } else if self.was_ready {
            self.faults.insert(key, reason.into());
        } else {
            self.faults.remove(&key);
        }
        self.refresh();
        Ok(())
    }

    pub fn observe_strategy_market(&mut self, event: &MarketEvent, observed_now_ms: TimeMs) {
        match event {
            MarketEvent::IndexPrice {
                ts_ms,
                symbol,
                price,
            } if price.is_finite() && *price > 0.0 => {
                self.observe_strategy_reference(ReferenceDataKind::IndexPrice, symbol, *ts_ms);
            }
            MarketEvent::FundingRate {
                ts_ms,
                symbol,
                rate,
                funding_time_ms,
                ..
            } if rate.is_finite() && *funding_time_ms > 0 => {
                self.observe_strategy_reference(ReferenceDataKind::FundingRate, symbol, *ts_ms);
            }
            MarketEvent::PriceLimits {
                ts_ms,
                symbol,
                mark_price,
                limit_down,
                limit_up,
            } => {
                if mark_price.is_finite() && *mark_price > 0.0 {
                    self.observe_strategy_reference(ReferenceDataKind::MarkPrice, symbol, *ts_ms);
                }
                if limit_down.is_finite()
                    && *limit_down > 0.0
                    && limit_up.is_finite()
                    && *limit_up > 0.0
                {
                    self.observe_strategy_reference(ReferenceDataKind::PriceLimits, symbol, *ts_ms);
                }
            }
            MarketEvent::Depth(_)
            | MarketEvent::Trade { .. }
            | MarketEvent::BurstSignal { .. }
            | MarketEvent::IndexPrice { .. }
            | MarketEvent::FundingRate { .. } => {}
        }
        self.refresh_strategy_references(observed_now_ms);
    }

    pub fn refresh_strategy_references(&mut self, now_ms: TimeMs) {
        for (key, state) in &self.required_strategy_references {
            let healthy = state.source_ts_ms.is_some_and(|source_ts_ms| {
                now_ms.saturating_sub(source_ts_ms) <= state.max_age_ms
            });
            set_membership(&mut self.ready_strategy_references, key, healthy);
            let fault_key = format!("strategy_reference:{key}");
            if healthy {
                self.faults.remove(&fault_key);
            } else if self.was_ready {
                let reason = state.source_ts_ms.map_or_else(
                    || format!("required strategy reference {key} has no valid observation"),
                    |source_ts_ms| {
                        format!(
                            "required strategy reference {key} is {}ms old (limit {}ms)",
                            now_ms.saturating_sub(source_ts_ms),
                            state.max_age_ms
                        )
                    },
                );
                self.faults.insert(fault_key, reason);
            } else {
                self.faults.remove(&fault_key);
            }
        }
        self.refresh();
    }

    fn observe_strategy_reference(
        &mut self,
        kind: ReferenceDataKind,
        symbol: &str,
        source_ts_ms: TimeMs,
    ) {
        let key = strategy_reference_key(kind, symbol);
        if let Some(state) = self.required_strategy_references.get_mut(&key) {
            state.source_ts_ms = Some(
                state
                    .source_ts_ms
                    .map_or(source_ts_ms, |current| current.max(source_ts_ms)),
            );
        }
    }

    pub fn mark_runtime_health(
        &mut self,
        component: &str,
        healthy: bool,
        reason: impl Into<String>,
    ) {
        set_fault(
            &mut self.faults,
            &format!("runtime:{component}"),
            healthy,
            reason,
        );
        self.refresh();
    }

    pub fn stop(&mut self) {
        self.phase = LivePhase::Stopping;
    }

    pub fn snapshot(&self) -> ReadinessSnapshot {
        ReadinessSnapshot {
            phase: self.phase,
            metadata_verified: self.metadata_verified,
            storage_ready: self.storage_ready,
            public_connectivity_ready: self.public_connectivity_ready,
            missing_reconciliation: missing(&self.required_accounts, &self.reconciled_accounts),
            missing_account_snapshots: missing(&self.required_accounts, &self.account_snapshots),
            missing_books: missing(&self.required_symbols, &self.ready_books),
            missing_private_streams: missing(&self.required_accounts, &self.ready_private_streams),
            missing_order_transports: missing(
                &self.required_order_transports,
                &self.ready_order_transports,
            ),
            missing_stablecoin_rates: missing(
                &self.required_stablecoin_rates,
                &self.ready_stablecoin_rates,
            ),
            missing_strategy_references: missing_map_keys(
                &self.required_strategy_references,
                &self.ready_strategy_references,
            ),
            faults: self.faults.clone(),
        }
    }

    fn refresh(&mut self) {
        if self.phase == LivePhase::Stopping {
            return;
        }
        if !self.faults.is_empty() {
            self.phase = LivePhase::Degraded;
            return;
        }
        if !self.metadata_verified {
            self.phase = LivePhase::Configured;
            return;
        }
        let accounts_bootstrapped = is_complete(&self.required_accounts, &self.reconciled_accounts)
            && is_complete(&self.required_accounts, &self.account_snapshots)
            && self.storage_ready;
        if !accounts_bootstrapped {
            self.phase = if self.was_ready {
                LivePhase::Degraded
            } else {
                LivePhase::Reconciling
            };
            return;
        }
        let streams_ready = is_complete(&self.required_symbols, &self.ready_books)
            && is_complete(&self.required_accounts, &self.ready_private_streams)
            && is_complete(
                &self.required_order_transports,
                &self.ready_order_transports,
            )
            && is_complete(
                &self.required_stablecoin_rates,
                &self.ready_stablecoin_rates,
            )
            && self.strategy_references_ready()
            && self.public_connectivity_ready;
        if streams_ready {
            self.phase = LivePhase::Ready;
            self.was_ready = true;
        } else {
            self.phase = if self.was_ready {
                LivePhase::Degraded
            } else {
                LivePhase::AwaitingStreams
            };
        }
    }

    fn require_account(&self, account_id: &str) -> Result<(), StartupError> {
        if self.required_accounts.contains(account_id) {
            Ok(())
        } else {
            Err(StartupError::UnknownAccount(account_id.to_string()))
        }
    }

    fn require_symbol(&self, symbol: &str) -> Result<(), StartupError> {
        if self.required_symbols.contains(symbol) {
            Ok(())
        } else {
            Err(StartupError::UnknownSymbol(symbol.to_string()))
        }
    }

    fn require_stablecoin_reference(&self, symbol: &str) -> Result<(), StartupError> {
        if self.required_stablecoin_rates.contains(symbol) {
            Ok(())
        } else {
            Err(StartupError::UnknownStablecoinReference(symbol.to_string()))
        }
    }
}

fn strategy_reference_key(kind: ReferenceDataKind, symbol: &str) -> String {
    format!("{}:{symbol}", kind.as_str())
}

fn missing_map_keys<T>(required: &BTreeMap<String, T>, ready: &HashSet<String>) -> Vec<String> {
    required
        .keys()
        .filter(|value| !ready.contains(*value))
        .cloned()
        .collect()
}

fn set_membership(values: &mut HashSet<String>, value: &str, present: bool) {
    if present {
        values.insert(value.to_string());
    } else {
        values.remove(value);
    }
}

fn set_fault(
    faults: &mut BTreeMap<String, String>,
    key: &str,
    healthy: bool,
    reason: impl Into<String>,
) {
    if healthy {
        faults.remove(key);
    } else {
        faults.insert(key.to_string(), reason.into());
    }
}

fn is_complete(required: &HashSet<String>, ready: &HashSet<String>) -> bool {
    required.is_subset(ready)
}

fn missing(required: &HashSet<String>, ready: &HashSet<String>) -> Vec<String> {
    let mut values = required.difference(ready).cloned().collect::<Vec<_>>();
    values.sort();
    values
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use reap_risk::{RiskLimits, StablecoinGuardConfig};
    use reap_strategy::ChaosConfig;
    use reap_venue::okx::{OkxAccountLevel, OkxPositionMode};

    use crate::{
        LiveAccountConfig, LiveStorageConfig, OkxTradeModeConfig, OkxVenueConfig, RuntimeConfig,
    };

    use super::*;

    fn config() -> LiveConfig {
        let mut strategy: ChaosConfig =
            toml::from_str(include_str!("../../../examples/iarb2-basic.toml")).unwrap();
        strategy.risk_groups[0].account_id = Some("main".to_string());
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
                api_key_policy: crate::OkxApiKeyPolicyConfig::default(),
                id_prefix: "reap".to_string(),
                node_id: 1,
                trade_modes: HashMap::from([
                    ("BTC-USDT".to_string(), OkxTradeModeConfig::Cash),
                    ("BTC-PERP".to_string(), OkxTradeModeConfig::Cross),
                ]),
            }],
        }
    }

    #[test]
    fn readiness_requires_every_bootstrap_and_stream_invariant() {
        let mut gate = StartupGate::new(&config());
        assert_eq!(gate.phase(), LivePhase::Configured);
        gate.mark_metadata_verified();
        assert_eq!(gate.phase(), LivePhase::Reconciling);
        gate.mark_storage(true, "open");
        gate.mark_public_connectivity(true, "connected");
        gate.mark_reconciled("main", true, "clean").unwrap();
        gate.mark_account_snapshot("main", true, "loaded").unwrap();
        assert_eq!(gate.phase(), LivePhase::AwaitingStreams);
        gate.mark_book("BTC-USDT", true, "snapshot").unwrap();
        gate.mark_book("BTC-PERP", true, "snapshot").unwrap();
        assert_eq!(gate.phase(), LivePhase::AwaitingStreams);
        gate.mark_private_stream("main", true, "heartbeat").unwrap();
        assert!(gate.snapshot().is_ready());
        assert!(gate.can_submit_new(true));
        assert!(!gate.can_submit_new(false));
    }

    #[test]
    fn ready_gate_degrades_immediately_and_recovers_explicitly() {
        let mut gate = StartupGate::new(&config());
        gate.mark_metadata_verified();
        gate.mark_storage(true, "open");
        gate.mark_public_connectivity(true, "connected");
        gate.mark_reconciled("main", true, "clean").unwrap();
        gate.mark_account_snapshot("main", true, "loaded").unwrap();
        gate.mark_book("BTC-USDT", true, "snapshot").unwrap();
        gate.mark_book("BTC-PERP", true, "snapshot").unwrap();
        gate.mark_private_stream("main", true, "heartbeat").unwrap();

        gate.mark_book("BTC-USDT", false, "sequence gap").unwrap();
        assert_eq!(gate.phase(), LivePhase::Degraded);
        assert!(!gate.can_submit_new(true));
        assert!(gate.can_cancel());
        gate.mark_book("BTC-USDT", true, "recovered snapshot")
            .unwrap();
        assert_eq!(gate.phase(), LivePhase::Ready);

        gate.mark_public_connectivity(false, "auxiliary channel disconnected");
        assert_eq!(gate.phase(), LivePhase::Degraded);
        gate.mark_public_connectivity(true, "redundant channel recovered");
        assert_eq!(gate.phase(), LivePhase::Ready);
    }

    #[test]
    fn tradable_gate_requires_every_account_order_transport() {
        let config = config();
        let mut gate = StartupGate::new_with_order_transport(&config, true);
        gate.mark_metadata_verified();
        gate.mark_storage(true, "open");
        gate.mark_public_connectivity(true, "connected");
        gate.mark_reconciled("main", true, "clean").unwrap();
        gate.mark_account_snapshot("main", true, "loaded").unwrap();
        gate.mark_book("BTC-USDT", true, "snapshot").unwrap();
        gate.mark_book("BTC-PERP", true, "snapshot").unwrap();
        gate.mark_private_stream("main", true, "heartbeat").unwrap();

        assert_eq!(gate.phase(), LivePhase::AwaitingStreams);
        assert_eq!(
            gate.snapshot().missing_order_transports,
            vec!["main".to_string()]
        );
        gate.mark_order_transport("main", true, "all sessions authenticated")
            .unwrap();
        assert_eq!(gate.phase(), LivePhase::Ready);

        gate.mark_order_transport("main", false, "one session disconnected")
            .unwrap();
        assert_eq!(gate.phase(), LivePhase::Degraded);
        assert!(!gate.can_submit_new(true));
        assert!(gate.can_cancel());
    }

    #[test]
    fn readiness_requires_and_monitors_stablecoin_references() {
        let mut config = config();
        config.risk.stablecoin_guards = vec![StablecoinGuardConfig {
            symbol: "USDT-USD".to_string(),
            max_downside_deviation: 0.01,
        }];
        let mut gate = StartupGate::new(&config);
        gate.mark_metadata_verified();
        gate.mark_storage(true, "open");
        gate.mark_public_connectivity(true, "connected");
        gate.mark_reconciled("main", true, "clean").unwrap();
        gate.mark_account_snapshot("main", true, "loaded").unwrap();
        gate.mark_book("BTC-USDT", true, "snapshot").unwrap();
        gate.mark_book("BTC-PERP", true, "snapshot").unwrap();
        gate.mark_private_stream("main", true, "heartbeat").unwrap();

        assert_eq!(gate.phase(), LivePhase::AwaitingStreams);
        assert_eq!(
            gate.snapshot().missing_stablecoin_rates,
            vec!["USDT-USD".to_string()]
        );
        gate.mark_stablecoin_rate("USDT-USD", true, "fresh")
            .unwrap();
        assert_eq!(gate.phase(), LivePhase::Ready);

        gate.mark_stablecoin_rate("USDT-USD", false, "depegged")
            .unwrap();
        let degraded = gate.snapshot();
        assert_eq!(degraded.phase, LivePhase::Degraded);
        assert_eq!(
            degraded.missing_stablecoin_rates,
            vec!["USDT-USD".to_string()]
        );
        assert_eq!(
            degraded.faults.get("stablecoin:USDT-USD"),
            Some(&"depegged".to_string())
        );
        gate.mark_stablecoin_rate("USDT-USD", true, "recovered")
            .unwrap();
        assert_eq!(gate.phase(), LivePhase::Ready);
        assert_eq!(
            gate.mark_stablecoin_rate("USDC-USD", true, "unknown"),
            Err(StartupError::UnknownStablecoinReference(
                "USDC-USD".to_string()
            ))
        );
    }

    #[test]
    fn strategy_references_require_fresh_independent_source_observations() {
        let mut config = config();
        config.strategy.reference_data_stale_threshold_ms = Some(1_000);
        config.strategy.instruments[0].index_symbol = Some("BTC-USDT-INDEX".to_string());
        config.strategy.instruments[1].kind = reap_strategy::InstrumentKindConfig::LinearSwap;
        let mut gate = StartupGate::new(&config);
        gate.mark_metadata_verified();
        gate.mark_storage(true, "open");
        gate.mark_public_connectivity(true, "connected");
        gate.mark_reconciled("main", true, "clean").unwrap();
        gate.mark_account_snapshot("main", true, "loaded").unwrap();
        gate.mark_book("BTC-USDT", true, "snapshot").unwrap();
        gate.mark_book("BTC-PERP", true, "snapshot").unwrap();
        gate.mark_private_stream("main", true, "heartbeat").unwrap();

        assert_eq!(
            gate.snapshot().missing_strategy_references,
            vec![
                "funding_rate:BTC-PERP".to_string(),
                "index_price:BTC-USDT-INDEX".to_string(),
                "mark_price:BTC-PERP".to_string(),
                "price_limits:BTC-PERP".to_string(),
                "price_limits:BTC-USDT".to_string(),
            ]
        );
        gate.observe_strategy_market(
            &MarketEvent::IndexPrice {
                ts_ms: 1_000,
                symbol: "BTC-USDT-INDEX".to_string(),
                price: 50_000.0,
            },
            2_001,
        );
        assert!(
            gate.snapshot()
                .missing_strategy_references
                .contains(&"index_price:BTC-USDT-INDEX".to_string())
        );

        for event in [
            MarketEvent::IndexPrice {
                ts_ms: 2_000,
                symbol: "BTC-USDT-INDEX".to_string(),
                price: 50_000.0,
            },
            MarketEvent::FundingRate {
                ts_ms: 2_000,
                symbol: "BTC-PERP".to_string(),
                rate: 0.0001,
                funding_time_ms: 30_000,
                settlement: None,
            },
            MarketEvent::PriceLimits {
                ts_ms: 2_000,
                symbol: "BTC-PERP".to_string(),
                mark_price: 50_000.0,
                limit_down: 0.0,
                limit_up: 0.0,
            },
            MarketEvent::PriceLimits {
                ts_ms: 2_000,
                symbol: "BTC-PERP".to_string(),
                mark_price: 0.0,
                limit_down: 40_000.0,
                limit_up: 60_000.0,
            },
            MarketEvent::PriceLimits {
                ts_ms: 2_000,
                symbol: "BTC-USDT".to_string(),
                mark_price: 0.0,
                limit_down: 40_000.0,
                limit_up: 60_000.0,
            },
        ] {
            gate.observe_strategy_market(&event, 2_000);
        }
        assert!(gate.snapshot().is_ready());

        gate.observe_strategy_market(
            &MarketEvent::PriceLimits {
                ts_ms: 3_001,
                symbol: "BTC-PERP".to_string(),
                mark_price: 50_001.0,
                limit_down: 0.0,
                limit_up: 0.0,
            },
            3_001,
        );
        let stale = gate.snapshot();
        assert_eq!(stale.phase, LivePhase::Degraded);
        assert!(
            !stale
                .missing_strategy_references
                .contains(&"mark_price:BTC-PERP".to_string())
        );
        assert!(
            stale
                .missing_strategy_references
                .contains(&"funding_rate:BTC-PERP".to_string())
        );
        assert!(
            stale
                .faults
                .contains_key("strategy_reference:funding_rate:BTC-PERP")
        );
    }
}
