mod support;

use reap_pm_core::{
    ConnectionEpoch, IngressSequence, PmAllowanceValue, PmMarketLifecycle, PmPositionAvailability,
    U256,
};
use reap_polymarket_adapter::{
    PmAccountPositionRoleError, PmAccountPositionSnapshot, PmInstrumentScope,
    PmLiveNormalizationError, PmReadOwnerGrant,
};
use reap_polymarket_wire::{PmLiveBalanceAllowance, parse_live_balance_allowance};
use serde_json::json;

use support::{
    STANDARD_EXCHANGE, account_scope, account_source, completion, connection, instrument,
    market_metadata_with_lifecycle, snapshot, trading_domain,
};

const EXTRA_SPENDER: &str = "0xcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcd";

fn balance(
    value: u64,
    exact_allowance: Option<u64>,
    extra_allowance: Option<u64>,
    legacy_scalar: bool,
) -> PmLiveBalanceAllowance {
    let mut allowances = serde_json::Map::new();
    if let Some(value) = exact_allowance {
        allowances.insert(STANDARD_EXCHANGE.to_owned(), json!(value.to_string()));
    }
    if let Some(value) = extra_allowance {
        allowances.insert(EXTRA_SPENDER.to_owned(), json!(value.to_string()));
    }
    let mut body = serde_json::Map::new();
    body.insert("balance".to_owned(), json!(value.to_string()));
    body.insert("allowances".to_owned(), allowances.into());
    if legacy_scalar {
        body.insert("allowance".to_owned(), json!("999999"));
    }
    parse_live_balance_allowance(&serde_json::to_vec(&body).unwrap()).unwrap()
}

#[test]
fn exact_balances_allowances_and_position_are_atomic_and_extras_are_diagnostic() {
    let (_, _, account) = PmReadOwnerGrant::allocate().split();
    let mut role = support::account_with(account);
    let collateral = balance(1_000, Some(100), Some(777), true);
    let conditional = balance(25, Some(1), None, false);
    let normalized = role
        .request_snapshot(ConnectionEpoch::new(1), IngressSequence::new(10))
        .unwrap()
        .complete_live(
            completion(1, 11, Some(1)),
            snapshot(1),
            &collateral,
            &conditional,
        )
        .unwrap();
    assert_eq!(normalized.foreign_diagnostics().count(), 2);
    let serviced = normalized.into_delivery().service_at(30_000).unwrap();
    role.reduce_snapshot_delivery(serviced, |_, envelope| {
        let payload = envelope.payload();
        let domain = trading_domain();
        assert_eq!(
            payload
                .expected_balance(domain.collateral())
                .unwrap()
                .unwrap()
                .balance(),
            U256::from_u64(1_000)
        );
        assert_eq!(
            payload
                .expected_balance(domain.outcome())
                .unwrap()
                .unwrap()
                .balance(),
            U256::from_u64(25)
        );
        let spenders = role.required_spenders();
        assert_eq!(
            payload
                .expected_allowance(spenders[0])
                .unwrap()
                .unwrap()
                .value(),
            PmAllowanceValue::Erc20(U256::from_u64(100))
        );
        let outcome = payload
            .expected_allowance(spenders[1])
            .unwrap()
            .unwrap()
            .value();
        let PmAllowanceValue::Erc1155Operator(approval) = outcome else {
            panic!("conditional operator approval")
        };
        assert!(approval.is_approved());
        let position = payload.expected_position(instrument()).unwrap().unwrap();
        assert_eq!(position.quantity(), U256::from_u64(25));
        assert_eq!(position.availability(), PmPositionAvailability::Tradable);
    })
    .unwrap();
}

#[test]
fn proxy_account_balance_and_position_normalization_accepts_split_identity() {
    let (_, _, account) = PmReadOwnerGrant::allocate().split();
    let mut role = support::account_with_account(account, support::proxy_account_scope());
    let collateral = balance(1_000, Some(100), None, false);
    let conditional = balance(25, Some(1), None, false);
    let serviced = role
        .request_snapshot(ConnectionEpoch::new(1), IngressSequence::new(10))
        .unwrap()
        .complete_live(
            completion(1, 11, Some(1)),
            snapshot(1),
            &collateral,
            &conditional,
        )
        .unwrap()
        .into_delivery()
        .service_at(30_000)
        .unwrap();
    role.reduce_snapshot_delivery(serviced, |_, envelope| {
        let position = envelope
            .payload()
            .expected_position(instrument())
            .unwrap()
            .unwrap();
        assert_eq!(position.quantity(), U256::from_u64(25));
    })
    .unwrap();
}

#[test]
fn missing_exact_spender_never_falls_back_to_the_first_allowance() {
    let (_, _, account) = PmReadOwnerGrant::allocate().split();
    let mut role = support::account_with(account);
    let collateral = balance(1_000, None, Some(777), true);
    let conditional = balance(25, Some(1), None, false);
    assert!(matches!(
        role.request_snapshot(ConnectionEpoch::new(1), IngressSequence::new(10))
            .unwrap()
            .complete_live(
                completion(1, 11, Some(1)),
                snapshot(1),
                &collateral,
                &conditional,
            ),
        Err(PmAccountPositionRoleError::Live(
            PmLiveNormalizationError::MissingRequiredAllowance
        ))
    ));
}

#[test]
fn nonready_lifecycle_is_unavailable_and_zero_conditional_allowance_is_not_approved() {
    let lifecycle = PmMarketLifecycle::new(true, true, false, false, true);
    let instrument_scope =
        PmInstrumentScope::from_metadata(instrument(), market_metadata_with_lifecycle(lifecycle))
            .unwrap();
    let (_, _, account) = PmReadOwnerGrant::allocate().split();
    let mut role = PmAccountPositionSnapshot::new(
        account,
        account_scope(),
        instrument_scope,
        account_source(),
        connection(),
    )
    .unwrap();
    let collateral = balance(1_000, Some(100), None, false);
    let conditional = balance(25, Some(0), None, false);
    let serviced = role
        .request_snapshot(ConnectionEpoch::new(1), IngressSequence::new(10))
        .unwrap()
        .complete_live(
            completion(1, 11, Some(1)),
            snapshot(1),
            &collateral,
            &conditional,
        )
        .unwrap()
        .into_delivery()
        .service_at(30_000)
        .unwrap();
    role.reduce_snapshot_delivery(serviced, |_, envelope| {
        let payload = envelope.payload();
        let outcome = payload
            .expected_allowance(role.required_spenders()[1])
            .unwrap()
            .unwrap()
            .value();
        let PmAllowanceValue::Erc1155Operator(approval) = outcome else {
            panic!("operator approval")
        };
        assert!(!approval.is_approved());
        assert_eq!(
            payload
                .expected_position(instrument())
                .unwrap()
                .unwrap()
                .availability(),
            PmPositionAvailability::Unavailable
        );
    })
    .unwrap();
}
