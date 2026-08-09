use std::collections::BTreeMap;
use std::str::FromStr as _;
use std::sync::{Arc, Mutex};

use futures_util::{SinkExt as _, StreamExt as _};
use reap_pm_core::{
    EvmAddress, PmOrderSalt, PmOrderSide, PmPrice, PmQuantity, PmTick, PmTokenId, U256,
};
use reap_polymarket_auth::{EoaPrivateKeyInput, FixedEoaSigner, PmClobDomain};
use reap_polymarket_wire::PmUnsignedClobV2Order;
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, watch};
use tokio::task::JoinHandle;
use tokio_tungstenite::{accept_async, tungstenite::Message};

pub(super) const TEST_KEY: &str =
    "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";
pub(super) const ADDRESS: &str = "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266";
pub(super) const PROXY_FUNDER: &str = "0x4444444444444444444444444444444444444444";
pub(super) const API_KEY: &str = "00000000-0000-4000-8000-000000000001";
pub(super) const API_SECRET: &str = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";
pub(super) const PASSPHRASE: &str = "pm-t1-vertical";
pub(super) const AUTH_SECONDS: u64 = 1_780_449_126;
pub(super) const CONDITION: &str =
    "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
pub(super) const MARKET: &str =
    "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

const MAX_REQUEST_BYTES: usize = 16 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum MutationFaultMode {
    Accepted,
    Rejected,
    Disconnect,
    PartialResponse,
    ForeignIdentity,
}

#[derive(Default)]
struct VenueState {
    request_counts: Mutex<BTreeMap<String, usize>>,
    placed_order: Mutex<Option<String>>,
    cancelled_order: Mutex<Option<String>>,
    maker_address: Mutex<String>,
    partial_fill: Mutex<bool>,
    place_mode: Mutex<Option<MutationFaultMode>>,
    cancel_mode: Mutex<Option<MutationFaultMode>>,
}

pub(super) struct LoopbackVenue {
    http_origin: String,
    user_ws_endpoint: String,
    public_ws_endpoint: String,
    state: Arc<VenueState>,
    user_frames: mpsc::Sender<String>,
    shutdown: Option<watch::Sender<bool>>,
    http_task: JoinHandle<()>,
    user_ws_task: JoinHandle<()>,
    public_ws_task: JoinHandle<()>,
}

impl LoopbackVenue {
    pub(super) async fn start() -> Self {
        let http = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind PM-T1 HTTP fixture");
        let user_ws = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind PM-T1 user WS fixture");
        let public_ws = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind PM-T1 public WS fixture");
        let http_origin = format!("http://{}", http.local_addr().expect("HTTP address"));
        let user_ws_endpoint = format!(
            "ws://{}/ws/user",
            user_ws.local_addr().expect("user WS address")
        );
        let public_ws_endpoint = format!(
            "ws://{}/ws/market",
            public_ws.local_addr().expect("public WS address")
        );
        let state = Arc::new(VenueState {
            place_mode: Mutex::new(Some(MutationFaultMode::Accepted)),
            cancel_mode: Mutex::new(Some(MutationFaultMode::Accepted)),
            maker_address: Mutex::new(ADDRESS.to_owned()),
            ..VenueState::default()
        });
        let (shutdown, shutdown_rx) = watch::channel(false);
        let (user_frames, frame_rx) = mpsc::channel(8);
        let http_task = tokio::spawn(run_http(http, Arc::clone(&state), shutdown_rx.clone()));
        let user_ws_task = tokio::spawn(run_user_ws(user_ws, frame_rx, shutdown_rx.clone()));
        let public_ws_task = tokio::spawn(run_public_ws(public_ws, shutdown_rx));
        Self {
            http_origin,
            user_ws_endpoint,
            public_ws_endpoint,
            state,
            user_frames,
            shutdown: Some(shutdown),
            http_task,
            user_ws_task,
            public_ws_task,
        }
    }

    pub(super) fn http_origin(&self) -> &str {
        &self.http_origin
    }

    pub(super) fn user_ws_endpoint(&self) -> &str {
        &self.user_ws_endpoint
    }

    pub(super) fn public_ws_endpoint(&self) -> &str {
        &self.public_ws_endpoint
    }

    pub(super) fn request_count(&self, method_and_path: &str) -> usize {
        *self
            .state
            .request_counts
            .lock()
            .expect("request-count lock")
            .get(method_and_path)
            .unwrap_or(&0)
    }

    pub(super) fn request_count_prefix(&self, method_and_path_prefix: &str) -> usize {
        self.state
            .request_counts
            .lock()
            .expect("request-count lock")
            .iter()
            .filter(|(request, _)| request.starts_with(method_and_path_prefix))
            .map(|(_, count)| *count)
            .sum()
    }

    pub(super) fn placed_order(&self) -> Option<String> {
        self.state
            .placed_order
            .lock()
            .expect("placed-order lock")
            .clone()
    }

    pub(super) fn cancelled_order(&self) -> Option<String> {
        self.state
            .cancelled_order
            .lock()
            .expect("cancelled-order lock")
            .clone()
    }

    pub(super) fn set_mutation_faults(&self, place: MutationFaultMode, cancel: MutationFaultMode) {
        *self.state.place_mode.lock().expect("place-mode lock") = Some(place);
        *self.state.cancel_mode.lock().expect("cancel-mode lock") = Some(cancel);
    }

    pub(super) async fn publish_partial_fill(&self) {
        let order = self
            .placed_order()
            .expect("place must precede private fill");
        let maker = self
            .state
            .maker_address
            .lock()
            .expect("maker-address lock")
            .clone();
        *self.state.partial_fill.lock().expect("partial-fill lock") = true;
        self.user_frames
            .send(user_fill_frame(&order, &maker).to_string())
            .await
            .expect("user WS fixture remains live");
    }

    pub(super) async fn publish_cancelled(&self) {
        let order = self
            .placed_order()
            .expect("place must precede private cancellation");
        let maker = self
            .state
            .maker_address
            .lock()
            .expect("maker-address lock")
            .clone();
        self.user_frames
            .send(user_cancel_frame(&order, &maker).to_string())
            .await
            .expect("user WS fixture remains live");
    }

    pub(super) async fn shutdown(mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(true);
        }
        for task in [self.http_task, self.user_ws_task, self.public_ws_task] {
            tokio::time::timeout(std::time::Duration::from_secs(2), task)
                .await
                .expect("loopback venue task stops")
                .expect("loopback venue task joins");
        }
    }
}

async fn run_http(
    listener: TcpListener,
    state: Arc<VenueState>,
    mut shutdown: watch::Receiver<bool>,
) {
    loop {
        tokio::select! {
            _ = wait_for_shutdown(&mut shutdown) => return,
            accepted = listener.accept() => {
                let Ok((stream, _)) = accepted else { return };
                let state = Arc::clone(&state);
                tokio::spawn(async move { handle_http(stream, state).await });
            }
        }
    }
}

async fn handle_http(mut stream: TcpStream, state: Arc<VenueState>) {
    let Some(request) = read_request(&mut stream).await else {
        return;
    };
    let head_end = request
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|position| position + 4)
        .expect("bounded request has headers");
    let head = std::str::from_utf8(&request[..head_end]).expect("fixture request head is UTF-8");
    let request_line = head.lines().next().expect("request line");
    let mut request_parts = request_line.split_whitespace();
    let method = request_parts.next().unwrap_or_default();
    let path = request_parts.next().unwrap_or_default();
    if path.starts_with("/data/")
        || path.starts_with("/balance-allowance?")
        || matches!((method, path), ("POST", "/order") | ("DELETE", "/order"))
    {
        assert_private_auth_headers(head);
    }
    let key = format!("{method} {path}");
    *state
        .request_counts
        .lock()
        .expect("request-count lock")
        .entry(key)
        .or_default() += 1;

    let body = &request[head_end..];
    match (method, path) {
        ("GET", path) if path.starts_with("/markets/") => {
            respond_json(&mut stream, 200, long_market()).await;
        }
        ("GET", path) if path.starts_with("/clob-markets/") => {
            respond_json(&mut stream, 200, short_market()).await;
        }
        ("GET", "/time") => respond_text(&mut stream, 200, &AUTH_SECONDS.to_string()).await,
        ("GET", path) if path.starts_with("/book?token_id=") => {
            respond_json(&mut stream, 200, rest_book()).await;
        }
        ("GET", path) if path.starts_with("/data/orders?") => {
            respond_json(&mut stream, 200, empty_page()).await;
        }
        ("GET", path) if path.starts_with("/data/trades?") => {
            let response = if *state.partial_fill.lock().expect("partial-fill lock") {
                let order = state
                    .placed_order
                    .lock()
                    .expect("placed-order lock")
                    .clone()
                    .expect("partial fill retains placed order");
                let maker = state
                    .maker_address
                    .lock()
                    .expect("maker-address lock")
                    .clone();
                trade_page(&order, &maker)
            } else {
                empty_page()
            };
            respond_json(&mut stream, 200, response).await;
        }
        ("GET", path) if path.starts_with("/balance-allowance?") => {
            let conditional = path.contains("asset_type=CONDITIONAL");
            let filled = *state.partial_fill.lock().expect("partial-fill lock");
            respond_json(&mut stream, 200, balance_allowance(conditional, filled)).await;
        }
        ("GET", path) if path.starts_with("/data/order/") => {
            let requested = path
                .strip_prefix("/data/order/")
                .expect("matched exact-order route");
            let cancelled = state
                .cancelled_order
                .lock()
                .expect("cancelled-order lock")
                .clone();
            if cancelled.as_deref() == Some(requested) {
                let partially_filled = *state.partial_fill.lock().expect("partial-fill lock");
                let maker = state
                    .maker_address
                    .lock()
                    .expect("maker-address lock")
                    .clone();
                respond_json(
                    &mut stream,
                    200,
                    cancelled_order_detail(requested, partially_filled, &maker),
                )
                .await;
            } else {
                respond_text(&mut stream, 404, "").await;
            }
        }
        ("POST", "/order") => {
            let mode = state
                .place_mode
                .lock()
                .expect("place-mode lock")
                .unwrap_or(MutationFaultMode::Accepted);
            respond_place(&mut stream, &state, mode, body).await;
        }
        ("DELETE", "/order") => {
            let mode = state
                .cancel_mode
                .lock()
                .expect("cancel-mode lock")
                .unwrap_or(MutationFaultMode::Accepted);
            respond_cancel(&mut stream, &state, mode, body).await;
        }
        _ => respond_json(&mut stream, 404, json!({"error":"unhandled fixture route"})).await,
    }
}

async fn respond_place(
    stream: &mut TcpStream,
    state: &Arc<VenueState>,
    mode: MutationFaultMode,
    body: &[u8],
) {
    if mode == MutationFaultMode::Disconnect {
        return;
    }
    if mode == MutationFaultMode::PartialResponse {
        let _ = stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 256\r\n\r\n{")
            .await;
        return;
    }
    let parsed: Value = serde_json::from_slice(body).expect("fixed place body JSON");
    if mode == MutationFaultMode::Rejected {
        respond_json(
            stream,
            200,
            json!({
                "success": false,
                "errorMsg": "synthetic rejection",
                "orderID": "",
                "status": "",
                "makingAmount": "",
                "takingAmount": "",
                "transactionsHashes": [],
                "tradeIDs": []
            }),
        )
        .await;
        return;
    }
    *state.maker_address.lock().expect("maker-address lock") = parsed["order"]["maker"]
        .as_str()
        .expect("fixed maker address")
        .to_owned();
    let expected = expected_order_id(&parsed);
    let observed = if mode == MutationFaultMode::ForeignIdentity {
        format!("0x{}", "ef".repeat(32))
    } else {
        expected
    };
    *state.placed_order.lock().expect("placed-order lock") = Some(observed.clone());
    respond_json(
        stream,
        200,
        json!({
            "success": true,
            "errorMsg": "",
            "orderID": observed,
            "status": "live",
            "makingAmount": parsed["order"]["makerAmount"],
            "takingAmount": parsed["order"]["takerAmount"],
            "transactionsHashes": [],
            "tradeIDs": []
        }),
    )
    .await;
}

async fn respond_cancel(
    stream: &mut TcpStream,
    state: &Arc<VenueState>,
    mode: MutationFaultMode,
    body: &[u8],
) {
    if mode == MutationFaultMode::Disconnect {
        return;
    }
    if mode == MutationFaultMode::PartialResponse {
        let _ = stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 256\r\n\r\n{")
            .await;
        return;
    }
    let parsed: Value = serde_json::from_slice(body).expect("fixed cancel body JSON");
    let order = parsed["orderID"].as_str().expect("fixed cancel orderID");
    if mode == MutationFaultMode::Rejected {
        respond_json(
            stream,
            200,
            json!({"canceled":[],"not_canceled":{(order):"synthetic rejection"}}),
        )
        .await;
        return;
    }
    let observed = if mode == MutationFaultMode::ForeignIdentity {
        format!("0x{}", "cd".repeat(32))
    } else {
        order.to_owned()
    };
    *state.cancelled_order.lock().expect("cancelled-order lock") = Some(observed.clone());
    respond_json(
        stream,
        200,
        json!({"canceled":[observed],"not_canceled":{}}),
    )
    .await;
}

fn expected_order_id(body: &Value) -> String {
    let order = &body["order"];
    assert_eq!(body["deferExec"], false);
    assert_eq!(body["orderType"], "GTC");
    assert_eq!(body["owner"], API_KEY);
    assert_eq!(body["postOnly"], true);
    assert_eq!(order["signer"], ADDRESS);
    assert_eq!(order["tokenId"], "123");
    assert_eq!(order["side"], "BUY");
    assert_eq!(order["makerAmount"], "2000000");
    assert_eq!(order["takerAmount"], "5000000");
    assert_eq!(order["expiration"], "0");
    assert!(
        order["signature"]
            .as_str()
            .is_some_and(|signature| signature.starts_with("0x") && signature.len() == 132),
        "fixed place must carry one canonical EOA signature"
    );
    let maker_text = order["maker"].as_str().expect("maker text");
    let maker = EvmAddress::parse(maker_text).expect("maker address");
    let signer_address = EvmAddress::parse(ADDRESS).expect("signer EOA");
    let salt = PmOrderSalt::from_u64(order["salt"].as_u64().expect("salt")).expect("valid salt");
    let token = PmTokenId::new(
        U256::from_str(order["tokenId"].as_str().expect("token text")).expect("token U256"),
    )
    .expect("nonzero token");
    let side = match order["side"].as_str().expect("side") {
        "BUY" => PmOrderSide::Buy,
        "SELL" => PmOrderSide::Sell,
        side => panic!("unexpected fixed side {side}"),
    };
    let timestamp = order["timestamp"]
        .as_str()
        .expect("timestamp text")
        .parse::<u64>()
        .expect("timestamp integer");
    let price = PmPrice::parse_decimal("0.40").expect("fixed price");
    let quantity = PmQuantity::parse_decimal("5").expect("fixed quantity");
    let tick = PmTick::parse_decimal("0.01").expect("fixed tick");
    let minimum = PmQuantity::parse_decimal("5").expect("fixed minimum");
    let unsigned = match order["signatureType"].as_u64().expect("signature type") {
        0 => {
            assert_eq!(maker_text, ADDRESS);
            PmUnsignedClobV2Order::new_goal_f(
                salt,
                maker,
                signer_address,
                token,
                side,
                price,
                quantity,
                tick,
                minimum,
                timestamp,
            )
        }
        1 => {
            assert_eq!(maker_text, PROXY_FUNDER);
            PmUnsignedClobV2Order::new_pm_t2_proxy(
                salt,
                maker,
                signer_address,
                token,
                side,
                price,
                quantity,
                tick,
                minimum,
                timestamp,
            )
        }
        profile => panic!("unexpected fixed signature profile {profile}"),
    }
    .expect("fixed unsigned order");
    let signer = FixedEoaSigner::bind(EoaPrivateKeyInput::new(TEST_KEY.into()), ADDRESS)
        .expect("fixture signer");
    signer
        .sign_clob_v2_order(PmClobDomain::Standard, unsigned)
        .expect("fixture order signature")
        .expected_order_id()
        .to_string()
}

async fn run_user_ws(
    listener: TcpListener,
    mut frames: mpsc::Receiver<String>,
    mut shutdown: watch::Receiver<bool>,
) {
    loop {
        let stream = tokio::select! {
            _ = wait_for_shutdown(&mut shutdown) => return,
            accepted = listener.accept() => match accepted {
                Ok((stream, _)) => stream,
                Err(_) => return,
            },
        };
        let Ok(mut socket) = accept_async(stream).await else {
            continue;
        };
        let Some(Ok(Message::Text(subscription))) = socket.next().await else {
            continue;
        };
        assert_user_subscription(subscription.as_str());
        loop {
            tokio::select! {
                _ = wait_for_shutdown(&mut shutdown) => {
                    let _ = socket.close(None).await;
                    return;
                }
                frame = frames.recv() => {
                    let Some(frame) = frame else { return };
                    if socket.send(Message::Text(frame.into())).await.is_err() {
                        break;
                    }
                }
                message = socket.next() => match message {
                    Some(Ok(Message::Text(text))) if text.as_str() == "PING" => {
                        let _ = socket.send(Message::Text("PONG".into())).await;
                    }
                    Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break,
                    _ => {}
                }
            }
        }
    }
}

async fn run_public_ws(listener: TcpListener, mut shutdown: watch::Receiver<bool>) {
    loop {
        let stream = tokio::select! {
            _ = wait_for_shutdown(&mut shutdown) => return,
            accepted = listener.accept() => match accepted {
                Ok((stream, _)) => stream,
                Err(_) => return,
            },
        };
        tokio::spawn(async move {
            if let Ok(mut socket) = accept_async(stream).await {
                let Some(Ok(Message::Text(_subscription))) = socket.next().await else {
                    return;
                };
                while let Some(Ok(message)) = socket.next().await {
                    if matches!(message, Message::Close(_)) {
                        break;
                    }
                    if matches!(&message, Message::Text(text) if text.as_str() == "PING") {
                        let _ = socket.send(Message::Text("PONG".into())).await;
                    }
                }
            }
        });
    }
}

async fn wait_for_shutdown(shutdown: &mut watch::Receiver<bool>) {
    while !*shutdown.borrow() {
        if shutdown.changed().await.is_err() {
            return;
        }
    }
}

async fn read_request(stream: &mut TcpStream) -> Option<Vec<u8>> {
    let mut raw = Vec::new();
    let mut expected = None;
    let mut chunk = [0_u8; 2_048];
    while raw.len() <= MAX_REQUEST_BYTES {
        let read = stream.read(&mut chunk).await.ok()?;
        if read == 0 {
            return (!raw.is_empty()).then_some(raw);
        }
        raw.extend_from_slice(&chunk[..read]);
        if expected.is_none()
            && let Some(position) = raw.windows(4).position(|window| window == b"\r\n\r\n")
        {
            let head_end = position + 4;
            let head = std::str::from_utf8(&raw[..head_end]).ok()?;
            let content_length = head
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().ok())
                        .flatten()
                })
                .unwrap_or(0);
            expected = Some(head_end.checked_add(content_length)?);
        }
        if expected.is_some_and(|length| raw.len() >= length) {
            return Some(raw);
        }
    }
    None
}

async fn respond_json(stream: &mut TcpStream, status: u16, value: Value) {
    respond_text(stream, status, &value.to_string()).await;
}

async fn respond_text(stream: &mut TcpStream, status: u16, body: &str) {
    let reason = match status {
        200 => "OK",
        404 => "Not Found",
        _ => "Synthetic",
    };
    let response = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(response.as_bytes()).await;
}

fn long_market() -> Value {
    json!({
        "enable_order_book": true,
        "active": true,
        "closed": false,
        "archived": false,
        "accepting_orders": true,
        "accepting_order_timestamp": "2026-08-08T00:00:00Z",
        "minimum_order_size": 5,
        "minimum_tick_size": 0.01,
        "condition_id": CONDITION,
        "question_id": MARKET,
        "question": "PM-T1 vertical",
        "description": "deterministic loopback venue",
        "market_slug": "pm-t1-vertical",
        "end_date_iso": "2027-01-01T00:00:00Z",
        "game_start_time": null,
        "seconds_delay": 0,
        "fpmm": "0x0000000000000000000000000000000000000000",
        "maker_base_fee": 0,
        "taker_base_fee": 0,
        "notifications_enabled": true,
        "neg_risk": false,
        "neg_risk_market_id": "",
        "neg_risk_request_id": "",
        "icon": "https://example.invalid/icon.png",
        "image": "https://example.invalid/image.png",
        "rewards": {"rates": null, "min_size": 0, "max_spread": 0},
        "is_50_50_outcome": false,
        "tokens": [{"token_id":"123","outcome":"Yes","price":0.5,"winner":false}],
        "tags": ["synthetic"]
    })
}

fn short_market() -> Value {
    json!({
        "t": [{"t":"123","o":"Yes"},{"t":"456","o":"No"}],
        "mts": 0.01,
        "mos": 5,
        "r": {"rates":null},
        "fd": {"r":0.02,"e":2,"to":true},
        "mbf": 0,
        "tbf": 0,
        "ao": true,
        "sd": 0,
        "gst": null,
        "cbos": true,
        "aot": "2026-08-08T00:00:00Z",
        "rfqe": false,
        "itode": false,
        "ibce": true,
        "oas": 0,
        "c": CONDITION,
        "nr": false
    })
}

fn rest_book() -> Value {
    json!({
        "market": CONDITION,
        "asset_id": "123",
        "timestamp": "1780449126000",
        "hash": "41ee5aaf04387ff1c526d7e2eb78e5a1aac445bd",
        "bids": [{"price":"0.30","size":"100"}],
        "asks": [{"price":"0.60","size":"75"}],
        "min_order_size": "5",
        "tick_size": "0.01",
        "neg_risk": false,
        "last_trade_price": "0.40"
    })
}

fn empty_page() -> Value {
    json!({"data":[],"next_cursor":"LTE=","limit":128,"count":0})
}

fn balance_allowance(conditional: bool, filled: bool) -> Value {
    let balance = if conditional && filled {
        // Conditional-token balances use six protocol decimal places. The
        // accepted BUY fill therefore adds exactly 2.5 outcome tokens.
        "10002500000"
    } else if filled {
        // The same 2.5 @ 0.40 BUY consumes exactly one collateral token.
        "9999000000"
    } else {
        "10000000000"
    };
    json!({
        "balance": balance,
        "allowances": {"0xE111180000d2663C0091e4f400237545B87B996B":"10000000000"}
    })
}

fn trade_page(order: &str, maker: &str) -> Value {
    json!({
        "data": [{
            "id": "pm-t1-fill-1",
            "market": CONDITION,
            "asset_id": "123",
            "side": "BUY",
            "size": "2.5",
            "price": "0.40",
            "status": "CONFIRMED",
            "trader_side": "TAKER",
            "fee_rate_bps": "0",
            "order_id": order,
            "maker_orders": [],
            "maker_address": maker,
            "owner": API_KEY,
            "match_time": "1780449126001",
            "last_update": "1780449126002"
        }],
        "next_cursor": "LTE=",
        "limit": 128,
        "count": 1
    })
}

fn cancelled_order_detail(order: &str, partially_filled: bool, maker: &str) -> Value {
    let size_matched = if partially_filled { "2.5" } else { "0" };
    json!({
        "id": order,
        "market": CONDITION,
        "asset_id": "123",
        "side": "BUY",
        "original_size": "5",
        "size_matched": size_matched,
        "price": "0.40",
        "status": "CANCELED",
        "maker_address": maker,
        "owner": API_KEY,
        "expiration": "0",
        "created_at": 1780449126,
        "order_type": "GTC",
        "outcome": "Yes"
    })
}

fn user_fill_frame(order: &str, maker: &str) -> Value {
    json!([
            {
                "event_type": "order",
                "id": order,
                "market": CONDITION,
                "asset_id": "123",
                "side": "BUY",
                "original_size": "5",
                "size_matched": "2.5",
                "price": "0.40",
                "type": "UPDATE",
                "maker_address": maker,
                "expiration": "0",
                "order_type": "GTC",
                "outcome": "Yes",
                "status": "LIVE",
                "created_at": "1780449126",
                "associate_trades": ["pm-t1-fill-1"],
                "owner": API_KEY,
                "order_owner": API_KEY,
                "timestamp": "1780449126001"
            },
            {
                "event_type": "trade",
                "id": "pm-t1-fill-1",
                "market": CONDITION,
                "asset_id": "123",
                "side": "BUY",
                "size": "2.5",
                "price": "0.40",
                "status": "MATCHED",
                "trader_side": "TAKER",
                "fee_rate_bps": "0",
                "taker_order_id": order,
                "maker_orders": [],
                "owner": API_KEY,
                "trade_owner": API_KEY,
                "timestamp": "1780449126001",
                "last_update": "1780449126002"
            }
    ])
}

fn user_cancel_frame(order: &str, maker: &str) -> Value {
    json!({
        "event_type": "order",
        "id": order,
        "market": CONDITION,
        "asset_id": "123",
        "side": "BUY",
        "original_size": "5",
        "size_matched": "2.5",
        "price": "0.40",
        "type": "CANCELLATION",
        "maker_address": maker,
        "expiration": "0",
        "order_type": "GTC",
        "outcome": "Yes",
        "status": "CANCELED",
        "created_at": "1780449126",
        "associate_trades": ["pm-t1-fill-1"],
        "owner": API_KEY,
        "order_owner": API_KEY,
        "timestamp": "1780449126003"
    })
}

fn assert_private_auth_headers(head: &str) {
    let header = |name: &str| {
        head.lines().find_map(|line| {
            let (actual, value) = line.split_once(':')?;
            actual.eq_ignore_ascii_case(name).then_some(value.trim())
        })
    };
    assert_eq!(header("POLY_ADDRESS"), Some(ADDRESS));
    assert_eq!(header("POLY_API_KEY"), Some(API_KEY));
    assert_eq!(header("POLY_PASSPHRASE"), Some(PASSPHRASE));
    let expected_timestamp = AUTH_SECONDS.to_string();
    assert_eq!(header("POLY_TIMESTAMP"), Some(expected_timestamp.as_str()));
    assert!(
        header("POLY_SIGNATURE").is_some_and(|value| !value.is_empty()),
        "private request must carry its L2 signature"
    );
}

fn assert_user_subscription(raw: &str) {
    let value: Value = serde_json::from_str(raw).expect("authenticated user subscription JSON");
    assert_eq!(value["type"], "user");
    assert_eq!(value["markets"], json!([CONDITION]));
    assert_eq!(value["auth"]["apiKey"], API_KEY);
    assert_eq!(value["auth"]["secret"], API_SECRET);
    assert_eq!(value["auth"]["passphrase"], PASSPHRASE);
}

#[test]
fn bounded_fault_modes_remain_explicit() {
    assert_eq!(
        [
            MutationFaultMode::Accepted,
            MutationFaultMode::Rejected,
            MutationFaultMode::Disconnect,
            MutationFaultMode::PartialResponse,
            MutationFaultMode::ForeignIdentity,
        ]
        .len(),
        5
    );
}
