use reap_core::{
    AccountUpdate, Balance, BookAction, Channel, EventId, EventKey, FillFee, FillLiquidity,
    FundingSettlement, Level, MarginSnapshot, MarketEvent, NormalizedEvent, Position,
    PositionMarginMode, RawEnvelope, SequencedBookUpdate, Side, Subscription, Venue,
};
use reap_okx_public_source::extract_legacy_index_ticker_fields;
use serde::Deserialize;
use serde_json::{Value, json};

use super::capabilities::{
    WS_ACCOUNT, WS_BOOKS, WS_FILLS, WS_FUNDING_RATE, WS_INDEX_TICKERS, WS_MARK_PRICE, WS_ORDERS,
    WS_POSITIONS, WS_PRICE_LIMIT, WS_SUBSCRIBE, WS_TRADES,
};

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
    account_id: Option<String>,
}

impl Default for OkxAdapter {
    fn default() -> Self {
        Self {
            public_ws_url: DEFAULT_PUBLIC_WS.to_string(),
            private_ws_url: DEFAULT_PRIVATE_WS.to_string(),
            account_id: None,
        }
    }
}

impl OkxAdapter {
    pub fn new(public_ws_url: impl Into<String>, private_ws_url: impl Into<String>) -> Self {
        Self {
            public_ws_url: public_ws_url.into(),
            private_ws_url: private_ws_url.into(),
            account_id: None,
        }
    }

    pub fn with_account_id(mut self, account_id: impl Into<String>) -> Self {
        self.account_id = Some(account_id.into());
        self
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
        action: Option<&str>,
        data: Vec<Value>,
        arg: &OkxArg,
    ) -> Result<Vec<ParsedEvent>, VenueError> {
        let action = match action {
            Some("snapshot") => BookAction::Snapshot,
            Some("update") => BookAction::Update,
            other => return Err(Self::invalid(format!("invalid book action {other:?}"))),
        };
        let symbol = arg
            .inst_id
            .clone()
            .or_else(|| envelope.symbol.clone())
            .ok_or_else(|| Self::invalid("book message has no instId"))?;

        data.into_iter()
            .map(|value| {
                let data: OkxBook = serde_json::from_value(value)
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
                            prev_seq_id: update.prev_seq_id,
                            seq_id: update.seq_id,
                            ts_ms: update.ts_ms,
                            raw_hash: envelope.raw_hash,
                        },
                    },
                    account_id: None,
                    event: VenueEvent::Book(update),
                })
            })
            .collect()
    }

    fn parse_trades(
        &self,
        envelope: &RawEnvelope,
        data: Vec<Value>,
        arg: &OkxArg,
    ) -> Result<Vec<ParsedEvent>, VenueError> {
        data.into_iter()
            .map(|value| {
                let data: OkxTrade = serde_json::from_value(value)
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
                    account_id: None,
                    event: VenueEvent::Normalized(event),
                })
            })
            .collect()
    }

    fn parse_funding_rates(
        &self,
        envelope: &RawEnvelope,
        data: Vec<Value>,
        arg: &OkxArg,
    ) -> Result<Vec<ParsedEvent>, VenueError> {
        let arg_symbol = arg
            .inst_id
            .clone()
            .or_else(|| envelope.symbol.clone())
            .ok_or_else(|| Self::invalid("funding-rate message has no instId"))?;
        data.into_iter()
            .map(|value| {
                let data: OkxFundingRate = serde_json::from_value(value)
                    .map_err(|error| Self::invalid(error.to_string()))?;
                let settlement = data.settlement()?;
                let symbol = if data.inst_id.is_empty() {
                    arg_symbol.clone()
                } else {
                    data.inst_id
                };
                let ts_ms = parse_u64("ts", &data.ts)?;
                Ok(normalized_market_event(
                    envelope,
                    arg,
                    symbol.clone(),
                    ts_ms,
                    MarketEvent::FundingRate {
                        ts_ms,
                        symbol,
                        rate: parse_f64("fundingRate", &data.funding_rate)?,
                        funding_time_ms: parse_u64("fundingTime", &data.funding_time)?,
                        settlement,
                    },
                ))
            })
            .collect()
    }

    fn parse_index_tickers(
        &self,
        envelope: &RawEnvelope,
        data: Vec<Value>,
        arg: &OkxArg,
    ) -> Result<Vec<ParsedEvent>, VenueError> {
        data.into_iter()
            .map(|value| {
                let data: OkxIndexTicker = serde_json::from_value(value)
                    .map_err(|error| Self::invalid(error.to_string()))?;
                let fields = extract_legacy_index_ticker_fields(
                    data.inst_id,
                    arg.inst_id.as_deref(),
                    envelope.symbol.as_deref(),
                    data.index_price,
                    &data.ts,
                )
                .map_err(|error| Self::invalid(error.to_string()))?;
                let symbol = fields.instrument().to_string();
                let ts_ms = fields.venue_ts_ms();
                Ok(normalized_market_event(
                    envelope,
                    arg,
                    symbol.clone(),
                    ts_ms,
                    MarketEvent::IndexPrice {
                        ts_ms,
                        symbol,
                        price: parse_f64("idxPx", fields.index_price_lexeme())?,
                    },
                ))
            })
            .collect()
    }

    fn parse_price_limits(
        &self,
        envelope: &RawEnvelope,
        data: Vec<Value>,
        arg: &OkxArg,
    ) -> Result<Vec<ParsedEvent>, VenueError> {
        data.into_iter()
            .map(|value| {
                let data: OkxPriceLimit = serde_json::from_value(value)
                    .map_err(|error| Self::invalid(error.to_string()))?;
                let symbol = data
                    .inst_id
                    .or_else(|| arg.inst_id.clone())
                    .or_else(|| envelope.symbol.clone())
                    .ok_or_else(|| Self::invalid("price-limit message has no instId"))?;
                let ts_ms = parse_u64("ts", &data.ts)?;
                Ok(normalized_market_event(
                    envelope,
                    arg,
                    symbol.clone(),
                    ts_ms,
                    MarketEvent::PriceLimits {
                        ts_ms,
                        symbol,
                        mark_price: 0.0,
                        limit_down: parse_optional_f64("sellLmt", &data.sell_limit)?,
                        limit_up: parse_optional_f64("buyLmt", &data.buy_limit)?,
                    },
                ))
            })
            .collect()
    }

    fn parse_mark_prices(
        &self,
        envelope: &RawEnvelope,
        data: Vec<Value>,
        arg: &OkxArg,
    ) -> Result<Vec<ParsedEvent>, VenueError> {
        data.into_iter()
            .map(|value| {
                let data: OkxMarkPrice = serde_json::from_value(value)
                    .map_err(|error| Self::invalid(error.to_string()))?;
                let symbol = data
                    .inst_id
                    .or_else(|| arg.inst_id.clone())
                    .or_else(|| envelope.symbol.clone())
                    .ok_or_else(|| Self::invalid("mark-price message has no instId"))?;
                let ts_ms = parse_u64("ts", &data.ts)?;
                Ok(normalized_market_event(
                    envelope,
                    arg,
                    symbol.clone(),
                    ts_ms,
                    MarketEvent::PriceLimits {
                        ts_ms,
                        symbol,
                        mark_price: parse_f64("markPx", &data.mark_price)?,
                        limit_down: 0.0,
                        limit_up: 0.0,
                    },
                ))
            })
            .collect()
    }

    fn parse_orders(
        &self,
        envelope: &RawEnvelope,
        data: Vec<Value>,
    ) -> Result<Vec<ParsedEvent>, VenueError> {
        data.into_iter()
            .map(|value| {
                let data: OkxOrder = serde_json::from_value(value)
                    .map_err(|error| Self::invalid(error.to_string()))?;
                let update = data.into_private_update()?;
                let cumulative_fill_bits = update.cumulative_filled_qty.to_bits();
                let state = format!("{:?}", update.state).to_lowercase();
                Ok(ParsedEvent {
                    id: EventId {
                        venue: Venue::Okx,
                        channel: Channel::Orders,
                        symbol: Some(scoped_private_symbol(
                            self.account_id.as_deref(),
                            &update.symbol,
                        )),
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
                    account_id: self.account_id.clone(),
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
        data: Vec<Value>,
    ) -> Result<Vec<ParsedEvent>, VenueError> {
        data.into_iter()
            .map(|value| {
                let data: OkxAccount = serde_json::from_value(value)
                    .map_err(|error| Self::invalid(error.to_string()))?;
                let ts_ms = parse_u64("uTime", &data.update_time)?;
                let exchange_ratio = parse_optional_f64_value("mgnRatio", &data.margin_ratio)?;
                let adjusted_equity_usd = parse_optional_f64_value("adjEq", &data.adjusted_equity)?;
                let notional_usd = parse_optional_f64_value("notionalUsd", &data.notional_usd)?;
                let margins = if exchange_ratio.is_none()
                    && adjusted_equity_usd.is_none()
                    && notional_usd.is_none()
                {
                    Vec::new()
                } else {
                    vec![MarginSnapshot {
                        account_id: self.account_id.clone(),
                        ratio: None,
                        exchange_ratio,
                        adjusted_equity_usd,
                        notional_usd,
                    }]
                };
                let balances = data
                    .details
                    .into_iter()
                    .map(|detail| {
                        Ok(Balance {
                            account_id: self.account_id.clone(),
                            currency: detail.currency,
                            total: parse_optional_f64("cashBal", &detail.cash_balance)?,
                            available: parse_optional_f64("availBal", &detail.available_balance)?,
                            equity: parse_optional_f64("eq", &detail.equity)?,
                            liability: parse_optional_f64("liab", &detail.liability)?,
                            max_loan: parse_optional_f64("maxLoan", &detail.max_loan)?,
                            forced_repayment_indicator: parse_forced_repayment_indicator(
                                &detail.forced_repayment_indicator,
                            )?,
                        })
                    })
                    .collect::<Result<Vec<_>, VenueError>>()?;
                let update = AccountUpdate {
                    ts_ms,
                    balances,
                    positions: Vec::new(),
                    margins,
                };
                Ok(ParsedEvent {
                    id: EventId {
                        venue: Venue::Okx,
                        channel: Channel::Account,
                        symbol: self.account_id.clone(),
                        key: if ts_ms == 0 {
                            EventKey::RawHash(envelope.raw_hash)
                        } else {
                            EventKey::Timestamp(ts_ms)
                        },
                    },
                    account_id: self.account_id.clone(),
                    event: VenueEvent::Account(update),
                })
            })
            .collect()
    }

    fn parse_positions(&self, data: Vec<Value>) -> Result<Vec<ParsedEvent>, VenueError> {
        data.into_iter()
            .map(|value| {
                let data: OkxPosition = serde_json::from_value(value)
                    .map_err(|error| Self::invalid(error.to_string()))?;
                let ts_ms = parse_u64("uTime", &data.update_time)?;
                let mut qty = parse_f64("pos", &data.qty)?;
                if data.position_side == "short" && qty > 0.0 {
                    qty = -qty;
                }
                let symbol = data.symbol;
                Ok(ParsedEvent {
                    id: EventId {
                        venue: Venue::Okx,
                        channel: Channel::Positions,
                        symbol: Some(match &self.account_id {
                            Some(account_id) => format!("{account_id}:{symbol}"),
                            None => symbol.clone(),
                        }),
                        key: EventKey::Timestamp(ts_ms),
                    },
                    account_id: self.account_id.clone(),
                    event: VenueEvent::Account(AccountUpdate {
                        ts_ms,
                        balances: Vec::new(),
                        positions: vec![Position {
                            symbol,
                            qty,
                            avg_price: parse_optional_f64("avgPx", &data.average_price)?,
                            margin_mode: Some(parse_position_margin_mode(&data.margin_mode)?),
                        }],
                        margins: Vec::new(),
                    }),
                })
            })
            .collect()
    }

    fn parse_fills(
        &self,
        envelope: &RawEnvelope,
        data: Vec<Value>,
    ) -> Result<Vec<ParsedEvent>, VenueError> {
        data.into_iter()
            .map(|value| {
                let data: OkxFill = serde_json::from_value(value)
                    .map_err(|error| Self::invalid(error.to_string()))?;
                let fill = data.into_remote_fill()?;
                Ok(ParsedEvent {
                    id: EventId {
                        venue: Venue::Okx,
                        channel: Channel::Fills,
                        symbol: Some(scoped_private_symbol(
                            self.account_id.as_deref(),
                            &fill.symbol,
                        )),
                        key: if fill.fill_id.is_empty() {
                            EventKey::RawHash(envelope.raw_hash)
                        } else {
                            EventKey::Fill(fill.fill_id.clone())
                        },
                    },
                    account_id: self.account_id.clone(),
                    event: VenueEvent::PrivateFill(fill),
                })
            })
            .collect()
    }
}

fn scoped_private_symbol(account_id: Option<&str>, symbol: &str) -> String {
    match account_id {
        Some(account_id) => format!("{account_id}:{symbol}"),
        None => symbol.to_string(),
    }
}

fn normalized_market_event(
    envelope: &RawEnvelope,
    arg: &OkxArg,
    symbol: String,
    ts_ms: u64,
    event: MarketEvent,
) -> ParsedEvent {
    ParsedEvent {
        id: EventId {
            venue: Venue::Okx,
            channel: Channel::Custom(arg.channel.clone()),
            symbol: Some(symbol),
            key: if ts_ms == 0 {
                EventKey::RawHash(envelope.raw_hash)
            } else {
                EventKey::TimestampHash {
                    ts_ms,
                    raw_hash: envelope.raw_hash,
                }
            },
        },
        account_id: None,
        event: VenueEvent::Normalized(NormalizedEvent::Market(event)),
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
        let OkxPush {
            event,
            code,
            message,
            arg,
            action,
            data,
        } = push;
        if event.is_some() {
            if event.as_deref() == Some("error") {
                return Err(Self::invalid(format!(
                    "websocket error {}: {}",
                    code.unwrap_or_default(),
                    message.unwrap_or_default()
                )));
            }
            return Ok(Vec::new());
        }
        let arg = arg.ok_or_else(|| Self::invalid("push message has no arg"))?;
        match arg.channel.as_str() {
            channel if channel == WS_BOOKS.endpoint_or_channel => {
                self.parse_book(envelope, action.as_deref(), data, &arg)
            }
            "books-l2-tbt" | "books50-l2-tbt" => {
                self.parse_book(envelope, action.as_deref(), data, &arg)
            }
            channel if channel == WS_TRADES.endpoint_or_channel || channel == "trades-all" => {
                self.parse_trades(envelope, data, &arg)
            }
            channel if channel == WS_FUNDING_RATE.endpoint_or_channel => {
                self.parse_funding_rates(envelope, data, &arg)
            }
            channel if channel == WS_INDEX_TICKERS.endpoint_or_channel => {
                self.parse_index_tickers(envelope, data, &arg)
            }
            channel if channel == WS_PRICE_LIMIT.endpoint_or_channel => {
                self.parse_price_limits(envelope, data, &arg)
            }
            channel if channel == WS_MARK_PRICE.endpoint_or_channel => {
                self.parse_mark_prices(envelope, data, &arg)
            }
            channel if channel == WS_ORDERS.endpoint_or_channel => {
                self.parse_orders(envelope, data)
            }
            channel if channel == WS_FILLS.endpoint_or_channel => self.parse_fills(envelope, data),
            channel if channel == WS_ACCOUNT.endpoint_or_channel => {
                self.parse_account(envelope, data)
            }
            channel if channel == WS_POSITIONS.endpoint_or_channel => self.parse_positions(data),
            channel => Err(VenueError::UnsupportedChannel {
                venue: Venue::Okx,
                channel: channel.to_string(),
            }),
        }
    }

    fn is_data_frame(&self, envelope: &RawEnvelope) -> Result<bool, VenueError> {
        if envelope.venue != Venue::Okx {
            return Err(Self::invalid("envelope venue is not okx"));
        }
        let value: Value = serde_json::from_str(&envelope.payload)
            .map_err(|error| Self::invalid(error.to_string()))?;
        Ok(value.get("event").is_none()
            && value.get("arg").is_some_and(Value::is_object)
            && value.get("data").is_some_and(Value::is_array))
    }

    fn subscription_message(&self, subscriptions: &[Subscription]) -> Result<String, VenueError> {
        let args = subscriptions
            .iter()
            .map(|subscription| {
                if subscription.venue != Venue::Okx {
                    return Err(Self::invalid("non-OKX subscription passed to OKX adapter"));
                }
                let channel = match subscription.channel {
                    Channel::Books => WS_BOOKS.endpoint_or_channel,
                    Channel::Trades => WS_TRADES.endpoint_or_channel,
                    Channel::Orders => WS_ORDERS.endpoint_or_channel,
                    Channel::Fills => WS_FILLS.endpoint_or_channel,
                    Channel::Account => WS_ACCOUNT.endpoint_or_channel,
                    Channel::Positions => WS_POSITIONS.endpoint_or_channel,
                    Channel::Custom(ref channel) => channel,
                };
                let mut arg = json!({ "channel": channel });
                if let Some(symbol) = &subscription.symbol {
                    arg["instId"] = Value::String(symbol.clone());
                } else if matches!(subscription.channel, Channel::Orders | Channel::Positions) {
                    arg["instType"] = Value::String("ANY".to_string());
                }
                Ok(arg)
            })
            .collect::<Result<Vec<_>, VenueError>>()?;
        Ok(serde_json::to_string(
            &json!({ "op": WS_SUBSCRIBE.endpoint_or_channel, "args": args }),
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
struct OkxFundingRate {
    #[serde(default, rename = "instId")]
    inst_id: String,
    #[serde(rename = "fundingRate")]
    funding_rate: String,
    #[serde(rename = "fundingTime")]
    funding_time: String,
    #[serde(default, rename = "prevFundingTime")]
    previous_funding_time: String,
    #[serde(default, rename = "settFundingRate")]
    settled_funding_rate: String,
    #[serde(default, rename = "settState")]
    settlement_state: String,
    ts: String,
}

impl OkxFundingRate {
    fn settlement(&self) -> Result<Option<FundingSettlement>, VenueError> {
        match self.settlement_state.as_str() {
            "" | "processing" => Ok(None),
            "settled" => {
                if self.previous_funding_time.is_empty() || self.settled_funding_rate.is_empty() {
                    return Ok(None);
                }
                let funding_time_ms = parse_u64("prevFundingTime", &self.previous_funding_time)?;
                let upcoming_funding_time_ms = parse_u64("fundingTime", &self.funding_time)?;
                if funding_time_ms == 0 || funding_time_ms >= upcoming_funding_time_ms {
                    return Err(OkxAdapter::invalid(format!(
                        "prevFundingTime {funding_time_ms} must precede fundingTime {upcoming_funding_time_ms}"
                    )));
                }
                Ok(Some(FundingSettlement {
                    funding_time_ms,
                    rate: parse_f64("settFundingRate", &self.settled_funding_rate)?,
                }))
            }
            state => Err(OkxAdapter::invalid(format!(
                "unknown funding settlement state {state:?}"
            ))),
        }
    }
}

#[derive(Debug, Deserialize)]
struct OkxIndexTicker {
    #[serde(default, rename = "instId")]
    inst_id: Option<String>,
    #[serde(rename = "idxPx")]
    index_price: String,
    ts: String,
}

#[derive(Debug, Deserialize)]
struct OkxPriceLimit {
    #[serde(default, rename = "instId")]
    inst_id: Option<String>,
    #[serde(default, rename = "buyLmt")]
    buy_limit: String,
    #[serde(default, rename = "sellLmt")]
    sell_limit: String,
    ts: String,
}

#[derive(Debug, Deserialize)]
struct OkxMarkPrice {
    #[serde(default, rename = "instId")]
    inst_id: Option<String>,
    #[serde(rename = "markPx")]
    mark_price: String,
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
    // Unlike order-level `fee`/`rebate`, these fields describe this update's fill.
    #[serde(default, rename = "fillFee")]
    fill_fee: String,
    #[serde(default, rename = "fillFeeCcy")]
    fill_fee_currency: String,
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
            last_fill_fee: parse_fill_fee(
                "fillFee",
                &self.fill_fee,
                "fillFeeCcy",
                &self.fill_fee_currency,
            )?,
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
    #[serde(default, rename = "mgnRatio")]
    margin_ratio: String,
    #[serde(default, rename = "adjEq")]
    adjusted_equity: String,
    #[serde(default, rename = "notionalUsd")]
    notional_usd: String,
}

#[derive(Debug, Deserialize)]
struct OkxPosition {
    #[serde(rename = "instId")]
    symbol: String,
    #[serde(rename = "pos")]
    qty: String,
    #[serde(default, rename = "avgPx")]
    average_price: String,
    #[serde(default, rename = "posSide")]
    position_side: String,
    #[serde(rename = "mgnMode")]
    margin_mode: String,
    #[serde(rename = "uTime")]
    update_time: String,
}

#[derive(Debug, Deserialize)]
struct OkxBalance {
    #[serde(rename = "ccy")]
    currency: String,
    #[serde(default, rename = "cashBal")]
    cash_balance: String,
    #[serde(default, rename = "availBal")]
    available_balance: String,
    #[serde(default, rename = "eq")]
    equity: String,
    #[serde(default, rename = "liab")]
    liability: String,
    #[serde(default, rename = "maxLoan")]
    max_loan: String,
    #[serde(default, rename = "twap")]
    forced_repayment_indicator: String,
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
    #[serde(default)]
    fee: String,
    #[serde(default, rename = "feeCcy")]
    fee_currency: String,
    ts: String,
}

impl OkxFill {
    fn into_remote_fill(self) -> Result<RemoteFill, VenueError> {
        let client_order_id = if self.client_order_id == "0" {
            String::new()
        } else {
            self.client_order_id
        };
        Ok(RemoteFill {
            fill_id: self.fill_id,
            exchange_order_id: self.order_id,
            client_order_id,
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
            fee: parse_fill_fee("fee", &self.fee, "feeCcy", &self.fee_currency)?,
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

fn parse_position_margin_mode(value: &str) -> Result<PositionMarginMode, VenueError> {
    match value {
        "cross" => Ok(PositionMarginMode::Cross),
        "isolated" => Ok(PositionMarginMode::Isolated),
        other => Err(OkxAdapter::invalid(format!(
            "invalid mgnMode {other:?}; expected cross or isolated"
        ))),
    }
}

fn parse_forced_repayment_indicator(value: &str) -> Result<Option<u8>, VenueError> {
    if value.is_empty() {
        return Ok(None);
    }
    let indicator = value
        .parse::<u8>()
        .map_err(|error| OkxAdapter::invalid(format!("invalid twap {value:?}: {error}")))?;
    if indicator > 5 {
        return Err(OkxAdapter::invalid(format!(
            "invalid twap {indicator}; expected 0 through 5"
        )));
    }
    Ok(Some(indicator))
}

fn parse_f64(name: &str, value: &str) -> Result<f64, VenueError> {
    let parsed: f64 = value
        .parse()
        .map_err(|error| OkxAdapter::invalid(format!("invalid {name} {value:?}: {error}")))?;
    if !parsed.is_finite() {
        return Err(OkxAdapter::invalid(format!("non-finite {name} {value:?}")));
    }
    Ok(parsed)
}

fn parse_optional_f64(name: &str, value: &str) -> Result<f64, VenueError> {
    if value.is_empty() {
        Ok(0.0)
    } else {
        parse_f64(name, value)
    }
}

fn parse_optional_f64_value(name: &str, value: &str) -> Result<Option<f64>, VenueError> {
    if value.is_empty() {
        Ok(None)
    } else {
        parse_f64(name, value).map(Some)
    }
}

fn parse_fill_fee(
    amount_name: &str,
    amount: &str,
    currency_name: &str,
    currency: &str,
) -> Result<Option<FillFee>, VenueError> {
    let amount = amount.trim();
    let currency = currency.trim();
    if amount.is_empty() && currency.is_empty() {
        return Ok(None);
    }
    if amount.is_empty() || currency.is_empty() {
        return Err(OkxAdapter::invalid(format!(
            "{amount_name} and {currency_name} must either both be present or both be absent"
        )));
    }
    Ok(Some(FillFee {
        amount: parse_f64(amount_name, amount)?,
        currency: currency.to_ascii_uppercase(),
    }))
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
        assert!(matches!(
            events[0].id.key,
            EventKey::BookSequence {
                action: BookAction::Snapshot,
                prev_seq_id: -1,
                seq_id: 10,
                ts_ms: 1000,
                raw_hash: 7,
            }
        ));
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
    fn parses_strategy_pricing_channels() {
        let adapter = OkxAdapter::default();
        let funding = r#"{"arg":{"channel":"funding-rate","instId":"BTC-USDT-SWAP"},"data":[{"instId":"BTC-USDT-SWAP","fundingRate":"0.0001","fundingTime":"2000","prevFundingTime":"1000","settFundingRate":"0.00008","settState":"settled","ts":"1001"}]}"#;
        let index = r#"{"arg":{"channel":"index-tickers","instId":"BTC-USDT"},"data":[{"instId":"BTC-USDT","idxPx":"50000","ts":"1001"}]}"#;
        let limits = r#"{"arg":{"channel":"price-limit","instId":"BTC-USDT-SWAP"},"data":[{"instId":"BTC-USDT-SWAP","buyLmt":"55000","sellLmt":"45000","ts":"1002"}]}"#;
        let mark = r#"{"arg":{"channel":"mark-price","instId":"BTC-USDT-SWAP"},"data":[{"instId":"BTC-USDT-SWAP","markPx":"50010","ts":"1003"}]}"#;

        let funding_events = adapter
            .parse(&envelope(
                Channel::Custom("funding-rate".to_string()),
                funding,
            ))
            .unwrap();
        let index_events = adapter
            .parse(&envelope(
                Channel::Custom("index-tickers".to_string()),
                index,
            ))
            .unwrap();
        let limit_events = adapter
            .parse(&envelope(
                Channel::Custom("price-limit".to_string()),
                limits,
            ))
            .unwrap();
        let mark_events = adapter
            .parse(&envelope(Channel::Custom("mark-price".to_string()), mark))
            .unwrap();

        assert!(matches!(
            funding_events[0].event,
            VenueEvent::Normalized(NormalizedEvent::Market(MarketEvent::FundingRate {
                rate,
                funding_time_ms: 2000,
                settlement: Some(FundingSettlement {
                    funding_time_ms: 1000,
                    rate: 0.00008,
                }),
                ..
            })) if rate == 0.0001
        ));
        assert!(matches!(
            index_events[0].event,
            VenueEvent::Normalized(NormalizedEvent::Market(MarketEvent::IndexPrice {
                price: 50_000.0,
                ..
            }))
        ));
        assert!(matches!(
            index_events[0].id.key,
            EventKey::TimestampHash {
                ts_ms: 1001,
                raw_hash: 7,
            }
        ));
        assert!(matches!(
            limit_events[0].event,
            VenueEvent::Normalized(NormalizedEvent::Market(MarketEvent::PriceLimits {
                limit_down: 45_000.0,
                limit_up: 55_000.0,
                ..
            }))
        ));
        assert!(matches!(
            mark_events[0].event,
            VenueEvent::Normalized(NormalizedEvent::Market(MarketEvent::PriceLimits {
                mark_price: 50_010.0,
                ..
            }))
        ));

        let non_finite = r#"{"arg":{"channel":"funding-rate","instId":"BTC-USDT-SWAP"},"data":[{"instId":"BTC-USDT-SWAP","fundingRate":"NaN","fundingTime":"2000","ts":"1000"}]}"#;
        assert!(
            adapter
                .parse(&envelope(
                    Channel::Custom("funding-rate".to_string()),
                    non_finite,
                ))
                .is_err()
        );

        let invalid_previous_time = r#"{"arg":{"channel":"funding-rate","instId":"BTC-USDT-SWAP"},"data":[{"instId":"BTC-USDT-SWAP","fundingRate":"0.0001","fundingTime":"2000","prevFundingTime":"2000","settFundingRate":"0.00008","settState":"settled","ts":"1001"}]}"#;
        let error = adapter
            .parse(&envelope(
                Channel::Custom("funding-rate".to_string()),
                invalid_previous_time,
            ))
            .unwrap_err();
        assert!(error.to_string().contains("must precede fundingTime"));

        let processing = r#"{"arg":{"channel":"funding-rate","instId":"BTC-USDT-SWAP"},"data":[{"instId":"BTC-USDT-SWAP","fundingRate":"0.0001","fundingTime":"2000","prevFundingTime":"1000","settFundingRate":"0.00008","settState":"processing","ts":"1001"}]}"#;
        let processing_events = adapter
            .parse(&envelope(
                Channel::Custom("funding-rate".to_string()),
                processing,
            ))
            .unwrap();
        assert!(matches!(
            processing_events[0].event,
            VenueEvent::Normalized(NormalizedEvent::Market(MarketEvent::FundingRate {
                settlement: None,
                ..
            }))
        ));
    }

    #[test]
    fn same_timestamp_public_values_have_distinct_dedup_identities() {
        let adapter = OkxAdapter::default();
        let first = r#"{"arg":{"channel":"index-tickers","instId":"USDT-USD"},"data":[{"instId":"USDT-USD","idxPx":"1.0","ts":"1000"}]}"#;
        let second = r#"{"arg":{"channel":"index-tickers","instId":"USDT-USD"},"data":[{"instId":"USDT-USD","idxPx":"0.98","ts":"1000"}]}"#;
        let mut first_envelope = envelope(Channel::Custom("index-tickers".to_string()), first);
        first_envelope.symbol = Some("USDT-USD".to_string());
        let mut second_envelope = envelope(Channel::Custom("index-tickers".to_string()), second);
        second_envelope.symbol = Some("USDT-USD".to_string());
        second_envelope.raw_hash = 8;

        let first_event = adapter.parse(&first_envelope).unwrap().remove(0);
        let second_event = adapter.parse(&second_envelope).unwrap().remove(0);

        assert_ne!(first_event.id.key, second_event.id.key);
        assert!(matches!(
            first_event.id.key,
            EventKey::TimestampHash {
                ts_ms: 1000,
                raw_hash: 7,
            }
        ));
        assert!(matches!(
            second_event.id.key,
            EventKey::TimestampHash {
                ts_ms: 1000,
                raw_hash: 8,
            }
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
    fn distinguishes_channel_data_from_private_control_frames() {
        let adapter = OkxAdapter::default();
        let data = envelope(
            Channel::Positions,
            r#"{"arg":{"channel":"positions","instType":"ANY"},"eventType":"snapshot","data":[]}"#,
        );
        let control = envelope(
            Channel::Positions,
            r#"{"event":"channel-conn-count","channel":"positions","connCount":"1","connId":"test"}"#,
        );

        assert!(adapter.is_data_frame(&data).unwrap());
        assert!(!adapter.is_data_frame(&control).unwrap());
    }

    #[test]
    fn parses_private_order_fill_and_account_updates() {
        let adapter = OkxAdapter::default().with_account_id("main");
        let order = r#"{"arg":{"channel":"orders","instType":"ANY"},"data":[{"ordId":"123","clOrdId":"reap1","instId":"BTC-USDT","side":"buy","state":"partially_filled","px":"100","sz":"1","accFillSz":"0.4","avgPx":"99.5","fillSz":"0.4","fillPx":"99.5","execType":"M","fillFee":"-0.0004","fillFeeCcy":"btc","fee":"-9","feeCcy":"USDT","tradeId":"fill1","uTime":"1000","fillTime":"1000"}]}"#;
        let fills = r#"{"arg":{"channel":"fills","instId":"BTC-USDT"},"data":[{"tradeId":"fill2","ordId":"123","clOrdId":"reap1","instId":"BTC-USDT","side":"buy","fillPx":"99.4","fillSz":"0.1","execType":"T","fee":"0.01","feeCcy":"usdt","ts":"1001"}]}"#;
        let account = r#"{"arg":{"channel":"account"},"data":[{"uTime":"1002","mgnRatio":"10","adjEq":"1000","notionalUsd":"100","details":[{"ccy":"USDT","cashBal":"1000","availBal":"900","twap":"2"}]}]}"#;
        let positions = r#"{"arg":{"channel":"positions","instType":"ANY"},"data":[{"instId":"BTC-USDT-SWAP","pos":"2","posSide":"short","mgnMode":"cross","avgPx":"50000","uTime":"1003"}]}"#;

        let order_events = adapter.parse(&envelope(Channel::Orders, order)).unwrap();
        let fill_events = adapter.parse(&envelope(Channel::Fills, fills)).unwrap();
        let account_events = adapter.parse(&envelope(Channel::Account, account)).unwrap();
        let position_events = adapter
            .parse(&envelope(Channel::Positions, positions))
            .unwrap();

        assert!(matches!(
            order_events[0].event,
            VenueEvent::PrivateOrder(PrivateOrderUpdate {
                state: PrivateOrderState::PartiallyFilled,
                last_fill_qty: 0.4,
                ..
            })
        ));
        let VenueEvent::PrivateOrder(order_update) = &order_events[0].event else {
            panic!("expected private order update");
        };
        assert_eq!(
            order_update.last_fill_fee,
            Some(FillFee {
                amount: -0.0004,
                currency: "BTC".to_string(),
            })
        );
        assert!(matches!(
            fill_events[0].event,
            VenueEvent::PrivateFill(RemoteFill {
                liquidity: FillLiquidity::Taker,
                qty: 0.1,
                ..
            })
        ));
        let VenueEvent::PrivateFill(fill) = &fill_events[0].event else {
            panic!("expected private fill");
        };
        assert_eq!(
            fill.fee,
            Some(FillFee {
                amount: 0.01,
                currency: "USDT".to_string(),
            })
        );
        assert!(matches!(
            account_events[0].event,
            VenueEvent::Account(AccountUpdate { ref balances, .. })
                if balances[0].available == 900.0
                    && balances[0].account_id.as_deref() == Some("main")
                    && balances[0].forced_repayment_indicator == Some(2)
        ));
        assert!(matches!(
            account_events[0].event,
            VenueEvent::Account(AccountUpdate { ref margins, .. })
                if margins[0].exchange_ratio == Some(10.0)
                    && margins[0].adjusted_equity_usd == Some(1_000.0)
                    && margins[0].notional_usd == Some(100.0)
        ));
        assert!(matches!(
            position_events[0].event,
            VenueEvent::Account(AccountUpdate { ref positions, .. })
                if positions[0].symbol == "BTC-USDT-SWAP"
                    && positions[0].qty == -2.0
                    && positions[0].margin_mode == Some(PositionMarginMode::Cross)
        ));
    }

    #[test]
    fn rejects_partial_fill_fee_pairs() {
        assert!(parse_fill_fee("fillFee", "-0.1", "fillFeeCcy", "").is_err());
        assert!(parse_fill_fee("fee", "", "feeCcy", "USDT").is_err());
        assert_eq!(
            parse_fill_fee("fee", "0", "feeCcy", "usd").unwrap(),
            Some(FillFee {
                amount: 0.0,
                currency: "USD".to_string(),
            })
        );
    }

    #[test]
    fn rejects_positions_without_a_supported_margin_mode() {
        let adapter = OkxAdapter::default().with_account_id("main");
        let missing = r#"{"arg":{"channel":"positions","instType":"ANY"},"data":[{"instId":"BTC-USDT-SWAP","pos":"2","posSide":"net","uTime":"1003"}]}"#;
        let invalid = r#"{"arg":{"channel":"positions","instType":"ANY"},"data":[{"instId":"BTC-USDT-SWAP","pos":"2","posSide":"net","mgnMode":"portfolio","uTime":"1003"}]}"#;

        assert!(
            adapter
                .parse(&envelope(Channel::Positions, missing))
                .is_err()
        );
        assert!(
            adapter
                .parse(&envelope(Channel::Positions, invalid))
                .is_err()
        );
    }

    #[test]
    fn rejects_invalid_forced_repayment_indicator() {
        let adapter = OkxAdapter::default().with_account_id("main");
        let account = r#"{"arg":{"channel":"account"},"data":[{"uTime":"1002","details":[{"ccy":"USDT","twap":"6"}]}]}"#;

        assert!(adapter.parse(&envelope(Channel::Account, account)).is_err());
    }

    #[test]
    fn private_deduplication_identity_is_scoped_by_account() {
        let order = r#"{"arg":{"channel":"orders","instType":"ANY"},"data":[{"ordId":"123","clOrdId":"reap1","instId":"BTC-USDT","side":"buy","state":"live","px":"100","sz":"1","accFillSz":"0","avgPx":"","fillSz":"0","fillPx":"","execType":"","tradeId":"","uTime":"1000","fillTime":""}]}"#;
        let maker = OkxAdapter::default()
            .with_account_id("maker")
            .parse(&envelope(Channel::Orders, order))
            .unwrap();
        let hedge = OkxAdapter::default()
            .with_account_id("hedge")
            .parse(&envelope(Channel::Orders, order))
            .unwrap();

        assert_eq!(maker[0].id.symbol.as_deref(), Some("maker:BTC-USDT"));
        assert_eq!(hedge[0].id.symbol.as_deref(), Some("hedge:BTC-USDT"));
        assert_ne!(maker[0].id, hedge[0].id);
    }
}
