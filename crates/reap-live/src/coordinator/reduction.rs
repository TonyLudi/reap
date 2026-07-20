use reap_core::{MarketEvent, NormalizedEvent, SystemEvent, SystemEventKind, TimeMs};
use reap_engine::ChaosEngineOutput;
use reap_risk::RiskDecision;
use reap_storage::{SafetyLatchRecord, SafetyLatchScope, SafetyLatchSource, StorageRecord};

use super::{CoordinatorOutput, LiveAction, LiveCoordinator, ReconcileAction};

impl LiveCoordinator {
    pub(super) fn process_normalized(&mut self, event: NormalizedEvent) -> CoordinatorOutput {
        let observed_now_ms = event.ts_ms();
        self.process_normalized_at(event, observed_now_ms)
    }

    pub(super) fn process_normalized_at(
        &mut self,
        event: NormalizedEvent,
        observed_now_ms: TimeMs,
    ) -> CoordinatorOutput {
        let mut local_clock = || observed_now_ms;
        self.process_normalized_at_with_clock(event, observed_now_ms, &mut local_clock)
    }

    pub(super) fn process_normalized_at_with_clock(
        &mut self,
        event: NormalizedEvent,
        observed_now_ms: TimeMs,
        local_clock: &mut dyn FnMut() -> TimeMs,
    ) -> CoordinatorOutput {
        self.process_normalized_inner(event, observed_now_ms, None, || None, local_clock)
    }

    #[cfg(test)]
    pub(super) fn process_normalized_arrived_at(
        &mut self,
        event: NormalizedEvent,
        observed_now_ms: TimeMs,
        arrival_ns: u64,
    ) -> CoordinatorOutput {
        self.process_normalized_inner(
            event,
            observed_now_ms,
            Some(arrival_ns),
            || Some((arrival_ns, observed_now_ms)),
            || observed_now_ms,
        )
    }

    pub(super) fn process_normalized_received_at(
        &mut self,
        event: NormalizedEvent,
        observed_now_ms: TimeMs,
        receipt_ns: u64,
        processing_clock: impl FnOnce() -> (u64, TimeMs),
        finish_clock: impl FnMut() -> TimeMs,
    ) -> CoordinatorOutput {
        self.process_normalized_inner(
            event,
            observed_now_ms,
            Some(receipt_ns),
            || Some(processing_clock()),
            finish_clock,
        )
    }

    fn process_normalized_inner(
        &mut self,
        event: NormalizedEvent,
        observed_now_ms: TimeMs,
        receipt_ns: Option<u64>,
        processing_clock: impl FnOnce() -> Option<(u64, TimeMs)>,
        mut finish_clock: impl FnMut() -> TimeMs,
    ) -> CoordinatorOutput {
        let strategy_references_were_ready = self.startup.strategy_references_ready();
        let account_halt = match &event {
            NormalizedEvent::System(system)
                if system.kind == SystemEventKind::AccountHalted
                    && system
                        .account_id
                        .as_deref()
                        .is_some_and(|account_id| self.private_states.contains_key(account_id)) =>
            {
                let account_id = system
                    .account_id
                    .clone()
                    .expect("checked account halt must have an account id");
                self.halted_accounts
                    .insert(account_id.clone(), system.reason.clone());
                Some((account_id, system.reason.clone()))
            }
            _ => None,
        };
        let order_transport_stale = match &event {
            NormalizedEvent::System(system)
                if system.kind == SystemEventKind::OrderTransportStale
                    && system
                        .account_id
                        .as_deref()
                        .is_some_and(|account_id| self.private_states.contains_key(account_id)) =>
            {
                Some((
                    system
                        .account_id
                        .clone()
                        .expect("checked order transport event must have an account id"),
                    system.reason.clone(),
                ))
            }
            _ => None,
        };
        if let NormalizedEvent::System(system) = &event {
            self.apply_system_to_startup(system);
        }
        match &event {
            NormalizedEvent::Market(market) => self
                .startup
                .observe_strategy_market(market, observed_now_ms),
            NormalizedEvent::Timer(_) => self.startup.refresh_strategy_references(observed_now_ms),
            NormalizedEvent::Order(_)
            | NormalizedEvent::Account(_)
            | NormalizedEvent::Control(_)
            | NormalizedEvent::System(_) => {}
        }
        let strategy_reference_readiness_lost =
            strategy_references_were_ready && !self.startup.strategy_references_ready();
        let now_ms = event.ts_ms();
        let mut records = vec![StorageRecord::Normalized(event.clone())];
        match &event {
            NormalizedEvent::Order(update) => records.push(StorageRecord::Order {
                account_id: self
                    .config
                    .account_for_symbol(&update.symbol)
                    .map(|account| account.id.clone()),
                update: update.clone(),
            }),
            NormalizedEvent::System(system) => records.push(StorageRecord::System(system.clone())),
            _ => {}
        }
        let sync_stablecoin_readiness = self.event_updates_stablecoin_readiness(&event);
        let strategy_is_live = self.strategy_is_live();
        let uses_depth_processing_clock =
            matches!(&event, NormalizedEvent::Market(MarketEvent::Depth(_)))
                && receipt_ns.is_some();
        let uses_trade_processing_clock =
            matches!(&event, NormalizedEvent::Market(MarketEvent::Trade { .. }))
                && receipt_ns.is_some();
        let engine_output = if uses_depth_processing_clock {
            self.engine.on_chaos_event_with_strategy_clock(
                event,
                strategy_is_live,
                || processing_clock().expect("received depth must provide a processing clock"),
                &mut finish_clock,
            )
        } else if let Some(receipt_ns) = receipt_ns {
            self.engine.on_chaos_event_at_with_finish_clock(
                event,
                receipt_ns,
                observed_now_ms,
                strategy_is_live,
                || {
                    if uses_trade_processing_clock {
                        processing_clock()
                            .expect("received trade must provide a processing clock")
                            .0
                    } else {
                        receipt_ns
                    }
                },
                &mut finish_clock,
            )
        } else {
            self.engine.on_chaos_event(event)
        };
        if sync_stablecoin_readiness {
            self.sync_stablecoin_readiness(now_ms);
        }
        let mut output = CoordinatorOutput {
            actions: Vec::new(),
            records,
        };
        let routing_observed_now_ms = if uses_depth_processing_clock {
            finish_clock()
        } else {
            observed_now_ms
        };
        self.handle_engine_output(
            now_ms,
            routing_observed_now_ms,
            engine_output,
            &mut finish_clock,
            &mut output,
        );
        if let Some((account_id, reason)) = account_halt {
            self.ensure_account_cancels(
                now_ms,
                &account_id,
                &format!("account {account_id} halted: {reason}"),
                &mut output,
            );
        }
        if let Some((account_id, reason)) = order_transport_stale {
            self.ensure_account_cancels(
                now_ms,
                &account_id,
                &format!("order transport stale for account {account_id}: {reason}"),
                &mut output,
            );
            output.actions.push(LiveAction::Reconcile(ReconcileAction {
                ts_ms: now_ms,
                account_id,
                reason: format!("order transport disconnected: {reason}"),
            }));
        }
        if strategy_reference_readiness_lost {
            let missing = self
                .startup
                .snapshot()
                .missing_strategy_references
                .join(", ");
            let account_ids = self
                .config
                .accounts
                .iter()
                .map(|account| account.id.clone())
                .collect::<Vec<_>>();
            for account_id in account_ids {
                self.ensure_account_cancels(
                    observed_now_ms,
                    &account_id,
                    &format!("strategy reference data stale: {missing}"),
                    &mut output,
                );
            }
        }
        output
    }

    fn event_updates_stablecoin_readiness(&self, event: &NormalizedEvent) -> bool {
        match event {
            NormalizedEvent::Timer(_) => !self.config.risk.stablecoin_guards.is_empty(),
            NormalizedEvent::Market(reap_core::MarketEvent::IndexPrice { symbol, .. }) => self
                .config
                .risk
                .stablecoin_guards
                .iter()
                .any(|guard| guard.symbol == *symbol),
            NormalizedEvent::System(system) => {
                system.kind == SystemEventKind::KillSwitchReset
                    && !self.config.risk.stablecoin_guards.is_empty()
            }
            NormalizedEvent::Order(_)
            | NormalizedEvent::Account(_)
            | NormalizedEvent::Control(_)
            | NormalizedEvent::Market(_) => false,
        }
    }

    fn sync_stablecoin_readiness(&mut self, now_ms: TimeMs) {
        let health = self.engine.risk().stablecoin_guard_health(now_ms);
        for guard in health {
            if let Err(error) =
                self.startup
                    .mark_stablecoin_rate(&guard.symbol, guard.healthy, guard.reason)
            {
                self.startup.mark_runtime_health(
                    "stablecoin_guard",
                    false,
                    format!("stablecoin readiness configuration mismatch: {error}"),
                );
            }
        }
    }

    pub(super) fn handle_engine_output(
        &mut self,
        now_ms: TimeMs,
        observed_now_ms: TimeMs,
        engine_output: ChaosEngineOutput,
        local_send_clock: &mut dyn FnMut() -> TimeMs,
        output: &mut CoordinatorOutput,
    ) {
        #[cfg(test)]
        self.chaos_decision_trace
            .push(super::ChaosDecisionTraceBatch {
                typed_intents: engine_output
                    .intents
                    .iter()
                    .map(|intent| (intent.purpose(), intent.to_order_intent()))
                    .collect(),
                rejected: engine_output.rejected.clone(),
                system_events: engine_output.system_events.clone(),
                safety_cancel_candidates: engine_output
                    .safety_cancel_candidates
                    .iter()
                    .map(|candidate| {
                        (
                            candidate.order_id().to_string(),
                            candidate.reason().to_string(),
                        )
                    })
                    .collect(),
            });
        #[cfg(test)]
        self.chaos_intent_trace.extend(
            engine_output
                .intents
                .iter()
                .map(|intent| (intent.purpose(), intent.to_order_intent())),
        );
        for system in engine_output.system_events {
            if system.kind == SystemEventKind::RiskBreach {
                output
                    .records
                    .push(StorageRecord::SafetyLatch(SafetyLatchRecord {
                        ts_ms: system.ts_ms,
                        scope: SafetyLatchScope::Global,
                        active: true,
                        source: SafetyLatchSource::Risk,
                        request_id: None,
                        reason: system.reason.clone(),
                    }));
            }
            self.apply_system_to_startup(&system);
            output.records.push(StorageRecord::System(system));
        }
        for rejection in engine_output.rejected {
            let RiskDecision::Rejected { intent, reason } = rejection else {
                continue;
            };
            output.records.push(StorageRecord::IntentRejected {
                ts_ms: now_ms,
                intent,
                reason: format!("{reason:?}"),
            });
        }
        for intent in engine_output.intents {
            let legacy = intent.to_order_intent();
            output.records.push(StorageRecord::Intent {
                ts_ms: now_ms,
                intent: legacy.clone(),
            });
            self.route_chaos_intent(
                now_ms,
                observed_now_ms,
                intent,
                legacy,
                local_send_clock,
                output,
            );
        }
        for candidate in engine_output.safety_cancel_candidates {
            let legacy = candidate.to_order_intent();
            output.records.push(StorageRecord::Intent {
                ts_ms: now_ms,
                intent: legacy.clone(),
            });
            self.route_safety_cancel(now_ms, candidate, legacy, output);
        }
    }

    fn apply_system_to_startup(&mut self, event: &SystemEvent) {
        match event.kind {
            SystemEventKind::FeedHeartbeat | SystemEventKind::FeedRecovered => {
                if let Some(symbol) = event.symbol.as_deref() {
                    let _ = self.startup.mark_book(symbol, true, &event.reason);
                }
            }
            SystemEventKind::FeedStale
            | SystemEventKind::FeedGap
            | SystemEventKind::BookRecoveryStarted
            | SystemEventKind::BookRecoveryFailed => {
                if let Some(symbol) = event.symbol.as_deref() {
                    let _ = self.startup.mark_book(symbol, false, &event.reason);
                }
            }
            SystemEventKind::PrivateStreamHeartbeat | SystemEventKind::PrivateStreamRecovered => {
                if let Some(account_id) = event.account_id.as_deref() {
                    let _ = self
                        .startup
                        .mark_private_stream(account_id, true, &event.reason);
                }
            }
            SystemEventKind::PrivateStreamStale => {
                if let Some(account_id) = event.account_id.as_deref() {
                    let _ = self
                        .startup
                        .mark_private_stream(account_id, false, &event.reason);
                }
            }
            SystemEventKind::OrderTransportHeartbeat | SystemEventKind::OrderTransportRecovered => {
                if let Some(account_id) = event.account_id.as_deref() {
                    let _ = self
                        .startup
                        .mark_order_transport(account_id, true, &event.reason);
                }
            }
            SystemEventKind::OrderTransportStale => {
                if let Some(account_id) = event.account_id.as_deref() {
                    let _ = self
                        .startup
                        .mark_order_transport(account_id, false, &event.reason);
                    let _ = self
                        .startup
                        .mark_reconciled(account_id, false, &event.reason);
                }
            }
            SystemEventKind::ReconcileDrift => {
                if let Some(account_id) = event.account_id.as_deref() {
                    let _ = self
                        .startup
                        .mark_reconciled(account_id, false, &event.reason);
                }
            }
            SystemEventKind::RiskBreach | SystemEventKind::KillSwitchActivated => {
                self.startup
                    .mark_runtime_health("risk", false, &event.reason);
            }
            SystemEventKind::KillSwitchReset => {
                self.startup
                    .mark_runtime_health("risk", true, &event.reason);
            }
            SystemEventKind::AccountHalted
            | SystemEventKind::SymbolHalted
            | SystemEventKind::SymbolResumed => {}
        }
    }
}
