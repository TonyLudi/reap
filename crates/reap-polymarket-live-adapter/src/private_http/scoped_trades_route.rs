//! Fixed exact-market authenticated trade-query route.
//!
//! The canonical account cut remains unfiltered. This helper is reachable only
//! through the explicitly named exact-scope reconciliation capability.

use reap_polymarket_wire::PmWireScope;
use reqwest::Url;

pub(super) fn url(origin: &Url, cursor: &str, scope: PmWireScope) -> Url {
    let mut url = origin.clone();
    url.set_path("/data/trades");
    url.query_pairs_mut()
        .append_pair("market", &scope.condition().to_string())
        .append_pair("asset_id", &scope.token().units().to_string())
        .append_pair("next_cursor", cursor);
    url
}
