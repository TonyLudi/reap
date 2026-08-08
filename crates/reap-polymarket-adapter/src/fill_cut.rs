use reap_pm_core::{
    ConnectionEpoch, PmAccountScope, PmAssetId, PmConditionId, PmFillFee, PmFillId,
    PmFillQueryCursor, PmFillRole, PmFillSettlementStatus, PmMarketId, PmOrderSide, PmPrice,
    PmQuantity, PmReconciliationRequestBoundary, PmSign, PmSnapshotEvidence, PmTokenId,
    PmVenueOrderId,
};
use sha2::{Digest, Sha256};
use thiserror::Error;

/// The canonical full-account cut is deliberately no larger than the core
/// reconciliation carrier it will eventually produce.
pub const MAX_PM_FULL_ACCOUNT_FILL_LEGS: usize = reap_pm_core::MAX_PM_RECONCILIATION_FILLS;

const SNAPSHOT_DIGEST_DOMAIN: &[u8] = b"reap.pm.full-account-fill-snapshot.v1\0";
const CUT_CURSOR_DOMAIN: &[u8] = b"reap.pm.complete-fill-cut.v1\0";

/// Stable non-secret identity bound into a full-account fill cut.
///
/// The configured condition/question/token triple identifies the product
/// consuming the account-wide cut. Individual legs retain their own
/// condition and token so foreign-product rows are neither silently dropped
/// nor mistaken for the configured instrument.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PmFullAccountFillCutScope {
    account: PmAccountScope,
    configured_condition: PmConditionId,
    configured_market: PmMarketId,
    configured_token: PmTokenId,
}

impl PmFullAccountFillCutScope {
    #[must_use]
    pub const fn new(
        account: PmAccountScope,
        configured_condition: PmConditionId,
        configured_market: PmMarketId,
        configured_token: PmTokenId,
    ) -> Self {
        Self {
            account,
            configured_condition,
            configured_market,
            configured_token,
        }
    }

    #[must_use]
    pub const fn account(self) -> PmAccountScope {
        self.account
    }

    #[must_use]
    pub const fn configured_condition(self) -> PmConditionId {
        self.configured_condition
    }

    #[must_use]
    pub const fn configured_market(self) -> PmMarketId {
        self.configured_market
    }

    #[must_use]
    pub const fn configured_token(self) -> PmTokenId {
        self.configured_token
    }
}

/// The venue identity of one normalized account-visible fill leg.
///
/// Polymarket trade identity alone is insufficient because a single trade can
/// contain multiple maker-order legs. This mirrors the core fill-key identity
/// without assigning a local account handle or ownership association.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PmAccountFillLegKey {
    venue_order: PmVenueOrderId,
    fill: PmFillId,
}

impl PmAccountFillLegKey {
    #[must_use]
    pub const fn new(venue_order: PmVenueOrderId, fill: PmFillId) -> Self {
        Self { venue_order, fill }
    }

    #[must_use]
    pub const fn venue_order(self) -> PmVenueOrderId {
        self.venue_order
    }

    #[must_use]
    pub const fn fill(self) -> PmFillId {
        self.fill
    }
}

/// One secret-free, fully normalized account-visible fill leg.
///
/// Raw bodies, authenticated credential-owner identifiers, transport
/// cursors, timestamps, transaction hashes, and local client-order
/// associations are absent by construction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PmAccountFillLeg {
    key: PmAccountFillLegKey,
    condition: PmConditionId,
    token: PmTokenId,
    side: PmOrderSide,
    role: PmFillRole,
    settlement: PmFillSettlementStatus,
    price: PmPrice,
    quantity: PmQuantity,
    fee: PmFillFee,
}

impl PmAccountFillLeg {
    #[allow(
        clippy::too_many_arguments,
        reason = "the canonical leg keeps every exact semantic field explicit"
    )]
    #[must_use]
    pub const fn new(
        key: PmAccountFillLegKey,
        condition: PmConditionId,
        token: PmTokenId,
        side: PmOrderSide,
        role: PmFillRole,
        settlement: PmFillSettlementStatus,
        price: PmPrice,
        quantity: PmQuantity,
        fee: PmFillFee,
    ) -> Self {
        Self {
            key,
            condition,
            token,
            side,
            role,
            settlement,
            price,
            quantity,
            fee,
        }
    }

    #[must_use]
    pub const fn key(self) -> PmAccountFillLegKey {
        self.key
    }

    #[must_use]
    pub const fn condition(self) -> PmConditionId {
        self.condition
    }

    #[must_use]
    pub const fn token(self) -> PmTokenId {
        self.token
    }

    #[must_use]
    pub const fn side(self) -> PmOrderSide {
        self.side
    }

    #[must_use]
    pub const fn role(self) -> PmFillRole {
        self.role
    }

    #[must_use]
    pub const fn settlement(self) -> PmFillSettlementStatus {
        self.settlement
    }

    #[must_use]
    pub const fn price(self) -> PmPrice {
        self.price
    }

    #[must_use]
    pub const fn quantity(self) -> PmQuantity {
        self.quantity
    }

    #[must_use]
    pub const fn fee(self) -> PmFillFee {
        self.fee
    }
}

/// Collision-resistant identity of canonical full-account snapshot content.
///
/// This is not itself a reconciliation cursor. Equal content intentionally
/// has an equal digest; the cursor additionally chains the prior durable cut
/// and exact local request/completion evidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PmFullAccountFillSnapshotDigest([u8; 32]);

impl PmFullAccountFillSnapshotDigest {
    #[must_use]
    pub const fn bytes(self) -> [u8; 32] {
        self.0
    }
}

/// Exact causal evidence for one complete terminally paginated fill cut.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PmCompleteFillCutEvidence {
    connection_epoch: ConnectionEpoch,
    snapshot: PmSnapshotEvidence,
    boundary: PmReconciliationRequestBoundary,
}

impl PmCompleteFillCutEvidence {
    pub fn new(
        connection_epoch: ConnectionEpoch,
        snapshot: PmSnapshotEvidence,
        boundary: PmReconciliationRequestBoundary,
    ) -> Result<Self, PmFillCutError> {
        if connection_epoch.value() == 0 {
            return Err(PmFillCutError::ZeroConnectionEpoch);
        }
        Ok(Self {
            connection_epoch,
            snapshot,
            boundary,
        })
    }

    #[must_use]
    pub const fn connection_epoch(self) -> ConnectionEpoch {
        self.connection_epoch
    }

    #[must_use]
    pub const fn snapshot(self) -> PmSnapshotEvidence {
        self.snapshot
    }

    #[must_use]
    pub const fn boundary(self) -> PmReconciliationRequestBoundary {
        self.boundary
    }
}

/// Canonically sorted, duplicate-converged content of one complete account
/// fill snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PmCanonicalFullAccountFillSnapshot {
    scope: PmFullAccountFillCutScope,
    legs: Box<[PmAccountFillLeg]>,
    digest: PmFullAccountFillSnapshotDigest,
}

impl PmCanonicalFullAccountFillSnapshot {
    pub fn new(
        scope: PmFullAccountFillCutScope,
        legs: Box<[PmAccountFillLeg]>,
    ) -> Result<Self, PmFillCutError> {
        if legs.len() > MAX_PM_FULL_ACCOUNT_FILL_LEGS {
            return Err(PmFillCutError::TooManyLegs);
        }
        let mut legs = Vec::from(legs);
        // Sort by the core identity first, not by condition/token facts. This
        // guarantees every repeated venue-order/fill key is adjacent, so a
        // condition, token, or execution conflict cannot hide elsewhere in
        // the canonical ordering.
        legs.sort_unstable_by_key(|leg| leg.key());

        // Converge exact page-overlap duplicates in place without allocating
        // a second potentially maximum-sized collection. A repeated venue
        // identity with different semantics invalidates the entire cut.
        let mut output = 0;
        for input in 0..legs.len() {
            let leg = legs[input];
            if output != 0 && legs[output - 1].key() == leg.key() {
                if legs[output - 1] != leg {
                    return Err(PmFillCutError::ConflictingLeg);
                }
                continue;
            }
            legs[output] = leg;
            output += 1;
        }
        legs.truncate(output);
        let legs = legs.into_boxed_slice();
        let digest = digest_snapshot(scope, &legs);
        Ok(Self {
            scope,
            legs,
            digest,
        })
    }

    #[must_use]
    pub const fn scope(&self) -> PmFullAccountFillCutScope {
        self.scope
    }

    #[must_use]
    pub fn legs(&self) -> &[PmAccountFillLeg] {
        &self.legs
    }

    #[must_use]
    pub const fn digest(&self) -> PmFullAccountFillSnapshotDigest {
        self.digest
    }

    /// Derive a new account-scoped equality cursor from the prior durable cut,
    /// exact causal evidence, and canonical snapshot content.
    ///
    /// Including the prior cursor means a newly completed cut advances even
    /// when its normalized content is unchanged. The transport pagination
    /// continuation is deliberately not an input.
    pub fn derive_cursor(
        &self,
        prior: Option<PmFillQueryCursor>,
        evidence: PmCompleteFillCutEvidence,
    ) -> Result<PmFillQueryCursor, PmFillCutError> {
        if prior.is_some_and(|cursor| cursor.account_scope() != self.scope.account()) {
            return Err(PmFillCutError::PriorCursorScopeMismatch);
        }
        let mut digest = Sha256::new();
        digest.update(CUT_CURSOR_DOMAIN);
        encode_scope(&mut digest, self.scope);
        match prior {
            None => digest.update([0]),
            Some(cursor) => {
                digest.update([1]);
                digest.update(cursor.opaque());
            }
        }
        digest.update(evidence.connection_epoch().value().to_be_bytes());
        digest.update(evidence.boundary().request_sequence().value().to_be_bytes());
        digest.update(
            evidence
                .boundary()
                .completion_sequence()
                .value()
                .to_be_bytes(),
        );
        digest.update(evidence.snapshot().revision().value().to_be_bytes());
        digest.update(self.digest.bytes());
        checked_cursor(self.scope.account(), prior, digest.finalize().into())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum PmFillCutError {
    #[error("full-account fill snapshot exceeds its fixed normalized-leg bound")]
    TooManyLegs,
    #[error("one exact account fill identity carries conflicting normalized facts")]
    ConflictingLeg,
    #[error("complete fill-cut connection epoch must be nonzero")]
    ZeroConnectionEpoch,
    #[error("prior fill cursor belongs to another exact account scope")]
    PriorCursorScopeMismatch,
    #[error("derived complete fill-cut cursor did not advance")]
    CursorDidNotAdvance,
}

fn digest_snapshot(
    scope: PmFullAccountFillCutScope,
    legs: &[PmAccountFillLeg],
) -> PmFullAccountFillSnapshotDigest {
    let mut digest = Sha256::new();
    digest.update(SNAPSHOT_DIGEST_DOMAIN);
    encode_scope(&mut digest, scope);
    digest.update(
        u32::try_from(legs.len())
            .expect("bounded full-account fill-leg count fits u32")
            .to_be_bytes(),
    );
    for leg in legs {
        encode_leg(&mut digest, *leg);
    }
    PmFullAccountFillSnapshotDigest(digest.finalize().into())
}

fn encode_scope(digest: &mut Sha256, scope: PmFullAccountFillCutScope) {
    encode_ascii(digest, scope.account().environment().as_str());
    digest.update(scope.account().chain().value().to_be_bytes());
    digest.update(scope.account().signer().address().bytes());
    digest.update(scope.account().funder().address().bytes());
    digest.update(scope.account().handle().ordinal().to_be_bytes());
    digest.update(scope.configured_condition().bytes());
    digest.update(scope.configured_market().bytes());
    digest.update(scope.configured_token().units().to_be_bytes());
}

fn encode_leg(digest: &mut Sha256, leg: PmAccountFillLeg) {
    encode_ascii(digest, leg.key().venue_order().as_str());
    encode_ascii(digest, leg.key().fill().as_str());
    digest.update(leg.condition().bytes());
    digest.update(leg.token().units().to_be_bytes());
    digest.update([match leg.side() {
        PmOrderSide::Buy => 0,
        PmOrderSide::Sell => 1,
    }]);
    digest.update([match leg.role() {
        PmFillRole::Maker => 0,
        PmFillRole::Taker => 1,
    }]);
    digest.update([match leg.settlement() {
        PmFillSettlementStatus::Matched => 0,
        PmFillSettlementStatus::Mined => 1,
        PmFillSettlementStatus::Confirmed => 2,
        PmFillSettlementStatus::Retrying => 3,
        PmFillSettlementStatus::Failed => 4,
        PmFillSettlementStatus::MatchedNotBroadcasted => 5,
    }]);
    digest.update(leg.price().units().to_be_bytes());
    digest.update(leg.quantity().protocol_units().to_be_bytes());
    encode_fee(digest, leg.fee());
}

fn encode_fee(digest: &mut Sha256, fee: PmFillFee) {
    match fee {
        PmFillFee::Unknown => digest.update([0]),
        PmFillFee::Incomplete => digest.update([1]),
        PmFillFee::Known { asset, delta } => {
            digest.update([2]);
            match asset {
                PmAssetId::Collateral { contract } => {
                    digest.update([0]);
                    digest.update(contract.bytes());
                }
                PmAssetId::Outcome { contract, token } => {
                    digest.update([1]);
                    digest.update(contract.bytes());
                    digest.update(token.units().to_be_bytes());
                }
            }
            digest.update([match delta.sign() {
                PmSign::Positive => 0,
                PmSign::Negative => 1,
            }]);
            digest.update(delta.magnitude().to_be_bytes());
        }
    }
}

fn encode_ascii(digest: &mut Sha256, value: &str) {
    digest.update(
        u16::try_from(value.len())
            .expect("bounded PM identity length fits u16")
            .to_be_bytes(),
    );
    digest.update(value.as_bytes());
}

fn checked_cursor(
    account: PmAccountScope,
    prior: Option<PmFillQueryCursor>,
    opaque: [u8; 32],
) -> Result<PmFillQueryCursor, PmFillCutError> {
    if prior.is_some_and(|cursor| cursor.opaque() == opaque) {
        return Err(PmFillCutError::CursorDidNotAdvance);
    }
    Ok(PmFillQueryCursor::new(account, opaque))
}

#[cfg(test)]
mod tests {
    use reap_pm_core::{
        EvmAddress, IngressSequence, PmChainId, PmEnvironmentId, PmFunderId,
        PmReconciliationRequestBoundary, PmSignedUnits, PmSignerId, SnapshotRevision, U256,
    };

    use super::*;

    fn condition(byte: u8) -> PmConditionId {
        PmConditionId::from_bytes([byte; 32]).unwrap()
    }

    fn market(byte: u8) -> PmMarketId {
        PmMarketId::from_bytes([byte; 32]).unwrap()
    }

    fn token(value: u64) -> PmTokenId {
        PmTokenId::new(U256::from_u64(value)).unwrap()
    }

    fn account() -> PmAccountScope {
        PmAccountScope::new(
            PmEnvironmentId::new("fill-cut-test").unwrap(),
            PmChainId::new(137).unwrap(),
            PmSignerId::new(EvmAddress::from_bytes([1; 20]).unwrap()),
            PmFunderId::new(EvmAddress::from_bytes([2; 20]).unwrap()),
            reap_pm_core::PmAccountHandle::from_ordinal(3),
        )
    }

    fn scope() -> PmFullAccountFillCutScope {
        PmFullAccountFillCutScope::new(account(), condition(3), market(4), token(5))
    }

    fn key(order: &str, fill: &str) -> PmAccountFillLegKey {
        PmAccountFillLegKey::new(
            PmVenueOrderId::new(order).unwrap(),
            PmFillId::new(fill).unwrap(),
        )
    }

    fn leg(order: &str, fill: &str) -> PmAccountFillLeg {
        PmAccountFillLeg::new(
            key(order, fill),
            condition(3),
            token(5),
            PmOrderSide::Buy,
            PmFillRole::Maker,
            PmFillSettlementStatus::Matched,
            PmPrice::from_units(400_000).unwrap(),
            PmQuantity::from_protocol_units(U256::from_u64(250_000)).unwrap(),
            PmFillFee::Unknown,
        )
    }

    fn evidence(request: u64, completion: u64) -> PmCompleteFillCutEvidence {
        PmCompleteFillCutEvidence::new(
            ConnectionEpoch::new(7),
            PmSnapshotEvidence::new(SnapshotRevision::new(9)).unwrap(),
            PmReconciliationRequestBoundary::new(
                IngressSequence::new(request),
                IngressSequence::new(completion),
            )
            .unwrap(),
        )
        .unwrap()
    }

    #[test]
    fn canonical_order_and_exact_duplicates_are_digest_invariant() {
        let first = leg("order-a", "fill-a");
        let second = leg("order-b", "fill-b");
        let canonical = PmCanonicalFullAccountFillSnapshot::new(
            scope(),
            vec![first, second].into_boxed_slice(),
        )
        .unwrap();
        let reordered_and_repeated = PmCanonicalFullAccountFillSnapshot::new(
            scope(),
            vec![second, first, second, first].into_boxed_slice(),
        )
        .unwrap();

        assert_eq!(canonical.legs(), reordered_and_repeated.legs());
        assert_eq!(canonical.digest(), reordered_and_repeated.digest());
        assert_eq!(
            canonical.derive_cursor(None, evidence(10, 11)).unwrap(),
            reordered_and_repeated
                .derive_cursor(None, evidence(10, 11))
                .unwrap()
        );
    }

    #[test]
    fn every_normalized_semantic_field_is_digest_sensitive() {
        let base = leg("order-a", "fill-a");
        let known_fee = PmFillFee::Known {
            asset: PmAssetId::collateral(EvmAddress::from_bytes([8; 20]).unwrap()),
            delta: PmSignedUnits::from_parts(PmSign::Negative, U256::from_u64(7)).unwrap(),
        };
        let variants = [
            PmAccountFillLeg {
                key: key("order-b", "fill-a"),
                ..base
            },
            PmAccountFillLeg {
                key: key("order-a", "fill-b"),
                ..base
            },
            PmAccountFillLeg {
                condition: condition(6),
                ..base
            },
            PmAccountFillLeg {
                token: token(6),
                ..base
            },
            PmAccountFillLeg {
                side: PmOrderSide::Sell,
                ..base
            },
            PmAccountFillLeg {
                role: PmFillRole::Taker,
                ..base
            },
            PmAccountFillLeg {
                settlement: PmFillSettlementStatus::Mined,
                ..base
            },
            PmAccountFillLeg {
                price: PmPrice::from_units(410_000).unwrap(),
                ..base
            },
            PmAccountFillLeg {
                quantity: PmQuantity::from_protocol_units(U256::from_u64(260_000)).unwrap(),
                ..base
            },
            PmAccountFillLeg {
                fee: known_fee,
                ..base
            },
        ];
        let base_digest =
            PmCanonicalFullAccountFillSnapshot::new(scope(), vec![base].into_boxed_slice())
                .unwrap()
                .digest();
        for variant in variants {
            assert_ne!(
                PmCanonicalFullAccountFillSnapshot::new(scope(), vec![variant].into_boxed_slice())
                    .unwrap()
                    .digest(),
                base_digest
            );
        }
    }

    #[test]
    fn repeated_key_with_different_facts_rejects_the_whole_cut() {
        let base = leg("order-a", "fill-a");
        let conflict = PmAccountFillLeg::new(
            base.key(),
            condition(99),
            base.token(),
            base.side(),
            base.role(),
            base.settlement(),
            base.price(),
            base.quantity(),
            base.fee(),
        );
        assert_eq!(
            PmCanonicalFullAccountFillSnapshot::new(
                scope(),
                vec![base, conflict].into_boxed_slice()
            ),
            Err(PmFillCutError::ConflictingLeg)
        );
    }

    #[test]
    fn multi_order_legs_and_foreign_products_are_retained() {
        let first = leg("maker-a", "shared-trade");
        let mut second = leg("maker-b", "shared-trade");
        second.condition = condition(77);
        second.token = token(88);
        let snapshot = PmCanonicalFullAccountFillSnapshot::new(
            scope(),
            vec![second, first].into_boxed_slice(),
        )
        .unwrap();

        assert_eq!(snapshot.legs().len(), 2);
        assert!(snapshot.legs().contains(&first));
        assert!(snapshot.legs().contains(&second));
        let without_foreign =
            PmCanonicalFullAccountFillSnapshot::new(scope(), vec![first].into_boxed_slice())
                .unwrap();
        assert_ne!(snapshot.digest(), without_foreign.digest());
    }

    #[test]
    fn identical_snapshot_content_still_advances_each_complete_cut() {
        let snapshot = PmCanonicalFullAccountFillSnapshot::new(
            scope(),
            vec![leg("order-a", "fill-a")].into_boxed_slice(),
        )
        .unwrap();
        let first = snapshot.derive_cursor(None, evidence(10, 11)).unwrap();
        let second = snapshot
            .derive_cursor(Some(first), evidence(20, 21))
            .unwrap();
        assert_ne!(first, second);
        assert_eq!(
            snapshot
                .derive_cursor(Some(first), evidence(20, 21))
                .unwrap(),
            second,
            "retrying one exact complete cut is deterministic"
        );
    }

    #[test]
    fn returning_to_prior_content_never_returns_to_a_prior_causal_cursor() {
        let snapshot_a = PmCanonicalFullAccountFillSnapshot::new(
            scope(),
            vec![leg("order-a", "fill-a")].into_boxed_slice(),
        )
        .unwrap();
        let snapshot_b = PmCanonicalFullAccountFillSnapshot::new(
            scope(),
            vec![leg("order-b", "fill-b")].into_boxed_slice(),
        )
        .unwrap();
        let snapshot_a_again = PmCanonicalFullAccountFillSnapshot::new(
            scope(),
            vec![leg("order-a", "fill-a")].into_boxed_slice(),
        )
        .unwrap();
        let cursor_a1 = snapshot_a.derive_cursor(None, evidence(10, 11)).unwrap();
        let cursor_b = snapshot_b
            .derive_cursor(Some(cursor_a1), evidence(20, 21))
            .unwrap();
        let cursor_a2 = snapshot_a_again
            .derive_cursor(Some(cursor_b), evidence(30, 31))
            .unwrap();

        assert_eq!(snapshot_a.digest(), snapshot_a_again.digest());
        assert_ne!(cursor_a1, cursor_b);
        assert_ne!(cursor_b, cursor_a2);
        assert_ne!(cursor_a1, cursor_a2);
        assert_eq!(
            snapshot_a_again
                .derive_cursor(Some(cursor_b), evidence(30, 31))
                .unwrap(),
            cursor_a2,
            "retrying exact prior plus evidence remains deterministic"
        );
    }

    #[test]
    fn scope_and_fixed_bound_are_fail_closed() {
        let snapshot = PmCanonicalFullAccountFillSnapshot::new(scope(), Box::new([])).unwrap();
        let foreign_account = PmAccountScope::new(
            account().environment(),
            account().chain(),
            account().signer(),
            account().funder(),
            reap_pm_core::PmAccountHandle::from_ordinal(99),
        );
        assert_eq!(
            snapshot.derive_cursor(
                Some(PmFillQueryCursor::new(foreign_account, [1; 32])),
                evidence(10, 11)
            ),
            Err(PmFillCutError::PriorCursorScopeMismatch)
        );

        let legs = (0..=MAX_PM_FULL_ACCOUNT_FILL_LEGS)
            .map(|index| leg(&format!("order-{index}"), "fill"))
            .collect::<Vec<_>>()
            .into_boxed_slice();
        assert_eq!(
            PmCanonicalFullAccountFillSnapshot::new(scope(), legs),
            Err(PmFillCutError::TooManyLegs)
        );
    }

    #[test]
    fn a_hash_collision_with_the_prior_cursor_fails_closed() {
        let prior = PmFillQueryCursor::new(account(), [7; 32]);
        assert_eq!(
            checked_cursor(account(), Some(prior), [7; 32]),
            Err(PmFillCutError::CursorDidNotAdvance)
        );
    }
}
