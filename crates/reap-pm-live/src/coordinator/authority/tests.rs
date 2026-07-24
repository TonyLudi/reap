use reap_pm_core::{
    EvmAddress, PmAccountHandle, PmChainId, PmClientOrderId, PmEnvironmentId, PmFunderId,
    PmMarketHandle, PmMarketId, PmSignerId, PmTokenHandle, PmTokenId, PmVenueOrderId,
};

use super::*;

const EOA: &str = "0xabababababababababababababababababababab";
const MARKET: &str = "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

fn account_scope() -> PmAccountScope {
    let eoa = EvmAddress::parse(EOA).expect("valid EOA");
    PmAccountScope::new(
        PmEnvironmentId::new("phase5-authority-test").expect("valid environment"),
        PmChainId::new(137).expect("valid chain"),
        PmSignerId::new(eoa),
        PmFunderId::new(eoa),
        PmAccountHandle::from_ordinal(7),
    )
}

fn instrument() -> PmInstrumentHandle {
    PmInstrumentHandle::new(
        PmMarketHandle::from_ordinal(1),
        PmTokenHandle::from_ordinal(2),
    )
}

fn instrument_id() -> PmInstrumentId {
    PmInstrumentId::new(
        PmMarketId::parse(MARKET).expect("valid market"),
        PmTokenId::new(U256::from_u64(123)).expect("nonzero token"),
    )
}

fn client_order() -> PmClientOrderKey {
    PmClientOrderKey::new(
        account_scope().handle(),
        PmClientOrderId::from_bytes([0x11; 16]).expect("nonzero client order"),
    )
}

fn revisions() -> PmAuthorityRevisions {
    PmAuthorityRevisions::new(
        SnapshotRevision::new(7),
        SnapshotRevision::new(8),
        9,
        10,
        11,
    )
    .expect("nonzero revisions")
}

fn quote_facts() -> PmQuoteAuthorityFacts {
    let account_scope = account_scope();
    let instrument = instrument();
    let side = PmOrderSide::Buy;
    let price = PmPrice::parse_decimal("0.40").expect("valid price");
    let quantity = PmQuantity::parse_decimal("10").expect("valid quantity");
    let amounts = exact_order_amounts(side, price, quantity).expect("exact amounts");
    let execution = PmFixtureOwnedExecution::new(account_scope, instrument);
    PmQuoteAuthorityFacts {
        account_scope,
        instrument,
        instrument_id: instrument_id(),
        intent: PmOwnedIntentId::new(1).expect("nonzero intent"),
        client_order: client_order(),
        side,
        price,
        quantity,
        maker_amount: amounts.maker(),
        taker_amount: amounts.taker(),
        reservation: PmExactReservation::policy_approved(amounts.maker(), U256::ZERO)
            .expect("nonzero reservation"),
        profile: execution.place_profile(),
        salt: PmOrderSalt::from_u64(123).expect("valid salt"),
        timestamp_ms: 1_760_000_000_000,
        revisions: revisions(),
        approved_at_monotonic_ns: 100,
        expires_at_monotonic_ns: 200,
    }
}

#[test]
fn quote_journal_intent_carries_every_execution_authority_fact() {
    let facts = quote_facts();

    assert_eq!(
        quote_journal_intent(facts),
        PmJournalQuoteIntentV1 {
            intent_id: 1,
            client_order: facts.client_order,
            instrument: facts.instrument_id,
            side: PmJournalSideV1::Buy,
            price_units: 400_000,
            quantity: facts.quantity,
            reserved_collateral: U256::from_u64(4_000_000),
            reserved_outcome: U256::ZERO,
            profile: PmJournalQuoteProfileV1::PassiveGtcPostOnlyEoa,
            metadata_revision: 7,
            book_revision: 8,
            model_revision: 9,
            book_readiness_revision: 10,
            private_readiness_revision: 11,
            expires_at_monotonic_ns: 200,
            salt: facts.salt,
            timestamp_ms: 1_760_000_000_000,
            maker: facts.account_scope.funder().address(),
            signer: facts.account_scope.signer().address(),
            maker_amount: U256::from_u64(4_000_000),
            taker_amount: U256::from_u64(10_000_000),
        }
    );
}

#[test]
fn cancel_journal_intent_carries_exact_owned_order_identity_and_reason() {
    let quote = quote_facts();
    let venue_order = PmVenueOrderKey::new(
        quote.account_scope.handle(),
        PmVenueOrderId::new("venue-order-1").expect("valid venue order"),
    );
    let execution = PmFixtureOwnedExecution::new(quote.account_scope, quote.instrument);
    let cancel = PmCancelAuthorityFacts {
        account_scope: quote.account_scope,
        instrument: quote.instrument,
        instrument_id: quote.instrument_id,
        intent: quote.intent,
        client_order: quote.client_order,
        venue_order,
        side: quote.side,
        price: quote.price,
        quantity: quote.quantity,
        maker_amount: quote.maker_amount,
        taker_amount: quote.taker_amount,
        reservation: quote.reservation,
        quote_profile: quote.profile,
        cancel_purpose: execution.cancel_purpose(),
        salt: quote.salt,
        timestamp_ms: quote.timestamp_ms,
        approved_at_monotonic_ns: quote.approved_at_monotonic_ns,
        expires_at_monotonic_ns: quote.expires_at_monotonic_ns,
        reason: PmJournalCancelReasonV1::SafetyHalt,
    };

    assert_eq!(
        cancel_journal_intent(cancel),
        PmJournalCancelIntentV1 {
            client_order: quote.client_order,
            venue_order,
            reason: PmJournalCancelReasonV1::SafetyHalt,
        }
    );
}

#[test]
fn preparation_requires_exact_current_revisions_and_strictly_live_expiry() {
    let approved = revisions();
    assert_eq!(validate_current(approved, approved, 199, 200), Ok(()));
    assert_eq!(
        validate_current(
            approved,
            PmAuthorityRevisions::new(
                SnapshotRevision::new(7),
                SnapshotRevision::new(8),
                11,
                10,
                11,
            )
            .expect("nonzero revisions"),
            199,
            200,
        ),
        Err(PmAuthorityError::RevisionChanged)
    );
    assert_eq!(
        validate_current(approved, approved, 200, 200),
        Err(PmAuthorityError::ApprovalExpired)
    );
}
