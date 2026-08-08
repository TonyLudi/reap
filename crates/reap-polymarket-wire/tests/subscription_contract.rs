mod support;

use reap_polymarket_wire::PmMarketSubscription;

#[test]
fn subscription_is_exact_and_scoped_to_one_configured_token() {
    let subscription = PmMarketSubscription::new(support::scope().token());
    let serialized = subscription.to_json().unwrap();

    assert_eq!(
        serialized,
        include_str!("../fixtures/market_subscription.json").trim()
    );
    assert!(!serialized.contains("initial_dump"));
    assert!(!serialized.contains("operation"));
    assert_eq!(subscription.token(), support::scope().token());
}
