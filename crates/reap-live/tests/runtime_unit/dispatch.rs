use super::*;

struct GatedOrderTransport {
    started: mpsc::UnboundedSender<String>,
    gates: OrderGates,
    active: Arc<AtomicUsize>,
    max_active: Arc<AtomicUsize>,
}

type OrderGates = Arc<Mutex<HashMap<String, Arc<Semaphore>>>>;
type GatedOrderHarness = (
    GatedOrderTransport,
    mpsc::UnboundedReceiver<String>,
    OrderGates,
    Arc<AtomicUsize>,
);

#[async_trait]
impl RegularExecution for GatedOrderTransport {
    async fn place_regular_order(
        &self,
        order: PreparedRegularSubmit,
    ) -> Result<OkxOrderAck, OrderTransportError> {
        self.execute(&order.order().symbol, order.client_order_id())
            .await
    }

    async fn cancel_regular_order(
        &self,
        order: PreparedRegularCancel,
    ) -> Result<OkxOrderAck, CancelOrderTransportError> {
        self.execute(order.symbol(), order.client_order_id())
            .await
            .map_err(CancelOrderTransportError::failed)
    }

    async fn cancel_regular_order_via_rest(
        &self,
        _order: PreparedRegularCancel,
    ) -> Result<OkxOrderAck, OrderTransportError> {
        unreachable!("gated command tests never use REST cancellation fallback")
    }
}

impl GatedOrderTransport {
    async fn execute(
        &self,
        symbol: &str,
        client_order_id: &str,
    ) -> Result<OkxOrderAck, OrderTransportError> {
        let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
        self.max_active.fetch_max(active, Ordering::SeqCst);
        self.started.send(symbol.to_string()).unwrap();
        let gate = self
            .gates
            .lock()
            .unwrap()
            .get(symbol)
            .unwrap_or_else(|| panic!("missing gate for {symbol}"))
            .clone();
        gate.acquire().await.unwrap().forget();
        self.active.fetch_sub(1, Ordering::SeqCst);
        Ok(OkxOrderAck {
            exchange_order_id: format!("exchange-{client_order_id}"),
            client_order_id: client_order_id.to_string(),
        })
    }
}

fn gated_order_transport(symbols: &[&str]) -> GatedOrderHarness {
    let (started_tx, started_rx) = mpsc::unbounded_channel();
    let gates = Arc::new(Mutex::new(
        symbols
            .iter()
            .map(|symbol| ((*symbol).to_string(), Arc::new(Semaphore::new(0))))
            .collect(),
    ));
    let max_active = Arc::new(AtomicUsize::new(0));
    (
        GatedOrderTransport {
            started: started_tx,
            gates: Arc::clone(&gates),
            active: Arc::new(AtomicUsize::new(0)),
            max_active: Arc::clone(&max_active),
        },
        started_rx,
        gates,
        max_active,
    )
}

fn release_order(gates: &OrderGates, symbol: &str) {
    gates.lock().unwrap().get(symbol).unwrap().add_permits(1);
}

async fn receive_started(receiver: &mut mpsc::UnboundedReceiver<String>) -> String {
    tokio::time::timeout(Duration::from_secs(1), receiver.recv())
        .await
        .expect("order operation did not start")
        .expect("order start channel closed")
}

#[tokio::test]
async fn order_task_is_bounded_and_serializes_each_underlying() {
    let symbols = [
        "BTC-USDT-SWAP",
        "BTC-USDT-260925",
        "ETH-USDT-SWAP",
        "SOL-USDT-SWAP",
    ];
    let (transport, mut started, gates, max_active) = gated_order_transport(&symbols);
    let (gateway, policy, client_order_ids) =
        runtime_order_gateway_with_execution(&symbols, Vec::new(), Box::new(transport));
    let (command_tx, command_rx) = mpsc::channel(16);
    let (event_tx, mut event_rx) = mpsc::channel(16);
    let task = tokio::spawn(run_order_task(
        "main".to_string(),
        gateway,
        command_rx,
        event_tx,
        2,
        16,
    ));

    for (symbol, id) in [
        (symbols[0], "btc-swap"),
        (symbols[1], "btc-future"),
        (symbols[2], "eth"),
    ] {
        command_tx
            .send(OrderTaskCommand::Submit {
                action: submit_action(&policy, &client_order_ids, symbol, id),
                enqueued_at: Instant::now(),
            })
            .await
            .unwrap();
    }

    let first = receive_started(&mut started).await;
    let second = receive_started(&mut started).await;
    assert_eq!(
        HashSet::from([first, second]),
        HashSet::from([symbols[0].to_string(), symbols[2].to_string()])
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(25), started.recv())
            .await
            .is_err(),
        "the worker exceeded its in-flight bound"
    );

    release_order(&gates, symbols[0]);
    assert_eq!(receive_started(&mut started).await, symbols[1]);
    command_tx
        .send(OrderTaskCommand::Submit {
            action: submit_action(&policy, &client_order_ids, symbols[3], "sol"),
            enqueued_at: Instant::now(),
        })
        .await
        .unwrap();
    assert!(
        tokio::time::timeout(Duration::from_millis(25), started.recv())
            .await
            .is_err(),
        "a later underlying started before a bounded slot was free"
    );

    release_order(&gates, symbols[2]);
    assert_eq!(receive_started(&mut started).await, symbols[3]);
    release_order(&gates, symbols[1]);
    release_order(&gates, symbols[3]);

    let (flushed_tx, flushed_rx) = oneshot::channel();
    command_tx
        .send(OrderTaskCommand::Flush(flushed_tx))
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(1), flushed_rx)
        .await
        .expect("command flush timed out")
        .unwrap();
    for _ in 0..4 {
        assert!(matches!(
            event_rx.recv().await,
            Some(RuntimeEvent::SubmitComplete { .. })
        ));
    }
    assert_eq!(max_active.load(Ordering::SeqCst), 2);

    command_tx.send(OrderTaskCommand::Shutdown).await.unwrap();
    task.await.unwrap();
}

#[tokio::test]
async fn order_task_rejects_authority_for_a_different_account_before_preparation() {
    let symbol = "BTC-USDT-SWAP";
    let (transport, mut started, _gates, _) = gated_order_transport(&[symbol]);
    let (gateway, policy, client_order_ids) =
        runtime_order_gateway_with_execution(&[symbol], Vec::new(), Box::new(transport));
    let (command_tx, command_rx) = mpsc::channel(2);
    let (event_tx, mut event_rx) = mpsc::channel(2);
    let task = tokio::spawn(run_order_task(
        "other-account".to_string(),
        gateway,
        command_rx,
        event_tx,
        1,
        2,
    ));

    command_tx
        .send(OrderTaskCommand::Submit {
            action: submit_action(&policy, &client_order_ids, symbol, "wrong-account"),
            enqueued_at: Instant::now(),
        })
        .await
        .unwrap();
    let event = event_rx.recv().await.expect("worker must fail closed");
    assert!(matches!(
        event,
        RuntimeEvent::Fatal(RuntimeTaskFailure::Gateway(message))
            if message.contains("received submit authority for account main")
    ));
    assert!(
        tokio::time::timeout(Duration::from_millis(25), started.recv())
            .await
            .is_err(),
        "wrong-account authority reached the order transport"
    );

    command_tx.send(OrderTaskCommand::Shutdown).await.unwrap();
    task.await.unwrap();
}

#[tokio::test]
async fn reconciliation_completes_while_an_order_command_is_blocked() {
    let symbol = "BTC-USDT-SWAP";
    let responses = vec![
            Ok(HttpResponse {
                status: 200,
                body: r#"{"code":"0","msg":"","data":[]}"#.to_string(),
            }),
            Ok(HttpResponse {
                status: 200,
                body: r#"{"code":"0","msg":"","data":[]}"#.to_string(),
            }),
            Ok(HttpResponse {
                status: 200,
                body: r#"{"code":"0","msg":"","data":[{"uTime":"100","details":[{"ccy":"USDT","cashBal":"100","availBal":"90","eq":"100","liab":"0","maxLoan":"0"}]}]}"#.to_string(),
            }),
            Ok(HttpResponse {
                status: 200,
                body: r#"{"code":"0","msg":"","data":[]}"#.to_string(),
            }),
        ];
    let (transport, mut started, gates, _) = gated_order_transport(&[symbol]);
    let (gateway, policy, client_order_ids) =
        runtime_order_gateway_with_execution(&[symbol], responses, Box::new(transport));
    let io = gateway.reconciliation_client();
    let (command_tx, command_rx) = mpsc::channel(8);
    let (reconcile_tx, reconcile_rx) = mpsc::channel(2);
    let (event_tx, mut event_rx) = mpsc::channel(8);
    let order_task = tokio::spawn(run_order_task(
        "main".to_string(),
        gateway,
        command_rx,
        event_tx.clone(),
        1,
        8,
    ));
    let reconcile_task = tokio::spawn(run_reconcile_task(
        "main".to_string(),
        io,
        reconcile_rx,
        event_tx,
        10_000,
        2,
        2,
    ));

    command_tx
        .send(OrderTaskCommand::Submit {
            action: submit_action(&policy, &client_order_ids, symbol, "blocked"),
            enqueued_at: Instant::now(),
        })
        .await
        .unwrap();
    assert_eq!(receive_started(&mut started).await, symbol);
    reconcile_tx
        .send(ReconcileTaskCommand::Reconcile {
            restored_orders: Vec::new(),
            command_flush: None,
        })
        .await
        .unwrap();

    let event = tokio::time::timeout(Duration::from_secs(1), event_rx.recv())
        .await
        .expect("reconciliation was blocked behind order acknowledgement")
        .expect("runtime event channel closed");
    let RuntimeEvent::RemoteState {
        remote_orders,
        remote_account,
        ..
    } = event
    else {
        panic!("blocked command completed before independent reconciliation");
    };
    assert!(remote_orders.is_empty());
    assert_eq!(remote_account.balances.len(), 1);

    release_order(&gates, symbol);
    let (flushed_tx, flushed_rx) = oneshot::channel();
    command_tx
        .send(OrderTaskCommand::Flush(flushed_tx))
        .await
        .unwrap();
    flushed_rx.await.unwrap();
    assert!(matches!(
        event_rx.recv().await,
        Some(RuntimeEvent::SubmitComplete { .. })
    ));

    command_tx.send(OrderTaskCommand::Shutdown).await.unwrap();
    reconcile_tx
        .send(ReconcileTaskCommand::Shutdown)
        .await
        .unwrap();
    order_task.await.unwrap();
    reconcile_task.await.unwrap();
}
