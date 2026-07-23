use reap_pm_core::{
    EvmAddress, PmAssetId, PmEventError, PmInstrumentHandle, PmMarketEvent, PmMarketMetadata,
    PmProductSource, PmSpenderDomain, SnapshotRevision,
};
use reap_polymarket_wire::{PmBookParserConfig, PmClobMetadata, PmLifecycleMetadata, PmWireScope};
use sha2::{Digest, Sha256};
use thiserror::Error;

const METADATA_FINGERPRINT_PREFIX: &[u8] = b"reap.pm.public-metadata.v1\0";
const DOMAIN_FINGERPRINT_PREFIX: &[u8] = b"reap.pm.clob-v2-domain.v1\0";
const POLYGON_CHAIN_ID: u64 = 137;
const PUSD: &str = "0xC011a7E12a19f7B1f670d46F03B03f3342E82DFB";
const CONDITIONAL_TOKENS: &str = "0x4D97DCd97eC945f40cF65F87097ACe5EA0476045";
const STANDARD_EXCHANGE: &str = "0xE111180000d2663C0091e4f400237545B87B996B";
const NEGATIVE_RISK_EXCHANGE: &str = "0xe2222d279d744050d28e00520010520000310F59";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PmMetadataRevisionInput {
    revision: SnapshotRevision,
    monotonic_receive_ns: u64,
}

impl PmMetadataRevisionInput {
    pub fn new(
        revision: SnapshotRevision,
        monotonic_receive_ns: u64,
    ) -> Result<Self, PmMetadataJoinError> {
        if revision.value() == 0 {
            return Err(PmMetadataJoinError::ZeroRevision);
        }
        if monotonic_receive_ns == 0 {
            return Err(PmMetadataJoinError::ZeroReceiveTime);
        }
        Ok(Self {
            revision,
            monotonic_receive_ns,
        })
    }

    #[must_use]
    pub const fn revision(self) -> SnapshotRevision {
        self.revision
    }

    #[must_use]
    pub const fn monotonic_receive_ns(self) -> u64 {
        self.monotonic_receive_ns
    }
}

/// One atomic, checked public metadata observation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PmAuthoritativeMetadata {
    event: PmMarketEvent,
    parser_config: PmBookParserConfig,
    metadata_fingerprint: [u8; 32],
    domain_fingerprint: [u8; 32],
    monotonic_receive_ns: u64,
}

impl PmAuthoritativeMetadata {
    pub fn join(
        instrument: PmInstrumentHandle,
        source: PmProductSource,
        expected: PmMarketMetadata,
        lifecycle: PmLifecycleMetadata,
        clob: &PmClobMetadata,
        revision: PmMetadataRevisionInput,
    ) -> Result<Self, PmMetadataJoinError> {
        validate_wire_join(expected, lifecycle, clob)?;
        validate_goal_f_domain(expected)?;
        Self::from_validated_contract(instrument, source, expected, revision)
    }

    /// Verifies and reconstructs authority from a recorded capture header.
    ///
    /// This path is exclusively for offline capture verification and replay.
    /// Live observation must use [`Self::join`] with freshly parsed lifecycle
    /// and CLOB metadata. The recorded path revalidates the complete typed
    /// contract and both claimed fingerprints; it never treats persisted
    /// fingerprint bytes as authority by themselves.
    pub fn verify_recorded(
        instrument: PmInstrumentHandle,
        source: PmProductSource,
        recorded: PmMarketMetadata,
        revision: PmMetadataRevisionInput,
        claimed_metadata_fingerprint: [u8; 32],
        claimed_domain_fingerprint: [u8; 32],
    ) -> Result<Self, PmMetadataJoinError> {
        validate_ready_lifecycle(recorded.lifecycle())?;
        validate_goal_f_domain(recorded)?;
        let authority = Self::from_validated_contract(instrument, source, recorded, revision)?;
        if authority.metadata_fingerprint != claimed_metadata_fingerprint {
            return Err(PmMetadataJoinError::MetadataFingerprintMismatch);
        }
        if authority.domain_fingerprint != claimed_domain_fingerprint {
            return Err(PmMetadataJoinError::DomainFingerprintMismatch);
        }
        Ok(authority)
    }

    fn from_validated_contract(
        instrument: PmInstrumentHandle,
        source: PmProductSource,
        expected: PmMarketMetadata,
        revision: PmMetadataRevisionInput,
    ) -> Result<Self, PmMetadataJoinError> {
        let event = PmMarketEvent::new(source, instrument, revision.revision(), expected)
            .map_err(PmMetadataJoinError::Event)?;
        let parser_config = PmBookParserConfig::new(
            PmWireScope::new(
                expected.condition(),
                expected.market(),
                expected.outcome().token(),
            ),
            expected.tick(),
            expected.minimum_order_size(),
            expected.negative_risk(),
        );
        Ok(Self {
            event,
            parser_config,
            metadata_fingerprint: metadata_fingerprint(expected)?,
            domain_fingerprint: domain_fingerprint(expected)?,
            monotonic_receive_ns: revision.monotonic_receive_ns(),
        })
    }

    #[must_use]
    pub const fn event(self) -> PmMarketEvent {
        self.event
    }

    #[must_use]
    pub const fn parser_config(self) -> PmBookParserConfig {
        self.parser_config
    }

    #[must_use]
    pub const fn metadata_fingerprint(self) -> [u8; 32] {
        self.metadata_fingerprint
    }

    #[must_use]
    pub const fn domain_fingerprint(self) -> [u8; 32] {
        self.domain_fingerprint
    }

    #[must_use]
    pub const fn monotonic_receive_ns(self) -> u64 {
        self.monotonic_receive_ns
    }
}

#[derive(Debug, Error)]
pub enum PmMetadataJoinError {
    #[error("metadata revision must be nonzero")]
    ZeroRevision,
    #[error("metadata monotonic receive time must be nonzero")]
    ZeroReceiveTime,
    #[error("lifecycle condition differs from configured metadata")]
    LifecycleConditionMismatch,
    #[error("lifecycle market differs from configured metadata")]
    LifecycleMarketMismatch,
    #[error("CLOB condition differs from configured metadata")]
    ClobConditionMismatch,
    #[error("CLOB market differs from configured metadata")]
    ClobMarketMismatch,
    #[error("configured CLOB outcome token or label drifted")]
    OutcomeMismatch,
    #[error("market is inactive")]
    Inactive,
    #[error("market is closed")]
    Closed,
    #[error("market is archived")]
    Archived,
    #[error("market is not accepting orders")]
    NotAcceptingOrders,
    #[error("market order book is disabled")]
    OrderBookDisabled,
    #[error("market lifecycle differs from the configured contract")]
    LifecycleDrift,
    #[error("market tick differs from the configured contract")]
    TickDrift,
    #[error("market minimum order size differs from the configured contract")]
    MinimumDrift,
    #[error("market negative-risk domain differs from the configured contract")]
    NegativeRiskDrift,
    #[error("Goal F public metadata requires Polygon chain 137")]
    UnsupportedChain,
    #[error("Goal F public metadata requires its pinned CLOB V2 exchange")]
    UnsupportedExchange,
    #[error("Goal F public metadata requires exactly the pinned collateral and outcome spenders")]
    UnsupportedSpenderSet,
    #[error("metadata fingerprint serialization failed")]
    FingerprintSerialization,
    #[error("recorded public metadata fingerprint does not match the complete typed contract")]
    MetadataFingerprintMismatch,
    #[error("recorded public domain fingerprint does not match the complete typed contract")]
    DomainFingerprintMismatch,
    #[error("normalized market event is inconsistent with its configured source")]
    Event(PmEventError),
}

fn validate_wire_join(
    expected: PmMarketMetadata,
    lifecycle: PmLifecycleMetadata,
    clob: &PmClobMetadata,
) -> Result<(), PmMetadataJoinError> {
    if lifecycle.condition() != expected.condition() {
        return Err(PmMetadataJoinError::LifecycleConditionMismatch);
    }
    if lifecycle.market() != expected.market() {
        return Err(PmMetadataJoinError::LifecycleMarketMismatch);
    }
    if clob.condition() != expected.condition() {
        return Err(PmMetadataJoinError::ClobConditionMismatch);
    }
    if clob.market() != expected.market() {
        return Err(PmMetadataJoinError::ClobMarketMismatch);
    }
    if clob.configured_outcome() != expected.outcome() {
        return Err(PmMetadataJoinError::OutcomeMismatch);
    }

    let observed = lifecycle.lifecycle();
    if !observed.active() {
        return Err(PmMetadataJoinError::Inactive);
    }
    if observed.closed() {
        return Err(PmMetadataJoinError::Closed);
    }
    if observed.archived() {
        return Err(PmMetadataJoinError::Archived);
    }
    if !observed.accepting_orders() {
        return Err(PmMetadataJoinError::NotAcceptingOrders);
    }
    if !observed.order_book_enabled() {
        return Err(PmMetadataJoinError::OrderBookDisabled);
    }
    if observed != expected.lifecycle() {
        return Err(PmMetadataJoinError::LifecycleDrift);
    }
    if clob.tick() != expected.tick() {
        return Err(PmMetadataJoinError::TickDrift);
    }
    if clob.minimum_order_size() != expected.minimum_order_size() {
        return Err(PmMetadataJoinError::MinimumDrift);
    }
    if clob.negative_risk() != expected.negative_risk() {
        return Err(PmMetadataJoinError::NegativeRiskDrift);
    }
    Ok(())
}

fn validate_ready_lifecycle(
    lifecycle: reap_pm_core::PmMarketLifecycle,
) -> Result<(), PmMetadataJoinError> {
    if !lifecycle.active() {
        return Err(PmMetadataJoinError::Inactive);
    }
    if lifecycle.closed() {
        return Err(PmMetadataJoinError::Closed);
    }
    if lifecycle.archived() {
        return Err(PmMetadataJoinError::Archived);
    }
    if !lifecycle.accepting_orders() {
        return Err(PmMetadataJoinError::NotAcceptingOrders);
    }
    if !lifecycle.order_book_enabled() {
        return Err(PmMetadataJoinError::OrderBookDisabled);
    }
    Ok(())
}

fn validate_goal_f_domain(expected: PmMarketMetadata) -> Result<(), PmMetadataJoinError> {
    if expected.chain().value() != POLYGON_CHAIN_ID {
        return Err(PmMetadataJoinError::UnsupportedChain);
    }
    let exchange = parse_address(if expected.negative_risk() {
        NEGATIVE_RISK_EXCHANGE
    } else {
        STANDARD_EXCHANGE
    });
    if expected.exchange() != exchange {
        return Err(PmMetadataJoinError::UnsupportedExchange);
    }

    let domain = if expected.negative_risk() {
        PmSpenderDomain::NegativeRisk
    } else {
        PmSpenderDomain::Standard
    };
    let collateral = PmAssetId::collateral(parse_address(PUSD));
    let outcome = PmAssetId::outcome(
        parse_address(CONDITIONAL_TOKENS),
        expected.outcome().token(),
    );
    let requirements = expected.required_spenders().collect::<Vec<_>>();
    if requirements.len() != 2
        || !requirements.iter().any(|requirement| {
            requirement.chain() == expected.chain()
                && requirement.spender() == exchange
                && requirement.domain() == domain
                && requirement.asset() == collateral
        })
        || !requirements.iter().any(|requirement| {
            requirement.chain() == expected.chain()
                && requirement.spender() == exchange
                && requirement.domain() == domain
                && requirement.asset() == outcome
        })
    {
        return Err(PmMetadataJoinError::UnsupportedSpenderSet);
    }
    Ok(())
}

fn metadata_fingerprint(expected: PmMarketMetadata) -> Result<[u8; 32], PmMetadataJoinError> {
    let encoded =
        serde_json::to_vec(&expected).map_err(|_| PmMetadataJoinError::FingerprintSerialization)?;
    let mut digest = Sha256::new();
    digest.update(METADATA_FINGERPRINT_PREFIX);
    digest.update(encoded);
    Ok(digest.finalize().into())
}

fn domain_fingerprint(expected: PmMarketMetadata) -> Result<[u8; 32], PmMetadataJoinError> {
    let mut digest = Sha256::new();
    digest.update(DOMAIN_FINGERPRINT_PREFIX);
    digest.update(expected.chain().value().to_be_bytes());
    digest.update([u8::from(expected.negative_risk())]);
    digest.update(
        serde_json::to_vec(&expected.exchange())
            .map_err(|_| PmMetadataJoinError::FingerprintSerialization)?,
    );
    for requirement in expected.required_spenders() {
        digest.update(
            serde_json::to_vec(&requirement)
                .map_err(|_| PmMetadataJoinError::FingerprintSerialization)?,
        );
    }
    Ok(digest.finalize().into())
}

fn parse_address(value: &str) -> EvmAddress {
    EvmAddress::parse(value).expect("Goal F pinned contract address is valid")
}

#[cfg(test)]
mod tests {
    use reap_pm_core::{
        MAX_REQUIRED_SPENDERS, PmChainId, PmConditionId, PmMarketHandle, PmMarketId,
        PmMarketLifecycle, PmOutcomeLabel, PmOutcomeMetadata, PmQuantity, PmSourceHandle,
        PmSpenderRequirement, PmTick, PmTokenHandle, PmTokenId, U256,
    };
    use reap_polymarket_wire::{PmWireScope, parse_clob_metadata, parse_lifecycle_metadata};

    use super::*;

    const CONDITION: &str = "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const MARKET: &str = "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

    fn instrument() -> PmInstrumentHandle {
        PmInstrumentHandle::new(
            PmMarketHandle::from_ordinal(1),
            PmTokenHandle::from_ordinal(2),
        )
    }

    fn source() -> PmProductSource {
        PmProductSource::polymarket_market(PmSourceHandle::from_ordinal(1), instrument().token())
    }

    fn token() -> PmTokenId {
        PmTokenId::new(U256::from_u64(123)).unwrap()
    }

    fn scope() -> PmWireScope {
        PmWireScope::new(
            PmConditionId::parse(CONDITION).unwrap(),
            PmMarketId::parse(MARKET).unwrap(),
            token(),
        )
    }

    fn lifecycle(
        active: bool,
        closed: bool,
        archived: bool,
        accepting: bool,
        book: bool,
    ) -> PmLifecycleMetadata {
        let raw = format!(
            r#"{{"condition_id":"{CONDITION}","market_id":"{MARKET}","active":{active},"closed":{closed},"archived":{archived},"accepting_orders":{accepting},"enable_order_book":{book}}}"#
        );
        parse_lifecycle_metadata(raw.as_bytes(), scope()).unwrap()
    }

    fn clob(tick: &str, minimum: &str, negative_risk: bool, label: &str) -> PmClobMetadata {
        let raw = format!(
            r#"{{"condition_id":"{CONDITION}","market_id":"{MARKET}","minimum_tick_size":"{tick}","minimum_order_size":"{minimum}","neg_risk":{negative_risk},"tokens":[{{"token_id":"123","outcome":"{label}"}},{{"token_id":"456","outcome":"No"}}]}}"#
        );
        parse_clob_metadata(raw.as_bytes(), scope()).unwrap()
    }

    fn metadata(negative_risk: bool) -> PmMarketMetadata {
        let exchange = parse_address(if negative_risk {
            NEGATIVE_RISK_EXCHANGE
        } else {
            STANDARD_EXCHANGE
        });
        let domain = if negative_risk {
            PmSpenderDomain::NegativeRisk
        } else {
            PmSpenderDomain::Standard
        };
        let chain = PmChainId::new(137).unwrap();
        let mut requirements = [None; MAX_REQUIRED_SPENDERS];
        requirements[0] = Some(PmSpenderRequirement::new(
            chain,
            exchange,
            domain,
            PmAssetId::collateral(parse_address(PUSD)),
        ));
        requirements[1] = Some(PmSpenderRequirement::new(
            chain,
            exchange,
            domain,
            PmAssetId::outcome(parse_address(CONDITIONAL_TOKENS), token()),
        ));
        PmMarketMetadata::new(
            PmConditionId::parse(CONDITION).unwrap(),
            PmMarketId::parse(MARKET).unwrap(),
            PmOutcomeMetadata::new(token(), PmOutcomeLabel::new("Yes").unwrap()),
            PmMarketLifecycle::new(true, false, false, true, true),
            PmTick::parse_decimal("0.01").unwrap(),
            PmQuantity::parse_decimal("5").unwrap(),
            negative_risk,
            chain,
            exchange,
            requirements,
            2,
        )
        .unwrap()
    }

    fn metadata_with_lifecycle(lifecycle: PmMarketLifecycle) -> PmMarketMetadata {
        let base = metadata(false);
        let mut requirements = [None; MAX_REQUIRED_SPENDERS];
        for (index, requirement) in base.required_spenders().enumerate() {
            requirements[index] = Some(requirement);
        }
        PmMarketMetadata::new(
            base.condition(),
            base.market(),
            base.outcome(),
            lifecycle,
            base.tick(),
            base.minimum_order_size(),
            base.negative_risk(),
            base.chain(),
            base.exchange(),
            requirements,
            2,
        )
        .unwrap()
    }

    fn metadata_on_unsupported_chain() -> PmMarketMetadata {
        let chain = PmChainId::new(1).unwrap();
        let exchange = parse_address(STANDARD_EXCHANGE);
        let mut requirements = [None; MAX_REQUIRED_SPENDERS];
        requirements[0] = Some(PmSpenderRequirement::new(
            chain,
            exchange,
            PmSpenderDomain::Standard,
            PmAssetId::collateral(parse_address(PUSD)),
        ));
        requirements[1] = Some(PmSpenderRequirement::new(
            chain,
            exchange,
            PmSpenderDomain::Standard,
            PmAssetId::outcome(parse_address(CONDITIONAL_TOKENS), token()),
        ));
        PmMarketMetadata::new(
            PmConditionId::parse(CONDITION).unwrap(),
            PmMarketId::parse(MARKET).unwrap(),
            PmOutcomeMetadata::new(token(), PmOutcomeLabel::new("Yes").unwrap()),
            PmMarketLifecycle::new(true, false, false, true, true),
            PmTick::parse_decimal("0.01").unwrap(),
            PmQuantity::parse_decimal("5").unwrap(),
            false,
            chain,
            exchange,
            requirements,
            2,
        )
        .unwrap()
    }

    fn join(
        expected: PmMarketMetadata,
        lifecycle: PmLifecycleMetadata,
        clob: &PmClobMetadata,
    ) -> Result<PmAuthoritativeMetadata, PmMetadataJoinError> {
        PmAuthoritativeMetadata::join(
            instrument(),
            source(),
            expected,
            lifecycle,
            clob,
            PmMetadataRevisionInput::new(SnapshotRevision::new(1), 10).unwrap(),
        )
    }

    #[test]
    fn atomic_join_produces_stable_full_and_domain_fingerprints() {
        let expected = metadata(false);
        let first = join(
            expected,
            lifecycle(true, false, false, true, true),
            &clob("0.01", "5", false, "Yes"),
        )
        .unwrap();
        let second = join(
            expected,
            lifecycle(true, false, false, true, true),
            &clob("0.01", "5", false, "Yes"),
        )
        .unwrap();

        assert_eq!(first, second);
        assert_ne!(first.metadata_fingerprint(), [0; 32]);
        assert_ne!(first.domain_fingerprint(), [0; 32]);
        assert_eq!(first.event().metadata(), expected);
        assert_eq!(first.parser_config().scope(), scope());
        assert_eq!(first.monotonic_receive_ns(), 10);
    }

    #[test]
    fn lifecycle_and_clob_drift_are_typed_and_fail_closed() {
        let expected = metadata(false);
        for (observed, error) in [
            (
                lifecycle(false, false, false, true, true),
                PmMetadataJoinError::Inactive,
            ),
            (
                lifecycle(true, true, false, true, true),
                PmMetadataJoinError::Closed,
            ),
            (
                lifecycle(true, false, true, true, true),
                PmMetadataJoinError::Archived,
            ),
            (
                lifecycle(true, false, false, false, true),
                PmMetadataJoinError::NotAcceptingOrders,
            ),
            (
                lifecycle(true, false, false, true, false),
                PmMetadataJoinError::OrderBookDisabled,
            ),
        ] {
            assert_eq!(
                join(expected, observed, &clob("0.01", "5", false, "Yes"))
                    .unwrap_err()
                    .to_string(),
                error.to_string()
            );
        }

        for (observed, expected_error) in [
            (clob("0.001", "5", false, "Yes"), "market tick"),
            (clob("0.01", "6", false, "Yes"), "minimum order"),
            (clob("0.01", "5", true, "Yes"), "negative-risk"),
            (clob("0.01", "5", false, "UP"), "outcome token or label"),
        ] {
            assert!(
                join(
                    expected,
                    lifecycle(true, false, false, true, true),
                    &observed
                )
                .unwrap_err()
                .to_string()
                .contains(expected_error)
            );
        }
    }

    #[test]
    fn fixed_domain_and_revision_inputs_are_checked() {
        assert!(matches!(
            PmMetadataRevisionInput::new(SnapshotRevision::new(0), 1),
            Err(PmMetadataJoinError::ZeroRevision)
        ));
        assert!(matches!(
            PmMetadataRevisionInput::new(SnapshotRevision::new(1), 0),
            Err(PmMetadataJoinError::ZeroReceiveTime)
        ));

        let mut wrong_spenders = [None; MAX_REQUIRED_SPENDERS];
        let base = metadata(false);
        wrong_spenders[0] = base.required_spenders().next();
        let wrong = PmMarketMetadata::new(
            base.condition(),
            base.market(),
            base.outcome(),
            base.lifecycle(),
            base.tick(),
            base.minimum_order_size(),
            false,
            base.chain(),
            base.exchange(),
            wrong_spenders,
            1,
        )
        .unwrap();
        assert!(matches!(
            join(
                wrong,
                lifecycle(true, false, false, true, true),
                &clob("0.01", "5", false, "Yes")
            ),
            Err(PmMetadataJoinError::UnsupportedSpenderSet)
        ));
    }

    #[test]
    fn recorded_authority_revalidates_lifecycle_domain_and_both_fingerprints() {
        let expected = metadata(false);
        let live = join(
            expected,
            lifecycle(true, false, false, true, true),
            &clob("0.01", "5", false, "Yes"),
        )
        .unwrap();
        let revision = PmMetadataRevisionInput::new(SnapshotRevision::new(1), 10).unwrap();

        let replay = PmAuthoritativeMetadata::verify_recorded(
            instrument(),
            source(),
            expected,
            revision,
            live.metadata_fingerprint(),
            live.domain_fingerprint(),
        )
        .unwrap();
        assert_eq!(replay, live);

        assert!(matches!(
            PmAuthoritativeMetadata::verify_recorded(
                instrument(),
                source(),
                metadata_with_lifecycle(PmMarketLifecycle::new(false, false, false, true, true)),
                revision,
                live.metadata_fingerprint(),
                live.domain_fingerprint(),
            ),
            Err(PmMetadataJoinError::Inactive)
        ));
        assert!(matches!(
            PmAuthoritativeMetadata::verify_recorded(
                instrument(),
                source(),
                metadata_on_unsupported_chain(),
                revision,
                live.metadata_fingerprint(),
                live.domain_fingerprint(),
            ),
            Err(PmMetadataJoinError::UnsupportedChain)
        ));
        assert!(matches!(
            PmAuthoritativeMetadata::verify_recorded(
                instrument(),
                source(),
                expected,
                revision,
                [0; 32],
                live.domain_fingerprint(),
            ),
            Err(PmMetadataJoinError::MetadataFingerprintMismatch)
        ));
        assert!(matches!(
            PmAuthoritativeMetadata::verify_recorded(
                instrument(),
                source(),
                expected,
                revision,
                live.metadata_fingerprint(),
                [0; 32],
            ),
            Err(PmMetadataJoinError::DomainFingerprintMismatch)
        ));
    }
}
