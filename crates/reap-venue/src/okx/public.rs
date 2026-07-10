use reap_core::{
    AccountUpdate, Balance, BookAction, Channel, EventId, EventKey, FillLiquidity, Level,
    MarketEvent, NormalizedEvent, RawEnvelope, SequencedBookUpdate, Side, Subscription, Venue,
};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{
    ParsedEvent, PrivateOrderState, PrivateOrderUpdate, RemoteFill, VenueAdapter, VenueError,
    VenueEvent,
};

const DEFAULT_PUBLIC_WS: &str = "wss://ws.okx.com:8443/ws/v5/public";
const DEFAULT_PRIVATE_WS: &str = "wss://ws.okx.com:8443/ws/v5/private";

#[derive(Debug, Clone)]
pub struct OkxAdapter {
    public_ws_url: String,
    private_ws_url: String,
}

impl Default for OkxAdapter {
    fn default() -> Self {
        Self {
            public_ws_url: DEFAULT_PUBLIC_WS.to_string(),
            private_ws_url: DEFAULT_PRIVATE_WS.to_string(),
        }
    }
}

impl OkxAdapter {
    pub fn new(public_ws_url: impl Into<String>, private_ws_url: impl Into<String>) -> Self {
        Self {
            public_ws_url: public_ws_url.into(),
            private_ws_url: private_ws_url.into(),
        }
    }

    fn invalid(message: impl Into<String>) -> VenueError {
        VenueError::InvalidPayload {
            venue: Venue::Okx,
            message: message.into(),
        }
    }

    fn parse_book(
        &self,
        envelope: &RawEnvelope,
        push: &OkxPush,
        arg: &OkxArg,
    ) -> Result<Vec<ParsedEvent>, VenueError> {
        let action = match push.action.as_deref() {
            Some("snapshot") => BookAction::Snapshot,
            Some("update") => BookAction::Update,
            other => return Err(Self::invalid(format!("invalid book action {other:?}"))),
        };
        let symbol = arg
            .inst_id
            .clone()
            .or_else(|| envelope.symbol.clone())
            .ok_or_else(|| Self::invalid("book message has no instId"))?;

        push.data
            .iter()
            .map(|value| {
                let data: OkxBook = serde_json::from_value(value.clone())
                    .map_err(|error| Self::invalid(error.to_string()))?;
                let update = SequencedBookUpdate {
                    action,
                    symbol: symbol.clone(),
                    ts_ms: parse_u64("ts", &data.ts)?,
                    prev_seq_id: data.prev_seq_id,
                    seq_id: data.seq_id,
                    bids: parse_levels("bids", data.bids)?,
                    asks: parse_levels("asks", data.asks)?,
                };
                Ok(ParsedEvent {
                    id: EventId {
                        venue: Venue::Okx,
                        channel: Channel::Books,
                        symbol: Some(symbol.clone()),
                        key: EventKey::BookSequence {
                            action,
                            seq_id: update.seq_id,
                        },
                    },
                    event: VenueEvent::Book(update),
                })
            })
            .collect()
    }

    fn parse_trades(
        &self,
        envelope: &RawEnvelope,
        push: &OkxPush,
        arg: &OkxArg,
    ) -> Result<Vec<ParsedEvent>, VenueError> {
        push.data
            .iter()
            .map(|value| {
                let data: OkxTrade = serde_json::from_value(value.clone())
                    .map_err(|error| Self::invalid(error.to_string()))?;
                let symbol = if data.inst_id.is_empty() {
                    arg.inst_id
                        .clone()
                        .or_else(|| envelope.symbol.clone())
                        .ok_or_else(|| Self::invalid("trade message has no instId"))?
                } else {
                    data.inst_id
                };
                let ts_ms = parse_u64("ts", &data.ts)?;
                let trade_id = data.trade_id;
                let event = NormalizedEvent::from(MarketEvent::Trade {
                    ts_ms,
                    symbol: symbol.clone(),
                    price: parse_f64("px", &data.px)?,
                    qty: parse_f64("sz", &data.sz)?,
                    taker_side: parse_side(&data.side)?,
                });
                Ok(ParsedEvent {
                    id: EventId {
                        venue: Venue::Okx,
                        channel: Channel::Trades,
                        symbol: Some(symbol),
                        key: if trade_id.is_empty() {
                            EventKey::RawHash(envelope.raw_hash)
                        } else {
                            EventKey::Trade(trade_id)
                        },
                    },
                    event: VenueEvent::Normalized(event),
                })
            })
            .collect()
    }

    fn parse_orders(
        &self,
        envelope: &RawEnvelope,
        push: &OkxPush,
    ) -> Result<Vec<ParsedEvent>, VenueError> {
        push.data
            .iter()
            .map(|value| {
                let data: OkxOrder = serde_json::from_value(value.clone())
                    .map_err(|error| Self::invalid(error.to_string()))?;
                let update = data.into_private_update()?;
                let cumulative_fill_bits = update.cumulative_filled_qty.to_bits();
                let state = format!("{:?}", update.state).to_lowercase();
                Ok(ParsedEvent {
                    id: EventId {
                        venue: Venue::Okx,
                        channel: Channel::Orders,
                        symbol: Some(update.symbol.clone()),
                        key: EventKey::OrderVersion {
                            order_id: if update.exchange_order_id.is_empty() {
                                update.client_order_id.clone()
                            } else {
                                update.exchange_order_id.clone()
                            },
                            update_time_ms: update.ts_ms,
                            state,
                            cumulative_fill_bits,
                        },
                    },
                    event: VenueEvent::PrivateOrder(update),
                })
            })
            .collect::<Result<Vec<_>, VenueError>>()
            .map(|events| {
                if events.is_empty() && envelope.raw_hash == 0 {
                    Vec::new()
                } else {
                    events
                }
            })
    }

    fn parse_account(
        &self,
        envelope: &RawEnvelope,
        push: &OkxPush,
    ) -> Result<Vec<ParsedEvent>, VenueError> {
        push.data
            .iter()
            .map(|value| {
                let data: OkxAccount = serde_json::from_value(value.clone())
                    .map_err(|error| Self::invalid(error.to_string()))?;
                let ts_ms = parse_u64("uTime", &data.update_time)?;
                let balances = data
                    .details
                    .into_iter()
                    .map(|detail| {
                        Ok(Balance {
                            currency: detail.currency,
                            total: parse_optional_f64("cashBal", &detail.cash_balance)?,
                            available: parse_optional_f64("availBal", &detail.available_balance)?,
                        })
                    })
                    .collect::<Result<Vec<_>, VenueError>>()?;
                let update = AccountUpdate {
                    ts_ms,
                    balances,
                    positions: Vec::new(),
                };
                Ok(ParsedEvent {
                    id: EventId {
                        venue: Venue::Okx,
                        channel: Channel::Account,
                        symbol: None,
                        key: if ts_ms == 0 {
                            EventKey::RawHash(envelope.raw_hash)
                        } else {
                            EventKey::Timestamp(ts_ms)
                        },
                    },
                    event: VenueEvent::Account(update),
                })
            })
            .collect()
    }

    fn parse_fills(
        &self,
        envelope: &RawEnvelope,
        push: &OkxPush,
    ) -> Result<Vec<ParsedEvent>, VenueError> {
        push.data
            .iter()
            .map(|value| {
                let data: OkxFill = serde_json::from_value(value.clone())
                    .map_err(|error| Self::invalid(error.to_string()))?;
                let fill = data.into_remote_fill()?;
                Ok(ParsedEvent {
                    id: EventId {
                        venue: Venue::Okx,
                        channel: Channel::Fills,
                        symbol: Some(fill.symbol.clone()),
                        key: if fill.fill_id.is_empty() {
                            EventKey::RawHash(envelope.raw_hash)
                        } else {
                            EventKey::Fill(fill.fill_id.clone())
                        },
                    },
                    event: VenueEvent::PrivateFill(fill),
                })
            })
            .collect()
    }
}

impl VenueAdapter for OkxAdapter {
    fn venue(&self) -> Venue {
        Venue::Okx
    }

    fn websocket_url(&self, private: bool) -> &str {
        if private {
            &self.private_ws_url
        } else {
            &self.public_ws_url
        }
    }

    fn parse(&self, envelope: &RawEnvelope) -> Result<Vec<ParsedEvent>, VenueError> {
        if envelope.venue != Venue::Okx {
            return Err(Self::invalid("envelope venue is not okx"));
        }
        let push: OkxPush = serde_json::from_str(&envelope.payload)
            .map_err(|error| Self::invalid(error.to_string()))?;
        if push.event.is_some() {
            if push.event.as_deref() == Some("error") {
                return Err(Self::invalid(format!(
                    "websocket error {}: {}",
                    push.code.unwrap_or_default(),
                    push.message.unwrap_or_default()
                )));
            }
            return Ok(Vec::new());
        }
        let arg = push
            .arg
            .as_ref()
            .ok_or_else(|| Self::invalid("push message has no arg"))?;
        match arg.channel.as_str() {
            "books" | "books-l2-tbt" | "books50-l2-tbt" => self.parse_book(envelope, &push, arg),
            "trades" | "trades-all" => self.parse_trades(envelope, &push, arg),
            "orders" => self.parse_orders(envelope, &push),
            "fills" => self.parse_fills(envelope, &push),
            "account" => self.parse_account(envelope, &push),
            channel => Err(VenueError::UnsupportedChannel {
                venue: Venue::Okx,
                channel: channel.to_string(),
            }),
        }
    }

    fn subscription_message(&self, subscriptions: &[Subscription]) -> Result<String, VenueError> {
        let args = subscriptions
            .iter()
            .map(|subscription| {
                if subscription.venue != Venue::Okx {
                    return Err(Self::invalid("non-OKX subscription passed to OKX adapter"));
                }
                let channel = match subscription.channel {
                    Channel::Books => "books",
                    Channel::Trades => "trades",
                    Channel::Orders => "orders",
                    Channel::Fills => "fills",
                    Channel::Account => "account",
                    Channel::Custom(ref channel) => channel,
                };
                let mut arg = json!({ "channel": channel });
                if let Some(symbol) = &subscription.symbol {
                    arg["instId"] = Value::String(symbol.clone());
                } else if subscription.channel == Channel::Orders {
                    arg["instType"] = Value::String("ANY".to_string());
                }
                Ok(arg)
            })
            .collect::<Result<Vec<_>, VenueError>>()?;
        Ok(serde_json::to_string(
            &json!({ "op": "subscribe", "args": args }),
        )?)
    }
}

#[derive(Debug, Deserialize)]
struct OkxPush {
    #[serde(default)]
    event: Option<String>,
    #[serde(default)]
    code: Option<String>,
    #[serde(default, rename = "msg")]
    message: Option<String>,
    #[serde(default)]
    arg: Option<OkxArg>,
    #[serde(default)]
    action: Option<String>,
    #[serde(default)]
    data: Vec<Value>,
}

#[derive(Debug, Deserialize)]
struct OkxArg {
    channel: String,
    #[serde(default, rename = "instId")]
    inst_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OkxBook {
    asks: Vec<Vec<String>>,
    bids: Vec<Vec<String>>,
    ts: String,
    #[serde(rename = "prevSeqId")]
    prev_seq_id: i64,
    #[serde(rename = "seqId")]
    seq_id: i64,
}

#[derive(Debug, Deserialize)]
struct OkxTrade {
    #[serde(default, rename = "instId")]
    inst_id: String,
    #[serde(default, rename = "tradeId")]
    trade_id: String,
    px: String,
    sz: String,
    side: String,
    ts: String,
}

#[derive(Debug, Deserialize)]
struct OkxOrder {
    #[serde(default, rename = "ordId")]
    order_id: String,
    #[serde(default, rename = "clOrdId")]
    client_order_id: String,
    #[serde(rename = "instId")]
    symbol: String,
    side: String,
    state: String,
    #[serde(default)]
    px: String,
    #[serde(default)]
    sz: String,
    #[serde(default, rename = "accFillSz")]
    cumulative_filled_qty: String,
    #[serde(default, rename = "avgPx")]
    average_fill_price: String,
    #[serde(default, rename = "fillSz")]
    last_fill_qty: String,
    #[serde(default, rename = "fillPx")]
    last_fill_price: String,
    #[serde(default, rename = "execType")]
    execution_type: String,
    #[serde(default, rename = "tradeId")]
    trade_id: String,
    #[serde(default, rename = "uTime")]
    update_time: String,
    #[serde(default, rename = "fillTime")]
    fill_time: String,
    #[serde(default, rename = "cancelSourceReason")]
    cancel_reason: String,
    #[serde(default, rename = "msg")]
    message: String,
}

impl OkxOrder {
    fn into_private_update(self) -> Result<PrivateOrderUpdate, VenueError> {
        let state = match self.state.as_str() {
            "effective" | "pending" => PrivateOrderState::Pending,
            "live" => PrivateOrderState::Live,
            "partially_filled" => PrivateOrderState::PartiallyFilled,
            "filled" => PrivateOrderState::Filled,
            "canceled" | "mmp_canceled" => PrivateOrderState::Cancelled,
            "order_failed" | "failed" => PrivateOrderState::Rejected,
            state => return Err(OkxAdapter::invalid(format!("unknown order state {state}"))),
        };
        let ts = if self.fill_time.is_empty() {
            &self.update_time
        } else {
            &self.fill_time
        };
        let reject_reason = if self.message.is_empty() {
            self.cancel_reason
        } else {
            self.message
        };
        Ok(PrivateOrderUpdate {
            ts_ms: parse_optional_u64("uTime", ts)?,
            exchange_order_id: self.order_id,
            client_order_id: self.client_order_id,
            symbol: self.symbol,
            side: parse_side(&self.side)?,
            state,
            price: parse_optional_f64("px", &self.px)?,
            qty: parse_optional_f64("sz", &self.sz)?,
            cumulative_filled_qty: parse_optional_f64("accFillSz", &self.cumulative_filled_qty)?,
            average_fill_price: parse_optional_f64("avgPx", &self.average_fill_price)?,
            last_fill_qty: parse_optional_f64("fillSz", &self.last_fill_qty)?,
            last_fill_price: parse_optional_f64("fillPx", &self.last_fill_price)?,
            liquidity: match self.execution_type.as_str() {
                "M" => Some(FillLiquidity::Maker),
                "T" => Some(FillLiquidity::Taker),
                _ => None,
            },
            fill_id: (!self.trade_id.is_empty()).then_some(self.trade_id),
            reject_reason,
        })
    }
}

#[derive(Debug, Deserialize)]
struct OkxAccount {
    #[serde(default, rename = "uTime")]
    update_time: String,
    #[serde(default)]
    details: Vec<OkxBalance>,
}

#[derive(Debug, Deserialize)]
struct OkxBalance {
    #[serde(rename = "ccy")]
    currency: String,
    #[serde(default, rename = "cashBal")]
    cash_balance: String,
    #[serde(default, rename = "availBal")]
    available_balance: String,
}

#[derive(Debug, Deserialize)]
struct OkxFill {
    #[serde(default, rename = "tradeId")]
    fill_id: String,
    #[serde(default, rename = "ordId")]
    order_id: String,
    #[serde(default, rename = "clOrdId")]
    client_order_id: String,
    #[serde(rename = "instId")]
    symbol: String,
    side: String,
    #[serde(rename = "fillPx")]
    price: String,
    #[serde(rename = "fillSz")]
    qty: String,
    #[serde(default, rename = "execType")]
    execution_type: String,
    ts: String,
}

impl OkxFill {
    fn into_remote_fill(self) -> Result<RemoteFill, VenueError> {
        Ok(RemoteFill {
            fill_id: self.fill_id,
            exchange_order_id: self.order_id,
            client_order_id: self.client_order_id,
            symbol: self.symbol,
            side: parse_side(&self.side)?,
            price: parse_f64("fillPx", &self.price)?,
            qty: parse_f64("fillSz", &self.qty)?,
            liquidity: match self.execution_type.as_str() {
                "M" => FillLiquidity::Maker,
                "T" | "" => FillLiquidity::Taker,
                value => {
                    return Err(OkxAdapter::invalid(format!("invalid execType {value}")));
                }
            },
            ts_ms: parse_u64("ts", &self.ts)?,
        })
    }
}

fn parse_levels(name: &str, levels: Vec<Vec<String>>) -> Result<Vec<Level>, VenueError> {
    levels
        .into_iter()
        .map(|level| {
            if level.len() < 2 {
                return Err(OkxAdapter::invalid(format!(
                    "{name} level has fewer than two fields"
                )));
            }
            Ok(Level::new(
                parse_f64("level price", &level[0])?,
                parse_f64("level size", &level[1])?,
            ))
        })
        .collect()
}

fn parse_side(value: &str) -> Result<Side, VenueError> {
    match value {
        "buy" => Ok(Side::Buy),
        "sell" => Ok(Side::Sell),
        side => Err(OkxAdapter::invalid(format!("invalid side {side}"))),
    }
}

fn parse_f64(name: &str, value: &str) -> Result<f64, VenueError> {
    value
        .parse()
        .map_err(|error| OkxAdapter::invalid(format!("invalid {name} {value:?}: {error}")))
}

fn parse_optional_f64(name: &str, value: &str) -> Result<f64, VenueError> {
    if value.is_empty() {
        Ok(0.0)
    } else {
        parse_f64(name, value)
    }
}

fn parse_u64(name: &str, value: &str) -> Result<u64, VenueError> {
    value
        .parse()
        .map_err(|error| OkxAdapter::invalid(format!("invalid {name} {value:?}: {error}")))
}

fn parse_optional_u64(name: &str, value: &str) -> Result<u64, VenueError> {
    if value.is_empty() {
        Ok(0)
    } else {
        parse_u64(name, value)
    }
}

#[cfg(test)]
mod tests {
    use reap_core::{ConnId, FeedPriority};

    use super::*;

    fn envelope(channel: Channel, payload: &str) -> RawEnvelope {
        RawEnvelope {
            venue: Venue::Okx,
            conn_id: ConnId::new("test"),
            channel,
            symbol: Some("BTC-USDT".to_string()),
            recv_ts_ns: 1,
            raw_hash: 7,
            payload: payload.to_string(),
        }
    }

    #[test]
    fn parses_sequenced_book_snapshot() {
        let payload = r#"{"arg":{"channel":"books","instId":"BTC-USDT"},"action":"snapshot","data":[{"asks":[["101","2","0","1"]],"bids":[["100","3","0","1"]],"ts":"1000","prevSeqId":-1,"seqId":10}]}"#;
        let events = OkxAdapter::default()
            .parse(&envelope(Channel::Books, payload))
            .unwrap();

        assert_eq!(events.len(), 1);
        let VenueEvent::Book(book) = &events[0].event else {
            panic!("expected book event");
        };
        assert_eq!(book.action, BookAction::Snapshot);
        assert_eq!(book.seq_id, 10);
        assert_eq!(book.bids[0], Level::new(100.0, 3.0));
    }

    #[test]
    fn parses_trade_with_trade_id() {
        let payload = r#"{"arg":{"channel":"trades","instId":"BTC-USDT"},"data":[{"instId":"BTC-USDT","tradeId":"42","px":"100.5","sz":"0.2","side":"sell","ts":"1001"}]}"#;
        let events = OkxAdapter::default()
            .parse(&envelope(Channel::Trades, payload))
            .unwrap();

        assert!(matches!(events[0].id.key, EventKey::Trade(ref id) if id == "42"));
        assert!(matches!(
            events[0].event,
            VenueEvent::Normalized(NormalizedEvent::Market(MarketEvent::Trade {
                taker_side: Side::Sell,
                ..
            }))
        ));
    }

    #[test]
    fn builds_public_and_private_subscriptions() {
        let adapter = OkxAdapter::default();
        let public = adapter
            .subscription_message(&[Subscription::public(
                Venue::Okx,
                Channel::Books,
                "BTC-USDT",
                FeedPriority::Critical,
            )])
            .unwrap();
        let private = adapter
            .subscription_message(&[Subscription::private(
                Venue::Okx,
                Channel::Orders,
                FeedPriority::Critical,
            )])
            .unwrap();

        assert_eq!(
            serde_json::from_str::<Value>(&public).unwrap(),
            json!({"op":"subscribe","args":[{"channel":"books","instId":"BTC-USDT"}]})
        );
        assert_eq!(
            serde_json::from_str::<Value>(&private).unwrap(),
            json!({"op":"subscribe","args":[{"channel":"orders","instType":"ANY"}]})
        );
    }

    #[test]
    fn parses_private_order_fill_and_account_updates() {
        let adapter = OkxAdapter::default();
        let order = r#"{"arg":{"channel":"orders","instType":"ANY"},"data":[{"ordId":"123","clOrdId":"reap1","instId":"BTC-USDT","side":"buy","state":"partially_filled","px":"100","sz":"1","accFillSz":"0.4","avgPx":"99.5","fillSz":"0.4","fillPx":"99.5","execType":"M","tradeId":"fill1","uTime":"1000","fillTime":"1000"}]}"#;
        let fills = r#"{"arg":{"channel":"fills","instId":"BTC-USDT"},"data":[{"tradeId":"fill2","ordId":"123","clOrdId":"reap1","instId":"BTC-USDT","side":"buy","fillPx":"99.4","fillSz":"0.1","execType":"T","ts":"1001"}]}"#;
        let account = r#"{"arg":{"channel":"account"},"data":[{"uTime":"1002","details":[{"ccy":"USDT","cashBal":"1000","availBal":"900"}]}]}"#;

        let order_events = adapter.parse(&envelope(Channel::Orders, order)).unwrap();
        let fill_events = adapter.parse(&envelope(Channel::Fills, fills)).unwrap();
        let account_events = adapter.parse(&envelope(Channel::Account, account)).unwrap();

        assert!(matches!(
            order_events[0].event,
            VenueEvent::PrivateOrder(PrivateOrderUpdate {
                state: PrivateOrderState::PartiallyFilled,
                last_fill_qty: 0.4,
                ..
            })
        ));
        assert!(matches!(
            fill_events[0].event,
            VenueEvent::PrivateFill(RemoteFill {
                liquidity: FillLiquidity::Taker,
                qty: 0.1,
                ..
            })
        ));
        assert!(matches!(
            account_events[0].event,
            VenueEvent::Account(AccountUpdate { ref balances, .. }) if balances[0].available == 900.0
        ));
    }
}
