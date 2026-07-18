use reap_core::{NormalizedEvent, SystemEvent, SystemEventKind};
use reap_storage::{SafetyLatchRecord, SafetyLatchScope, SafetyLatchSource, StorageRecord};
use tokio::sync::mpsc;

use crate::{OperatorCommand, OperatorEnvelope, OperatorResponse, OperatorStatus};

use super::{LiveRuntime, LiveRuntimeError, unix_time_ms};

impl LiveRuntime {
    pub(super) async fn handle_operator_envelope(
        &mut self,
        envelope: OperatorEnvelope,
    ) -> Result<(), LiveRuntimeError> {
        let OperatorEnvelope {
            request_id,
            command,
            response,
        } = envelope;
        self.composition.evidence.operator_commands = self
            .composition
            .evidence
            .operator_commands
            .saturating_add(1);
        let result = self.execute_operator_command(&request_id, command).await;
        match result {
            Ok(operator_response) => {
                let _ = response.send(operator_response);
                Ok(())
            }
            Err(error) => {
                let _ = response.send(OperatorResponse::rejected(
                    request_id,
                    format!("operator command failed: {error}"),
                ));
                Err(error)
            }
        }
    }

    async fn execute_operator_command(
        &mut self,
        request_id: &str,
        command: OperatorCommand,
    ) -> Result<OperatorResponse, LiveRuntimeError> {
        match command {
            OperatorCommand::Status => Ok(OperatorResponse::accepted(
                request_id,
                "runtime status",
                Some(self.operator_status()),
            )),
            OperatorCommand::KillSwitch { reason } => {
                self.coordinator.set_order_entry_enabled(false);
                self.commit_operator_system_event(
                    request_id,
                    SystemEventKind::KillSwitchActivated,
                    None,
                    SafetyLatchScope::Global,
                    true,
                    reason,
                )
                .await?;
                self.composition.evidence.operator_mutations = self
                    .composition
                    .evidence
                    .operator_mutations
                    .saturating_add(1);
                Ok(OperatorResponse::accepted(
                    request_id,
                    "kill switch activated",
                    Some(self.operator_status()),
                ))
            }
            OperatorCommand::KillAccount { account_id, reason } => {
                if !self.coordinator.manages_account(&account_id) {
                    return Ok(OperatorResponse::rejected(
                        request_id,
                        format!("account {account_id} is not managed by this runtime"),
                    ));
                }
                let now_ms = unix_time_ms();
                let reason = format!("authenticated operator request {request_id}: {reason}");
                let mut output =
                    self.coordinator
                        .halt_account(now_ms, &account_id, reason.clone())?;
                output.records.insert(
                    0,
                    StorageRecord::SafetyLatch(SafetyLatchRecord {
                        ts_ms: now_ms,
                        scope: SafetyLatchScope::Account {
                            account_id: account_id.clone(),
                        },
                        active: true,
                        source: SafetyLatchSource::Operator,
                        request_id: Some(request_id.to_string()),
                        reason,
                    }),
                );
                self.commit_output(output).await?;
                self.composition.evidence.operator_mutations = self
                    .composition
                    .evidence
                    .operator_mutations
                    .saturating_add(1);
                Ok(OperatorResponse::accepted(
                    request_id,
                    format!("account {account_id} halted"),
                    Some(self.operator_status()),
                ))
            }
            OperatorCommand::HaltSymbol { symbol, reason } => {
                if !self.coordinator.manages_symbol(&symbol) {
                    return Ok(OperatorResponse::rejected(
                        request_id,
                        format!("symbol {symbol} is not managed by this runtime"),
                    ));
                }
                self.commit_operator_system_event(
                    request_id,
                    SystemEventKind::SymbolHalted,
                    Some(symbol.clone()),
                    SafetyLatchScope::Symbol { symbol },
                    true,
                    reason,
                )
                .await?;
                self.composition.evidence.operator_mutations = self
                    .composition
                    .evidence
                    .operator_mutations
                    .saturating_add(1);
                Ok(OperatorResponse::accepted(
                    request_id,
                    "symbol halted",
                    Some(self.operator_status()),
                ))
            }
            OperatorCommand::ResumeSymbol { symbol, reason } => {
                if !self.coordinator.manages_symbol(&symbol) {
                    return Ok(OperatorResponse::rejected(
                        request_id,
                        format!("symbol {symbol} is not managed by this runtime"),
                    ));
                }
                if let Some(account_id) = self.coordinator.halted_account_for_symbol(&symbol) {
                    return Ok(OperatorResponse::rejected(
                        request_id,
                        format!(
                            "symbol {symbol} belongs to halted account {account_id}; account kills cannot be reset live"
                        ),
                    ));
                }
                self.commit_operator_system_event(
                    request_id,
                    SystemEventKind::SymbolResumed,
                    Some(symbol.clone()),
                    SafetyLatchScope::Symbol { symbol },
                    false,
                    reason,
                )
                .await?;
                self.composition.evidence.operator_mutations = self
                    .composition
                    .evidence
                    .operator_mutations
                    .saturating_add(1);
                Ok(OperatorResponse::accepted(
                    request_id,
                    "symbol resumed",
                    Some(self.operator_status()),
                ))
            }
            OperatorCommand::Shutdown { reason } => {
                self.coordinator.set_order_entry_enabled(false);
                self.composition.evidence.operator_mutations = self
                    .composition
                    .evidence
                    .operator_mutations
                    .saturating_add(1);
                self.dispatch.operator_shutdown_reason = Some(format!(
                    "authenticated operator shutdown {request_id}: {reason}"
                ));
                Ok(OperatorResponse::accepted(
                    request_id,
                    "graceful shutdown accepted",
                    Some(self.operator_status()),
                ))
            }
        }
    }

    async fn commit_operator_system_event(
        &mut self,
        request_id: &str,
        kind: SystemEventKind,
        symbol: Option<String>,
        scope: SafetyLatchScope,
        active: bool,
        reason: String,
    ) -> Result<(), LiveRuntimeError> {
        let now_ms = unix_time_ms();
        let reason = format!("authenticated operator request {request_id}: {reason}");
        let mut output = self
            .coordinator
            .process_event(NormalizedEvent::System(SystemEvent {
                ts_ms: now_ms,
                kind,
                venue: None,
                account_id: None,
                symbol,
                reason: reason.clone(),
            }));
        output.records.insert(
            0,
            StorageRecord::SafetyLatch(SafetyLatchRecord {
                ts_ms: now_ms,
                scope,
                active,
                source: SafetyLatchSource::Operator,
                request_id: Some(request_id.to_string()),
                reason,
            }),
        );
        self.commit_output(output).await
    }

    fn operator_status(&self) -> OperatorStatus {
        OperatorStatus {
            readiness: self.coordinator.readiness(),
            active_orders: self.coordinator.active_order_count(),
            kill_switch_active: self.coordinator.kill_switch_active(),
            halted_accounts: self.coordinator.halted_accounts().clone(),
            shutdown_in_progress: self.shutdown.in_progress
                || self.dispatch.operator_shutdown_reason.is_some(),
        }
    }
}

pub(super) async fn receive_operator(
    receiver: &mut Option<mpsc::Receiver<OperatorEnvelope>>,
) -> Option<OperatorEnvelope> {
    match receiver {
        Some(receiver) => receiver.recv().await,
        None => std::future::pending().await,
    }
}
