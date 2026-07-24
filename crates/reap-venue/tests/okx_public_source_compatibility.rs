use reap_core::{Channel, ConnId, EventKey, MarketEvent, NormalizedEvent, OkxVenue, RawEnvelope};
use reap_venue::{VenueAdapter, VenueEvent, okx::OkxAdapter};

#[test]
fn legacy_index_ticker_value_identity_and_serialized_bytes_stay_frozen() {
    let payload = r#"{"arg":{"channel":"index-tickers","instId":"BTC-USDT"},"data":[{"instId":"BTC-USDT","idxPx":"00050000.125000","ts":"1001"}]}"#;
    let envelope = RawEnvelope {
        venue: OkxVenue,
        conn_id: ConnId::new("legacy-okx"),
        channel: Channel::Custom("index-tickers".to_string()),
        symbol: Some("BTC-USDT".to_string()),
        recv_ts_ns: 1,
        raw_hash: 7,
        payload: payload.to_string(),
    };

    let event = OkxAdapter::default()
        .parse(&envelope)
        .unwrap()
        .into_iter()
        .next()
        .unwrap();
    assert!(matches!(
        event.id.key,
        EventKey::TimestampHash {
            ts_ms: 1001,
            raw_hash: 7
        }
    ));
    assert!(matches!(
        event.event,
        VenueEvent::Normalized(NormalizedEvent::Market(MarketEvent::IndexPrice {
            ts_ms: 1001,
            ref symbol,
            price: 50_000.125,
        })) if symbol == "BTC-USDT"
    ));

    assert_eq!(
        serde_json::to_string(&event).unwrap(),
        r#"{"id":{"venue":"okx","channel":{"custom":"index-tickers"},"symbol":"BTC-USDT","key":{"timestamp_hash":{"ts_ms":1001,"raw_hash":7}}},"account_id":null,"event":{"normalized":{"Market":{"IndexPrice":{"ts_ms":1001,"symbol":"BTC-USDT","price":50000.125}}}}}"#
    );
}
