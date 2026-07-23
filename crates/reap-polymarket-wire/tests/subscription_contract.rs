mod support;

use reap_polymarket_wire::PmMarketSubscription;

#[test]
fn subscription_is_exact_and_scoped_to_one_configured_token() {
    let subscription = PmMarketSubscription::new(support::scope().token());

    assert_eq!(
        subscription.to_json().unwrap(),
        include_str!("../fixtures/market_subscription.json").trim()
    );
    assert_eq!(subscription.token(), support::scope().token());
}
