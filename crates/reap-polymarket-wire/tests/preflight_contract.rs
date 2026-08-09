use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use reap_polymarket_wire::{
    MAX_PM_CLOSED_ONLY_BODY_BYTES, MAX_PM_GEOBLOCK_BODY_BYTES, PmWireError, parse_pm_closed_only,
    parse_pm_geoblock,
};

#[test]
fn geoblock_requires_the_exact_four_key_shape_and_canonical_scope() {
    let ipv4 =
        parse_pm_geoblock(br#"{"blocked":false,"ip":"203.0.113.42","country":"US","region":"NY"}"#)
            .unwrap();
    assert!(!ipv4.blocked());
    assert_eq!(ipv4.ip(), IpAddr::V4(Ipv4Addr::new(203, 0, 113, 42)));
    assert_eq!(ipv4.country(), "US");
    assert_eq!(ipv4.region(), "NY");

    let ipv6 =
        parse_pm_geoblock(br#"{"blocked":true,"ip":"2001:db8::1","country":"CA","region":"BC-1"}"#)
            .unwrap();
    assert!(ipv6.blocked());
    assert_eq!(
        ipv6.ip(),
        IpAddr::V6("2001:db8::1".parse::<Ipv6Addr>().unwrap())
    );

    for (raw, expected) in [
        (
            br#"{"ip":"203.0.113.42","country":"US","region":"NY"}"#.as_slice(),
            PmWireError::MissingField("blocked"),
        ),
        (
            br#"{"blocked":false,"country":"US","region":"NY"}"#.as_slice(),
            PmWireError::MissingField("ip"),
        ),
        (
            br#"{"blocked":false,"ip":"203.0.113.42","region":"NY"}"#.as_slice(),
            PmWireError::MissingField("country"),
        ),
        (
            br#"{"blocked":false,"ip":"203.0.113.42","country":"US"}"#.as_slice(),
            PmWireError::MissingField("region"),
        ),
    ] {
        assert_eq!(parse_pm_geoblock(raw), Err(expected));
    }
}

#[test]
fn geoblock_rejects_duplicates_extensions_and_noncanonical_fields() {
    for raw in [
        br#"{"blocked":false,"blocked":true,"ip":"203.0.113.42","country":"US","region":"NY"}"#
            .as_slice(),
        br#"{"blocked":false,"ip":"203.0.113.42","country":"US","region":"NY","extra":0}"#
            .as_slice(),
    ] {
        assert_eq!(parse_pm_geoblock(raw), Err(PmWireError::MalformedJson));
    }
    for (field, raw) in [
        (
            "ip",
            br#"{"blocked":false,"ip":"2001:0db8::1","country":"US","region":"NY"}"#.as_slice(),
        ),
        (
            "country",
            br#"{"blocked":false,"ip":"203.0.113.42","country":"us","region":"NY"}"#.as_slice(),
        ),
        (
            "region",
            br#"{"blocked":false,"ip":"203.0.113.42","country":"US","region":"New York"}"#
                .as_slice(),
        ),
    ] {
        assert_eq!(
            parse_pm_geoblock(raw),
            Err(PmWireError::InvalidIdentity(field))
        );
    }
    assert_eq!(
        parse_pm_geoblock(&[b' '; MAX_PM_GEOBLOCK_BODY_BYTES + 1]),
        Err(PmWireError::RestBodyTooLarge)
    );
}

#[test]
fn closed_only_is_one_exact_bounded_boolean() {
    assert!(
        !parse_pm_closed_only(br#"{"closed_only":false}"#)
            .unwrap()
            .closed_only()
    );
    assert!(
        parse_pm_closed_only(br#"{"closed_only":true}"#)
            .unwrap()
            .closed_only()
    );
    assert_eq!(
        parse_pm_closed_only(b"{}"),
        Err(PmWireError::MissingField("closed_only"))
    );
    for raw in [
        br#"{"closed_only":"false"}"#.as_slice(),
        br#"{"closed_only":false,"closed_only":true}"#.as_slice(),
        br#"{"closed_only":false,"extra":0}"#.as_slice(),
    ] {
        assert_eq!(parse_pm_closed_only(raw), Err(PmWireError::MalformedJson));
    }
    assert_eq!(
        parse_pm_closed_only(&[b' '; MAX_PM_CLOSED_ONLY_BODY_BYTES + 1]),
        Err(PmWireError::RestBodyTooLarge)
    );
}
