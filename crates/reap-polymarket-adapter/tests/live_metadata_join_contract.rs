mod support;

use reap_pm_core::{PmProductSource, PmSourceHandle, SnapshotRevision};
use reap_polymarket_adapter::{
    PmAuthoritativeMetadata, PmMetadataJoinError, PmMetadataRevisionInput,
};
use reap_polymarket_wire::{PmBookMarketBinding, PmWireError};

const OTHER_CONDITION: &str = "0xcccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";
const OTHER_MARKET: &str = "0xdddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd";

fn source() -> PmProductSource {
    PmProductSource::polymarket_market(
        PmSourceHandle::from_ordinal(11),
        support::instrument().token(),
    )
}

fn revision() -> PmMetadataRevisionInput {
    PmMetadataRevisionInput::new(SnapshotRevision::new(9), 8_000).unwrap()
}

fn long_market() -> String {
    format!(
        r#"{{
          "condition_id":"{}",
          "question_id":"{}",
          "active":true,
          "closed":false,
          "archived":false,
          "accepting_orders":true,
          "enable_order_book":true,
          "accepting_order_timestamp":"2026-08-08T00:00:00Z",
          "end_date_iso":"2027-01-01T00:00:00Z",
          "game_start_time":null,
          "seconds_delay":0,
          "minimum_order_size":5,
          "minimum_tick_size":0.01,
          "tokens":[]
        }}"#,
        support::CONDITION,
        support::MARKET,
    )
}

fn short_market(
    condition: Option<&str>,
    tick: &str,
    minimum: &str,
    negative_risk: bool,
    label: &str,
) -> String {
    let condition = condition
        .map(|value| format!(r#", "c":"{value}""#))
        .unwrap_or_default();
    format!(
        r#"{{
          "t":[{{"t":"123","o":"{label}"}},{{"t":"456","o":"No"}}],
          "mts":{tick},
          "mos":{minimum},
          "nr":{negative_risk},
          "fd":{{"r":0.02,"e":2,"to":true}},
          "mbf":0,
          "tbf":0,
          "ao":true,
          "sd":0,
          "gst":null,
          "cbos":true,
          "aot":"2026-08-08T00:00:00Z",
          "rfqe":false,
          "itode":false,
          "ibce":true,
          "oas":0
          {condition}
        }}"#
    )
}

fn join(long: &str, short: &str) -> Result<PmAuthoritativeMetadata, PmMetadataJoinError> {
    PmAuthoritativeMetadata::join_live_clob_v2_raw(
        support::instrument(),
        source(),
        support::market_metadata(),
        long.as_bytes(),
        short.as_bytes(),
        revision(),
    )
}

#[test]
fn live_two_source_join_uses_long_question_identity_and_condition_bound_books() {
    let authority = join(
        &long_market(),
        &short_market(None, "0.01", "5", false, "Yes"),
    )
    .expect("current two-source metadata is coherent");

    assert_eq!(authority.event().metadata(), support::market_metadata());
    assert_eq!(
        authority.parser_config().scope().market(),
        support::market_metadata().market()
    );
    assert_eq!(
        authority.parser_config().scope().condition(),
        support::market_metadata().condition()
    );
    assert_eq!(
        authority.parser_config().market_binding(),
        PmBookMarketBinding::ConditionId
    );
}

#[test]
fn either_source_identity_contradiction_fails_before_authority_is_joined() {
    let wrong_question = long_market().replace(support::MARKET, OTHER_MARKET);
    assert!(matches!(
        join(
            &wrong_question,
            &short_market(None, "0.01", "5", false, "Yes")
        ),
        Err(PmMetadataJoinError::Wire(PmWireError::MarketMismatch))
    ));

    assert!(matches!(
        join(
            &long_market(),
            &short_market(Some(OTHER_CONDITION), "0.01", "5", false, "Yes")
        ),
        Err(PmMetadataJoinError::Wire(PmWireError::ConditionMismatch))
    ));

    let wrong_long_condition = long_market().replace(support::CONDITION, OTHER_CONDITION);
    assert!(matches!(
        join(
            &wrong_long_condition,
            &short_market(None, "0.01", "5", false, "Yes")
        ),
        Err(PmMetadataJoinError::Wire(PmWireError::ConditionMismatch))
    ));
}

#[test]
fn lifecycle_and_trading_contract_contradictions_remain_typed() {
    let closed = long_market().replace(r#""closed":false"#, r#""closed":true"#);
    assert!(matches!(
        join(&closed, &short_market(None, "0.01", "5", false, "Yes")),
        Err(PmMetadataJoinError::Closed)
    ));

    for (short, expected) in [
        (
            short_market(None, "0.001", "5", false, "Yes"),
            PmMetadataJoinError::TickDrift,
        ),
        (
            short_market(None, "0.01", "6", false, "Yes"),
            PmMetadataJoinError::MinimumDrift,
        ),
        (
            short_market(None, "0.01", "5", true, "Yes"),
            PmMetadataJoinError::NegativeRiskDrift,
        ),
        (
            short_market(None, "0.01", "5", false, "UP"),
            PmMetadataJoinError::OutcomeMismatch,
        ),
    ] {
        assert_eq!(
            join(&long_market(), &short).unwrap_err().to_string(),
            expected.to_string()
        );
    }
}

#[test]
fn long_and_short_lifecycle_contradictions_fail_before_authority() {
    for (needle, replacement, field) in [
        (r#""ao":true"#, r#""ao":false"#, "accepting_orders"),
        (r#""sd":0"#, r#""sd":1"#, "seconds_delay"),
        (
            r#""aot":"2026-08-08T00:00:00Z""#,
            r#""aot":"2026-08-08T00:00:01Z""#,
            "accepting_order_timestamp",
        ),
        (
            r#""gst":null"#,
            r#""gst":"2026-12-31T00:00:00Z""#,
            "game_start_time",
        ),
    ] {
        let short = short_market(None, "0.01", "5", false, "Yes").replace(needle, replacement);
        assert!(matches!(
            join(&long_market(), &short),
            Err(PmMetadataJoinError::Wire(PmWireError::InvalidIdentity(
                observed
            ))) if observed == field
        ));
    }
}

#[test]
fn long_lifecycle_details_are_required_and_strict_before_join() {
    let missing_end =
        long_market().replace("          \"end_date_iso\":\"2027-01-01T00:00:00Z\",\n", "");
    assert!(matches!(
        join(&missing_end, &short_market(None, "0.01", "5", false, "Yes")),
        Err(PmMetadataJoinError::Wire(PmWireError::MissingField(
            "end_date_iso"
        )))
    ));

    let wrong_delay = long_market().replace(r#""seconds_delay":0"#, r#""seconds_delay":"0""#);
    assert!(matches!(
        join(&wrong_delay, &short_market(None, "0.01", "5", false, "Yes")),
        Err(PmMetadataJoinError::Wire(PmWireError::MalformedJson))
    ));
}
