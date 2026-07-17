use super::*;

pub(super) async fn run_reconcile_task(
    account_id: String,
    io: OkxReconciliationClient,
    mut commands: mpsc::Receiver<ReconcileTaskCommand>,
    events: mpsc::Sender<RuntimeEvent>,
    ambiguous_submit_grace_ms: u64,
    max_order_reconciliation_pages: usize,
    max_fill_reconciliation_pages: usize,
) {
    while let Some(command) = commands.recv().await {
        let ReconcileTaskCommand::Reconcile {
            restored_orders,
            command_flush,
        } = command
        else {
            return;
        };
        if let Some(command_flush) = command_flush
            && command_flush.await.is_err()
        {
            if events
                .send(RuntimeEvent::ReconcileFailed {
                    account_id: account_id.clone(),
                    ts_ms: unix_time_ms(),
                    reason: "order command task closed before its shutdown flush".to_string(),
                })
                .await
                .is_err()
            {
                return;
            }
            continue;
        }
        let result = reconcile_remote_account(
            &io,
            restored_orders,
            ambiguous_submit_grace_ms,
            max_order_reconciliation_pages,
            max_fill_reconciliation_pages,
        )
        .await;
        let event = match result {
            Ok((remote_orders, remote_fills, remote_account)) => RuntimeEvent::RemoteState {
                account_id: account_id.clone(),
                remote_orders,
                remote_fills,
                remote_account,
                ts_ms: unix_time_ms(),
            },
            Err(reason) => RuntimeEvent::ReconcileFailed {
                account_id: account_id.clone(),
                ts_ms: unix_time_ms(),
                reason,
            },
        };
        if events.send(event).await.is_err() {
            return;
        }
    }
}

async fn reconcile_remote_account(
    io: &OkxReconciliationClient,
    restored_orders: Vec<ReconcileOrderRef>,
    ambiguous_submit_grace_ms: u64,
    max_order_reconciliation_pages: usize,
    max_fill_reconciliation_pages: usize,
) -> Result<(Vec<RemoteOrder>, Vec<RemoteFill>, AccountUpdate), String> {
    let (mut remote_orders, remote_fills) = io
        .fetch_remote_state(
            None,
            None,
            max_order_reconciliation_pages,
            max_fill_reconciliation_pages,
        )
        .await
        .map_err(|error| error.to_string())?;
    let mut remote_ids = remote_orders
        .iter()
        .map(remote_order_id)
        .collect::<HashSet<_>>();
    for restored in restored_orders {
        if remote_ids.contains(&restored.order_id) {
            continue;
        }
        let details = match io
            .fetch_order_details(&restored.symbol, &restored.order_id)
            .await
        {
            Ok(details) => details,
            Err(error)
                if error.is_order_not_found()
                    && unix_time_ms().saturating_sub(restored.last_update_ms)
                        < ambiguous_submit_grace_ms =>
            {
                return Err(format!(
                    "order {} is not visible within the ambiguous-submit grace period",
                    restored.order_id
                ));
            }
            Err(error) if error.is_order_not_found() => RemoteOrder {
                exchange_order_id: String::new(),
                client_order_id: restored.order_id.clone(),
                symbol: restored.symbol,
                side: restored.side,
                state: PrivateOrderState::Rejected,
                price: restored.price,
                qty: restored.qty,
                cumulative_filled_qty: restored.filled_qty,
                average_fill_price: restored.average_fill_price,
                update_time_ms: unix_time_ms(),
            },
            Err(error) => return Err(error.to_string()),
        };
        remote_ids.insert(remote_order_id(&details));
        remote_orders.push(details);
    }
    let remote_account = io
        .fetch_remote_account_state()
        .await
        .map_err(|error| error.to_string())?;
    Ok((remote_orders, remote_fills, remote_account))
}
