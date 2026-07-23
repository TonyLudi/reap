mod support;

use reap_pm_core::{
    ConnectionEpoch, EvmAddress, IngressSequence, MAX_PM_ACCOUNT_DIAGNOSTIC_EXTRA_ROWS,
    PmAggregateError, PmAllowanceValue, PmAssetId, PmErc1155OperatorApproval, PmFunderId,
    PmPositionAvailability, PmSpenderDomain, PmSpenderId, PmSpenderRequirement, U256,
};
use reap_polymarket_adapter::{
    MAX_PM_FIXTURE_QUERY_PAGES, PmAccountPositionRoleError, PmFixtureAllowanceRow,
    PmFixtureBalanceRow, PmFixtureDeliveryError, PmFixturePositionRow,
};
use reap_polymarket_wire::parse_legacy_balance_allowance_fixture;

use support::{
    account_role, account_scope, address, completion, instrument_scope, snapshot, trading_domain,
};

fn required_rows(
    role: &reap_polymarket_adapter::PmFixtureAccountPositionSnapshot,
) -> (PmFixtureAllowanceRow, PmFixtureAllowanceRow) {
    let spenders = role.required_spenders();
    (
        PmFixtureAllowanceRow::new(
            spenders[0],
            match spenders[0].requirement().asset() {
                PmAssetId::Collateral { .. } => PmAllowanceValue::Erc20(U256::from_u64(100)),
                PmAssetId::Outcome { .. } => {
                    PmAllowanceValue::Erc1155Operator(PmErc1155OperatorApproval::from_bool(true))
                }
            },
        ),
        PmFixtureAllowanceRow::new(
            spenders[1],
            match spenders[1].requirement().asset() {
                PmAssetId::Collateral { .. } => PmAllowanceValue::Erc20(U256::from_u64(100)),
                PmAssetId::Outcome { .. } => {
                    PmAllowanceValue::Erc1155Operator(PmErc1155OperatorApproval::from_bool(true))
                }
            },
        ),
    )
}

fn diagnostic_spender() -> PmSpenderId {
    PmSpenderId::new(
        account_scope().handle(),
        PmSpenderRequirement::new(
            account_scope().chain(),
            address("0xcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcd"),
            PmSpenderDomain::Standard,
            PmAssetId::collateral(address("0xdededededededededededededededededededede")),
        ),
    )
}

fn wrong_observed_scope() -> reap_pm_core::PmAccountScope {
    let scope = account_scope();
    reap_pm_core::PmAccountScope::new(
        scope.environment(),
        scope.chain(),
        scope.signer(),
        PmFunderId::new(EvmAddress::from_bytes([9; 20]).unwrap()),
        scope.handle(),
    )
}

fn diagnostic_asset(seed: u8) -> PmAssetId {
    PmAssetId::collateral(EvmAddress::from_bytes([seed; 20]).unwrap())
}

fn page_cursor(index: usize) -> [u8; 32] {
    let mut cursor = [0_u8; 32];
    cursor[..8].copy_from_slice(&(index as u64).to_le_bytes());
    cursor
}

#[test]
fn complete_account_snapshot_is_atomic_and_unknown_extras_are_non_authoritative() {
    let mut role = account_role();
    let domain = trading_domain();
    let collateral = PmFixtureBalanceRow::new(domain.collateral(), U256::from_u64(1_000));
    let position = PmFixturePositionRow::new(
        instrument_scope(),
        U256::from_u64(25),
        PmPositionAvailability::Tradable,
    );
    let (first_allowance, second_allowance) = required_rows(&role);
    let extra_spender = diagnostic_spender();
    let extra_allowance =
        PmFixtureAllowanceRow::new(extra_spender, PmAllowanceValue::Erc20(U256::from_u64(9)));

    let mut assembly = role
        .request_snapshot(ConnectionEpoch::new(1), IngressSequence::new(10))
        .unwrap()
        .begin(snapshot(1));
    assembly
        .push_page(
            account_scope(),
            None,
            Some([1; 32]),
            &[collateral],
            &[first_allowance],
            &[],
        )
        .unwrap();
    assembly
        .push_page(
            account_scope(),
            Some([1; 32]),
            None,
            &[],
            &[second_allowance, extra_allowance],
            &[position],
        )
        .unwrap();
    let delivery = assembly
        .finish(completion(1, 11, Some(1)))
        .unwrap()
        .service_at(30_000)
        .unwrap();

    role.reduce_snapshot_delivery(delivery, |scope, envelope| {
        assert_eq!(scope.account_scope(), account_scope());
        let aggregate = envelope.payload();
        assert!(
            aggregate
                .expected_balance(domain.collateral())
                .unwrap()
                .is_some()
        );
        assert_eq!(aggregate.expected_balance(domain.outcome()), Some(None));
        assert_eq!(aggregate.expected_allowance(extra_spender), None);
        assert!(
            aggregate
                .allowances()
                .iter()
                .any(|row| row.spender() == extra_spender)
        );
        assert!(
            aggregate
                .expected_position(instrument_scope().handle())
                .unwrap()
                .is_some()
        );
        assert_eq!(
            aggregate.boundary().completion_sequence(),
            envelope.ordering().local_ingress_sequence()
        );
    })
    .unwrap();
}

#[test]
fn account_page_chain_requires_missing_to_terminal_unbroken_sequence() {
    let mut role = account_role();
    let missing = role
        .request_snapshot(ConnectionEpoch::new(1), IngressSequence::new(10))
        .unwrap()
        .begin(snapshot(1));
    assert!(matches!(
        missing.finish(completion(1, 11, Some(1))),
        Err(PmAccountPositionRoleError::MissingPage)
    ));

    let mut nonterminal = role
        .request_snapshot(ConnectionEpoch::new(1), IngressSequence::new(20))
        .unwrap()
        .begin(snapshot(2));
    nonterminal
        .push_page(account_scope(), None, Some([1; 32]), &[], &[], &[])
        .unwrap();
    assert!(matches!(
        nonterminal.finish(completion(1, 21, Some(2))),
        Err(PmAccountPositionRoleError::MissingTerminalPage)
    ));

    let mut broken = role
        .request_snapshot(ConnectionEpoch::new(1), IngressSequence::new(30))
        .unwrap()
        .begin(snapshot(3));
    broken
        .push_page(account_scope(), None, Some([1; 32]), &[], &[], &[])
        .unwrap();
    assert_eq!(
        broken
            .push_page(account_scope(), Some([2; 32]), None, &[], &[], &[],)
            .unwrap_err(),
        PmAccountPositionRoleError::BrokenCursorChain
    );

    let mut terminal = role
        .request_snapshot(ConnectionEpoch::new(1), IngressSequence::new(40))
        .unwrap()
        .begin(snapshot(4));
    terminal
        .push_page(account_scope(), None, None, &[], &[], &[])
        .unwrap();
    assert_eq!(
        terminal
            .push_page(account_scope(), None, None, &[], &[], &[])
            .unwrap_err(),
        PmAccountPositionRoleError::PageAfterTerminal
    );
}

#[test]
fn full_account_scope_spender_chain_instrument_and_scalar_are_strict() {
    let role = account_role();
    let scope = account_scope();
    let same_handle_wrong_funder = reap_pm_core::PmAccountScope::new(
        scope.environment(),
        scope.chain(),
        scope.signer(),
        PmFunderId::new(EvmAddress::from_bytes([9; 20]).unwrap()),
        scope.handle(),
    );
    assert_eq!(
        role.normalize_balance(
            same_handle_wrong_funder,
            trading_domain().collateral(),
            U256::from_u64(1),
            snapshot(1),
        )
        .unwrap_err(),
        PmAccountPositionRoleError::AccountScopeMismatch
    );

    let wrong_chain = PmSpenderId::new(
        scope.handle(),
        PmSpenderRequirement::new(
            reap_pm_core::PmChainId::new(1).unwrap(),
            address("0xcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcd"),
            PmSpenderDomain::Standard,
            trading_domain().collateral(),
        ),
    );
    assert_eq!(
        role.normalize_allowance(
            scope,
            wrong_chain,
            PmAllowanceValue::Erc20(U256::from_u64(1)),
            snapshot(1),
        )
        .unwrap_err(),
        PmAccountPositionRoleError::SpenderChainMismatch
    );

    let scalar =
        parse_legacy_balance_allowance_fixture(br#"{"balance":"1","allowance":"2"}"#).unwrap();
    assert_eq!(
        role.reject_legacy_scalar(&scalar).unwrap_err(),
        PmAccountPositionRoleError::UnscopedLegacyAllowance
    );
}

#[test]
fn duplicate_rows_and_completion_scope_mismatch_never_emit() {
    let mut role = account_role();
    let row = PmFixtureBalanceRow::new(trading_domain().collateral(), U256::from_u64(1));
    let duplicate = role
        .request_snapshot(ConnectionEpoch::new(1), IngressSequence::new(10))
        .unwrap()
        .complete(
            completion(1, 11, Some(1)),
            snapshot(1),
            account_scope(),
            &[row, row],
            &[],
            &[],
        );
    assert!(matches!(
        duplicate,
        Err(PmAccountPositionRoleError::Aggregate(
            PmAggregateError::DuplicateBalanceAsset
        ))
    ));

    let wrong_epoch = role
        .request_snapshot(ConnectionEpoch::new(1), IngressSequence::new(20))
        .unwrap()
        .complete(
            completion(2, 21, Some(2)),
            snapshot(2),
            account_scope(),
            &[],
            &[],
            &[],
        );
    assert!(matches!(
        wrong_epoch,
        Err(PmAccountPositionRoleError::Delivery(
            PmFixtureDeliveryError::CompletionEpochMismatch
        ))
    ));

    role.request_snapshot(ConnectionEpoch::new(2), IngressSequence::new(1))
        .expect("later epoch resets request sequence");
    assert!(
        role.request_snapshot(ConnectionEpoch::new(1), IngressSequence::new(21))
            .is_err()
    );
}

#[test]
fn account_row_caps_are_preflighted_before_normalization_and_failed_pages_are_retryable() {
    let mut role = account_role();

    let balance = PmFixtureBalanceRow::new(trading_domain().collateral(), U256::from_u64(1));
    let mut balances = role
        .request_snapshot(ConnectionEpoch::new(1), IngressSequence::new(10))
        .unwrap()
        .begin(snapshot(1));
    balances
        .push_page(
            account_scope(),
            None,
            Some(page_cursor(1)),
            &[balance],
            &[],
            &[],
        )
        .unwrap();
    let cap_plus_one_for_remaining = vec![balance; 2 + MAX_PM_ACCOUNT_DIAGNOSTIC_EXTRA_ROWS];
    assert!(matches!(
        balances.push_page(
            wrong_observed_scope(),
            Some(page_cursor(1)),
            None,
            &cap_plus_one_for_remaining,
            &[],
            &[],
        ),
        Err(PmAccountPositionRoleError::Aggregate(
            PmAggregateError::TooManyBalanceRows
        ))
    ));
    balances
        .push_page(account_scope(), Some(page_cursor(1)), None, &[], &[], &[])
        .unwrap();
    balances
        .finish(completion(1, 11, Some(1)))
        .expect("failed balance preflight must leave the assembly retryable");

    let (allowance, _) = required_rows(&role);
    let mut allowances = role
        .request_snapshot(ConnectionEpoch::new(1), IngressSequence::new(20))
        .unwrap()
        .begin(snapshot(2));
    let allowance_cap_plus_one = vec![allowance; 2 + MAX_PM_ACCOUNT_DIAGNOSTIC_EXTRA_ROWS + 1];
    assert!(matches!(
        allowances.push_page(
            wrong_observed_scope(),
            None,
            None,
            &[],
            &allowance_cap_plus_one,
            &[],
        ),
        Err(PmAccountPositionRoleError::Aggregate(
            PmAggregateError::TooManyAllowanceRows
        ))
    ));
    allowances
        .push_page(account_scope(), None, None, &[], &[], &[])
        .unwrap();
    allowances
        .finish(completion(1, 21, Some(2)))
        .expect("failed allowance preflight must leave the assembly retryable");

    let position = PmFixturePositionRow::new(
        instrument_scope(),
        U256::from_u64(1),
        PmPositionAvailability::Tradable,
    );
    let mut positions = role
        .request_snapshot(ConnectionEpoch::new(1), IngressSequence::new(30))
        .unwrap()
        .begin(snapshot(3));
    let position_cap_plus_one = vec![position; 1 + MAX_PM_ACCOUNT_DIAGNOSTIC_EXTRA_ROWS + 1];
    assert!(matches!(
        positions.push_page(
            wrong_observed_scope(),
            None,
            None,
            &[],
            &[],
            &position_cap_plus_one,
        ),
        Err(PmAccountPositionRoleError::Aggregate(
            PmAggregateError::TooManyPositionRows
        ))
    ));
    positions
        .push_page(account_scope(), None, None, &[], &[], &[])
        .unwrap();
    positions
        .finish(completion(1, 31, Some(3)))
        .expect("failed position preflight must leave the assembly retryable");
}

#[test]
fn diagnostic_row_cap_is_enforced_independently_of_expected_row_capacity() {
    let mut role = account_role();
    let diagnostic_rows = (1..=(MAX_PM_ACCOUNT_DIAGNOSTIC_EXTRA_ROWS + 1))
        .map(|index| PmFixtureBalanceRow::new(diagnostic_asset(index as u8), U256::from_u64(1)))
        .collect::<Vec<_>>();
    let mut assembly = role
        .request_snapshot(ConnectionEpoch::new(1), IngressSequence::new(10))
        .unwrap()
        .begin(snapshot(1));
    assert!(matches!(
        assembly.push_page(account_scope(), None, None, &diagnostic_rows, &[], &[],),
        Err(PmAccountPositionRoleError::Aggregate(
            PmAggregateError::TooManyBalanceRows
        ))
    ));
    assembly
        .push_page(account_scope(), None, None, &[], &[], &[])
        .unwrap();
    assembly
        .finish(completion(1, 11, Some(1)))
        .expect("diagnostic overflow must not advance the page chain");
}

#[test]
fn account_page_cap_is_preflighted_before_normalization_without_role_corruption() {
    let mut role = account_role();
    let mut assembly = role
        .request_snapshot(ConnectionEpoch::new(1), IngressSequence::new(10))
        .unwrap()
        .begin(snapshot(1));
    for page in 0..MAX_PM_FIXTURE_QUERY_PAGES {
        let requested = (page != 0).then(|| page_cursor(page));
        assembly
            .push_page(
                account_scope(),
                requested,
                Some(page_cursor(page + 1)),
                &[],
                &[],
                &[],
            )
            .unwrap();
    }

    let row = PmFixtureBalanceRow::new(trading_domain().collateral(), U256::from_u64(1));
    assert_eq!(
        assembly
            .push_page(
                wrong_observed_scope(),
                Some(page_cursor(MAX_PM_FIXTURE_QUERY_PAGES)),
                None,
                &[row],
                &[],
                &[],
            )
            .unwrap_err(),
        PmAccountPositionRoleError::TooManyPages
    );
    assert_eq!(
        assembly
            .push_page(
                account_scope(),
                Some(page_cursor(MAX_PM_FIXTURE_QUERY_PAGES)),
                None,
                &[],
                &[],
                &[],
            )
            .unwrap_err(),
        PmAccountPositionRoleError::TooManyPages
    );
    role.request_snapshot(ConnectionEpoch::new(1), IngressSequence::new(20))
        .expect("page saturation must not corrupt role request ordering");
}
