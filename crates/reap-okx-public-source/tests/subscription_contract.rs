use reap_core::{Channel, FeedPriority, Venue};
use reap_okx_public_source::{OkxIndexTickerSubscription, OkxIndexTickerSubscriptionError};

#[test]
fn one_configured_index_ticker_has_exact_subscription_bytes() {
    let subscription = OkxIndexTickerSubscription::new("BTC-USDT").unwrap();

    assert_eq!(
        subscription.wire_bytes(),
        br#"{"op":"subscribe","args":[{"channel":"index-tickers","instId":"BTC-USDT"}]}"#
    );
    assert_eq!(subscription.instrument(), "BTC-USDT");

    let core = subscription.as_core_subscription();
    assert_eq!(core.venue, Venue::Okx);
    assert_eq!(core.channel, Channel::Custom("index-tickers".to_string()));
    assert_eq!(core.symbol.as_deref(), Some("BTC-USDT"));
    assert_eq!(core.priority, FeedPriority::Critical);
    assert_eq!(core.connections, 1);
}

#[test]
fn the_fixed_subscription_cannot_be_retargeted_to_an_arbitrary_scope() {
    for invalid in [
        "",
        "BTCUSDT",
        "-BTC",
        "BTC-",
        "BTC--USDT",
        "btc-usdt",
        "BTC USDT",
        "BTC/USDT",
    ] {
        assert!(
            OkxIndexTickerSubscription::new(invalid).is_err(),
            "{invalid:?}"
        );
    }

    let oversized = format!("{}-USD", "A".repeat(64));
    assert!(matches!(
        OkxIndexTickerSubscription::new(&oversized),
        Err(OkxIndexTickerSubscriptionError::InstrumentTooLong)
    ));
}
